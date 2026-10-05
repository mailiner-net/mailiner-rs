//! Safe HTML formatter with cid resolution and remote-resource blocking.

use base64::{Engine, engine::general_purpose::STANDARD};
use mailiner_core::models::{MessageContent, MessagePart};
use regex::Regex;
use std::sync::OnceLock;

use super::quote::collapse_trailing_blockquotes;
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
        let current = body.clone();
        body = re
            .replace_all(&current, |caps: &regex::Captures| {
                let prefix = caps.get(1).unwrap().as_str();
                let q = caps.get(2).unwrap().as_str();
                let value = caps.get(3).unwrap().as_str();
                let q2 = caps.get(4).unwrap().as_str();
                let vtrim = value.trim();
                let lower = vtrim.to_ascii_lowercase();

                if lower.starts_with("cid:") {
                    let cid = vtrim[4..].trim();
                    if let Some((data_url, part_id)) =
                        resolve_cid(cid, all_parts, part.nested_in.as_deref())
                    {
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

                // Anchors navigate; they are not remote images or stylesheets.
                // Keep http(s), mailto, fragments, and relative URLs. Drop
                // scriptable and protocol-relative hrefs without the banner.
                // `<link>` and `<base>` hrefs are subresources. Ammonia drops
                // those tags, but the banner still has to record that a remote
                // target was removed.
                if attr == "href" {
                    let attr_at = caps.get(0).unwrap().start();
                    if is_navigation_element(&current, attr_at) {
                        if is_navigation_href(vtrim) {
                            return caps.get(0).unwrap().as_str().to_string();
                        }
                        return String::new();
                    }
                    if tag_name_before(&current, attr_at).is_some() {
                        prevented = true;
                        return String::new();
                    }
                    if is_navigation_href(vtrim) {
                        return caps.get(0).unwrap().as_str().to_string();
                    }
                    return String::new();
                }

                // Strip the attribute entirely. Allow-remote re-formats from the
                // retained original HTML source (no URL stored in sanitized output).
                prevented = true;
                String::new()
            })
            .into_owned();
    }

    let cleaned = collapse_trailing_blockquotes(&ammonia_clean(&body, opts.allow_remote_resources));

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

/// `href` values that navigate instead of fetching a subresource.
///
/// Remote images and stylesheets are blocked separately. A normal link must
/// stay clickable while those are blocked, and must not raise the banner.
fn is_navigation_href(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    if lower.is_empty()
        || lower.starts_with("javascript:")
        || lower.starts_with("vbscript:")
        || lower.starts_with("data:")
        || lower.starts_with("cid:")
        || lower.starts_with("//")
    {
        return false;
    }
    if lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:")
        || lower.starts_with('#')
    {
        return true;
    }
    // A scheme is `name:` in the segment before the first `/`, `?`, or `#`.
    // `?next=https://example.test` and `dir/a:b` are relative and stay.
    let head = lower
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(lower.as_str());
    !head.contains(':')
}

/// Element that owns the attribute match at `attr_at`.
///
/// Quote-aware so a `>` inside an earlier attribute value is not the end of
/// the tag. `None` when the match is not inside a start tag.
fn tag_name_before(html: &str, attr_at: usize) -> Option<&str> {
    let head = html.get(..attr_at)?;
    let bytes = head.as_bytes();
    let mut i = bytes.len();
    let mut quote: Option<u8> = None;
    while i > 0 {
        i -= 1;
        let c = bytes[i];
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            b'"' | b'\'' => quote = Some(c),
            b'>' => return None,
            b'<' => {
                let rest = head[i + 1..].trim_start();
                if rest.starts_with('/') || rest.starts_with('!') || rest.starts_with('?') {
                    return None;
                }
                let name_len = rest
                    .find(|ch: char| ch.is_ascii_whitespace() || ch == '/' || ch == '>')
                    .unwrap_or(rest.len());
                let name = &rest[..name_len];
                if name.is_empty() {
                    return None;
                }
                return Some(name);
            }
            _ => {}
        }
    }
    None
}

fn is_navigation_element(html: &str, attr_at: usize) -> bool {
    tag_name_before(html, attr_at)
        .is_some_and(|name| name.eq_ignore_ascii_case("a") || name.eq_ignore_ascii_case("area"))
}

