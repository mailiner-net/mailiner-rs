//! Collapse trailing email quotes behind a script-free `<details>` control.

use regex::Regex;
use std::sync::OnceLock;

/// Label on the collapsed quote disclosure.
pub const SHOW_QUOTED_TEXT: &str = "Show quoted text";

/// Injected into the message shadow root so the toggle is styled after email CSS.
pub const QUOTE_TOGGLE_CSS: &str = r#"
details.mlnr-quote {
  display: block !important;
  margin: 0.75em 0 0;
}
details.mlnr-quote > summary.mlnr-quote-toggle {
  display: list-item !important;
  cursor: pointer;
  font-family: system-ui, sans-serif;
  font-size: 13px;
  font-weight: 500;
  line-height: 1.4;
  user-select: none;
  opacity: 0.75;
}
details.mlnr-quote > summary.mlnr-quote-toggle:hover {
  opacity: 1;
}
"#;

/// Split off a trailing `>`-prefixed quote (and its attribution) when the
/// message also has visible reply text.
pub fn split_trailing_plain_quote(text: &str) -> Option<(&str, &str)> {
    if text.is_empty() {
        return None;
    }
    let lines = line_spans(text);
    let last_content = lines
        .iter()
        .rposition(|&span| !is_blank(line_content(text, span)))?;
    if !is_quoted_line(line_content(text, lines[last_content])) {
        return None;
    }

    let mut first_quote = last_content;
    let mut idx = last_content;
    loop {
        let content = line_content(text, lines[idx]);
        if is_quoted_line(content) {
            first_quote = idx;
        } else if !is_blank(content) {
            break;
        }
        if idx == 0 {
            return None;
        }
        idx -= 1;
    }

    let mut collapse_from = first_quote;
    if looks_like_attribution(line_content(text, lines[idx])) {
        collapse_from = idx;
    }
    if collapse_from == 0
        || lines[..collapse_from]
            .iter()
            .all(|&span| is_blank(line_content(text, span)))
    {
        return None;
    }

    let split_at = lines[collapse_from].0;
    Some((&text[..split_at], &text[split_at..]))
}

/// Wrap trailing top-level `<blockquote>` regions in a collapsed `<details>`.
///
/// Leaves the document unchanged when the quote is the entire body (so a
/// forwarded-only message stays readable).
pub fn collapse_trailing_blockquotes(html: &str) -> String {
    let quotes = top_level_blockquote_spans(html);
    let Some((start, end)) = trailing_blockquote_range(html, &quotes) else {
        return html.to_string();
    };
    wrap_quote_fragment(html, start, end)
}

pub fn wrap_quote_html(inner: &str) -> String {
    format!(
        r#"<details class="mlnr-quote"><summary class="mlnr-quote-toggle">{SHOW_QUOTED_TEXT}</summary>{inner}</details>"#
    )
}

fn wrap_quote_fragment(html: &str, start: usize, end: usize) -> String {
    let mut out = String::with_capacity(html.len() + 96);
    out.push_str(&html[..start]);
    out.push_str(&wrap_quote_html(&html[start..end]));
    out.push_str(&html[end..]);
    out
}

fn line_spans(text: &str) -> Vec<(usize, usize)> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            lines.push((start, i + 1));
            start = i + 1;
        }
    }
    if start < text.len() {
        lines.push((start, text.len()));
    }
    if lines.is_empty() {
        lines.push((0, 0));
    }
    lines
}

fn line_content(text: &str, span: (usize, usize)) -> &str {
    text[span.0..span.1].trim_end_matches(['\n', '\r'])
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

fn is_quoted_line(line: &str) -> bool {
    line.trim_start().starts_with('>')
}

fn looks_like_attribution(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 8 || trimmed.len() > 300 || is_quoted_line(trimmed) {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    const MARKERS: &[&str] = &[
        "wrote:",
        "wrote :",
        "a écrit",
        "a ecrit",
        "schrieb",
        "escribi",
        "original message",
        "forwarded message",
        "mensaje original",
    ];
    MARKERS.iter().any(|m| lower.contains(m))
        || (trimmed.starts_with("----") && trimmed.chars().filter(|&c| c == '-').count() >= 4)
}

fn re_blockquote_tag() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<blockquote\b[^>]*>|</blockquote\s*>").unwrap())
}

fn re_style_block() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<style\b[^>]*>.*?</style>").unwrap())
}

fn re_html_tag() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<[^>]+>").unwrap())
}

