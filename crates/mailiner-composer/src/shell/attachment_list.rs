//! Non-UI helpers for compose attachments (size labels, MIME guess).

use crate::model::{caps, AttachmentData, AttachmentId, DraftDocument, FileAttachment};

/// Human-readable size (B / KiB / MiB).
pub fn human_size(bytes: u64) -> String {
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

/// Guess a MIME type from a filename extension.
///
/// Unknown or missing extensions become `application/octet-stream`.
pub fn guess_content_type(filename: &str) -> String {
    let name = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e)
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "txt" => "text/plain",
        "html" | "htm" => "text/html",
        "csv" => "text/csv",
        "json" => "application/json",
        "xml" => "application/xml",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "tar" => "application/x-tar",
        "7z" => "application/x-7z-compressed",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "ics" => "text/calendar",
        "eml" => "message/rfc822",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Prefer a useful browser-reported type; otherwise guess from the filename.
pub fn resolve_content_type(filename: &str, reported: Option<&str>) -> String {
    if let Some(ct) = reported {
        let t = ct.trim();
        if !t.is_empty() && !t.eq_ignore_ascii_case("application/octet-stream") {
            return t.to_string();
        }
    }
    guess_content_type(filename)
}

/// Bytes already counted toward [`caps::MAX_DRAFT_BYTES`] (bodies + files + inlines).
pub fn draft_payload_bytes(draft: &DraftDocument) -> u64 {
    let mut total = draft.plain_body.len() as u64 + draft.html_body.len() as u64;
    for a in &draft.attachments {
        total = total.saturating_add(a.size);
    }
    for img in &draft.inline_images {
        let sz = match &img.data {
            AttachmentData::Bytes(b) => b.len() as u64,
            AttachmentData::Pending => 0,
        };
        total = total.saturating_add(sz);
    }
    total
}

/// True when `current + extra` would fail [`caps::MAX_DRAFT_BYTES`].
pub fn would_exceed_draft_cap(current: u64, extra: u64) -> bool {
    current.saturating_add(extra) > caps::MAX_DRAFT_BYTES
}

/// Buffered [`FileAttachment`] with a fresh id and `size` matching `data`.
pub fn file_attachment(
    filename: impl Into<String>,
    content_type: impl Into<String>,
    data: Vec<u8>,
) -> FileAttachment {
    let size = data.len() as u64;
    FileAttachment {
        id: AttachmentId::new(),
        filename: filename.into(),
        content_type: content_type.into(),
        size,
        data: AttachmentData::Bytes(data),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_size_units() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(500), "500 B");
        assert_eq!(human_size(2048), "2.0 KiB");
        assert_eq!(human_size(2 * 1024 * 1024), "2.0 MiB");
    }

    #[test]
    fn guess_from_extension() {
        assert_eq!(guess_content_type("a.pdf"), "application/pdf");
        assert_eq!(guess_content_type("photo.JPG"), "image/jpeg");
        assert_eq!(guess_content_type(r"C:\tmp\notes.txt"), "text/plain");
        assert_eq!(guess_content_type("noext"), "application/octet-stream");
        assert_eq!(
            guess_content_type("archive.unknownxyz"),
            "application/octet-stream"
        );
    }

    #[test]
    fn resolve_prefers_reported_unless_generic() {
        assert_eq!(
            resolve_content_type("a.bin", Some("application/pdf")),
            "application/pdf"
        );
        assert_eq!(
            resolve_content_type("a.pdf", Some("application/octet-stream")),
            "application/pdf"
        );
        assert_eq!(resolve_content_type("a.pdf", Some("  ")), "application/pdf");
        assert_eq!(resolve_content_type("a.pdf", None), "application/pdf");
    }

    #[test]
    fn file_attachment_sets_size_from_bytes() {
        let att = file_attachment("note.txt", "text/plain", b"hi".to_vec());
        assert_eq!(att.filename, "note.txt");
        assert_eq!(att.content_type, "text/plain");
        assert_eq!(att.size, 2);
        assert!(matches!(att.data, AttachmentData::Bytes(ref b) if b == b"hi"));
    }

    #[test]
    fn file_attachment_keeps_zero_byte_payload() {
        let att = file_attachment("empty.txt", "text/plain", Vec::new());
        assert_eq!(att.size, 0);
        assert!(matches!(att.data, AttachmentData::Bytes(ref b) if b.is_empty()));
    }

    #[test]
    fn draft_payload_includes_bodies_and_files() {
        let id = crate::identity::FromIdentity::new("Me", "me@example.com");
        let mut d = DraftDocument::new_empty(&id);
        d.plain_body = "hello".into();
        d.html_body = "<p>x</p>".into();
        d.attachments.push(file_attachment(
            "a.bin",
            "application/octet-stream",
            vec![0; 10],
        ));
        assert_eq!(draft_payload_bytes(&d), 5 + 8 + 10);
        assert!(!would_exceed_draft_cap(draft_payload_bytes(&d), 0));
        assert!(would_exceed_draft_cap(caps::MAX_DRAFT_BYTES, 1));
    }
}
