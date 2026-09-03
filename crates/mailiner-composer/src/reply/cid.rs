//! CID → InlineImage rehydration for reply/forward HTML quotes.

use mailiner_core::{MessageContent, MessagePart};
use mailiner_html::SAFE_IMAGE_TYPES;

use crate::model::{caps, AttachmentData, InlineId, InlineImage};

/// Result of scanning HTML for `cid:` images and attaching matching parts.
#[derive(Debug, Clone)]
pub struct CidRehydrateResult {
    /// Quoted HTML. Known `cid:` img srcs are kept; missing/unsafe ones are dropped.
    pub html: String,
    /// Inline images to attach on the draft (`multipart/related` at export).
    pub images: Vec<InlineImage>,
    /// Soft warnings (missing cid, oversize, unsafe type, empty payload).
    pub warnings: Vec<String>,
}

/// Normalize a Content-ID / cid token for comparison (strip `<>`, lowercase).
pub fn normalize_cid(cid: &str) -> String {
    cid.trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim()
        .to_ascii_lowercase()
}

/// Content-ID without angle brackets (matches [`InlineImage::content_id`]).
pub fn bare_content_id(cid: &str) -> String {
    cid.trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim()
        .to_string()
}

/// Extract unique `cid:` tokens referenced by `<img src>` attributes.
pub fn extract_cid_refs(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    for_each_img_tag(html, |tag| {
        let Some(src) = find_src_attr(tag) else {
            return;
        };
        let Some(token) = cid_token(src.value) else {
            return;
        };
        let norm = normalize_cid(token);
        if !norm.is_empty() && !out.iter().any(|c: &String| normalize_cid(c) == norm) {
            out.push(token.to_string());
        }
    });
    out
}

fn cid_token(src: &str) -> Option<&str> {
    let src = src.trim();
    src.get(..4)
        .filter(|p| p.eq_ignore_ascii_case("cid:"))
        .and_then(|_| src.get(4..))
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

fn is_img_open(s: &str) -> bool {
    s.len() >= 4
        && s[..4].eq_ignore_ascii_case("<img")
        && s[4..]
            .chars()
            .next()
            .is_none_or(|c| c.is_whitespace() || c == '/' || c == '>')
}

fn for_each_img_tag(html: &str, mut visit: impl FnMut(&str)) {
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' && is_img_open(&html[i..]) {
            if let Some(end_rel) = find_tag_end(&html[i..]) {
                visit(&html[i..=i + end_rel]);
                i += end_rel + 1;
                continue;
            }
        }
        i += html[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }
}

struct SrcAttr<'a> {
    value: &'a str,
    /// Start of the attribute value (inside quotes).
    value_start: usize,
    /// Index of the closing quote, or one past an unquoted value.
    value_end: usize,
}

fn find_src_attr(tag: &str) -> Option<SrcAttr<'_>> {
    parse_tag_attrs(tag)
        .into_iter()
        .find(|a| a.name.eq_ignore_ascii_case("src"))
        .map(|a| SrcAttr {
            value: a.value,
            value_start: a.value_start,
            value_end: a.value_end,
        })
}

struct ParsedAttr<'a> {
    name: &'a str,
    value: &'a str,
    value_start: usize,
    value_end: usize,
}

fn parse_tag_attrs(tag: &str) -> Vec<ParsedAttr<'_>> {
    let mut attrs = Vec::new();
    if !tag.starts_with('<') {
        return attrs;
    }
    let mut i = 1usize;
    // Skip tag name.
    while i < tag.len() {
        let ch = tag[i..].chars().next().unwrap();
        if ch.is_whitespace() || ch == '/' || ch == '>' {
            break;
        }
        i += ch.len_utf8();
    }
    while i < tag.len() {
        while i < tag.len() {
            let ch = tag[i..].chars().next().unwrap();
            if !ch.is_whitespace() {
                break;
            }
            i += ch.len_utf8();
        }
        if i >= tag.len() {
            break;
        }
        let ch = tag[i..].chars().next().unwrap();
        if ch == '>' || ch == '/' {
            break;
        }
        let name_start = i;
        while i < tag.len() {
            let ch = tag[i..].chars().next().unwrap();
            if ch.is_whitespace() || ch == '=' || ch == '>' || ch == '/' {
                break;
            }
            i += ch.len_utf8();
        }
        let name = &tag[name_start..i];
        while i < tag.len() {
            let ch = tag[i..].chars().next().unwrap();
            if !ch.is_whitespace() {
                break;
            }
            i += ch.len_utf8();
        }
        if i >= tag.len() || !tag[i..].starts_with('=') {
            continue;
        }
        i += 1;
        while i < tag.len() {
            let ch = tag[i..].chars().next().unwrap();
            if !ch.is_whitespace() {
                break;
            }
            i += ch.len_utf8();
        }
        if i >= tag.len() {
            break;
        }
        let first = tag[i..].chars().next().unwrap();
        if first == '"' || first == '\'' {
            i += first.len_utf8();
            let value_start = i;
            let mut closed = false;
            while i < tag.len() {
                let ch = tag[i..].chars().next().unwrap();
                if ch == first {
                    attrs.push(ParsedAttr {
                        name,
                        value: &tag[value_start..i],
                        value_start,
                        value_end: i,
                    });
                    i += ch.len_utf8();
                    closed = true;
                    break;
                }
                i += ch.len_utf8();
            }
            if !closed {
                break;
            }
        } else {
            let value_start = i;
            while i < tag.len() {
                let ch = tag[i..].chars().next().unwrap();
                if ch.is_whitespace() || ch == '>' || ch == '/' {
                    break;
                }
                i += ch.len_utf8();
            }
            attrs.push(ParsedAttr {
                name,
                value: &tag[value_start..i],
                value_start,
                value_end: i,
            });
        }
    }
    attrs
}

