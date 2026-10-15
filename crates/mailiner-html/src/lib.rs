//! Shared HTML/CSS sanitization for Mailiner (viewer + composer).

mod css;
mod format_marker;

pub use css::sanitize_css;

/// Image MIME types allowed for data: / cid inline content.
pub const SAFE_IMAGE_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/jpg",
    "image/gif",
    "image/webp",
    "image/bmp",
];

/// `<input accept>` list matching [`SAFE_IMAGE_TYPES`] (no SVG).
pub const SAFE_IMAGE_ACCEPT: &str = "image/png,image/jpeg,image/gif,image/webp,image/bmp";

/// True if `content_type` is a raster image allowed for inline/paste.
pub fn is_safe_image_content_type(content_type: &str) -> bool {
    let ct = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    SAFE_IMAGE_TYPES.iter().any(|t| *t == ct)
}

/// True if `data:` URL is a safe raster image.
pub fn is_safe_data_image(url_lower: &str) -> bool {
    let u = url_lower.trim();
    if !u.starts_with("data:") {
        return false;
    }
    SAFE_IMAGE_TYPES
        .iter()
        .any(|t| u.starts_with(&format!("data:{t}")))
}

fn is_safe_img_src(
    value: &str,
    allow_blob: bool,
    allow_cid: bool,
    allow_data: bool,
    allow_remote: bool,
) -> bool {
    let v = value.trim();
    let lower = v.to_ascii_lowercase();
    if lower.starts_with("javascript:") || lower.starts_with("vbscript:") {
        return false;
    }
    if lower.starts_with("cid:") {
        return allow_cid;
    }
    if lower.starts_with("blob:") {
        return allow_blob;
    }
    if lower.starts_with("data:") {
        return allow_data && is_safe_data_image(&lower);
    }
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return allow_remote;
    }
    false
}

fn is_safe_href(value: &str) -> bool {
    let v = value.trim();
    let lower = v.to_ascii_lowercase();
    if lower.starts_with("javascript:") || lower.starts_with("vbscript:") {
        return false;
    }
    if lower.starts_with("data:") {
        return false;
    }
    // Protocol-relative URLs are remote; reject.
    if lower.starts_with("//") {
        return false;
    }
    lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:")
        || lower.starts_with('#')
        || (!lower.contains(':') && !lower.is_empty()) // path-relative
}

/// Tags allowed in compose edit/export HTML fragments.
fn allowed_tags() -> [&'static str; 28] {
    [
        "p",
        "br",
        "div",
        "span",
        "blockquote",
        "pre",
        "code",
        "ul",
        "ol",
        "li",
        "a",
        "b",
        "strong",
        "i",
        "em",
        "u",
        "img",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "hr",
        "table",
        "tr",
        "td",
        "th",
    ]
}

/// Sanitize HTML for contenteditable inject / DOM→draft sync / paste.
///
/// - No `style` attributes; no `<style>` tags
/// - `img[src]`: cid, blob, safe data: — **strip remote images**
/// - `a[href]`: http(s), mailto
/// - strip srcset and other URL attrs via ammonia defaults + filter
///
/// U+200B is kept, including the collapsed-caret marker inside `strong` /
/// `b` / `em` / `i` / `u`. Stripping it here would delete the live caret on
/// the next editor sync. [`sanitize_for_export`] removes that marker.
pub fn sanitize_for_edit(html: &str) -> String {
    sanitize_fragment(html, PolicyKind::Edit)
}

/// Sanitize HTML after blob/data→cid rewrite, before MIME.
///
/// `img[src]` allows **cid only** (plus safe data: residual rejected in v1 — cid only).
///
/// A `strong`/`b`/`em`/`i`/`u` element whose text is only U+200B is removed,
/// and a leading U+200B inside those tags is removed. That character is the
/// collapsed-caret marker. Other U+200B characters stay, so a pasted
/// word-break hint in ordinary text survives.
pub fn sanitize_for_export(html: &str) -> String {
    let mut out = sanitize_fragment(html, PolicyKind::Export);
    // Strip composer-internal markers.
    out = strip_data_mlnr(&out);
    format_marker::strip_collapsed_format_markers(out)
}

#[derive(Clone, Copy)]
enum PolicyKind {
    Edit,
    Export,
}

