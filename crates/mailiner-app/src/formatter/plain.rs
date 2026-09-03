//! Plain-text formatter: escape, linkify, wrap in <pre>.

use linkify::{LinkFinder, LinkKind};
use mailiner_core::models::MessagePart;

use super::quote::{split_trailing_plain_quote, wrap_quote_html};
use super::{FormatResult, text_content};

pub fn format_plain(part: &MessagePart) -> Option<FormatResult> {
    let text = text_content(part)?;
    let html = match split_trailing_plain_quote(text) {
        Some((visible, quoted)) => {
            let visible = pre_plain(&linkify_escaped(visible));
            let quoted = wrap_quote_html(&pre_plain(&linkify_escaped(quoted)));
            format!("{visible}{quoted}")
        }
        None => pre_plain(&linkify_escaped(text)),
    };
    Some(FormatResult {
        html,
        prevented_remote_resources: false,
        inlined_part_ids: vec![],
    })
}

fn pre_plain(inner: &str) -> String {
    format!(r#"<pre class="mlnr-plain">{inner}</pre>"#)
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

    fn plain_part(text: &str) -> MessagePart {
        use chrono::Utc;
        use mailiner_core::ids::{FolderId, MessageId, MessagePartId};
        use mailiner_core::models::{MessageContent, PartKind, TransferEncoding};
        let now = Utc::now();
        MessagePart {
            id: MessagePartId::new("p1"),
            envelope_id: MessageId::new(FolderId::new("INBOX"), "1"),
            path: vec!["1".into()],
            kind: PartKind::TextPlain,
            content_type: "text/plain".into(),
            charset: Some("UTF-8".into()),
            content_id: None,
            description: None,
            filename: None,
            encoding: TransferEncoding::SevenBit,
            original_size: None,
            size: text.len() as u64,
            is_attachment: false,
            is_hidden: false,
            content: MessageContent::Text(text.into()),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn wraps_trailing_quote_in_details() {
        let r = format_plain(&plain_part("My reply.\n\n> quoted line\n")).unwrap();
        assert!(r.html.contains("<pre class=\"mlnr-plain\">"), "{r:?}");
        assert!(r.html.contains("My reply."), "{r:?}");
        assert!(r.html.contains("<details class=\"mlnr-quote\">"), "{r:?}");
        assert!(r.html.contains("Show quoted text"), "{r:?}");
        assert!(r.html.contains("&gt; quoted line"), "{r:?}");
        assert!(
            r.html.find("My reply.").unwrap() < r.html.find("<details").unwrap(),
            "{r:?}"
        );
    }

    #[test]
    fn linkifies_inside_collapsed_quote() {
        let r = format_plain(&plain_part(
            "See below.\n> visit https://example.com/quoted\n",
        ))
        .unwrap();
        assert!(r.html.contains("<details class=\"mlnr-quote\">"), "{r:?}");
        let details = &r.html[r.html.find("<details").unwrap()..];
        assert!(
            details.contains("<a href=\"https://example.com/quoted\""),
            "{r:?}"
        );
    }

    #[test]
    fn unquoted_body_has_no_details() {
        let r = format_plain(&plain_part("Just hello.")).unwrap();
        assert!(!r.html.contains("<details"), "{r:?}");
        assert!(r.html.contains("Just hello."), "{r:?}");
    }
}
