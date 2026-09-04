//! Convert `imap_proto::BodyStructure` → owned `mailiner_core::BodyPart`.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use imap_proto::types::{
    Address, BodyContentCommon, BodyContentSinglePart, BodyStructure, ContentEncoding, Envelope,
};
use mailiner_core::body::{BodyPart, ContentDisposition};
use mailiner_core::models::{EmailAddr, EmailAddress, NestedMessageHeaders};
use mailiner_mime::params::normalize_params;

pub fn convert_body_structure(bs: &BodyStructure<'_>) -> BodyPart {
    match bs {
        BodyStructure::Basic { common, other, .. } => single_part(common, other),
        BodyStructure::Text { common, other, .. } => single_part(common, other),
        BodyStructure::Message {
            common,
            other,
            envelope,
            body,
            ..
        } => {
            let mut part = single_part(common, other);
            part.nested_message = Some(Box::new(convert_body_structure(body)));
            part.nested_headers = Some(convert_nested_envelope(envelope));
            part
        }
        BodyStructure::Multipart { common, bodies, .. } => BodyPart {
            type_: "multipart".into(),
            subtype: common.ty.subtype.to_ascii_lowercase(),
            parameters: params_to_map(common.ty.params.as_ref()),
            id: None,
            description: None,
            encoding: None,
            size: None,
            md5: None,
            disposition: disposition_from_common(common),
            location: common.location.as_ref().map(|s| s.to_string()),
            subparts: bodies.iter().map(convert_body_structure).collect(),
            nested_message: None,
            nested_headers: None,
        },
    }
}

fn single_part(common: &BodyContentCommon<'_>, other: &BodyContentSinglePart<'_>) -> BodyPart {
    BodyPart {
        type_: common.ty.ty.to_ascii_lowercase(),
        subtype: common.ty.subtype.to_ascii_lowercase(),
        parameters: params_to_map(common.ty.params.as_ref()),
        id: other.id.as_ref().map(|s| s.to_string()),
        description: other.description.as_ref().map(|s| s.to_string()),
        encoding: Some(encoding_to_string(&other.transfer_encoding)),
        size: Some(other.octets as u64),
        md5: other.md5.as_ref().map(|s| s.to_string()),
        disposition: disposition_from_common(common),
        location: common.location.as_ref().map(|s| s.to_string()),
        subparts: vec![],
        nested_message: None,
        nested_headers: None,
    }
}

fn convert_nested_envelope(envelope: &Envelope<'_>) -> NestedMessageHeaders {
    NestedMessageHeaders {
        subject: envelope.subject.as_ref().map(|b| envelope_bytes_to_text(b)),
        from: convert_envelope_addresses(envelope.from.as_deref()),
        to: convert_envelope_addresses(envelope.to.as_deref()),
        cc: convert_envelope_addresses(envelope.cc.as_deref()),
        date: envelope
            .date
            .as_ref()
            .and_then(|b| parse_envelope_date(&envelope_bytes_to_text(b))),
    }
}

fn envelope_bytes_to_text(bytes: &[u8]) -> String {
    let raw = match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    };
    mailiner_mime::mime_words_decode(raw.trim())
}

fn convert_envelope_addresses(addrs: Option<&[Address<'_>]>) -> Option<EmailAddress> {
    let addrs = addrs?;
    let list: Vec<EmailAddr> = addrs.iter().filter_map(convert_envelope_address).collect();
    if list.is_empty() {
        None
    } else {
        Some(EmailAddress::List(list))
    }
}

fn convert_envelope_address(addr: &Address<'_>) -> Option<EmailAddr> {
    let name = addr
        .name
        .as_ref()
        .map(|b| envelope_bytes_to_text(b))
        .filter(|s| !s.is_empty());
    let mailbox = addr
        .mailbox
        .as_ref()
        .map(|b| envelope_bytes_to_text(b))
        .filter(|s| !s.is_empty());
    let host = addr
        .host
        .as_ref()
        .map(|b| envelope_bytes_to_text(b))
        .filter(|s| !s.is_empty());
    let email = match (mailbox, host) {
        (Some(local), Some(domain)) => Some(format!("{local}@{domain}")),
        (Some(local), None) => Some(local),
        (None, Some(domain)) => Some(domain),
        (None, None) => None,
    };
    if name.is_none() && email.is_none() {
        None
    } else {
        Some(EmailAddr { name, email })
    }
}

fn parse_envelope_date(raw: &str) -> Option<DateTime<Utc>> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc2822(s) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Some(idx) = s.rfind('(') {
        let trimmed = s[..idx].trim();
        if let Ok(dt) = DateTime::parse_from_rfc2822(trimmed) {
            return Some(dt.with_timezone(&Utc));
        }
    }
    None
}