fn sanitize_fragment(html: &str, kind: PolicyKind) -> String {
    use std::collections::{HashMap, HashSet};

    // Edit: blob + cid + safe data; no remote images.
    // Export: cid only on img (blob/data must be rewritten before export).
    let (allow_blob, allow_cid, allow_data, allow_remote_img) = match kind {
        PolicyKind::Edit => (true, true, true, false),
        PolicyKind::Export => (false, true, false, false),
    };

    let tags: HashSet<&str> = allowed_tags().into_iter().collect();
    let clean_rm: HashSet<&str> = [
        "script", "style", "iframe", "object", "embed", "svg", "math",
    ]
    .into_iter()
    .collect();
    let generic: HashSet<&str> = [
        "title",
        "alt",
        "colspan",
        "rowspan",
        "width",
        "height",
        "class",
        // Composer quote markers (stripped again on export).
        "data-mlnr-quote",
    ]
    .into_iter()
    .collect();
    let mut tag_attrs: HashMap<&str, HashSet<&str>> = HashMap::new();
    tag_attrs.insert("a", ["href", "title", "class"].into_iter().collect());
    tag_attrs.insert(
        "img",
        ["src", "alt", "width", "height", "class"]
            .into_iter()
            .collect(),
    );
    tag_attrs.insert("div", ["class", "data-mlnr-quote"].into_iter().collect());
    tag_attrs.insert("p", ["class"].into_iter().collect());
    tag_attrs.insert("blockquote", ["class"].into_iter().collect());
    let schemes: HashSet<&str> = ["http", "https", "mailto", "cid", "blob", "data"]
        .into_iter()
        .collect();

    let mut builder = ammonia::Builder::default();
    builder
        .tags(tags)
        // Drop these tags *and* their contents (not just unwrap children).
        .clean_content_tags(clean_rm)
        .strip_comments(true)
        // No style attributes in v1.
        .generic_attributes(generic)
        .tag_attributes(tag_attrs)
        .url_schemes(schemes)
        .attribute_filter(
            move |element, attribute, value| match (element, attribute) {
                ("a", "href") => {
                    if is_safe_href(value) {
                        Some(value.into())
                    } else {
                        None
                    }
                }
                ("img", "src") => {
                    if is_safe_img_src(value, allow_blob, allow_cid, allow_data, allow_remote_img) {
                        Some(value.into())
                    } else {
                        None
                    }
                }
                ("img", "srcset") | (_, "srcset") | (_, "imagesrcset") => None,
                (_, "style") => None,
                (_, "background") | (_, "poster") => None,
                _ => Some(value.into()),
            },
        );

    builder.clean(html).to_string()
}

