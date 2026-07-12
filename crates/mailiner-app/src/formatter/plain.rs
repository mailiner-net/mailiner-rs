//! Plain-text formatter: escape, linkify, wrap in <pre>.

use linkify::{LinkFinder, LinkKind};
use mailiner_core::models::MessagePart;

use super::{text_content, FormatResult};

pub fn format_plain(part: &MessagePart) -> Option<FormatResult> {
    let text = text_content(part)?;
    let linked = linkify_escaped(text);
    Some(FormatResult {
        html: format!(r#"<pre class="mlnr-plain">{linked}</pre>"#),
        prevented_remote_resources: false,
        inlined_part_ids: vec![],
    })
}

fn escape_html(s: &str) -> String {
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

fn linkify_escaped(text: &str) -> String {
    let finder = LinkFinder::new();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for link in finder.links(text) {
        // Skip email-domain false positives after '@'
        if link.start() > 0 && text.as_bytes()[link.start() - 1] == b'@' {
            continue;
        }
        if !matches!(link.kind(), LinkKind::Url) {
            continue;
        }
        out.push_str(&escape_html(&text[last..link.start()]));
        let url = link.as_str();
        // Only allow http(s) and similar safe schemes for href
        let href = if url.contains("://") {
            url
        } else {
            // linkify may omit scheme; prefix https
            // Keep display text as original span
            url
        };
        let href_attr = escape_html(if href.contains("://") {
            href
        } else {
            // relative-looking host: force https
            // Actually linkify Url includes scheme usually
            href
        });
        let display = escape_html(url);
        let href_final = if href_attr.starts_with("http://")
            || href_attr.starts_with("https://")
            || href_attr.starts_with("ftp://")
        {
            href_attr
        } else {
            format!("https://{href_attr}")
        };
        out.push_str(&format!(
            r#"<a href="{href_final}" target="_blank" rel="noopener noreferrer" referrerpolicy="no-referrer">{display}</a>"#
        ));
        last = link.end();
    }
    out.push_str(&escape_html(&text[last..]));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html() {
        assert_eq!(escape_html("<b>&"), "&lt;b&gt;&amp;");
    }

    #[test]
    fn no_link_after_at() {
        let s = linkify_escaped("user@example.com");
        // email should not become a bare domain link mid-address for the domain part
        // linkify may still find example.com — we skip if preceded by @
        assert!(!s.contains("<a href=\"https://example.com\""));
    }
}