fn disposition_from_common(common: &BodyContentCommon<'_>) -> Option<ContentDisposition> {
    common.disposition.as_ref().map(|d| ContentDisposition {
        type_: d.ty.to_string(),
        attributes: params_to_map(d.params.as_ref()),
    })
}

fn encoding_to_string(enc: &ContentEncoding<'_>) -> String {
    match enc {
        ContentEncoding::SevenBit => "7BIT".into(),
        ContentEncoding::EightBit => "8BIT".into(),
        ContentEncoding::Binary => "BINARY".into(),
        ContentEncoding::Base64 => "BASE64".into(),
        ContentEncoding::QuotedPrintable => "QUOTED-PRINTABLE".into(),
        ContentEncoding::Other(s) => s.to_ascii_uppercase(),
    }
}

fn params_to_map(
    params: Option<&Vec<(impl AsRef<str>, impl AsRef<str>)>>,
) -> BTreeMap<String, String> {
    // BodyParams is Option<Vec<(Cow, Cow)>> — pass as Option of iterator of refs
    match params {
        None => BTreeMap::new(),
        Some(v) => {
            let pairs: Vec<(String, String)> = v
                .iter()
                .map(|(k, val)| (k.as_ref().to_string(), val.as_ref().to_string()))
                .collect();
            normalize_params(Some(&pairs))
        }
    }
}

/// Walk converted structure using mime heuristics.
pub fn structure_has_attachments(part: &BodyPart) -> bool {
    if mailiner_mime::is_attachment(part) {
        return true;
    }
    if part.subparts.iter().any(structure_has_attachments) {
        return true;
    }
    part.nested_message
        .as_deref()
        .is_some_and(structure_has_attachments)
}

/// First display text part for a list snippet. Prefers `text/plain` over HTML.
pub struct PreviewTextPart<'a> {
    pub section: String,
    pub part: &'a BodyPart,
}

pub fn first_preview_text(root: &BodyPart) -> Option<PreviewTextPart<'_>> {
    let mut plain = None;
    let mut html = None;
    walk_preview_text(root, &[], &mut plain, &mut html);
    plain.or(html)
}

