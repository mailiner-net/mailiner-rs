//! Safe HTML formatter with cid resolution and remote-resource blocking.

use base64::{Engine, engine::general_purpose::STANDARD};
use mailiner_core::models::{MessageContent, MessagePart};
use regex::Regex;
use std::sync::OnceLock;

use super::sanitize::sanitize_css;
use super::{FormatOptions, FormatResult, text_content};

const SAFE_IMAGE_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/jpg",
    "image/gif",
    "image/webp",
    "image/bmp",
];

fn re_style_block() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<style\b[^>]*>(.*?)</style>").unwrap())
}

fn re_attr(attr: &str) -> Regex {
    Regex::new(&format!(r#"(?i)(\s{attr}\s*=\s*)(["'])([^"']*)(["'])"#)).unwrap()
}

pub fn format_html(
    part: &MessagePart,
    all_parts: &[MessagePart],
    opts: &FormatOptions,
) -> Option<FormatResult> {
    let html = text_content(part)?;
    let mut prevented = false;
    let mut inlined = Vec::new();

    // 1) Sanitize <style> blocks. `html` / `body` selectors are left intact:
    // the viewer mounts the result as a real document inside a shadow root.
    let mut body = re_style_block()
        .replace_all(html, |caps: &regex::Captures| {
            let css = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let clean = sanitize_css(css, opts.allow_remote_resources);
            format!("<style>{clean}</style>")
        })
        .into_owned();

    if !opts.allow_remote_resources
        && (html.to_ascii_lowercase().contains("@import")
            || html.to_ascii_lowercase().contains("url(http"))
    {
        prevented = true;
    }

    // 2) Process remote-capable attributes
    for attr in ["src", "srcset", "href", "imagesrcset", "background"] {
        let re = re_attr(attr);
        body = re
            .replace_all(&body, |caps: &regex::Captures| {
                let prefix = caps.get(1).unwrap().as_str();
                let q = caps.get(2).unwrap().as_str();
                let value = caps.get(3).unwrap().as_str();
                let q2 = caps.get(4).unwrap().as_str();
                let vtrim = value.trim();
                let lower = vtrim.to_ascii_lowercase();

                if lower.starts_with("cid:") {
                    let cid = vtrim[4..].trim();
                    if let Some((data_url, part_id)) = resolve_cid(cid, all_parts) {
                        inlined.push(part_id);
                        return format!("{prefix}{q}{data_url}{q2}");
                    }
                    return String::new();
                }

                if lower.starts_with("javascript:") || lower.starts_with("vbscript:") {
                    return String::new();
                }

                if lower.starts_with("data:") {
                    if is_safe_data_image(&lower) {
                        return caps.get(0).unwrap().as_str().to_string();
                    }
                    return String::new();
                }

                if opts.allow_remote_resources {
                    return caps.get(0).unwrap().as_str().to_string();
                }

                // Strip the attribute entirely. Allow-remote re-formats from the
                // retained original HTML source (no URL stored in sanitized output).
                prevented = true;
                String::new()
            })
            .into_owned();
    }

    let cleaned = ammonia_clean(&body, opts.allow_remote_resources);

    Some(FormatResult {
        html: cleaned,
        prevented_remote_resources: prevented,
        inlined_part_ids: inlined,
    })
}

fn is_safe_data_image(lower: &str) -> bool {
    SAFE_IMAGE_TYPES
        .iter()
        .any(|t| lower.starts_with(&format!("data:{t}")))
}

fn resolve_cid(cid: &str, parts: &[MessagePart]) -> Option<(String, String)> {
    let cid_norm = cid.trim().trim_matches(|c| c == '<' || c == '>');
    let part = parts.iter().find(|p| {
        p.content_id
            .as_deref()
            .map(|id| {
                let id = id.trim().trim_matches(|c| c == '<' || c == '>');
                id.eq_ignore_ascii_case(cid_norm)
            })
            .unwrap_or(false)
            || p.description
                .as_deref()
                .map(|d| d.trim().trim_matches(|c| c == '<' || c == '>') == cid_norm)
                .unwrap_or(false)
    })?;

    let ct = part.content_type.to_ascii_lowercase();
    let ct_main = ct.split(';').next().unwrap_or(&ct).trim();
    if ct_main == "image/svg+xml" || ct_main.contains("svg") {
        return None;
    }
    if !SAFE_IMAGE_TYPES.iter().any(|t| *t == ct_main) {
        return None;
    }

    let bytes = match &part.content {
        MessageContent::Binary(b) => b.as_slice(),
        MessageContent::Text(t) => t.as_bytes(),
        MessageContent::Empty => return None,
    };
    let b64 = STANDARD.encode(bytes);
    let url = format!("data:{ct_main};base64,{b64}");
    Some((url, part.id.to_string()))
}

/// HTML-email presentational attributes (tables / fonts / images).
const EMAIL_PRESENTATIONAL_ATTRS: &[&str] = &[
    "class",
    "style",
    "id",
    "dir",
    "width",
    "height",
    "align",
    "valign",
    "bgcolor",
    "background",
    "border",
    "cellpadding",
    "cellspacing",
    "color",
    "face",
    "size",
    "nowrap",
];

fn ammonia_clean(html: &str, allow_remote: bool) -> String {
    use ammonia::Builder;

    let mut b = Builder::default();
    // `style` is in default clean_content_tags; remove before allowing the tag.
    b.rm_clean_content_tags(["style"]);
    b.add_tags(["style", "font"]);
    // HTML mail (LinkedIn, newsletters) is almost entirely inline `style=` plus
    // table presentational attrs. Ammonia's defaults drop all of those.
    b.add_generic_attributes(EMAIL_PRESENTATIONAL_ATTRS);
    b.add_tag_attributes("font", ["color", "face", "size"]);
    // Default schemes are http/https/mailto — allow data: for cid→data:image inlines.
    b.add_url_schemes(["data"]);

    b.attribute_filter(move |_element, attribute, value| {
        let attr = attribute.to_ascii_lowercase();
        let val = value.to_ascii_lowercase();
        if attr == "style" {
            let clean = sanitize_css(value, allow_remote);
            if clean.trim().is_empty() {
                return None;
            }
            return Some(clean.into());
        }
        if matches!(
            attr.as_str(),
            "href" | "src" | "srcset" | "background" | "poster"
        ) {
            if val.starts_with("javascript:") || val.starts_with("vbscript:") {
                return None;
            }
            if val.starts_with("data:") {
                if is_safe_data_image(&val) {
                    return Some(value.into());
                }
                return None;
            }
        }
        Some(value.into())
    });

    let mut clean = b.clean(html).to_string();

    static RE_SVG: OnceLock<Regex> = OnceLock::new();
    // `regex` crate does not support backreferences; match svg and math separately.
    let re = RE_SVG
        .get_or_init(|| Regex::new(r"(?is)<svg\b[^>]*>.*?</svg>|<math\b[^>]*>.*?</math>").unwrap());
    clean = re.replace_all(&clean, "").into_owned();
    clean
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use mailiner_core::ids::{FolderId, MessageId, MessagePartId};
    use mailiner_core::models::{PartKind, TransferEncoding};

    fn html_part(html: &str) -> MessagePart {
        let now = Utc::now();
        MessagePart {
            id: MessagePartId::new("html"),
            envelope_id: MessageId::new(FolderId::new("INBOX"), "1"),
            path: vec!["1".into()],
            kind: PartKind::TextHtml,
            content_type: "text/html".into(),
            charset: Some("UTF-8".into()),
            content_id: None,
            description: None,
            filename: None,
            encoding: TransferEncoding::SevenBit,
            original_size: None,
            size: html.len() as u64,
            is_attachment: false,
            is_hidden: false,
            content: MessageContent::Text(html.into()),
            created_at: now,
            updated_at: now,
        }
    }

    fn png_part(cid: &str, bytes: &[u8]) -> MessagePart {
        let now = Utc::now();
        MessagePart {
            id: MessagePartId::new("img"),
            envelope_id: MessageId::new(FolderId::new("INBOX"), "1"),
            path: vec!["2".into()],
            kind: PartKind::Image,
            content_type: "image/png".into(),
            charset: None,
            content_id: Some(cid.into()),
            description: None,
            filename: None,
            encoding: TransferEncoding::Base64,
            original_size: Some(bytes.len() as u64),
            size: bytes.len() as u64,
            is_attachment: true,
            is_hidden: true,
            content: MessageContent::Binary(bytes.to_vec()),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn cid_to_data_url() {
        let html = html_part(r#"<img src="cid:logo@x">"#);
        let img = png_part("<logo@x>", b"\x89PNG");
        let r = format_html(&html, &[html.clone(), img], &FormatOptions::default()).unwrap();
        assert!(r.html.contains("data:image/png;base64,"));
        assert!(r.inlined_part_ids.iter().any(|id| id == "img"));
    }

    #[test]
    fn rejects_svg_cid() {
        let html = html_part(r#"<img src="cid:evil">"#);
        let now = Utc::now();
        let svg = MessagePart {
            id: MessagePartId::new("svg"),
            envelope_id: MessageId::new(FolderId::new("INBOX"), "1"),
            path: vec!["2".into()],
            kind: PartKind::Image,
            content_type: "image/svg+xml".into(),
            charset: None,
            content_id: Some("<evil>".into()),
            description: None,
            filename: None,
            encoding: TransferEncoding::SevenBit,
            original_size: None,
            size: 10,
            is_attachment: true,
            is_hidden: true,
            content: MessageContent::Text("<svg onload=alert(1)>".into()),
            created_at: now,
            updated_at: now,
        };
        let r = format_html(&html, &[html.clone(), svg], &FormatOptions::default()).unwrap();
        assert!(!r.html.contains("data:image/svg"));
    }

    #[test]
    fn strips_script() {
        let html = html_part("<p>Hi<script>alert(1)</script></p>");
        let r = format_html(&html, &[html.clone()], &FormatOptions::default()).unwrap();
        assert!(r.html.contains("Hi"));
        assert!(!r.html.to_ascii_lowercase().contains("<script"));
    }

    #[test]
    fn keeps_inline_styles() {
        let html = html_part(r#"<p class="lead" style="color:#c00;font-size:16px">Hi</p>"#);
        let r = format_html(&html, &[html.clone()], &FormatOptions::default()).unwrap();
        assert!(r.html.contains("style="), "{r:?}");
        assert!(r.html.contains("color"), "{r:?}");
        assert!(r.html.contains("font-size"), "{r:?}");
        assert!(r.html.contains("lead"), "{r:?}");
    }

    #[test]
    fn strips_remote_url_from_inline_style_by_default() {
        let html =
            html_part(r#"<p style="background:url(https://evil.example/x.png);color:red">Hi</p>"#);
        let r = format_html(&html, &[html.clone()], &FormatOptions::default()).unwrap();
        assert!(!r.html.contains("evil.example"), "{r:?}");
        assert!(r.html.contains("color"), "{r:?}");
    }

    #[test]
    fn keeps_remote_url_in_inline_style_when_allowed() {
        let html = html_part(r#"<p style="background:url(https://ok.example/x.png)">Hi</p>"#);
        let r = format_html(
            &html,
            &[html.clone()],
            &FormatOptions {
                allow_remote_resources: true,
            },
        )
        .unwrap();
        assert!(r.html.contains("ok.example"), "{r:?}");
    }

    #[test]
    fn keeps_table_presentational_attrs() {
        let html = html_part(
            r##"<table width="512" cellpadding="0" cellspacing="0" bgcolor="#ffffff"><tr><td align="center" valign="top">x</td></tr></table>"##,
        );
        let r = format_html(&html, &[html.clone()], &FormatOptions::default()).unwrap();
        assert!(r.html.contains("width="), "{r:?}");
        assert!(r.html.contains("cellpadding="), "{r:?}");
        assert!(r.html.contains("cellspacing="), "{r:?}");
        assert!(r.html.contains("bgcolor="), "{r:?}");
        assert!(r.html.contains("align="), "{r:?}");
        assert!(r.html.contains("valign="), "{r:?}");
    }

    #[test]
    fn keeps_body_selector_for_document_mount() {
        let html = html_part("<style>body {font-family: Arial}</style><p>Hi</p>");
        let r = format_html(&html, &[html.clone()], &FormatOptions::default()).unwrap();
        let lower = r.html.to_ascii_lowercase();
        assert!(lower.contains("body {") || lower.contains("body{"), "{r:?}");
        assert!(!lower.contains(":host"), "{r:?}");
    }
}