fn strip_data_mlnr(html: &str) -> String {
    // Best-effort strip data-mlnr-* attributes (export).
    let re = regex::Regex::new(r#"(?i)\sdata-mlnr-[a-z0-9_-]+\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+)"#)
        .expect("regex");
    re.replace_all(html, "").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_keeps_blob_img() {
        let html = r#"<p>x<img src="blob:https://example/uuid-1" alt="a"></p>"#;
        let out = sanitize_for_edit(html);
        assert!(out.contains("blob:"), "{out}");
    }

    #[test]
    fn edit_strips_remote_img() {
        let html = r#"<p><img src="https://tracker.example/pixel.gif"></p>"#;
        let out = sanitize_for_edit(html);
        assert!(!out.contains("tracker"), "{out}");
    }

    #[test]
    fn edit_keeps_https_link() {
        let html = r#"<p><a href="https://example.com">t</a></p>"#;
        let out = sanitize_for_edit(html);
        assert!(out.contains("https://example.com"), "{out}");
    }

    #[test]
    fn edit_strips_javascript_href() {
        let html = r#"<p><a href="javascript:alert(1)">x</a></p>"#;
        let out = sanitize_for_edit(html);
        assert!(!out.to_ascii_lowercase().contains("javascript"), "{out}");
    }

    #[test]
    fn export_allows_cid_img() {
        let html = r#"<p><img src="cid:img1@mailiner" alt="i"></p>"#;
        let out = sanitize_for_export(html);
        assert!(out.contains("cid:img1@mailiner"), "{out}");
    }

    #[test]
    fn export_strips_blob() {
        let html = r#"<p><img src="blob:https://x/1"></p>"#;
        let out = sanitize_for_export(html);
        assert!(!out.contains("blob:"), "{out}");
    }

    #[test]
    fn export_strips_data_img() {
        let html = r#"<p><img src="data:image/png;base64,aaa"></p>"#;
        let out = sanitize_for_export(html);
        assert!(!out.contains("data:image"), "{out}");
    }

    #[test]
    fn edit_strips_protocol_relative_href() {
        let html = r#"<p><a href="//evil.example/x">x</a></p>"#;
        let out = sanitize_for_edit(html);
        assert!(!out.contains("evil"), "{out}");
    }

    #[test]
    fn export_strips_data_mlnr() {
        let html = r#"<div data-mlnr-quote="1"><p>hi</p></div>"#;
        let out = sanitize_for_export(html);
        assert!(!out.contains("data-mlnr"), "{out}");
    }

    #[test]
    fn strips_script_and_style() {
        let html = r#"<p>a</p><script>alert(1)</script><style>body{}</style>"#;
        let out = sanitize_for_edit(html);
        assert!(!out.contains("script"), "{out}");
        assert!(!out.contains("alert"), "{out}");
        assert!(!out.contains("style"), "{out}");
    }

    #[test]
    fn safe_data_image_helper() {
        assert!(is_safe_data_image("data:image/png;base64,xx"));
        assert!(!is_safe_data_image("data:text/html;base64,xx"));
    }

    #[test]
    fn safe_image_content_type_allows_raster_only() {
        assert!(is_safe_image_content_type("image/png"));
        assert!(is_safe_image_content_type("IMAGE/JPEG; charset=binary"));
        assert!(is_safe_image_content_type(" image/webp "));
        assert!(!is_safe_image_content_type("image/svg+xml"));
        assert!(!is_safe_image_content_type("application/octet-stream"));
        assert!(!is_safe_image_content_type(""));
    }

    #[test]
    fn edit_keeps_collapsed_format_caret_marker() {
        let html = "<p><strong>\u{200b}</strong></p>";
        let out = sanitize_for_edit(html);
        assert!(out.contains('\u{200b}'), "{out}");
        assert!(out.to_ascii_lowercase().contains("strong"), "{out}");
    }

    #[test]
    fn export_drops_format_element_that_is_only_zwsp() {
        assert_eq!(sanitize_for_export("<b>\u{200b}</b>"), "");
        assert_eq!(sanitize_for_export("<i>\u{200b}</i>"), "");
        assert_eq!(sanitize_for_export("<u>\u{200b}</u>"), "");
        assert_eq!(sanitize_for_export("<em>\u{200b}</em>"), "");
        assert_eq!(sanitize_for_export("<strong>\u{200b}</strong>"), "");
        let nested = sanitize_for_export("<p>Hi<strong><em>\u{200b}</em></strong></p>");
        assert_eq!(nested, "<p>Hi</p>");
    }

    #[test]
    fn export_drops_leading_zwsp_inside_format_tags() {
        let out = sanitize_for_export("<p><strong>\u{200b}Hello</strong></p>");
        assert_eq!(out, "<p><strong>Hello</strong></p>");
        let nested = sanitize_for_export("<p><strong>\u{200b}<em>\u{200b}Yes</em></strong></p>");
        assert_eq!(nested, "<p><strong><em>Yes</em></strong></p>");
        let aliases = sanitize_for_export("<b>\u{200b}B</b><i>\u{200b}I</i><u>&#x200B;U</u>");
        assert_eq!(aliases, "<b>B</b><i>I</i><u>U</u>");
        let entity = sanitize_for_export("<em>&#8203;Go</em>");
        assert_eq!(entity, "<em>Go</em>");
        let upper = sanitize_for_export("<STRONG>\u{200b}Hi</STRONG>");
        assert_eq!(upper, "<strong>Hi</strong>");
    }

    #[test]
    fn export_keeps_zwsp_outside_format_markers() {
        let plain = sanitize_for_export("<p>wo\u{200b}rd</p>");
        assert_eq!(plain, "<p>wo\u{200b}rd</p>");
        let span = sanitize_for_export("<p><span>\u{200b}x</span></p>");
        assert_eq!(span, "<p><span>\u{200b}x</span></p>");
        let middle = sanitize_for_export("<strong>a\u{200b}b</strong>");
        assert_eq!(middle, "<strong>a\u{200b}b</strong>");
        let with_img =
            sanitize_for_export("<strong>\u{200b}<img src=\"cid:a@b\" alt=\"t\"></strong>");
        assert_eq!(with_img, "<strong><img src=\"cid:a@b\" alt=\"t\"></strong>");
        let classed = sanitize_for_export("<strong class=\"x\">\u{200b}Hi</strong>");
        assert_eq!(classed, "<strong class=\"x\">Hi</strong>");
    }

    #[test]
    fn export_zwsp_does_not_panic_on_multibyte_text() {
        let out = sanitize_for_export("<p>a\u{200b}b</p><strong>你好世界</strong>");
        assert_eq!(out, "<p>a\u{200b}b</p><strong>你好世界</strong>");
        assert_eq!(
            sanitize_for_export("<strong>\u{200b}你好</strong>"),
            "<strong>你好</strong>"
        );
        assert_eq!(
            sanitize_for_export("<strong>你好\u{200b}</strong>"),
            "<strong>你好\u{200b}</strong>"
        );
    }

    #[test]
    fn export_strips_leading_zwsp_nested_in_span() {
        assert_eq!(
            sanitize_for_export("<strong><span>\u{200b}Hi</span></strong>"),
            "<strong><span>Hi</span></strong>"
        );
        assert_eq!(
            sanitize_for_export("<p>Hi<strong><span>\u{200b}</span></strong></p>"),
            "<p>Hi</p>"
        );
        assert_eq!(
            sanitize_for_export("<strong><span><span>\u{200b}Yes</span></span></strong>"),
            "<strong><span><span>Yes</span></span></strong>"
        );
        // Not leading: a word-break wrapped in a span stays.
        assert_eq!(
            sanitize_for_export("<strong>a<span>\u{200b}</span>b</strong>"),
            "<strong>a<span>\u{200b}</span>b</strong>"
        );
        // Outside a formatting tag, the span is ordinary text.
        assert_eq!(
            sanitize_for_export("<p><span>\u{200b}x</span></p>"),
            "<p><span>\u{200b}x</span></p>"
        );
    }
}