fn walk_preview_text<'a>(
    part: &'a BodyPart,
    path: &[String],
    plain: &mut Option<PreviewTextPart<'a>>,
    html: &mut Option<PreviewTextPart<'a>>,
) {
    if mailiner_mime::is_attachment(part) {
        return;
    }
    if part.type_ == "multipart" {
        for (i, sub) in part.subparts.iter().enumerate() {
            let mut child = path.to_vec();
            child.push((i + 1).to_string());
            walk_preview_text(sub, &child, plain, html);
        }
        return;
    }
    if part.type_ != "text" {
        return;
    }
    let section = if path.is_empty() {
        "TEXT".to_string()
    } else {
        path.join(".")
    };
    let target = PreviewTextPart { section, part };
    match part.subtype.as_str() {
        "plain" if plain.is_none() => *plain = Some(target),
        "html" if html.is_none() => *html = Some(target),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imap_proto::types::{BodyContentCommon, BodyContentSinglePart, ContentType};

    fn text_plain_bs() -> BodyStructure<'static> {
        BodyStructure::Text {
            common: BodyContentCommon {
                ty: ContentType {
                    ty: "TEXT".into(),
                    subtype: "PLAIN".into(),
                    params: Some(vec![("CHARSET".into(), "UTF-8".into())]),
                },
                disposition: None,
                language: None,
                location: None,
            },
            other: BodyContentSinglePart {
                id: None,
                md5: None,
                description: None,
                transfer_encoding: ContentEncoding::SevenBit,
                octets: 42,
            },
            lines: 1,
            extension: None,
        }
    }

    #[test]
    fn converts_text_lowercases() {
        let bp = convert_body_structure(&text_plain_bs());
        assert_eq!(bp.type_, "text");
        assert_eq!(bp.subtype, "plain");
        assert_eq!(bp.encoding.as_deref(), Some("7BIT"));
        assert_eq!(bp.size, Some(42));
        assert_eq!(bp.charset(), Some("UTF-8"));
    }

    #[test]
    fn converts_multipart() {
        let bs = BodyStructure::Multipart {
            common: BodyContentCommon {
                ty: ContentType {
                    ty: "MULTIPART".into(),
                    subtype: "MIXED".into(),
                    params: None,
                },
                disposition: None,
                language: None,
                location: None,
            },
            bodies: vec![text_plain_bs()],
            extension: None,
        };
        let bp = convert_body_structure(&bs);
        assert_eq!(bp.type_, "multipart");
        assert_eq!(bp.subtype, "mixed");
        assert_eq!(bp.subparts.len(), 1);
        assert_eq!(bp.subparts[0].subtype, "plain");
    }

    #[test]
    fn preview_text_single_part_is_text() {
        let bp = convert_body_structure(&text_plain_bs());
        let preview = first_preview_text(&bp).expect("text part");
        assert_eq!(preview.section, "TEXT");
        assert_eq!(preview.part.subtype, "plain");
    }

    #[test]
    fn preview_text_prefers_plain_over_html() {
        let root = mailiner_core::mock_multipart_structure();
        let preview = first_preview_text(&root).expect("plain part");
        assert_eq!(preview.section, "1.1");
        assert_eq!(preview.part.subtype, "plain");
    }

    #[test]
    fn preview_text_skips_attached_multipart() {
        let root = BodyPart {
            type_: "multipart".into(),
            subtype: "mixed".into(),
            subparts: vec![
                BodyPart {
                    type_: "multipart".into(),
                    subtype: "alternative".into(),
                    disposition: Some(mailiner_core::ContentDisposition {
                        type_: "ATTACHMENT".into(),
                        attributes: Default::default(),
                    }),
                    subparts: vec![BodyPart {
                        type_: "text".into(),
                        subtype: "plain".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                BodyPart {
                    type_: "text".into(),
                    subtype: "plain".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let preview = first_preview_text(&root).expect("outer text");
        assert_eq!(preview.section, "2");
    }

    #[test]
    fn preview_text_skips_attachments() {
        let root = BodyPart {
            type_: "multipart".into(),
            subtype: "mixed".into(),
            subparts: vec![
                BodyPart {
                    type_: "application".into(),
                    subtype: "pdf".into(),
                    disposition: Some(mailiner_core::ContentDisposition {
                        type_: "ATTACHMENT".into(),
                        attributes: Default::default(),
                    }),
                    ..Default::default()
                },
                BodyPart {
                    type_: "text".into(),
                    subtype: "html".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let preview = first_preview_text(&root).expect("html after attachment");
        assert_eq!(preview.section, "2");
        assert_eq!(preview.part.subtype, "html");
    }

    #[test]
    fn converts_message_rfc822_envelope() {
        use std::borrow::Cow;

        use imap_proto::types::{Address, Envelope};

        let nested = text_plain_bs();
        let bs = BodyStructure::Message {
            common: BodyContentCommon {
                ty: ContentType {
                    ty: "MESSAGE".into(),
                    subtype: "RFC822".into(),
                    params: Some(vec![("NAME".into(), "note.eml".into())]),
                },
                disposition: None,
                language: None,
                location: None,
            },
            other: BodyContentSinglePart {
                id: None,
                md5: None,
                description: None,
                transfer_encoding: ContentEncoding::SevenBit,
                octets: 120,
            },
            envelope: Envelope {
                date: Some(Cow::Borrowed(b"Wed, 01 Jan 2020 00:00:00 +0000")),
                subject: Some(Cow::Borrowed(b"Inner subject")),
                from: Some(vec![Address {
                    name: Some(Cow::Borrowed(b"Ada")),
                    adl: None,
                    mailbox: Some(Cow::Borrowed(b"ada")),
                    host: Some(Cow::Borrowed(b"example.com")),
                }]),
                sender: None,
                reply_to: None,
                to: Some(vec![Address {
                    name: None,
                    adl: None,
                    mailbox: Some(Cow::Borrowed(b"bob")),
                    host: Some(Cow::Borrowed(b"example.com")),
                }]),
                cc: None,
                bcc: None,
                in_reply_to: None,
                message_id: Some(Cow::Borrowed(b"<inner@example.com>")),
            },
            body: Box::new(nested),
            lines: 4,
            extension: None,
        };
        let bp = convert_body_structure(&bs);
        assert!(bp.is_rfc822());
        assert!(bp.nested_message.is_some());
        let headers = bp.nested_headers.expect("envelope");
        assert_eq!(headers.subject.as_deref(), Some("Inner subject"));
        assert_eq!(
            headers.from.map(|a| a.to_string()).as_deref(),
            Some("Ada <ada@example.com>")
        );
        assert_eq!(
            headers.to.map(|a| a.to_string()).as_deref(),
            Some("bob@example.com")
        );
        assert!(headers.date.is_some());
    }
}
