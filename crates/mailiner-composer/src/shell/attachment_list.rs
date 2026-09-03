//! Non-UI helpers for compose attachments (size labels, MIME guess).

use crate::model::convert::plain_to_html;
use crate::model::{
    caps, AttachmentData, AttachmentId, DraftDocument, FileAttachment, InlineId, InlineImage,
};
use crate::sanitize::is_safe_image_content_type;

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
        let sz = match &a.data {
            AttachmentData::Bytes(b) => b.len() as u64,
            AttachmentData::Pending => 0,
        };
        total = total.saturating_add(sz);
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

/// Filename extension for a safe raster image type (`bin` if unknown).
pub fn image_extension(content_type: &str) -> &'static str {
    let ct = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match ct.as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        _ => "bin",
    }
}

/// Filename for a pasted/inserted image when the browser name is empty.
pub fn image_filename(name: &str, content_type: &str, index: usize) -> String {
    let name = name.trim();
    if !name.is_empty() {
        return name.to_string();
    }
    let ext = image_extension(content_type);
    if index == 0 {
        format!("pasted-image.{ext}")
    } else {
        format!("pasted-image-{}.{ext}", index + 1)
    }
}

/// Buffered [`InlineImage`] with a fresh id and `Content-ID` `img-{uuid}@mailiner`.
pub fn inline_image(
    filename: Option<String>,
    content_type: impl Into<String>,
    data: Vec<u8>,
) -> InlineImage {
    let id = InlineId::new();
    InlineImage {
        content_id: format!("img-{}@mailiner", id.0),
        id,
        content_type: content_type.into(),
        filename,
        data: AttachmentData::Bytes(data),
        edit_url: None,
    }
}

/// HTML fragment for a plain draft that also carries CID images.
///
/// The typed body is converted with [`plain_to_html`]; each image is appended
/// as `<img src="cid:…">` so export can wrap them in `multipart/related`.
pub fn html_for_plain_with_inlines(plain: &str, images: &[InlineImage]) -> String {
    if images.is_empty() {
        return String::new();
    }
    let mut html = plain_to_html(plain);
    for img in images {
        html.push_str("<p><img src=\"cid:");
        html.push_str(&escape_html_attr(&img.content_id));
        html.push_str("\" alt=\"");
        let alt = img.filename.as_deref().unwrap_or("image");
        html.push_str(&escape_html_attr(alt));
        html.push_str("\"></p>");
    }
    html
}

fn escape_html_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// True when `content_type` or `filename` looks like a paste/insert raster image.
pub fn looks_like_inline_image(filename: &str, content_type: Option<&str>) -> bool {
    if content_type.is_some_and(is_safe_image_content_type) {
        return true;
    }
    is_safe_image_content_type(&guess_content_type(filename))
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

    #[test]
    fn draft_payload_counts_bytes_not_declared_size() {
        let id = crate::identity::FromIdentity::new("Me", "me@example.com");
        let mut d = DraftDocument::new_empty(&id);
        d.attachments.push(FileAttachment {
            id: AttachmentId::new(),
            filename: "a.bin".into(),
            content_type: "application/octet-stream".into(),
            size: 999,
            data: AttachmentData::Bytes(vec![1, 2, 3]),
        });
        d.attachments.push(FileAttachment {
            id: AttachmentId::new(),
            filename: "p.bin".into(),
            content_type: "application/octet-stream".into(),
            size: 1000,
            data: AttachmentData::Pending,
        });
        assert_eq!(draft_payload_bytes(&d), 3);
    }

    #[test]
    fn draft_payload_counts_inline_images() {
        let id = crate::identity::FromIdentity::new("Me", "me@example.com");
        let mut d = DraftDocument::new_empty(&id);
        d.inline_images.push(inline_image(
            Some("dot.png".into()),
            "image/png",
            vec![0; 7],
        ));
        assert_eq!(draft_payload_bytes(&d), 7);
    }

    #[test]
    fn image_filename_keeps_browser_name() {
        assert_eq!(image_filename("shot.png", "image/png", 0), "shot.png");
        assert_eq!(image_filename("  ", "image/png", 0), "pasted-image.png");
        assert_eq!(image_filename("", "image/jpeg", 1), "pasted-image-2.jpg");
        assert_eq!(image_extension("image/webp"), "webp");
    }

    #[test]
    fn looks_like_inline_image_from_type_or_name() {
        assert!(looks_like_inline_image("x.bin", Some("image/png")));
        assert!(looks_like_inline_image("photo.JPG", None));
        assert!(!looks_like_inline_image("notes.txt", Some("text/plain")));
        assert!(!looks_like_inline_image("icon.svg", Some("image/svg+xml")));
    }

    #[test]
    fn inline_image_sets_cid_and_bytes() {
        let img = inline_image(Some("dot.png".into()), "image/png", b"PNG".to_vec());
        assert!(img.content_id.starts_with("img-"));
        assert!(img.content_id.ends_with("@mailiner"));
        assert!(!img.content_id.contains('<'));
        assert_eq!(img.filename.as_deref(), Some("dot.png"));
        assert!(matches!(img.data, AttachmentData::Bytes(ref b) if b == b"PNG"));
    }

    #[test]
    fn html_for_plain_with_inlines_appends_cid_imgs() {
        let img = InlineImage {
            id: InlineId::new(),
            content_id: "pic@mailiner".into(),
            content_type: "image/png".into(),
            filename: Some("a\"b.png".into()),
            data: AttachmentData::Bytes(b"PNG".to_vec()),
            edit_url: None,
        };
        let html = html_for_plain_with_inlines("hello <x>", &[img]);
        assert!(html.contains("<p>hello &lt;x&gt;</p>"), "{html}");
        assert!(html.contains("src=\"cid:pic@mailiner\""), "{html}");
        assert!(html.contains("alt=\"a&quot;b.png\""), "{html}");
        assert!(html_for_plain_with_inlines("hello", &[]).is_empty());
    }
}
