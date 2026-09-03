//! Message formatters: plain text + safe HTML (viewer policy).

mod html;
mod plain;
pub(crate) mod quote;
mod sanitize;

use mailiner_core::models::{MessageContent, MessagePart, PartKind};

pub use html::format_html;
pub use plain::format_plain;

#[derive(Debug, Clone, Default)]
pub struct FormatOptions {
    pub allow_remote_resources: bool,
    /// Prefer a non-hidden `text/plain` part over HTML when both exist.
    pub prefer_plain: bool,
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

/// Clear decoded payloads on parts that were inlined as `data:` URLs.
///
/// Keeps metadata (id, type, filename, sizes) so the part can still be listed.
pub fn drop_inlined_payloads(parts: &mut [MessagePart], inlined_part_ids: &[String]) {
    if inlined_part_ids.is_empty() {
        return;
    }
    for part in parts {
        if inlined_part_ids.iter().any(|id| part.id.as_str() == id) {
            part.content = MessageContent::Empty;
        }
    }
}

/// Keep referenced CID payloads for reply/forward; drop the rest.
///
/// Referenced parts are retained up to [`mailiner_composer::caps::MAX_DRAFT_BYTES`]
/// so a large newsletter cannot pin unbounded decoded binaries in the viewer.
pub fn retain_reply_cid_payloads(parts: &mut [MessagePart], referenced_ids: &[String]) {
    let mut kept = 0u64;
    for part in parts {
        let referenced = referenced_ids.iter().any(|id| part.id.as_str() == id);
        if referenced {
            let size = match &part.content {
                MessageContent::Binary(b) => b.len() as u64,
                MessageContent::Text(t) => t.len() as u64,
                MessageContent::Empty => 0,
            };
            if kept.saturating_add(size) > mailiner_composer::caps::MAX_DRAFT_BYTES {
                part.content = MessageContent::Empty;
            } else {
                kept = kept.saturating_add(size);
            }
            continue;
        }
        if !part.is_display_part() {
            part.content = MessageContent::Empty;
        }
    }
}

impl FormatResult {
    /// Drop Binary/Text on [`Self::inlined_part_ids`] after a successful format.
    pub fn drop_inlined_payloads(&self, parts: &mut [MessagePart]) {
        drop_inlined_payloads(parts, &self.inlined_part_ids);
    }
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

    /// Prefer HTML unless `prefer_plain`. Falls back to the first formatable part.
    pub fn format(&mut self, parts: &[MessagePart]) -> Option<FormatResult> {
        let preferred = if self.options.prefer_plain {
            PartKind::TextPlain
        } else {
            PartKind::TextHtml
        };
        if let Some(result) = self.format_first(
            parts.iter().filter(|p| !p.is_hidden && p.kind == preferred),
            parts,
        ) {
            return Some(result);
        }
        self.format_first(parts.iter().filter(|p| !p.is_hidden), parts)
    }

    fn format_first<'a>(
        &mut self,
        candidates: impl Iterator<Item = &'a MessagePart>,
        all: &[MessagePart],
    ) -> Option<FormatResult> {
        for part in candidates {
            if let Some(result) = self.format_part(part, all) {
                self.prevented_remote_resources |= result.prevented_remote_resources;
                return Some(result);
            }
        }
        None
    }

