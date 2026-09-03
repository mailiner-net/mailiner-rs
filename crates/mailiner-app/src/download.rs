//! Attachment download: stream wire chunks → TE decode → progressive Blob parts.

use mailiner_core::models::TransferEncoding;
use mailiner_mime::{MAX_BINARY_DECODE_BYTES, StreamingTransferDecoder};

/// Hard cap for attachment downloads (decoded size). Larger than cid image cap.
pub const MAX_DOWNLOAD_BYTES: usize = 100 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum DownloadStatus {
    Idle,
    /// Enqueued by Save all; IMAP fetch has not started.
    Queued,
    InProgress {
        /// Transfer-encoded octets received so far (matches BODYSTRUCTURE size).
        received: u64,
        total: Option<u64>,
    },
    Finished,
    Error(String),
}

impl DownloadStatus {
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Queued | Self::InProgress { .. })
    }
}

impl Default for DownloadStatus {
    fn default() -> Self {
        Self::Idle
    }
}

/// Human-readable size (B / KiB / MiB).
pub fn size_to_human(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= MIB {
        format!("{:.1} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.1} KiB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}

/// Download-status key for the full-message `.eml` export.
pub const EML_DOWNLOAD_KEY: &str = "EML";

/// Max length of the filename stem (before `.eml`).
const MAX_EML_STEM: usize = 120;

/// Type/subtype of a Content-Type header, without parameters.
pub fn primary_mime(content_type: &str) -> &str {
    content_type.split(';').next().unwrap_or("").trim()
}

/// How an attachment can be shown inline (never HTML/SVG — XSS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewKind {
    Image,
    Pdf,
}

/// Safe image types and PDF. SVG/HTML are excluded even when labeled `image/*`.
pub fn preview_kind(content_type: &str) -> Option<PreviewKind> {
    let mime = primary_mime(content_type);
    if mime.eq_ignore_ascii_case("image/png")
        || mime.eq_ignore_ascii_case("image/jpeg")
        || mime.eq_ignore_ascii_case("image/jpg")
        || mime.eq_ignore_ascii_case("image/gif")
        || mime.eq_ignore_ascii_case("image/webp")
    {
        Some(PreviewKind::Image)
    } else if mime.eq_ignore_ascii_case("application/pdf") {
        Some(PreviewKind::Pdf)
    } else {
        None
    }
}

pub fn is_previewable_content_type(content_type: &str) -> bool {
    preview_kind(content_type).is_some()
}

/// MIME type to stamp on the Blob (no parameters; `image/jpg` → `image/jpeg`).
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
fn blob_mime_type(content_type: &str) -> &str {
    let mime = primary_mime(content_type);
    if mime.is_empty() {
        return "application/octet-stream";
    }
    if mime.eq_ignore_ascii_case("image/jpg") {
        return "image/jpeg";
    }
    mime
}

/// Suggested filename for an attachment part.
pub fn attachment_filename(
    filename: &Option<String>,
    description: &Option<String>,
    content_type: &str,
) -> String {
    if let Some(f) = filename {
        if !f.is_empty() {
            return f.clone();
        }
    }
    if let Some(d) = description {
        if !d.is_empty() {
            return d.clone();
        }
    }
    let ext = match primary_mime(content_type).to_ascii_lowercase().as_str() {
        "application/pdf" => "pdf",
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "text/plain" => "txt",
        "message/rfc822" => "eml",
        _ => "bin",
    };
    format!("attachment.{ext}")
}

/// Suggested `.eml` filename from a message subject.
///
/// Strips characters that are unsafe in download filenames. Empty or
/// unusable subjects fall back to `message.eml`.
pub fn eml_filename(subject: &str) -> String {
    let trimmed = subject.trim();
    let without_ext = if trimmed.len() >= 4 && trimmed.to_ascii_lowercase().ends_with(".eml") {
        &trimmed[..trimmed.len() - 4]
    } else {
        trimmed
    };
    let stem = sanitize_filename_stem(without_ext);
    if stem.is_empty() {
        "message.eml".into()
    } else {
        format!("{stem}.eml")
    }
}

fn sanitize_filename_stem(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(MAX_EML_STEM));
    let mut last_was_space = false;
    for ch in raw.chars() {
        if ch.is_whitespace() {
            if !last_was_space && !out.is_empty() && out.len() + 1 <= MAX_EML_STEM {
                out.push(' ');
                last_was_space = true;
            }
            continue;
        }
        if ch.is_control()
            || matches!(
                ch,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '\0'
            )
        {
            continue;
        }
        let add = ch.len_utf8();
        if out.len() + add > MAX_EML_STEM {
            break;
        }
        last_was_space = false;
        out.push(ch);
    }
    out.trim().trim_matches('.').trim().to_string()
}

/// Revoke a blob: object URL. Safe to call twice or with an empty string.
pub fn revoke_object_url(url: &str) {
    if url.is_empty() {
        return;
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = web_sys::Url::revoke_object_url(url);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = url;
    }
}

// Re-export for callers that check content-part caps.
#[allow(dead_code)]
pub fn content_binary_cap() -> usize {
    MAX_BINARY_DECODE_BYTES
}

/// Trigger a browser download of `text` (JSON export, etc.).
pub fn save_text_download(filename: &str, mime: &str, text: &str) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        use web_sys::{Blob, BlobPropertyBag, HtmlAnchorElement, Url};

        let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
        let document = window.document().ok_or_else(|| "no document".to_string())?;

        let props = BlobPropertyBag::new();
        let ct = if mime.is_empty() {
            "application/octet-stream"
        } else {
            mime
        };
        props.set_type(ct);

        let bytes = text.as_bytes();
        let u8 = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
        u8.copy_from(bytes);
        let parts = js_sys::Array::new();
        parts.push(&u8);
        let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &props)
            .map_err(|e| format!("blob: {e:?}"))?;
        let url =
            Url::create_object_url_with_blob(&blob).map_err(|e| format!("object url: {e:?}"))?;

        let a = document
            .create_element("a")
            .map_err(|e| format!("create a: {e:?}"))?
            .dyn_into::<HtmlAnchorElement>()
            .map_err(|_| "not an anchor".to_string())?;
        a.set_href(&url);
        a.set_download(filename);
        let body = document.body().ok_or_else(|| "no body".to_string())?;
        let _ = body.append_child(&a);
        a.click();
        let _ = body.remove_child(&a);
        let _ = Url::revoke_object_url(&url);
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (filename, mime, text);
        Ok(())
    }
}