fn find_part_for_cid<'a>(cid: &str, parts: &'a [MessagePart]) -> Option<&'a MessagePart> {
    let cid_norm = normalize_cid(cid);
    parts.iter().find(|p| {
        p.content_id
            .as_deref()
            .is_some_and(|id| normalize_cid(id) == cid_norm)
            || p.description
                .as_deref()
                .is_some_and(|d| normalize_cid(d) == cid_norm)
    })
}

fn part_bytes(part: &MessagePart) -> Option<&[u8]> {
    match &part.content {
        MessageContent::Binary(b) if !b.is_empty() => Some(b.as_slice()),
        MessageContent::Text(t) if !t.is_empty() => Some(t.as_bytes()),
        _ => None,
    }
}

fn content_type_main(part: &MessagePart) -> String {
    part.content_type
        .split(';')
        .next()
        .unwrap_or(&part.content_type)
        .trim()
        .to_ascii_lowercase()
}

fn is_safe_image_type(content_type: &str) -> bool {
    SAFE_IMAGE_TYPES.contains(&content_type)
}

/// Map `cid:` images in HTML onto [`InlineImage`] entries.
///
/// Successful matches keep `cid:` in `html` (canonical `cid:{bare}`) and copy the
/// part bytes onto the draft. Missing, empty, unsafe, or oversize cids drop the
/// `<img>` and record a [`CidRehydrateResult::warnings`] entry.
///
/// Respects remaining budget vs [`caps::MAX_INLINES`], [`caps::MAX_INLINE_BYTES`],
/// and [`caps::MAX_DRAFT_BYTES`] (`remaining_draft_bytes`).
pub fn rehydrate_cids(
    html: &str,
    parts: &[MessagePart],
    existing_inline_count: usize,
    remaining_draft_bytes: u64,
) -> CidRehydrateResult {
    let refs = extract_cid_refs(html);
    let mut images = Vec::new();
    let mut warnings = Vec::new();
    // (cid token, Some(canonical cid src) = keep, None = drop img)
    let mut replacements: Vec<(String, Option<String>)> = Vec::new();
    let mut budget = caps::MAX_INLINES.saturating_sub(existing_inline_count);
    let mut remaining_bytes = remaining_draft_bytes.saturating_sub(html.len() as u64);

    for token in refs {
        let bare = bare_content_id(&token);
        if budget == 0 {
            warnings.push(format!("Inline image limit reached; dropped cid:{bare}"));
            replacements.push((token, None));
            continue;
        }

        let Some(part) = find_part_for_cid(&token, parts) else {
            warnings.push(format!("Missing inline image for cid:{bare}"));
            replacements.push((token, None));
            continue;
        };

        let ct = content_type_main(part);
        if !is_safe_image_type(&ct) {
            warnings.push(format!(
                "Skipped unsafe inline image type ({ct}) for cid:{bare}"
            ));
            replacements.push((token, None));
            continue;
        }

        let Some(bytes) = part_bytes(part) else {
            warnings.push(format!("Inline image not loaded for cid:{bare}"));
            replacements.push((token, None));
            continue;
        };

        let size = bytes.len() as u64;
        if size > caps::MAX_INLINE_BYTES {
            warnings.push(format!(
                "Inline image too large ({size} bytes) for cid:{bare}"
            ));
            replacements.push((token, None));
            continue;
        }
        if size > remaining_bytes {
            warnings.push(format!(
                "Inline image would exceed draft size ({size} bytes) for cid:{bare}"
            ));
            replacements.push((token, None));
            continue;
        }

        let content_id = part
            .content_id
            .as_deref()
            .map(bare_content_id)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| bare.clone());
        images.push(InlineImage {
            id: InlineId::new(),
            content_id: content_id.clone(),
            content_type: ct,
            filename: part.filename.clone(),
            data: AttachmentData::Bytes(bytes.to_vec()),
            edit_url: None,
        });
        replacements.push((token, Some(format!("cid:{content_id}"))));
        budget = budget.saturating_sub(1);
        remaining_bytes = remaining_bytes.saturating_sub(size);
    }

    CidRehydrateResult {
        html: apply_cid_rewrites(html, &replacements),
        images,
        warnings,
    }
}

