//! BODYSTRUCTURE → flat MessagePart list (TS MessageParser parity).

mod attachment;
mod image;
mod multipart;
mod text;

use chrono::Utc;
use mailiner_core::body::BodyPart;
use mailiner_core::ids::{MessageId, MessagePartId};
use mailiner_core::models::{MessageContent, MessagePart, PartKind, TransferEncoding};

use crate::heuristics::is_attachment;

pub const ATTACHMENT_MIME: &str = "x-vnd-mailiner/attachment";

pub struct ParseContext<'a> {
    pub envelope_id: &'a MessageId,
    pub registry: &'a MessageParser,
}

pub trait PartParser: Send + Sync {
    fn mime_types(&self) -> &[&str];
    fn parse(
        &self,
        ctx: &ParseContext<'_>,
        part: &BodyPart,
        part_id: &str,
        path: &[String],
    ) -> Vec<MessagePart>;
}

pub struct MessageParser {
    parsers: Vec<Box<dyn PartParser>>,
}

impl MessageParser {
    pub fn with_defaults() -> Self {
        let mut p = Self {
            parsers: Vec::new(),
        };
        p.add(Box::new(text::TextPlainParser));
        p.add(Box::new(text::TextHtmlParser));
        p.add(Box::new(multipart::MultipartAlternativeParser));
        p.add(Box::new(multipart::MultipartRelatedParser));
        p.add(Box::new(multipart::MultipartMixedParser));
        p.add(Box::new(image::ImageParser));
        p.add(Box::new(attachment::AttachmentParser));
        p
    }

    fn add(&mut self, parser: Box<dyn PartParser>) {
        self.parsers.push(parser);
    }

    pub fn can_parse(&self, mime: &str) -> bool {
        !self.parsers_for(mime).is_empty()
    }

    fn parsers_for(&self, mime: &str) -> Vec<&dyn PartParser> {
        let mime_l = mime.to_ascii_lowercase();
        let mut exact = Vec::new();
        let mut wild = Vec::new();
        let type_wild = mime_l
            .split_once('/')
            .map(|(t, _)| format!("{}/*", t))
            .unwrap_or_default();

        for p in &self.parsers {
            for mt in p.mime_types() {
                let mt_l = mt.to_ascii_lowercase();
                if mt_l == mime_l {
                    exact.push(p.as_ref());
                } else if mt_l == type_wild {
                    wild.push(p.as_ref());
                }
            }
        }
        if !exact.is_empty() {
            exact
        } else {
            wild
        }
    }

    pub fn parse(&self, envelope_id: &MessageId, root: &BodyPart) -> Vec<MessagePart> {
        self.parse_part(envelope_id, root, "", &[])
    }

    pub fn parse_as(
        &self,
        envelope_id: &MessageId,
        part: &BodyPart,
        mime: &str,
        part_id: &str,
        path: &[String],
    ) -> Vec<MessagePart> {
        let ctx = ParseContext {
            envelope_id,
            registry: self,
        };
        for parser in self.parsers_for(mime) {
            let result = parser.parse(&ctx, part, part_id, path);
            if !result.is_empty() {
                return result;
            }
        }
        // Unknown leaf → attachment fallback
        if part.type_ != "multipart" {
            return attachment::AttachmentParser.parse(&ctx, part, part_id, path);
        }
        Vec::new()
    }

    pub fn parse_part(
        &self,
        envelope_id: &MessageId,
        part: &BodyPart,
        part_id: &str,
        path: &[String],
    ) -> Vec<MessagePart> {
        let ct = part.content_type();
        self.parse_as(envelope_id, part, &ct, part_id, path)
    }
}