/// Accumulates decoded octets as discrete Blob parts.
///
/// Wire chunks are TE-decoded incrementally; each decoded slice is pushed into the
/// sink immediately so we never hold a second full-size contiguous `Vec<u8>` of the
/// whole file for TE+decoded dual buffering.
///
/// On `wasm32`, parts live in a JS `Array` of `Uint8Array` and are assembled into a
/// `Blob` at the end. On native (unit tests), parts are kept as `Vec<Vec<u8>>`.
pub struct StreamingBlobDownload {
    decoder: Option<StreamingTransferDecoder>,
    content_type: String,
    filename: String,
    /// Transfer-encoded bytes seen (progress).
    pub wire_received: u64,
    /// Decoded bytes committed to the sink.
    pub decoded_len: usize,
    #[cfg(target_arch = "wasm32")]
    parts: js_sys::Array,
    #[cfg(not(target_arch = "wasm32"))]
    parts: Vec<Vec<u8>>,
}

impl StreamingBlobDownload {
    pub fn new(encoding: TransferEncoding, filename: String, content_type: String) -> Self {
        Self {
            decoder: Some(StreamingTransferDecoder::new(encoding)),
            content_type,
            filename,
            wire_received: 0,
            decoded_len: 0,
            #[cfg(target_arch = "wasm32")]
            parts: js_sys::Array::new(),
            #[cfg(not(target_arch = "wasm32"))]
            parts: Vec::new(),
        }
    }

    /// Feed one transfer-encoded wire chunk; decoded bytes are appended to the Blob.
    pub fn push_wire_chunk(&mut self, wire: &[u8]) -> Result<(), String> {
        self.wire_received = self.wire_received.saturating_add(wire.len() as u64);
        if self.wire_received as usize > MAX_DOWNLOAD_BYTES {
            return Err(format!(
                "download exceeded limit ({} bytes)",
                MAX_DOWNLOAD_BYTES
            ));
        }

        let decoded = self
            .decoder
            .as_mut()
            .ok_or_else(|| "download already finished".to_string())?
            .push(wire)
            .map_err(|e| e.to_string())?;
        self.append_decoded(&decoded)
    }

    /// Finish TE decode and assemble a Blob object URL (caller must revoke).
    pub fn finish(mut self) -> Result<FinishedAttachment, String> {
        let decoder = self
            .decoder
            .take()
            .ok_or_else(|| "download already finished".to_string())?;
        let tail = decoder.finish().map_err(|e| e.to_string())?;
        self.append_decoded(&tail)?;
        self.into_finished()
    }

    /// Finish TE decode and trigger the browser download from assembled Blob parts.
    pub fn finish_and_save(self) -> Result<(), String> {
        let finished = self.finish()?;
        let result = finished.trigger_save();
        finished.revoke();
        result
    }