fn re_replaced_open() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?is)<(img|hr|picture|video|audio|canvas|object|embed|iframe)\b([^>]*?)/?>"#)
            .unwrap()
    })
}

fn attr_value<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let lower = attrs.to_ascii_lowercase();
    let needle = name.to_ascii_lowercase();
    let mut rest = lower.as_str();
    let mut offset = 0;
    while let Some(pos) = rest.find(&needle) {
        let at = offset + pos;
        let before_ok = at == 0
            || attrs
                .as_bytes()
                .get(at - 1)
                .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'/'));
        let after = at + needle.len();
        if before_ok {
            let tail = attrs[after..].trim_start();
            if let Some(eq) = tail.strip_prefix('=') {
                let eq = eq.trim_start();
                return Some(quoted_or_token(eq));
            }
        }
        offset = at + needle.len();
        rest = &lower[offset..];
    }
    None
}

fn quoted_or_token(s: &str) -> &str {
    let bytes = s.as_bytes();
    match bytes.first() {
        Some(q @ (b'"' | b'\'')) => s[1..]
            .find(*q as char)
            .map(|end| &s[1..1 + end])
            .unwrap_or(&s[1..]),
        _ => s
            .split(|c: char| c.is_ascii_whitespace() || c == '/')
            .next()
            .unwrap_or(""),
    }
}