fn apply_cid_rewrites(html: &str, replacements: &[(String, Option<String>)]) -> String {
    if replacements.is_empty() {
        return html.to_string();
    }

    let mut map = std::collections::HashMap::new();
    for (token, new_src) in replacements {
        map.insert(normalize_cid(token), new_src.clone());
    }

    let mut out = String::with_capacity(html.len());
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' && is_img_open(&html[i..]) {
            if let Some(end_rel) = find_tag_end(&html[i..]) {
                let tag = &html[i..=i + end_rel];
                if let Some(rewritten) = rewrite_or_drop_img(tag, &map) {
                    out.push_str(&rewritten);
                }
                i += end_rel + 1;
                continue;
            }
        }
        let ch = html[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn find_tag_end(tag_start: &str) -> Option<usize> {
    let mut in_quote: Option<char> = None;
    for (idx, ch) in tag_start.char_indices() {
        match in_quote {
            Some(q) if ch == q => in_quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => in_quote = Some(ch),
            None if ch == '>' => return Some(idx),
            _ => {}
        }
    }
    None
}

fn rewrite_or_drop_img(
    tag: &str,
    map: &std::collections::HashMap<String, Option<String>>,
) -> Option<String> {
    let Some(src) = find_src_attr(tag) else {
        return Some(tag.to_string());
    };
    let Some(token) = cid_token(src.value) else {
        return Some(tag.to_string());
    };
    let norm = normalize_cid(token);
    match map.get(&norm) {
        None => Some(tag.to_string()),
        Some(None) => None,
        Some(Some(new_src)) => {
            if src.value.trim() == new_src {
                return Some(tag.to_string());
            }
            let mut out = String::with_capacity(tag.len() + new_src.len());
            out.push_str(&tag[..src.value_start]);
            out.push_str(new_src);
            out.push_str(&tag[src.value_end..]);
            Some(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use mailiner_core::{FolderId, MessageId, MessagePartId, PartKind, TransferEncoding};

    fn png_part(cid: &str, bytes: &[u8]) -> MessagePart {
        MessagePart {
            id: MessagePartId::new("img"),
            envelope_id: MessageId::new(FolderId::new("INBOX"), "1"),
            path: vec!["2".into()],
            kind: PartKind::Image,
            content_type: "image/png".into(),
            charset: None,
            content_id: Some(cid.into()),
            description: None,
            filename: Some("logo.png".into()),
            encoding: TransferEncoding::Base64,
            original_size: Some(bytes.len() as u64),
            size: bytes.len() as u64,
            is_attachment: true,
            is_hidden: true,
            content: MessageContent::Binary(bytes.to_vec()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn extract_cids() {
        let html = r#"<p><img src="cid:logo@x"><img src='cid:<other@y>'></p>"#;
        let refs = extract_cid_refs(html);
        assert_eq!(refs.len(), 2);
    }

    #[test]
    fn rehydrate_keeps_cid_and_copies_bytes() {
        let png = [0x89, b'P', b'N', b'G', 0, 0, 0, 0];
        let parts = vec![png_part("logo@x", &png)];
        let html = r#"<p>Hi <img src="cid:logo@x" alt="logo"></p>"#;
        let r = rehydrate_cids(html, &parts, 0, caps::MAX_DRAFT_BYTES);
        assert_eq!(r.images.len(), 1);
        assert!(r.html.contains("cid:logo@x"), "{}", r.html);
        assert!(!r.html.contains("data:"), "{}", r.html);
        assert_eq!(r.images[0].content_id, "logo@x");
        assert_eq!(r.images[0].content_type, "image/png");
        assert_eq!(r.images[0].filename.as_deref(), Some("logo.png"));
        assert!(matches!(&r.images[0].data, AttachmentData::Bytes(b) if b == &png));
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn missing_cid_drops_img() {
        let html = r#"<p><img src="cid:missing@x" alt="x">ok</p>"#;
        let r = rehydrate_cids(html, &[], 0, caps::MAX_DRAFT_BYTES);
        assert!(r.images.is_empty());
        assert!(!r.html.contains("<img"), "{}", r.html);
        assert!(r.html.contains("ok"));
        assert!(r.warnings.iter().any(|w| w.contains("Missing")));
    }

    #[test]
    fn empty_payload_drops_img() {
        let mut part = png_part("logo@x", b"png");
        part.content = MessageContent::Empty;
        let html = r#"<img src="cid:logo@x">"#;
        let r = rehydrate_cids(html, &[part], 0, caps::MAX_DRAFT_BYTES);
        assert!(r.images.is_empty());
        assert!(!r.html.contains("<img"), "{}", r.html);
        assert!(r.warnings.iter().any(|w| w.contains("not loaded")));
    }

    #[test]
    fn rejects_svg() {
        let mut part = png_part("evil", b"<svg/>");
        part.content_type = "image/svg+xml".into();
        let html = r#"<img src="cid:evil">"#;
        let r = rehydrate_cids(html, &[part], 0, caps::MAX_DRAFT_BYTES);
        assert!(r.images.is_empty());
        assert!(!r.html.contains("<img"), "{}", r.html);
        assert!(r.warnings.iter().any(|w| w.contains("unsafe")));
    }

    #[test]
    fn oversize_dropped() {
        let big = vec![0u8; (caps::MAX_INLINE_BYTES as usize) + 1];
        let parts = vec![png_part("big@x", &big)];
        let html = r#"<img src="cid:big@x">"#;
        let r = rehydrate_cids(html, &parts, 0, caps::MAX_DRAFT_BYTES);
        assert!(r.images.is_empty());
        assert!(r.warnings.iter().any(|w| w.contains("too large")));
    }

    #[test]
    fn bracketed_content_id_matches() {
        let png = [1, 2, 3, 4];
        let parts = vec![png_part("<logo@x>", &png)];
        let html = r#"<img src="cid:logo@x">"#;
        let r = rehydrate_cids(html, &parts, 0, caps::MAX_DRAFT_BYTES);
        assert_eq!(r.images.len(), 1);
        assert_eq!(r.images[0].content_id, "logo@x");
        assert!(r.html.contains("cid:logo@x"), "{}", r.html);
    }

    #[test]
    fn description_fallback_and_unreferenced_part_skipped() {
        let mut part = png_part("unused@x", b"PNG");
        part.content_id = None;
        part.description = Some("<logo@x>".into());
        let extra = png_part("other@x", b"XXX");
        let html = r#"<img src="cid:logo@x">"#;
        let r = rehydrate_cids(html, &[part, extra], 0, caps::MAX_DRAFT_BYTES);
        assert_eq!(r.images.len(), 1);
        assert_eq!(r.images[0].content_id, "logo@x");
    }

    #[test]
    fn duplicate_cid_refs_share_one_inline() {
        let png = [1u8, 2, 3];
        let parts = vec![png_part("logo@x", &png)];
        let html = r#"<img src="cid:logo@x"><img src="CID:LOGO@X">"#;
        let r = rehydrate_cids(html, &parts, 0, caps::MAX_DRAFT_BYTES);
        assert_eq!(r.images.len(), 1);
        assert_eq!(r.html.matches("cid:logo@x").count(), 2);
    }

    #[test]
    fn inline_budget_drops_overflow() {
        let png = [1u8, 2, 3];
        let parts = vec![png_part("logo@x", &png)];
        let html = r#"<p>keep<img src="cid:logo@x"></p>"#;
        let r = rehydrate_cids(html, &parts, caps::MAX_INLINES, caps::MAX_DRAFT_BYTES);
        assert!(r.images.is_empty());
        assert!(!r.html.contains("<img"), "{}", r.html);
        assert!(r.warnings.iter().any(|w| w.contains("limit")));
    }

    #[test]
    fn draft_byte_budget_drops_overflow() {
        let png = [1u8, 2, 3, 4];
        let parts = vec![png_part("logo@x", &png)];
        let html = r#"<p>keep<img src="cid:logo@x"></p>"#;
        let remaining = html.len() as u64 + 2;
        let r = rehydrate_cids(html, &parts, 0, remaining);
        assert!(r.images.is_empty());
        assert!(!r.html.contains("<img"), "{}", r.html);
        assert!(r.warnings.iter().any(|w| w.contains("draft size")));
    }

    #[test]
    fn src_in_alt_is_not_treated_as_attribute() {
        let png = [1u8, 2, 3];
        let parts = vec![png_part("logo@x", &png)];
        let html = r#"<img alt="see src=cid:fake@x" src="cid:logo@x">"#;
        let r = rehydrate_cids(html, &parts, 0, caps::MAX_DRAFT_BYTES);
        assert_eq!(r.images.len(), 1);
        assert_eq!(r.images[0].content_id, "logo@x");
        assert!(r.html.contains("cid:logo@x"), "{}", r.html);
        assert!(r.html.contains("src=cid:fake@x"), "{}", r.html);
        assert!(r.warnings.is_empty());
    }
}