    fn into_finished(self) -> Result<FinishedAttachment, String> {
        #[cfg(target_arch = "wasm32")]
        {
            let object_url = create_object_url(&self.parts, &self.content_type)?;
            Ok(FinishedAttachment {
                object_url,
                filename: self.filename,
                content_type: self.content_type,
            })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = self.parts;
            Ok(FinishedAttachment {
                object_url: String::new(),
                filename: self.filename,
                content_type: self.content_type,
            })
        }
    }

    fn append_decoded(&mut self, data: &[u8]) -> Result<(), String> {
        if data.is_empty() {
            return Ok(());
        }
        self.decoded_len = self.decoded_len.saturating_add(data.len());
        if self.decoded_len > MAX_DOWNLOAD_BYTES {
            return Err(format!(
                "decoded attachment exceeds limit ({} bytes)",
                self.decoded_len
            ));
        }

        #[cfg(target_arch = "wasm32")]
        {
            use js_sys::Uint8Array;
            let u8 = Uint8Array::new_with_length(data.len() as u32);
            u8.copy_from(data);
            self.parts.push(&u8);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.parts.push(data.to_vec());
        }
        Ok(())
    }

    /// Test helper: finish decode and return concatenated decoded bytes.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub fn finish_to_bytes(mut self) -> Result<Vec<u8>, String> {
        let decoder = self
            .decoder
            .take()
            .ok_or_else(|| "download already finished".to_string())?;
        let tail = decoder.finish().map_err(|e| e.to_string())?;
        self.append_decoded(&tail)?;
        Ok(self.parts.into_iter().flatten().collect())
    }
}

/// Assembled attachment Blob, held as an object URL until [`Self::revoke`].
pub struct FinishedAttachment {
    pub object_url: String,
    pub filename: String,
    pub content_type: String,
}

impl FinishedAttachment {
    /// Trigger a browser file save without revoking the object URL.
    pub fn trigger_save(&self) -> Result<(), String> {
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsCast;
            use web_sys::HtmlAnchorElement;

            if self.object_url.is_empty() {
                return Err("no object url".into());
            }
            let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
            let document = window.document().ok_or_else(|| "no document".to_string())?;
            let a = document
                .create_element("a")
                .map_err(|e| format!("create a: {e:?}"))?
                .dyn_into::<HtmlAnchorElement>()
                .map_err(|_| "not an anchor".to_string())?;
            a.set_href(&self.object_url);
            a.set_download(&self.filename);
            let body = document.body().ok_or_else(|| "no body".to_string())?;
            let _ = body.append_child(&a);
            a.click();
            let _ = body.remove_child(&a);
            Ok(())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (&self.object_url, &self.filename, &self.content_type);
            Ok(())
        }
    }

    pub fn revoke(&self) {
        revoke_object_url(&self.object_url);
    }
}

