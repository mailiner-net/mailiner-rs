//! Message formatters: plain text + safe HTML (viewer policy).

mod html;
mod plain;
mod sanitize;

use mailiner_core::models::{MessageContent, MessagePart, PartKind};

pub use html::format_html;
pub use plain::format_plain;

#[derive(Debug, Clone, Default)]
pub struct FormatOptions {
    pub allow_remote_resources: bool,
}

#[derive(Debug, Clone)]
pub struct FormatResult {
    /// Sanitized HTML. The viewer parses this as a document (`DOMParser`) and
    /// adopts `<html>` into an open shadow root.
    pub html: String,
    /// True if any remote resource attribute was stripped.
    pub prevented_remote_resources: bool,
    /// Part ids that were inlined via cid: and should stay hidden.
    pub inlined_part_ids: Vec<String>,
}

pub struct MessageFormatter {
    pub options: FormatOptions,
    pub prevented_remote_resources: bool,
}

impl MessageFormatter {
    pub fn new(options: FormatOptions) -> Self {
        Self {
            options,
            prevented_remote_resources: false,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(FormatOptions::default())
    }

    /// First non-hidden part that a formatter accepts wins.
    pub fn format(&mut self, parts: &[MessagePart]) -> Option<FormatResult> {
        for part in parts.iter().filter(|p| !p.is_hidden) {
            if let Some(result) = self.format_part(part, parts) {
                self.prevented_remote_resources |= result.prevented_remote_resources;
                return Some(result);
            }
        }
        None
    }

    fn format_part(&self, part: &MessagePart, all: &[MessagePart]) -> Option<FormatResult> {
        match part.kind {
            PartKind::TextHtml => format_html(part, all, &self.options),
            PartKind::TextPlain | PartKind::Other => {
                // Prefer plain formatter for text/plain; also fallback for other text.
                if part.content_type.to_ascii_lowercase().starts_with("text/") {
                    format_plain(part)
                } else {
                    None
                }
            }
            PartKind::Image | PartKind::Attachment => None,
        }
    }
}

pub(crate) fn text_content(part: &MessagePart) -> Option<&str> {
    match &part.content {
        MessageContent::Text(s) => Some(s.as_str()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use mailiner_core::ids::{MessageId, MessagePartId};
    use mailiner_core::models::{MessageContent, PartKind, TransferEncoding};

    fn part(kind: PartKind, ct: &str, text: &str) -> MessagePart {
        let now = Utc::now();
        MessagePart {
            id: MessagePartId::new("p1"),
            envelope_id: MessageId::new("1"),
            path: vec!["TEXT".into()],
            kind,
            content_type: ct.into(),
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
    fn prefers_html_when_present() {
        let parts = vec![part(
            PartKind::TextHtml,
            "text/html",
            "<p>Hi<script>alert(1)</script></p>",
        )];
        let mut f = MessageFormatter::with_defaults();
        let r = f.format(&parts).unwrap();
        assert!(r.html.contains("Hi"));
        assert!(!r.html.to_ascii_lowercase().contains("script"));
    }

    #[test]
    fn plain_linkifies() {
        let parts = vec![part(
            PartKind::TextPlain,
            "text/plain",
            "See https://example.com/path for details",
        )];
        let mut f = MessageFormatter::with_defaults();
        let r = f.format(&parts).unwrap();
        assert!(r.html.contains("<a href=\"https://example.com/path\""));
        assert!(r.html.contains("rel=\"noopener noreferrer\""));
    }

    #[test]
    fn blocks_remote_img_by_default() {
        let parts = vec![part(
            PartKind::TextHtml,
            "text/html",
            r#"<p><img src="https://tracker.example/pixel.gif"></p>"#,
        )];
        let mut f = MessageFormatter::with_defaults();
        let r = f.format(&parts).unwrap();
        assert!(r.prevented_remote_resources);
        assert!(!r.html.contains("https://tracker.example"));
    }

    #[test]
    fn strips_javascript_urls() {
        let parts = vec![part(
            PartKind::TextHtml,
            "text/html",
            r#"<a href="javascript:alert(1)">x</a>"#,
        )];
        let mut f = MessageFormatter::with_defaults();
        let r = f.format(&parts).unwrap();
        assert!(!r.html.to_ascii_lowercase().contains("javascript:"));
    }
}