fn resolve_cid(
    cid: &str,
    parts: &[MessagePart],
    nested_in: Option<&str>,
) -> Option<(String, String)> {
    let cid_norm = cid.trim().trim_matches(|c| c == '<' || c == '>');
    let part = parts.iter().find(|p| {
        p.in_scope(nested_in)
            && (p
                .content_id
                .as_deref()
                .map(|id| {
                    let id = id.trim().trim_matches(|c| c == '<' || c == '>');
                    id.eq_ignore_ascii_case(cid_norm)
                })
                .unwrap_or(false)
                || p.description
                    .as_deref()
                    .map(|d| d.trim().trim_matches(|c| c == '<' || c == '>') == cid_norm)
                    .unwrap_or(false))
    })?;

    let ct = part.content_type.to_ascii_lowercase();
    let ct_main = ct.split(';').next().unwrap_or(&ct).trim();
    if ct_main == "image/svg+xml" || ct_main.contains("svg") {
        return None;
    }
    if !SAFE_IMAGE_TYPES.contains(&ct_main) {
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
            nested_in: None,
            nested_headers: None,
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
            nested_in: None,
            nested_headers: None,
            content: MessageContent::Binary(bytes.to_vec()),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn cid_to_data_url() {
        let html = html_part(r#"<img src="cid:logo@x">"#);
        let img = png_part("<logo@x>", b"\x89PNG");
        let r = format_html(
            &html,
            &[html.clone(), img.clone()],
            &FormatOptions::default(),
        )
        .unwrap();
        assert!(r.html.contains("data:image/png;base64,"));
        assert!(r.inlined_part_ids.iter().any(|id| id == "img"));

        let mut parts = vec![html, img];
        r.drop_inlined_payloads(&mut parts);
        assert!(matches!(parts[1].content, MessageContent::Empty));
        assert_eq!(parts[1].content_id.as_deref(), Some("<logo@x>"));
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
            nested_in: None,
            nested_headers: None,
            content: MessageContent::Text("<svg onload=alert(1)>".into()),
            created_at: now,
            updated_at: now,
        };
        let r = format_html(&html, &[html.clone(), svg], &FormatOptions::default()).unwrap();
        assert!(!r.html.contains("data:image/svg"));
    }

    #[test]
    fn cid_does_not_cross_rfc822_scope() {
        let html = html_part(r#"<img src="cid:logo@x">"#);
        let mut foreign = png_part("<logo@x>", b"\x89PNG");
        foreign.nested_in = Some("2".into());
        let r = format_html(&html, &[html.clone(), foreign], &FormatOptions::default()).unwrap();
        assert!(!r.html.contains("data:image/png;base64,"));
        assert!(r.inlined_part_ids.is_empty());
    }

    #[test]
    fn strips_script() {
        let html = html_part("<p>Hi<script>alert(1)</script></p>");
        let r = format_html(
            &html,
            std::slice::from_ref(&html),
            &FormatOptions::default(),
        )
        .unwrap();
        assert!(r.html.contains("Hi"));
        assert!(!r.html.to_ascii_lowercase().contains("<script"));
    }

    #[test]
    fn keeps_https_link_when_remote_resources_blocked() {
        let html = html_part(r#"<p><a href="https://mailiner.test">link</a></p>"#);
        let r = format_html(
            &html,
            std::slice::from_ref(&html),
            &FormatOptions::default(),
        )
        .unwrap();
        assert!(r.html.contains("https://mailiner.test"), "{r:?}");
        assert!(!r.prevented_remote_resources, "{r:?}");
    }

    #[test]
    fn keeps_relative_href_with_a_colon_after_the_path() {
        for href in [
            "?next=https://example.test",
            "dir/a:b",
            "/files/a:b",
            "#section:1",
        ] {
            let html = html_part(&format!(r#"<p><a href="{href}">x</a></p>"#));
            let r = format_html(
                &html,
                std::slice::from_ref(&html),
                &FormatOptions::default(),
            )
            .unwrap();
            assert!(r.html.contains(href), "{href} dropped from {}", r.html);
            assert!(!r.prevented_remote_resources, "{href} raised the banner");
        }
    }

    #[test]
    fn strips_unknown_scheme_without_remote_banner() {
        let html = html_part(r#"<a href="foo:bar">x</a>"#);
        let r = format_html(
            &html,
            std::slice::from_ref(&html),
            &FormatOptions::default(),
        )
        .unwrap();
        assert!(!r.html.contains("foo:bar"), "{}", r.html);
        assert!(!r.prevented_remote_resources);
    }

    #[test]
    fn link_and_base_hrefs_are_blocked_remote_resources() {
        for html_src in [
            r#"<link rel="stylesheet" href="https://evil.example/a.css">"#,
            r#"<base href="https://evil.example/">"#,
            r#"<a title="a>b" href="https://mailiner.test">ok</a><link href="https://evil.example/b.css">"#,
        ] {
            let html = html_part(html_src);
            let r = format_html(
                &html,
                std::slice::from_ref(&html),
                &FormatOptions::default(),
            )
            .unwrap();
            assert!(!r.html.contains("evil.example"), "{html_src} -> {}", r.html);
            assert!(
                r.prevented_remote_resources,
                "{html_src} did not raise the banner"
            );
        }
    }

    #[test]
    fn quoted_gt_in_anchor_title_stays_a_link() {
        let html = html_part(r#"<a title="a>b" href="https://mailiner.test">ok</a>"#);
        let r = format_html(
            &html,
            std::slice::from_ref(&html),
            &FormatOptions::default(),
        )
        .unwrap();
        assert!(r.html.contains("https://mailiner.test"), "{r:?}");
        assert!(!r.prevented_remote_resources, "{r:?}");
    }

    #[test]
    fn area_href_is_navigation() {
        let html = html_part(r#"<map><area href="https://mailiner.test"></map>"#);
        let r = format_html(
            &html,
            std::slice::from_ref(&html),
            &FormatOptions::default(),
        )
        .unwrap();
        assert!(!r.prevented_remote_resources, "{r:?}");
    }

    #[test]
    fn strips_unsafe_href_without_remote_banner() {
        for href in [
            "javascript:alert(1)",
            "vbscript:msgbox(1)",
            "data:text/html,hi",
            "//evil.example/phish",
        ] {
            let html = html_part(&format!(r#"<a href="{href}">x</a>"#));
            let r = format_html(
                &html,
                std::slice::from_ref(&html),
                &FormatOptions::default(),
            )
            .unwrap();
            assert!(
                !r.html.to_ascii_lowercase().contains(href),
                "{href} survived in {}",
                r.html
            );
            assert!(!r.prevented_remote_resources, "{href} raised the banner");
        }
    }

    #[test]
    fn blocked_image_still_flags_remote_resources_beside_a_link() {
        let html = html_part(
            r#"<a href="https://mailiner.test">link</a><img src="https://evil.example/a.png">"#,
        );
        let r = format_html(
            &html,
            std::slice::from_ref(&html),
            &FormatOptions::default(),
        )
        .unwrap();
        assert!(r.html.contains("https://mailiner.test"), "{r:?}");
        assert!(!r.html.contains("evil.example"), "{r:?}");
        assert!(r.prevented_remote_resources);
    }

    #[test]
    fn keeps_inline_styles() {
        let html = html_part(r#"<p class="lead" style="color:#c00;font-size:16px">Hi</p>"#);
        let r = format_html(
            &html,
            std::slice::from_ref(&html),
            &FormatOptions::default(),
        )
        .unwrap();
        assert!(r.html.contains("style="), "{r:?}");
        assert!(r.html.contains("color"), "{r:?}");
        assert!(r.html.contains("font-size"), "{r:?}");
        assert!(r.html.contains("lead"), "{r:?}");
    }

    #[test]
    fn strips_remote_url_from_inline_style_by_default() {
        let html =
            html_part(r#"<p style="background:url(https://evil.example/x.png);color:red">Hi</p>"#);
        let r = format_html(
            &html,
            std::slice::from_ref(&html),
            &FormatOptions::default(),
        )
        .unwrap();
        assert!(!r.html.contains("evil.example"), "{r:?}");
        assert!(r.html.contains("color"), "{r:?}");
    }

    #[test]
    fn keeps_remote_url_in_inline_style_when_allowed() {
        let html = html_part(r#"<p style="background:url(https://ok.example/x.png)">Hi</p>"#);
        let r = format_html(
            &html,
            std::slice::from_ref(&html),
            &FormatOptions {
                allow_remote_resources: true,
                prefer_plain: false,
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
        let r = format_html(
            &html,
            std::slice::from_ref(&html),
            &FormatOptions::default(),
        )
        .unwrap();
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
        let r = format_html(
            &html,
            std::slice::from_ref(&html),
            &FormatOptions::default(),
        )
        .unwrap();
        let lower = r.html.to_ascii_lowercase();
        assert!(lower.contains("body {") || lower.contains("body{"), "{r:?}");
        assert!(!lower.contains(":host"), "{r:?}");
    }

    #[test]
    fn wraps_trailing_blockquote_after_sanitize() {
        let html = html_part(
            "<p>Thanks.</p><blockquote><p>Hello<script>alert(1)</script></p></blockquote>",
        );
        let r = format_html(
            &html,
            std::slice::from_ref(&html),
            &FormatOptions::default(),
        )
        .unwrap();
        assert!(r.html.contains("<details class=\"mlnr-quote\">"), "{r:?}");
        assert!(r.html.contains("Show quoted text"), "{r:?}");
        assert!(r.html.contains("Thanks."), "{r:?}");
        assert!(r.html.contains("<blockquote>"), "{r:?}");
        assert!(!r.html.to_ascii_lowercase().contains("<script"), "{r:?}");
        assert!(!r.html.to_ascii_lowercase().contains("alert"), "{r:?}");
    }

    #[test]
    fn does_not_wrap_blockquote_when_it_is_the_whole_body() {
        let html = html_part("<blockquote><p>Forwarded</p></blockquote>");
        let r = format_html(
            &html,
            std::slice::from_ref(&html),
            &FormatOptions::default(),
        )
        .unwrap();
        assert!(!r.html.contains("<details"), "{r:?}");
        assert!(r.html.contains("Forwarded"), "{r:?}");
    }

    #[test]
    fn does_not_wrap_when_image_follows_blockquote() {
        let html = html_part(
            r#"<blockquote><p>old</p></blockquote><p><img src="data:image/png;base64,aaaa"></p>"#,
        );
        let r = format_html(
            &html,
            std::slice::from_ref(&html),
            &FormatOptions::default(),
        )
        .unwrap();
        assert!(!r.html.contains("<details"), "{r:?}");
    }
}