#[cfg(target_arch = "wasm32")]
fn create_object_url(parts: &js_sys::Array, content_type: &str) -> Result<String, String> {
    use web_sys::{Blob, BlobPropertyBag, Url};

    let props = BlobPropertyBag::new();
    props.set_type(blob_mime_type(content_type));
    let blob = Blob::new_with_u8_array_sequence_and_options(parts, &props)
        .map_err(|e| format!("blob: {e:?}"))?;
    Url::create_object_url_with_blob(&blob).map_err(|e| format!("object url: {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailiner_core::models::TransferEncoding;

    #[test]
    fn size_human() {
        assert_eq!(size_to_human(500), "500 B");
        assert!(size_to_human(2048).contains("KiB"));
        assert!(size_to_human(2 * 1024 * 1024).contains("MiB"));
    }

    #[test]
    fn stream_decode_base64_to_blob_parts() {
        // Wire fixture matches MockConnector section "2" (`UERGRGF0YQ==` → "PDFData").
        let mut dl = StreamingBlobDownload::new(
            TransferEncoding::Base64,
            "report.pdf".into(),
            "application/pdf".into(),
        );
        // Split awkwardly across quartet boundary.
        dl.push_wire_chunk(b"UERGRG").unwrap();
        dl.push_wire_chunk(b"F0YQ==").unwrap();
        let out = dl.finish_to_bytes().unwrap();
        assert_eq!(out, b"PDFData");
    }

    #[test]
    fn stream_decode_identity_chunks() {
        let mut dl = StreamingBlobDownload::new(
            TransferEncoding::SevenBit,
            "a.txt".into(),
            "text/plain".into(),
        );
        dl.push_wire_chunk(b"hello ").unwrap();
        dl.push_wire_chunk(b"world").unwrap();
        assert_eq!(dl.wire_received, 11);
        let out = dl.finish_to_bytes().unwrap();
        assert_eq!(out, b"hello world");
    }

    #[test]
    fn save_text_download_host_is_ok() {
        save_text_download("mailiner-accounts.json", "application/json", "{}").unwrap();
    }

    #[test]
    fn filename_fallback() {
        assert_eq!(
            attachment_filename(&Some("a.pdf".into()), &None, "application/pdf"),
            "a.pdf"
        );
        assert_eq!(
            attachment_filename(&None, &None, "application/pdf"),
            "attachment.pdf"
        );
    }

    #[test]
    fn eml_filename_fallback_when_empty() {
        assert_eq!(eml_filename(""), "message.eml");
        assert_eq!(eml_filename("   "), "message.eml");
        assert_eq!(eml_filename("..."), "message.eml");
        assert_eq!(eml_filename("///"), "message.eml");
        assert_eq!(eml_filename(".eml"), "message.eml");
    }

    #[test]
    fn eml_filename_uses_sanitized_subject() {
        assert_eq!(eml_filename("Hello World"), "Hello World.eml");
        assert_eq!(eml_filename("  Re: Hello / World?  "), "Re Hello World.eml");
        assert_eq!(eml_filename("foo/bar:baz*qux"), "foobarbazqux.eml");
        assert_eq!(eml_filename("hello.eml"), "hello.eml");
        assert_eq!(eml_filename("a\\b|c\"d"), "abcd.eml");
    }

    #[test]
    fn eml_filename_strips_controls_and_caps_length() {
        assert_eq!(eml_filename("hi\nthere\tnow"), "hi there now.eml");
        let long = "x".repeat(300);
        let name = eml_filename(&long);
        assert!(name.ends_with(".eml"));
        assert_eq!(name.len(), MAX_EML_STEM + 4);
        assert!(
            name.chars()
                .all(|c| c == 'x' || c == '.' || c == 'e' || c == 'm' || c == 'l')
        );
        let wide = "é".repeat(200);
        let wide_name = eml_filename(&wide);
        assert!(wide_name.ends_with(".eml"));
        assert!(wide_name.len() <= MAX_EML_STEM + 4);
    }

    #[test]
    fn eml_filename_blocks_path_traversal() {
        assert_eq!(eml_filename("../../etc/passwd"), "etcpasswd.eml");
        assert_eq!(eml_filename("..\\..\\secret"), "secret.eml");
    }

    #[test]
    fn rejects_oversize_wire() {
        let mut dl = StreamingBlobDownload::new(
            TransferEncoding::Binary,
            "big.bin".into(),
            "application/octet-stream".into(),
        );
        dl.wire_received = MAX_DOWNLOAD_BYTES as u64;
        let err = dl.push_wire_chunk(b"x").unwrap_err();
        assert!(err.contains("limit"));
    }

    #[test]
    fn preview_allowlist_images_and_pdf() {
        for mime in [
            "image/png",
            "image/jpeg",
            "image/jpg",
            "image/gif",
            "image/webp",
            "application/pdf",
        ] {
            assert!(
                is_previewable_content_type(mime),
                "expected previewable: {mime}"
            );
        }
        assert_eq!(preview_kind("image/png"), Some(PreviewKind::Image));
        assert_eq!(preview_kind("application/pdf"), Some(PreviewKind::Pdf));
    }

    #[test]
    fn preview_allowlist_ignores_params_and_case() {
        assert_eq!(
            preview_kind("IMAGE/JPEG; name=\"Photo.JPG\""),
            Some(PreviewKind::Image)
        );
        assert_eq!(
            preview_kind("Application/PDF; name=report.pdf"),
            Some(PreviewKind::Pdf)
        );
        assert!(is_previewable_content_type(
            "image/webp; charset=binary; name=x.webp"
        ));
    }

    #[test]
    fn preview_allowlist_rejects_html_svg_and_other() {
        for mime in [
            "image/svg+xml",
            "image/svg+xml; charset=utf-8",
            "text/html",
            "text/html; charset=utf-8",
            "application/xhtml+xml",
            "text/plain",
            "application/octet-stream",
            "image/bmp",
            "image/tiff",
            "application/javascript",
            "",
        ] {
            assert!(
                !is_previewable_content_type(mime),
                "expected not previewable: {mime}"
            );
            assert_eq!(preview_kind(mime), None);
        }
    }

    #[test]
    fn blob_mime_normalizes_jpg_and_empty() {
        assert_eq!(blob_mime_type("image/jpg"), "image/jpeg");
        assert_eq!(blob_mime_type("IMAGE/JPG; name=a.jpg"), "image/jpeg");
        assert_eq!(blob_mime_type(""), "application/octet-stream");
        assert_eq!(
            blob_mime_type("application/pdf; name=x.pdf"),
            "application/pdf"
        );
    }
}
