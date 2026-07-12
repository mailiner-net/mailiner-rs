//! Attachment download helpers (progress + browser save).

use mailiner_core::models::TransferEncoding;
use mailiner_mime::{decode_part_content, DecodeError, MAX_BINARY_DECODE_BYTES};

/// Hard cap for attachment downloads (decoded size). Larger than cid image cap.
pub const MAX_DOWNLOAD_BYTES: usize = 100 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum DownloadStatus {
    Idle,
    InProgress {
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

/// Decode transfer-encoded attachment bytes with download size cap.
pub fn decode_attachment(
    raw: &[u8],
    encoding: TransferEncoding,
    content_type: &str,
) -> Result<Vec<u8>, String> {
    if raw.len() > MAX_DOWNLOAD_BYTES {
        return Err(format!(
            "attachment exceeds download limit ({} bytes)",
            raw.len()
        ));
    }
    match decode_part_content(raw, encoding, content_type, None) {
        Ok(mailiner_core::MessageContent::Binary(b)) => {
            if b.len() > MAX_DOWNLOAD_BYTES {
                return Err(format!(
                    "decoded attachment exceeds limit ({} bytes)",
                    b.len()
                ));
            }
            Ok(b)
        }
        Ok(mailiner_core::MessageContent::Text(t)) => {
            let b = t.into_bytes();
            if b.len() > MAX_DOWNLOAD_BYTES {
                return Err(format!(
                    "decoded attachment exceeds limit ({} bytes)",
                    b.len()
                ));
            }
            Ok(b)
        }
        Ok(mailiner_core::MessageContent::Empty) => Ok(Vec::new()),
        Err(DecodeError::TooLarge(n)) => Err(format!("decoded payload too large ({n} bytes)")),
        Err(e) => Err(e.to_string()),
    }
}

/// Trigger a browser file download via object URL. No-op outside web feature.
pub fn save_bytes_to_disk(filename: &str, content_type: &str, data: &[u8]) -> Result<(), String> {
    #[cfg(feature = "web")]
    {
        use js_sys::{Array, Uint8Array};
        use wasm_bindgen::JsCast;
        use web_sys::{Blob, BlobPropertyBag, HtmlAnchorElement, Url};

        let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
        let document = window.document().ok_or_else(|| "no document".to_string())?;

        let u8 = Uint8Array::new_with_length(data.len() as u32);
        u8.copy_from(data);
        let parts = Array::new();
        parts.push(&u8.buffer());

        let props = BlobPropertyBag::new();
        let ct = if content_type.is_empty() {
            "application/octet-stream"
        } else {
            content_type
        };
        props.set_type(ct);

        let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &props)
            .map_err(|e| format!("blob: {e:?}"))?;
        let url = Url::create_object_url_with_blob(&blob)
            .map_err(|e| format!("object url: {e:?}"))?;

        let a = document
            .create_element("a")
            .map_err(|e| format!("create a: {e:?}"))?
            .dyn_into::<HtmlAnchorElement>()
            .map_err(|_| "not an anchor".to_string())?;
        a.set_href(&url);
        a.set_download(filename);
        // Must be in document for some browsers.
        let body = document.body().ok_or_else(|| "no body".to_string())?;
        let _ = body.append_child(&a);
        a.click();
        let _ = body.remove_child(&a);
        let _ = Url::revoke_object_url(&url);
        Ok(())
    }
    #[cfg(not(feature = "web"))]
    {
        let _ = (filename, content_type, data);
        // Native tests: succeed without writing a file.
        Ok(())
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
    fn decode_base64_attachment() {
        // Wire fixture matches MockConnector section "2" (`UERGRGF0YQ==` → "PDFData").
        let raw = b"UERGRGF0YQ==";
        let out = decode_attachment(raw, TransferEncoding::Base64, "application/pdf").unwrap();
        assert_eq!(out, b"PDFData");
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
}
