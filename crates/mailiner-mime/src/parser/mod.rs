//! BODYSTRUCTURE → flat MessagePart list (TS MessageParser parity).

mod attachment;
mod image;
mod multipart;
mod rfc822;
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
        p.add(Box::new(text::TextCalendarParser));
        p.add(Box::new(multipart::MultipartAlternativeParser));
        p.add(Box::new(multipart::MultipartRelatedParser));
        p.add(Box::new(multipart::MultipartEncryptedParser));
        p.add(Box::new(multipart::MultipartSignedParser));
        p.add(Box::new(multipart::MultipartMixedParser));
        p.add(Box::new(image::ImageParser));
        p.add(Box::new(rfc822::MessageRfc822Parser));
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
        // Unknown leaf → calendar if it looks like an invite, else attachment.
        if part.type_ != "multipart" {
            if part.is_calendar() {
                return text::TextCalendarParser.parse(&ctx, part, part_id, path);
            }
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
        nested_in: None,
        nested_headers: None,
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
    fn message_rfc822_root_is_nested_body() {
        let root = BodyPart {
            type_: "message".into(),
            subtype: "rfc822".into(),
            encoding: Some("7BIT".into()),
            nested_message: Some(Box::new(BodyPart {
                type_: "text".into(),
                subtype: "plain".into(),
                encoding: Some("7BIT".into()),
                ..Default::default()
            })),
            ..Default::default()
        };
        let parser = MessageParser::with_defaults();
        // Root message/rfc822 is the message itself — only the nested body.
        let parts = parser.parse(&MessageId::new(FolderId::new("INBOX"), "1"), &root);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].kind, PartKind::TextPlain);
        assert_eq!(parts[0].section(), "1");
        assert!(parts[0].nested_in.is_none());
    }

    fn rfc822_fixture() -> BodyPart {
        use mailiner_core::models::{EmailAddr, EmailAddress, NestedMessageHeaders};

        BodyPart {
            type_: "multipart".into(),
            subtype: "mixed".into(),
            subparts: vec![
                BodyPart {
                    type_: "text".into(),
                    subtype: "plain".into(),
                    encoding: Some("7BIT".into()),
                    ..Default::default()
                },
                BodyPart {
                    type_: "message".into(),
                    subtype: "rfc822".into(),
                    encoding: Some("7BIT".into()),
                    disposition: Some(ContentDisposition {
                        type_: "ATTACHMENT".into(),
                        attributes: [("FILENAME".into(), "note.eml".into())].into(),
                    }),
                    nested_headers: Some(NestedMessageHeaders {
                        subject: Some("Inner subject".into()),
                        from: Some(EmailAddress::List(vec![EmailAddr {
                            name: Some("Ada".into()),
                            email: Some("ada@example.com".into()),
                        }])),
                        to: None,
                        cc: None,
                        date: None,
                    }),
                    nested_message: Some(Box::new(BodyPart {
                        type_: "multipart".into(),
                        subtype: "mixed".into(),
                        subparts: vec![
                            BodyPart {
                                type_: "text".into(),
                                subtype: "html".into(),
                                encoding: Some("7BIT".into()),
                                ..Default::default()
                            },
                            BodyPart {
                                type_: "application".into(),
                                subtype: "pdf".into(),
                                encoding: Some("BASE64".into()),
                                disposition: Some(ContentDisposition {
                                    type_: "ATTACHMENT".into(),
                                    attributes: [("FILENAME".into(), "inner.pdf".into())].into(),
                                }),
                                ..Default::default()
                            },
                        ],
                        ..Default::default()
                    })),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn message_rfc822_nested_parts_use_imap_sections() {
        let parser = MessageParser::with_defaults();
        let parts = parser.parse(
            &MessageId::new(FolderId::new("INBOX"), "1"),
            &rfc822_fixture(),
        );

        let outer = parts
            .iter()
            .find(|p| p.kind == PartKind::TextPlain && p.is_top_level())
            .expect("outer plain");
        assert_eq!(outer.section(), "1");

        let rfc822 = parts
            .iter()
            .find(|p| p.is_rfc822() && p.is_top_level())
            .expect("rfc822 attachment");
        assert_eq!(rfc822.section(), "2");
        assert!(rfc822.is_attachment);
        assert_eq!(rfc822.filename.as_deref(), Some("note.eml"));
        assert_eq!(
            rfc822
                .nested_headers
                .as_ref()
                .and_then(|h| h.subject.as_deref()),
            Some("Inner subject")
        );

        let inner_html = parts
            .iter()
            .find(|p| p.kind == PartKind::TextHtml)
            .expect("nested html");
        assert_eq!(inner_html.section(), "2.1");
        assert_eq!(inner_html.nested_in.as_deref(), Some("2"));
        assert!(!inner_html.is_attachment);
        assert!(inner_html.should_prefetch());

        let inner_pdf = parts
            .iter()
            .find(|p| p.filename.as_deref() == Some("inner.pdf"))
            .expect("nested pdf");
        assert_eq!(inner_pdf.section(), "2.2");
        assert_eq!(inner_pdf.nested_in.as_deref(), Some("2"));
        assert!(inner_pdf.is_attachment);
        assert!(!inner_pdf.should_prefetch());

        assert_eq!(
            parts
                .iter()
                .filter(|p| p.is_top_level() && p.is_attachment && !p.is_hidden)
                .count(),
            1,
            "only the .eml should be a top-level attachment"
        );
    }

    #[test]
    fn message_rfc822_single_part_nested_uses_n_dot_1() {
        let root = BodyPart {
            type_: "multipart".into(),
            subtype: "mixed".into(),
            subparts: vec![BodyPart {
                type_: "message".into(),
                subtype: "rfc822".into(),
                encoding: Some("7BIT".into()),
                nested_message: Some(Box::new(BodyPart {
                    type_: "text".into(),
                    subtype: "plain".into(),
                    encoding: Some("7BIT".into()),
                    ..Default::default()
                })),
                ..Default::default()
            }],
            ..Default::default()
        };
        let parser = MessageParser::with_defaults();
        let parts = parser.parse(&MessageId::new(FolderId::new("INBOX"), "1"), &root);
        let inner = parts
            .iter()
            .find(|p| p.kind == PartKind::TextPlain)
            .expect("nested plain");
        assert_eq!(inner.section(), "1.1");
        assert_eq!(inner.nested_in.as_deref(), Some("1"));
    }

    #[test]
    fn message_rfc822_nested_of_nested_keeps_inner_owner() {
        let root = BodyPart {
            type_: "multipart".into(),
            subtype: "mixed".into(),
            subparts: vec![BodyPart {
                type_: "message".into(),
                subtype: "rfc822".into(),
                encoding: Some("7BIT".into()),
                nested_message: Some(Box::new(BodyPart {
                    type_: "multipart".into(),
                    subtype: "mixed".into(),
                    subparts: vec![BodyPart {
                        type_: "message".into(),
                        subtype: "rfc822".into(),
                        encoding: Some("7BIT".into()),
                        nested_message: Some(Box::new(BodyPart {
                            type_: "text".into(),
                            subtype: "plain".into(),
                            encoding: Some("7BIT".into()),
                            ..Default::default()
                        })),
                        ..Default::default()
                    }],
                    ..Default::default()
                })),
                ..Default::default()
            }],
            ..Default::default()
        };
        let parser = MessageParser::with_defaults();
        let parts = parser.parse(&MessageId::new(FolderId::new("INBOX"), "1"), &root);
        let inner_rfc = parts
            .iter()
            .find(|p| p.is_rfc822() && p.section() == "1.1")
            .expect("inner rfc822");
        assert_eq!(inner_rfc.nested_in.as_deref(), Some("1"));
        let deepest = parts
            .iter()
            .find(|p| p.kind == PartKind::TextPlain)
            .expect("deepest body");
        assert_eq!(deepest.section(), "1.1.1");
        assert_eq!(deepest.nested_in.as_deref(), Some("1.1"));
    }

    fn calendar_part(disposition_filename: Option<&str>) -> BodyPart {
        BodyPart {
            type_: "text".into(),
            subtype: "calendar".into(),
            encoding: Some("7BIT".into()),
            parameters: [("METHOD".into(), "REQUEST".into())].into(),
            disposition: disposition_filename.map(|name| ContentDisposition {
                type_: "attachment".into(),
                attributes: [("FILENAME".into(), name.into())].into(),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn text_calendar_is_calendar_kind() {
        let parser = MessageParser::with_defaults();
        let parts = parser.parse(
            &MessageId::new(FolderId::new("INBOX"), "1"),
            &calendar_part(None),
        );
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].kind, PartKind::Calendar);
        assert_eq!(parts[0].section(), "TEXT");
        assert!(!parts[0].is_attachment);
        assert!(parts[0].should_prefetch());
    }

    #[test]
    fn calendar_attachment_stays_downloadable_and_prefetched() {
        let root = BodyPart {
            type_: "multipart".into(),
            subtype: "mixed".into(),
            subparts: vec![
                BodyPart {
                    type_: "text".into(),
                    subtype: "plain".into(),
                    encoding: Some("7BIT".into()),
                    ..Default::default()
                },
                calendar_part(Some("invite.ics")),
            ],
            ..Default::default()
        };
        let parser = MessageParser::with_defaults();
        let parts = parser.parse(&MessageId::new(FolderId::new("INBOX"), "1"), &root);
        let cal = parts
            .iter()
            .find(|p| p.kind == PartKind::Calendar)
            .expect("calendar part");
        assert_eq!(cal.section(), "2");
        assert!(cal.is_attachment);
        assert!(cal.should_prefetch());
        assert_eq!(cal.filename.as_deref(), Some("invite.ics"));
        assert_eq!(cal.content_type, "text/calendar");
    }

    #[test]
    fn alternative_keeps_calendar_with_html() {
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
                calendar_part(None),
            ],
            ..Default::default()
        };
        let parser = MessageParser::with_defaults();
        let parts = parser.parse(&MessageId::new(FolderId::new("INBOX"), "1"), &root);
        assert!(parts.iter().any(|p| p.kind == PartKind::TextHtml));
        let cal = parts
            .iter()
            .find(|p| p.kind == PartKind::Calendar)
            .expect("calendar alternative");
        assert_eq!(cal.section(), "3");
        assert!(!cal.is_attachment);
        assert!(cal.should_prefetch());
    }

    #[test]
    fn multipart_signed_smime_hides_signature() {
        let root = BodyPart {
            type_: "multipart".into(),
            subtype: "signed".into(),
            parameters: [("PROTOCOL".into(), "application/pkcs7-signature".into())].into(),
            subparts: vec![
                BodyPart {
                    type_: "text".into(),
                    subtype: "plain".into(),
                    encoding: Some("7BIT".into()),
                    ..Default::default()
                },
                BodyPart {
                    type_: "application".into(),
                    subtype: "pkcs7-signature".into(),
                    encoding: Some("BASE64".into()),
                    size: Some(80),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let parser = MessageParser::with_defaults();
        let parts = parser.parse(&MessageId::new(FolderId::new("INBOX"), "1"), &root);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].kind, PartKind::TextPlain);
        assert!(!parts[0].is_hidden);
        assert!(parts[1].is_hidden);
        assert!(parts[1].should_prefetch());
        assert!(mailiner_core::is_smime_mime(&parts[1].content_type));
    }

    #[test]
    fn multipart_signed_pgp_is_not_smime() {
        let root = BodyPart {
            type_: "multipart".into(),
            subtype: "signed".into(),
            parameters: [("PROTOCOL".into(), "application/pgp-signature".into())].into(),
            subparts: vec![
                BodyPart {
                    type_: "text".into(),
                    subtype: "plain".into(),
                    encoding: Some("7BIT".into()),
                    ..Default::default()
                },
                BodyPart {
                    type_: "application".into(),
                    subtype: "pgp-signature".into(),
                    encoding: Some("7BIT".into()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let parser = MessageParser::with_defaults();
        let parts = parser.parse(&MessageId::new(FolderId::new("INBOX"), "1"), &root);
        let sig = parts
            .iter()
            .find(|p| p.content_type.contains("pgp-signature"))
            .expect("pgp signature stays an attachment");
        assert!(!sig.is_hidden);
        assert!(!mailiner_core::is_smime_mime(&sig.content_type));
    }

    #[test]
    fn pkcs7_mime_root_is_prefetched_attachment() {
        let root = BodyPart {
            type_: "application".into(),
            subtype: "pkcs7-mime".into(),
            encoding: Some("BASE64".into()),
            size: Some(200),
            parameters: [("SMIME-TYPE".into(), "signed-data".into())].into(),
            ..Default::default()
        };
        let parser = MessageParser::with_defaults();
        let parts = parser.parse(&MessageId::new(FolderId::new("INBOX"), "1"), &root);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].kind, PartKind::Attachment);
        assert!(parts[0].should_prefetch());
        assert!(mailiner_core::is_smime_mime(&parts[0].content_type));
    }

    #[test]
    fn ics_filename_octet_stream_is_calendar() {
        let root = BodyPart {
            type_: "application".into(),
            subtype: "octet-stream".into(),
            encoding: Some("BASE64".into()),
            disposition: Some(ContentDisposition {
                type_: "attachment".into(),
                attributes: [("FILENAME".into(), "invite.ics".into())].into(),
            }),
            ..Default::default()
        };
        let parser = MessageParser::with_defaults();
        let parts = parser.parse(&MessageId::new(FolderId::new("INBOX"), "1"), &root);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].kind, PartKind::Calendar);
        assert!(parts[0].is_attachment);
        assert!(parts[0].should_prefetch());
    }
}
