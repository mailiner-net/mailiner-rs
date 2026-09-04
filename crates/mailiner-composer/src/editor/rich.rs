//! Shadow-hosted contenteditable body editor.
//!
//! When mounted, the contenteditable host must set `spellcheck` to [`super::SPELLCHECK`].
//! HTML is always passed through [`crate::sanitize::sanitize_for_edit`] before inject
//! and [`crate::sanitize::sanitize_for_export`] before the wire.

use crate::model::convert::{html_to_plain, plain_to_html};
use crate::model::{AttachmentData, InlineImage};
use crate::sanitize::{sanitize_for_edit, sanitize_for_export};
use crate::shell::attachment_list::html_for_plain_with_inlines;

/// Empty editor placeholder so the caret has a paragraph to type in.
pub const EMPTY_EDIT_HTML: &str = "<p><br></p>";

/// Sanitize `html` and rewrite `cid:` images to data URLs for the editor.
pub fn html_for_edit(html: &str, images: &[InlineImage]) -> String {
    let clean = sanitize_for_edit(html);
    let rewritten = rewrite_cids_to_data(&clean, images);
    if rewritten.trim().is_empty() {
        EMPTY_EDIT_HTML.to_string()
    } else {
        rewritten
    }
}

/// Build edit HTML from a plain-text body (mode switch / new rich draft).
pub fn html_for_edit_from_plain(plain: &str, images: &[InlineImage]) -> String {
    let mut html = plain_to_html(plain);
    if !images.is_empty() && !contains_cid_or_data_img(&html) {
        html = html_for_plain_with_inlines(plain, images);
    }
    html_for_edit(&html, images)
}

/// Sanitize HTML read back from the contenteditable (keep blob/data for later rewrite).
pub fn html_from_editor(html: &str) -> String {
    let clean = sanitize_for_edit(html);
    if clean.trim().is_empty() {
        String::new()
    } else {
        clean
    }
}

/// Rewrite edit-time blob/data image URLs back to `cid:` and sanitize for MIME.
pub fn html_for_export(html: &str, images: &[InlineImage]) -> String {
    let with_cid = rewrite_data_to_cid(html, images);
    sanitize_for_export(&with_cid)
}

/// Plain alternative of an HTML fragment (lossy).
pub fn plain_alternative(html: &str) -> String {
    html_to_plain(html)
}

/// `<img>` tag for inserting a draft inline into the editor.
pub fn editor_img_tag(image: &InlineImage) -> String {
    let src = data_url_for_image(image).unwrap_or_else(|| format!("cid:{}", image.content_id));
    let alt = image.filename.as_deref().unwrap_or("image");
    format!(
        "<p><img src=\"{}\" alt=\"{}\"></p>",
        escape_attr(&src),
        escape_attr(alt)
    )
}

/// data: URL for an inline raster, or the stored `edit_url`.
pub fn data_url_for_image(image: &InlineImage) -> Option<String> {
    if let Some(url) = image.edit_url.as_deref() {
        let lower = url.to_ascii_lowercase();
        if lower.starts_with("data:image/") || lower.starts_with("blob:") {
            return Some(url.to_string());
        }
    }
    match &image.data {
        AttachmentData::Bytes(bytes) if !bytes.is_empty() => {
            let ct = image
                .content_type
                .split(';')
                .next()
                .unwrap_or("image/png")
                .trim();
            Some(format!(
                "data:{ct};base64,{}",
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)
            ))
        }
        _ => None,
    }
}

fn contains_cid_or_data_img(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    lower.contains("cid:") || lower.contains("data:image") || lower.contains("blob:")
}

fn rewrite_cids_to_data(html: &str, images: &[InlineImage]) -> String {
    let mut out = html.to_string();
    for img in images {
        let Some(data) = data_url_for_image(img) else {
            continue;
        };
        let needle = format!("cid:{}", img.content_id);
        out = replace_ignore_ascii_case(&out, &needle, &data);
    }
    out
}

fn rewrite_data_to_cid(html: &str, images: &[InlineImage]) -> String {
    let mut out = html.to_string();
    for img in images {
        let cid = format!("cid:{}", img.content_id);
        if let Some(data) = data_url_for_image(img) {
            out = out.replace(&data, &cid);
        }
        if let Some(url) = img.edit_url.as_deref() {
            if url != cid {
                out = out.replace(url, &cid);
            }
        }
    }
    out
}

fn replace_ignore_ascii_case(haystack: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return haystack.to_string();
    }
    let lower_h = haystack.to_ascii_lowercase();
    let lower_n = needle.to_ascii_lowercase();
    let mut out = String::with_capacity(haystack.len());
    let mut i = 0;
    while let Some(rel) = lower_h[i..].find(&lower_n) {
        let at = i + rel;
        out.push_str(&haystack[i..at]);
        out.push_str(replacement);
        i = at + needle.len();
    }
    out.push_str(&haystack[i..]);
    out
}

fn escape_attr(s: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AttachmentData, InlineId, InlineImage};

    fn png_inline(cid: &str, bytes: &[u8]) -> InlineImage {
        InlineImage {
            id: InlineId::new(),
            content_id: cid.into(),
            content_type: "image/png".into(),
            filename: Some("dot.png".into()),
            data: AttachmentData::Bytes(bytes.to_vec()),
            edit_url: None,
        }
    }

    #[test]
    fn edit_sanitizes_and_keeps_text() {
        let html = html_for_edit("<p>Hi<script>alert(1)</script></p>", &[]);
        assert!(html.contains("Hi"), "{html}");
        assert!(!html.to_ascii_lowercase().contains("script"), "{html}");
        assert!(!html.contains("alert"), "{html}");
    }

    #[test]
    fn edit_rewrites_cid_to_data_and_export_restores() {
        let img = png_inline("pic@mailiner", &[0x89, b'P', b'N', b'G']);
        let edit = html_for_edit(
            r#"<p><img src="cid:pic@mailiner" alt="d"></p>"#,
            std::slice::from_ref(&img),
        );
        assert!(edit.contains("data:image/png;base64,"), "{edit}");
        assert!(!edit.contains("cid:pic@mailiner"), "{edit}");
        let exported = html_for_export(&edit, &[img]);
        assert!(exported.contains("cid:pic@mailiner"), "{exported}");
        assert!(!exported.contains("data:image"), "{exported}");
    }

    #[test]
    fn from_plain_roundtrips_text() {
        let html = html_for_edit_from_plain("Hello\n\nWorld", &[]);
        assert!(html.contains("Hello"), "{html}");
        assert!(html.contains("World"), "{html}");
        let plain = plain_alternative(&html);
        assert!(plain.contains("Hello"), "{plain}");
        assert!(plain.contains("World"), "{plain}");
        assert_eq!(plain.trim(), "Hello\n\nWorld");
    }

    #[test]
    fn empty_edit_uses_placeholder() {
        assert_eq!(html_for_edit("   ", &[]), EMPTY_EDIT_HTML);
        assert!(
            html_from_editor("<p></p>").is_empty() || html_from_editor("<p></p>").contains('p')
        );
    }

    #[test]
    fn editor_img_tag_escapes_alt() {
        let mut img = png_inline("x@mailiner", &[1, 2, 3]);
        img.filename = Some(r#"a"b.png"#.into());
        let tag = editor_img_tag(&img);
        assert!(tag.contains("alt=\"a&quot;b.png\""), "{tag}");
        assert!(tag.contains("data:image/png"), "{tag}");
    }
}
