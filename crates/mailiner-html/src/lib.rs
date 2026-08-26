//! Shared HTML/CSS sanitization for Mailiner (viewer + composer).

mod css;

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
pub fn sanitize_for_edit(html: &str) -> String {
    sanitize_fragment(html, PolicyKind::Edit)
}

/// Sanitize HTML after blob/data→cid rewrite, before MIME.
///
/// `img[src]` allows **cid only** (plus safe data: residual rejected in v1 — cid only).
pub fn sanitize_for_export(html: &str) -> String {
    let mut out = sanitize_fragment(html, PolicyKind::Export);
    // Strip composer-internal markers.
    out = strip_data_mlnr(&out);
    out
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
}
