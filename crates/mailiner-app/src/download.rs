//! Attachment download: stream wire chunks → TE decode → progressive Blob parts.

use mailiner_core::models::TransferEncoding;
use mailiner_mime::{StreamingTransferDecoder, MAX_BINARY_DECODE_BYTES};

/// Hard cap for attachment downloads (decoded size). Larger than cid image cap.
pub const MAX_DOWNLOAD_BYTES: usize = 100 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum DownloadStatus {
    Idle,
    InProgress {
        /// Transfer-encoded octets received so far (matches BODYSTRUCTURE size).
        received: u64,
        total: Option<u64>,
    },
    Finished,
    Error(String),
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
    let ext = match content_type.split(';').next().unwrap_or("").trim() {
        "application/pdf" => "pdf",
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "text/plain" => "txt",
        "message/rfc822" => "eml",
        _ => "bin",
    };
    format!("attachment.{ext}")
}

// Re-export for callers that check content-part caps.
#[allow(dead_code)]
pub fn content_binary_cap() -> usize {
    MAX_BINARY_DECODE_BYTES
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

    /// Finish TE decode and trigger the browser download from assembled Blob parts.
    pub fn finish_and_save(mut self) -> Result<(), String> {
        let decoder = self
            .decoder
            .take()
            .ok_or_else(|| "download already finished".to_string())?;
        let tail = decoder.finish().map_err(|e| e.to_string())?;
        self.append_decoded(&tail)?;
        self.save()
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

    fn save(self) -> Result<(), String> {
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsCast;
            use web_sys::{Blob, BlobPropertyBag, HtmlAnchorElement, Url};

            let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
            let document = window.document().ok_or_else(|| "no document".to_string())?;

            let props = BlobPropertyBag::new();
            let ct = if self.content_type.is_empty() {
                "application/octet-stream"
            } else {
                self.content_type.as_str()
            };
            props.set_type(ct);

            let blob = Blob::new_with_u8_array_sequence_and_options(&self.parts, &props)
                .map_err(|e| format!("blob: {e:?}"))?;
            let url = Url::create_object_url_with_blob(&blob)
                .map_err(|e| format!("object url: {e:?}"))?;

            let a = document
                .create_element("a")
                .map_err(|e| format!("create a: {e:?}"))?
                .dyn_into::<HtmlAnchorElement>()
                .map_err(|_| "not an anchor".to_string())?;
            a.set_href(&url);
            a.set_download(&self.filename);
            let body = document.body().ok_or_else(|| "no body".to_string())?;
            let _ = body.append_child(&a);
            a.click();
            let _ = body.remove_child(&a);
            let _ = Url::revoke_object_url(&url);
            Ok(())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = self.filename;
            let _ = self.content_type;
            let _ = self.parts;
            Ok(())
        }
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
}
