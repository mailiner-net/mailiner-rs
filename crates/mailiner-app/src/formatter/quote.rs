//! Collapse trailing email quotes behind a script-free `<details>` control.

use regex::Regex;
use std::sync::OnceLock;

/// Label on the collapsed quote disclosure.
pub const SHOW_QUOTED_TEXT: &str = "Show quoted text";

/// Injected into the message shadow root so the toggle is styled after email CSS.
pub const QUOTE_TOGGLE_CSS: &str = r#"
details.mlnr-quote {
  margin: 0.75em 0 0;
}
details.mlnr-quote > summary.mlnr-quote-toggle {
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
    let no_tags = re_html_tag().replace_all(html, "");
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
}