fn parse_px(raw: Option<&str>) -> Option<u32> {
    let raw = raw?;
    let digits: String = raw
        .trim()
        .trim_end_matches("px")
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn style_decl_hidden(style: &str) -> bool {
    let lower = style.to_ascii_lowercase();
    for decl in lower.split(';') {
        let decl = decl.trim();
        if let Some(rest) = decl.strip_prefix("display:") {
            if rest.trim().starts_with("none") {
                return true;
            }
        } else if let Some(rest) = decl.strip_prefix("visibility:")
            && (rest.trim().starts_with("hidden") || rest.trim().starts_with("collapse"))
        {
            return true;
        }
    }
    false
}

fn has_boolean_hidden(attrs: &str) -> bool {
    let lower = attrs.to_ascii_lowercase();
    let mut rest = lower.as_str();
    while let Some(pos) = rest.find("hidden") {
        let before = rest[..pos].chars().next_back();
        // Only a standalone attribute token (`hidden` / `hidden="..."`), not
        // `aria-hidden` or a URL path segment like `/hidden/`.
        if !before.is_none_or(|c| c.is_ascii_whitespace()) {
            rest = &rest[pos + 6..];
            continue;
        }
        let after = rest[pos + 6..].chars().next();
        if after.is_none_or(|c| c.is_ascii_whitespace() || matches!(c, '=' | '/')) {
            return true;
        }
        rest = &rest[pos + 6..];
    }
    false
}

fn replaced_element_is_visible(tag: &str, attrs: &str) -> bool {
    if has_boolean_hidden(attrs) {
        return false;
    }
    if attr_value(attrs, "style")
        .as_deref()
        .is_some_and(style_decl_hidden)
    {
        return false;
    }
    let tag = tag.to_ascii_lowercase();
    match tag.as_str() {
        "img" => {
            let src = attr_value(attrs, "src").unwrap_or_default();
            let srcset = attr_value(attrs, "srcset").unwrap_or_default();
            if src.trim().is_empty() && srcset.trim().is_empty() {
                return false;
            }
            match (
                parse_px(attr_value(attrs, "width")),
                parse_px(attr_value(attrs, "height")),
            ) {
                (Some(w), Some(h)) if w <= 1 && h <= 1 => false,
                _ => true,
            }
        }
        "hr" | "picture" | "canvas" | "video" | "audio" | "object" | "embed" | "iframe" => true,
        _ => !attr_value(attrs, "src")
            .unwrap_or_default()
            .trim()
            .is_empty(),
    }
}

fn has_visible_replaced(html: &str) -> bool {
    re_replaced_open().captures_iter(html).any(|caps| {
        let tag = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let attrs = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        replaced_element_is_visible(tag, attrs)
    })
}

fn style_ranges(html: &str) -> Vec<(usize, usize)> {
    re_style_block()
        .find_iter(html)
        .map(|m| (m.start(), m.end()))
        .collect()
}

fn in_ranges(pos: usize, ranges: &[(usize, usize)]) -> bool {
    ranges.iter().any(|&(a, b)| pos >= a && pos < b)
}

fn top_level_blockquote_spans(html: &str) -> Vec<(usize, usize)> {
    let skip = style_ranges(html);
    let mut stack = Vec::new();
    let mut top = Vec::new();
    for m in re_blockquote_tag().find_iter(html) {
        if in_ranges(m.start(), &skip) {
            continue;
        }
        let token = m.as_str();
        let is_close = token.as_bytes().get(1) == Some(&b'/');
        if is_close {
            if let Some(open_start) = stack.pop()
                && stack.is_empty()
            {
                top.push((open_start, m.end()));
            }
        } else {
            stack.push(m.start());
        }
    }
    top
}

fn trailing_blockquote_range(html: &str, quotes: &[(usize, usize)]) -> Option<(usize, usize)> {
    if quotes.is_empty() {
        return None;
    }
    let last = quotes[quotes.len() - 1];
    if !is_insignificant_html(&html[last.1..]) {
        return None;
    }
    let mut first = quotes.len() - 1;
    while first > 0 {
        let prev = quotes[first - 1];
        if is_insignificant_html(&html[prev.1..quotes[first].0]) {
            first -= 1;
        } else {
            break;
        }
    }
    let start = quotes[first].0;
    if is_insignificant_html(&html[..start]) {
        return None;
    }
    Some((start, last.1))
}

fn is_insignificant_html(html: &str) -> bool {
    let without_styles = re_style_block().replace_all(html, "");
    if has_visible_replaced(&without_styles) {
        return false;
    }
    let no_tags = re_html_tag().replace_all(&without_styles, "");
    no_tags
        .replace("&nbsp;", " ")
        .replace("&#160;", " ")
        .replace("&#xA0;", " ")
        .replace("&#xa0;", " ")
        .chars()
        .all(|c| c.is_whitespace() || c == '\u{a0}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_css_targets_injected_markup() {
        assert!(QUOTE_TOGGLE_CSS.contains("details.mlnr-quote"));
        assert!(QUOTE_TOGGLE_CSS.contains("mlnr-quote-toggle"));
        assert!(QUOTE_TOGGLE_CSS.contains("display: block"));
        assert!(QUOTE_TOGGLE_CSS.contains("display: list-item"));
    }

    #[test]
    fn no_quote_is_none() {
        assert_eq!(split_trailing_plain_quote("Just a reply."), None);
    }

    #[test]
    fn trailing_quote_split() {
        let text = "Thanks.\n\n> Hello\n> How are you?\n";
        let (visible, quoted) = split_trailing_plain_quote(text).unwrap();
        assert_eq!(visible, "Thanks.\n\n");
        assert!(quoted.starts_with('>'), "{quoted}");
        assert!(quoted.contains("How are you?"), "{quoted}");
    }

    #[test]
    fn includes_wrote_attribution() {
        let text = "Sounds good.\n\nOn Mon, Jane wrote:\n> Let's meet tomorrow.\n";
        let (visible, quoted) = split_trailing_plain_quote(text).unwrap();
        assert_eq!(visible, "Sounds good.\n\n");
        assert!(quoted.contains("On Mon, Jane wrote:"), "{quoted}");
        assert!(quoted.contains("> Let's meet tomorrow."), "{quoted}");
    }

    #[test]
    fn whole_message_quote_stays_visible() {
        assert_eq!(
            split_trailing_plain_quote("> only a quote\n> still\n"),
            None
        );
        assert_eq!(
            split_trailing_plain_quote("On Mon, Jane wrote:\n> only a quote\n"),
            None
        );
    }

    #[test]
    fn mid_message_quote_not_collapsed() {
        let text = "See this:\n\n> citation\n\nI agree.\n";
        assert_eq!(split_trailing_plain_quote(text), None);
    }

    #[test]
    fn nested_gt_prefix_is_quoted() {
        let text = "Reply\n> level1\n>> level2\n";
        let (visible, quoted) = split_trailing_plain_quote(text).unwrap();
        assert_eq!(visible, "Reply\n");
        assert!(quoted.contains(">> level2"), "{quoted}");
    }

    #[test]
    fn greater_than_in_sentence_is_not_a_quote() {
        assert_eq!(split_trailing_plain_quote("n > 3 is the threshold"), None);
    }

    #[test]
    fn html_trailing_blockquote() {
        let html = "<p>Thanks.</p><blockquote><p>Hello</p></blockquote>";
        let out = collapse_trailing_blockquotes(html);
        assert!(out.contains("<details class=\"mlnr-quote\">"), "{out}");
        assert!(out.contains("Show quoted text"), "{out}");
        assert!(out.contains("<p>Thanks.</p>"), "{out}");
        assert!(
            out.contains("<blockquote><p>Hello</p></blockquote>"),
            "{out}"
        );
        assert!(
            !out.contains("<details")
                || out.find("<p>Thanks.</p>").unwrap() < out.find("<details").unwrap(),
            "{out}"
        );
    }

    #[test]
    fn html_nested_blockquotes_wrap_as_one() {
        let html = "<p>Reply</p><blockquote>a<blockquote>b</blockquote></blockquote>";
        let out = collapse_trailing_blockquotes(html);
        assert_eq!(out.matches("<details").count(), 1, "{out}");
        assert!(
            out.contains("<blockquote>a<blockquote>b</blockquote></blockquote>"),
            "{out}"
        );
    }

    #[test]
    fn html_consecutive_trailing_blockquotes_wrap_together() {
        let html = "<p>Reply</p><blockquote>one</blockquote><blockquote>two</blockquote>";
        let out = collapse_trailing_blockquotes(html);
        assert_eq!(out.matches("<details").count(), 1, "{out}");
        let details = out.find("<details").unwrap();
        assert!(
            out[details..].contains("one") && out[details..].contains("two"),
            "{out}"
        );
    }

    #[test]
    fn html_blockquote_with_reply_after_stays_open() {
        let html = "<blockquote>old</blockquote><p>New reply</p>";
        let out = collapse_trailing_blockquotes(html);
        assert!(!out.contains("<details"), "{out}");
    }

    #[test]
    fn html_entire_body_blockquote_stays_open() {
        let html = "<blockquote><p>Forwarded only</p></blockquote>";
        let out = collapse_trailing_blockquotes(html);
        assert!(!out.contains("<details"), "{out}");
    }

    #[test]
    fn html_ignores_blockquote_string_inside_style() {
        let html = "<style>blockquote {color:red}</style><p>Hi</p>";
        let out = collapse_trailing_blockquotes(html);
        assert!(!out.contains("<details"), "{out}");
        assert!(out.contains("blockquote {color:red}"), "{out}");
    }

    #[test]
    fn html_style_preamble_does_not_make_quote_only_body_collapsible() {
        let html = "<style>body {color:red}</style><blockquote><p>Forwarded only</p></blockquote>";
        let out = collapse_trailing_blockquotes(html);
        assert!(!out.contains("<details"), "{out}");
        assert!(out.contains("Forwarded only"), "{out}");
    }

    #[test]
    fn html_image_after_blockquote_keeps_quote_open() {
        let html = "<blockquote>old</blockquote><img src=\"cid:sig\">";
        let out = collapse_trailing_blockquotes(html);
        assert!(!out.contains("<details"), "{out}");
        assert!(out.contains("<img"), "{out}");
    }

    #[test]
    fn html_stripped_tracking_img_after_blockquote_is_collapsed() {
        let html = "<p>Thanks.</p><blockquote>old</blockquote><img>";
        let out = collapse_trailing_blockquotes(html);
        assert!(out.contains("<details class=\"mlnr-quote\">"), "{out}");
        assert!(out.contains("<img>"), "{out}");
    }

    #[test]
    fn html_hidden_or_1px_img_after_blockquote_is_collapsed() {
        let hidden =
            "<p>Thanks.</p><blockquote>old</blockquote><img src=\"https://t/p.gif\" hidden>";
        assert!(
            collapse_trailing_blockquotes(hidden).contains("<details"),
            "{hidden}"
        );
        let pixel = "<p>Thanks.</p><blockquote>old</blockquote><img src=\"https://t/p.gif\" width=\"1\" height=\"1\">";
        assert!(
            collapse_trailing_blockquotes(pixel).contains("<details"),
            "{pixel}"
        );
        let none = r#"<p>Thanks.</p><blockquote>old</blockquote><img src="https://t/p.gif" style="display:none">"#;
        assert!(
            collapse_trailing_blockquotes(none).contains("<details"),
            "{none}"
        );
    }

    #[test]
    fn html_video_without_src_after_blockquote_stays_open() {
        let html = "<p>Thanks.</p><blockquote>old</blockquote><video poster=\"p.jpg\"></video>";
        let out = collapse_trailing_blockquotes(html);
        assert!(!out.contains("<details"), "{out}");
    }

    #[test]
    fn html_img_url_containing_hidden_stays_open() {
        let html = "<p>Thanks.</p><blockquote>old</blockquote><img src=\"https://cdn.example/hidden/sig.png\">";
        let out = collapse_trailing_blockquotes(html);
        assert!(!out.contains("<details"), "{out}");
    }
}