    fn format_part(&self, part: &MessagePart, all: &[MessagePart]) -> Option<FormatResult> {
        match part.kind {
            PartKind::TextHtml => format_html(part, all, &self.options),
            PartKind::TextPlain => {
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
    use mailiner_core::ids::{FolderId, MessageId, MessagePartId};
    use mailiner_core::models::{MessageContent, PartKind, TransferEncoding};

    fn part(kind: PartKind, ct: &str, text: &str) -> MessagePart {
        let now = Utc::now();
        MessagePart {
            id: MessagePartId::new("p1"),
            envelope_id: MessageId::new(FolderId::new("INBOX"), "1"),
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

    #[test]
    fn html_and_plain_uses_html_by_default() {
        // Parser emits alternatives in document order (plain then HTML).
        for parts in [
            vec![
                part(PartKind::TextHtml, "text/html", "<p>HTML body</p>"),
                part(PartKind::TextPlain, "text/plain", "PLAIN body"),
            ],
            vec![
                part(PartKind::TextPlain, "text/plain", "PLAIN body"),
                part(PartKind::TextHtml, "text/html", "<p>HTML body</p>"),
            ],
        ] {
            let mut f = MessageFormatter::with_defaults();
            let r = f.format(&parts).unwrap();
            assert!(r.html.contains("HTML body"));
            assert!(!r.html.contains("PLAIN body"));
        }
    }

    #[test]
    fn html_and_plain_uses_plain_when_preferred() {
        let parts = vec![
            part(PartKind::TextHtml, "text/html", "<p>HTML body</p>"),
            part(PartKind::TextPlain, "text/plain", "PLAIN body"),
        ];
        let mut f = MessageFormatter::new(FormatOptions {
            allow_remote_resources: false,
            prefer_plain: true,
        });
        let r = f.format(&parts).unwrap();
        assert!(r.html.contains("PLAIN body"));
        assert!(!r.html.contains("HTML body"));
    }

    fn png_part(id: &str, cid: &str, bytes: &[u8]) -> MessagePart {
        let now = Utc::now();
        MessagePart {
            id: MessagePartId::new(id),
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
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn format_then_drop_clears_inlined_binary() {
        let html = part(
            PartKind::TextHtml,
            "text/html",
            r#"<img src="cid:logo@x"><img src="cid:unused@x">"#,
        );
        let inlined = png_part("img", "<logo@x>", b"\x89PNG");
        let leftover = png_part("other", "<other@x>", b"\x89PNG extra");
        let mut parts = vec![html, inlined, leftover];
        let mut f = MessageFormatter::with_defaults();
        let r = f.format(&parts).unwrap();
        assert!(r.html.contains("data:image/png;base64,"));
        assert!(r.inlined_part_ids.iter().any(|id| id == "img"));
        assert!(!r.inlined_part_ids.iter().any(|id| id == "other"));

        r.drop_inlined_payloads(&mut parts);

        assert!(matches!(parts[1].content, MessageContent::Empty));
        assert!(parts[1].is_hidden);
        assert_eq!(parts[1].filename.as_deref(), Some("logo.png"));
        assert_eq!(parts[1].content_type, "image/png");
        assert_eq!(parts[1].content_id.as_deref(), Some("<logo@x>"));
        assert_eq!(parts[1].size, 4);
        assert!(matches!(parts[2].content, MessageContent::Binary(_)));
        assert!(matches!(parts[0].content, MessageContent::Text(_)));

        // Viewer caches HTML because a later format cannot rebuild data: URLs.
        let r2 = f.format(&parts).unwrap();
        assert!(!r2.html.contains("data:image/png;base64,"));
    }

    #[test]
    fn drop_is_noop_without_inlined_ids() {
        let mut parts = vec![png_part("img", "<logo@x>", b"\x89PNG")];
        drop_inlined_payloads(&mut parts, &[]);
        assert!(matches!(parts[0].content, MessageContent::Binary(_)));
    }

    #[test]
    fn retain_keeps_referenced_and_drops_unused() {
        let html = part(PartKind::TextHtml, "text/html", r#"<img src="cid:logo@x">"#);
        let inlined = png_part("img", "<logo@x>", b"\x89PNG");
        let leftover = png_part("other", "<other@x>", b"\x89PNG extra");
        let mut parts = vec![html, inlined, leftover];
        retain_reply_cid_payloads(&mut parts, &["img".into()]);
        assert!(matches!(parts[0].content, MessageContent::Text(_)));
        assert!(matches!(parts[1].content, MessageContent::Binary(_)));
        assert!(matches!(parts[2].content, MessageContent::Empty));
    }

    #[test]
    fn retain_caps_aggregate_referenced_bytes() {
        let too_big = vec![0u8; (mailiner_composer::caps::MAX_DRAFT_BYTES as usize) + 1];
        let mut parts = vec![png_part("img", "<logo@x>", &too_big)];
        retain_reply_cid_payloads(&mut parts, &["img".into()]);
        assert!(matches!(parts[0].content, MessageContent::Empty));
    }

    #[test]
    fn drop_preserves_visible_attachment_flag() {
        let mut parts = vec![png_part("img", "<logo@x>", b"\x89PNG")];
        parts[0].is_hidden = false;
        drop_inlined_payloads(&mut parts, &["img".into()]);
        assert!(matches!(parts[0].content, MessageContent::Empty));
        assert!(!parts[0].is_hidden);
    }
}