/// Build a leaf MessagePart from a BodyPart.
pub(crate) fn leaf_part(
    envelope_id: &MessageId,
    part: &BodyPart,
    id: &str,
    path: &[String],
    kind: PartKind,
    force_attachment: Option<bool>,
    force_hidden: Option<bool>,
) -> MessagePart {
    let now = Utc::now();
    let path = if path.is_empty() {
        vec!["TEXT".to_string()]
    } else {
        path.to_vec()
    };

    let is_att = force_attachment.unwrap_or_else(|| is_attachment(part));
    let encoding = part
        .encoding
        .as_deref()
        .map(TransferEncoding::from_wire)
        .unwrap_or(TransferEncoding::SevenBit);

    let original_size = part.size;
    let size = match (original_size, encoding) {
        (Some(s), TransferEncoding::Base64) => (s as f64 / 1.37).round() as u64,
        (Some(s), _) => s,
        (None, _) => 0,
    };

    let mut is_hidden = force_hidden.unwrap_or(false);
    if kind == PartKind::Image {
        // Image always attachment; hide if has CID and not disposition ATTACHMENT
        let disp_att = part
            .disposition
            .as_ref()
            .map(|d| d.type_.eq_ignore_ascii_case("ATTACHMENT"))
            .unwrap_or(false);
        if part.id.is_some() && !disp_att {
            is_hidden = true;
        }
    }

    MessagePart {
        id: MessagePartId::new(id),
        envelope_id: envelope_id.clone(),
        path,
        kind,
        content_type: part.content_type(),
        charset: part.charset().map(|s| s.to_string()),
        content_id: part.id.clone(),
        description: part.description.clone(),
        filename: part.filename().map(|s| s.to_string()),
        encoding,
        original_size,
        size,
        is_attachment: if kind == PartKind::Image || kind == PartKind::Attachment {
            true
        } else {
            is_att
        },
        is_hidden,
        content: MessageContent::Empty,
        created_at: now,
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailiner_core::body::ContentDisposition;
    use mailiner_core::connector::mock_multipart_structure;
    use mailiner_core::ids::FolderId;

    #[test]
    fn alternative_prefers_html() {
        let root = mock_multipart_structure();
        let parser = MessageParser::with_defaults();
        let parts = parser.parse(&MessageId::new(FolderId::new("INBOX"), "1"), &root);
        // Both alternatives stay displayable; formatter prefers HTML unless asked.
        assert!(
            parts.iter().any(|p| p.kind == PartKind::TextHtml),
            "expected html part, got {:?}",
            parts.iter().map(|p| &p.content_type).collect::<Vec<_>>()
        );
        let plain = parts
            .iter()
            .find(|p| p.kind == PartKind::TextPlain)
            .expect("plain alternative should be kept");
        assert_eq!(plain.section(), "1.1");
        assert!(!plain.is_hidden);
        assert!(plain.should_prefetch());

        let html = parts.iter().find(|p| p.kind == PartKind::TextHtml).unwrap();
        assert_eq!(html.section(), "1.2");
        assert!(!html.is_hidden);
        assert!(html.should_prefetch());

        let pdf = parts
            .iter()
            .find(|p| p.kind == PartKind::Attachment)
            .unwrap();
        assert_eq!(pdf.section(), "2");
        assert!(!pdf.should_prefetch());
        assert_eq!(pdf.filename.as_deref(), Some("report.pdf"));
    }

    #[test]
    fn alternative_plain_and_html_are_both_kept() {
        let root = BodyPart {
            type_: "multipart".into(),
            subtype: "alternative".into(),
            subparts: vec![
                BodyPart {
                    type_: "text".into(),
                    subtype: "plain".into(),
                    encoding: Some("7BIT".into()),
                    ..Default::default()
                },
                BodyPart {
                    type_: "text".into(),
                    subtype: "html".into(),
                    encoding: Some("7BIT".into()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let parser = MessageParser::with_defaults();
        let parts = parser.parse(&MessageId::new(FolderId::new("INBOX"), "1"), &root);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].kind, PartKind::TextPlain);
        assert_eq!(parts[0].section(), "1");
        assert!(!parts[0].is_hidden);
        assert_eq!(parts[1].kind, PartKind::TextHtml);
        assert_eq!(parts[1].section(), "2");
        assert!(!parts[1].is_hidden);
    }

    #[test]
    fn single_part_text_uses_text_section() {
        let root = BodyPart {
            type_: "text".into(),
            subtype: "plain".into(),
            encoding: Some("7BIT".into()),
            size: Some(5),
            parameters: [("CHARSET".into(), "UTF-8".into())].into(),
            ..Default::default()
        };
        let parser = MessageParser::with_defaults();
        let parts = parser.parse(&MessageId::new(FolderId::new("INBOX"), "1"), &root);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].section(), "TEXT");
        assert_eq!(parts[0].kind, PartKind::TextPlain);
    }

    #[test]
    fn image_with_cid_hidden() {
        let root = BodyPart {
            type_: "multipart".into(),
            subtype: "related".into(),
            subparts: vec![
                BodyPart {
                    type_: "text".into(),
                    subtype: "html".into(),
                    encoding: Some("7BIT".into()),
                    ..Default::default()
                },
                BodyPart {
                    type_: "image".into(),
                    subtype: "png".into(),
                    encoding: Some("BASE64".into()),
                    size: Some(100),
                    id: Some("<logo@x>".into()),
                    disposition: Some(ContentDisposition {
                        type_: "INLINE".into(),
                        attributes: Default::default(),
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let parser = MessageParser::with_defaults();
        let parts = parser.parse(&MessageId::new(FolderId::new("INBOX"), "1"), &root);
        let img = parts.iter().find(|p| p.kind == PartKind::Image).unwrap();
        assert!(img.is_hidden);
        assert!(img.is_attachment);
        assert!(img.should_prefetch());
        assert_eq!(img.section(), "2");
    }

    #[test]
    fn text_name_only_not_attachment_via_parser() {
        let root = BodyPart {
            type_: "text".into(),
            subtype: "plain".into(),
            parameters: [("NAME".into(), "readme.txt".into())].into(),
            encoding: Some("7BIT".into()),
            ..Default::default()
        };
        let parser = MessageParser::with_defaults();
        let parts = parser.parse(&MessageId::new(FolderId::new("INBOX"), "1"), &root);
        assert_eq!(parts.len(), 1);
        assert!(!parts[0].is_attachment);
        assert_eq!(parts[0].kind, PartKind::TextPlain);
    }

    #[test]
    fn unknown_type_becomes_attachment() {
        let root = BodyPart {
            type_: "application".into(),
            subtype: "octet-stream".into(),
            encoding: Some("BASE64".into()),
            size: Some(10),
            ..Default::default()
        };
        let parser = MessageParser::with_defaults();
        let parts = parser.parse(&MessageId::new(FolderId::new("INBOX"), "1"), &root);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].kind, PartKind::Attachment);
        assert!(parts[0].is_attachment);
    }

    #[test]
    fn message_rfc822_as_attachment() {
        let root = BodyPart {
            type_: "message".into(),
            subtype: "rfc822".into(),
            encoding: Some("7BIT".into()),
            nested_message: Some(Box::new(BodyPart {
                type_: "text".into(),
                subtype: "plain".into(),
                ..Default::default()
            })),
            ..Default::default()
        };
        let parser = MessageParser::with_defaults();
        // message/rfc822 is not text/image/multipart — falls through to attachment
        let parts = parser.parse(&MessageId::new(FolderId::new("INBOX"), "1"), &root);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].kind, PartKind::Attachment);
    }
}
