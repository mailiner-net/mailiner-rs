//! Draft → MIME export orchestration (`prepare_submit`).

use chrono::Utc;
use thiserror::Error;
use uuid::Uuid;

use crate::identity::FromIdentity;
use crate::model::attachment::AttachmentData;
use crate::model::{
    dedupe_addresses, html_to_plain, validate_draft, BodyMode, ComposerAddress, DraftDocument,
    DraftValidationError,
};
use crate::sanitize::sanitize_for_export;
use mailiner_mime::{
    base64_encode, format_disposition, format_mailbox, generate_boundary, qp_encode,
    serialize_message, MimeBody, MimePart,
};

/// SMTP envelope (RFC 5321 MAIL FROM / RCPT TO). Distinct from header From/To/Cc/Bcc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitEnvelope {
    /// Reverse-path mailbox.
    pub mail_from: String,
    /// Forward-path mailboxes (To ∪ Cc ∪ Bcc, unique, To then Cc then Bcc).
    pub rcpt_to: Vec<String>,
}

/// Validated, serialized message ready for SMTP DATA.
#[derive(Debug, Clone)]
pub struct PreparedMessage {
    /// SMTP envelope.
    pub envelope: SubmitEnvelope,
    /// Complete RFC 5322 message including headers. Not SMTP-dot-stuffed.
    ///
    /// `Bcc:` is omitted here (correct for SMTP DATA).
    pub rfc822: Vec<u8>,
    /// Formatted `Bcc` value for the Sent-folder copy. `None` when there is no Bcc.
    pub bcc_header: Option<String>,
    /// Message-ID token including angle brackets, also written into headers.
    pub message_id: String,
}

/// Failure to validate or serialize a draft for send.
#[derive(Debug, Error)]
pub enum PrepareSubmitError {
    /// [`validate_draft`] rejected the document.
    #[error("draft validation failed")]
    Validation(Vec<DraftValidationError>),
    /// MIME writer failed.
    #[error("MIME serialization failed: {0}")]
    Serialize(String),
}

/// Validate `draft` and build envelope + RFC 5322 bytes.
///
/// `Bcc` addresses are on the envelope only — the `Bcc:` header is omitted.
pub fn prepare_submit(
    draft: &DraftDocument,
    identity: &FromIdentity,
) -> Result<PreparedMessage, PrepareSubmitError> {
    validate_draft(draft, identity).map_err(PrepareSubmitError::Validation)?;

    let from = resolve_from(draft, identity);
    let mail_from = from.email.clone();
    let rcpt_to = envelope_recipients(draft);
    let message_id = generate_message_id(&identity.email);

    let mut headers: Vec<(String, String)> = vec![
        (
            "From".into(),
            format_mailbox(from.name.as_deref(), &from.email),
        ),
        ("To".into(), format_addr_list(&draft.to)),
    ];
    if !draft.cc.is_empty() {
        headers.push(("Cc".into(), format_addr_list(&draft.cc)));
    }
    headers.push(("Subject".into(), draft.subject.clone()));
    headers.push(("Date".into(), format_rfc5322_date(Utc::now())));
    headers.push(("Message-ID".into(), message_id.clone()));
    if let Some(irt) = draft.in_reply_to.as_deref() {
        headers.push(("In-Reply-To".into(), normalize_msg_id(irt)));
    }
    if !draft.references.is_empty() {
        let refs = draft
            .references
            .iter()
            .map(|r| normalize_msg_id(r))
            .collect::<Vec<_>>()
            .join(" ");
        headers.push(("References".into(), refs));
    }

    let root = build_tree(draft)?;
    let rfc822 = serialize_message(&headers, &root)
        .map_err(|e| PrepareSubmitError::Serialize(e.to_string()))?;
    let bcc_header = if draft.bcc.is_empty() {
        None
    } else {
        Some(format_addr_list(&draft.bcc))
    };

    Ok(PreparedMessage {
        envelope: SubmitEnvelope { mail_from, rcpt_to },
        rfc822,
        bcc_header,
        message_id,
    })
}

fn resolve_from(draft: &DraftDocument, identity: &FromIdentity) -> ComposerAddress {
    if let Some(from) = &draft.from {
        if crate::model::draft::is_valid_email_v1(&from.email) {
            return from.clone();
        }
    }
    ComposerAddress {
        name: if identity.display_name.is_empty() {
            None
        } else {
            Some(identity.display_name.clone())
        },
        email: identity.email.clone(),
    }
}

fn envelope_recipients(draft: &DraftDocument) -> Vec<String> {
    let mut addrs = Vec::new();
    addrs.extend(draft.to.iter().cloned());
    addrs.extend(draft.cc.iter().cloned());
    addrs.extend(draft.bcc.iter().cloned());
    dedupe_addresses(addrs)
        .into_iter()
        .map(|a| a.email)
        .collect()
}

fn format_addr_list(list: &[ComposerAddress]) -> String {
    list.iter()
        .map(|a| format_mailbox(a.name.as_deref(), &a.email))
        .collect::<Vec<_>>()
        .join(", ")
}

fn normalize_msg_id(raw: &str) -> String {
    let t = raw.trim();
    if t.starts_with('<') && t.ends_with('>') {
        t.to_string()
    } else {
        format!("<{t}>")
    }
}

fn message_id_domain(email: &str) -> String {
    let Some((_, domain)) = email.rsplit_once('@') else {
        return "mailiner.invalid".into();
    };
    let d = domain.trim().to_ascii_lowercase();
    if d.is_empty() || !d.contains('.') || d == "localhost" || d == "127.0.0.1" {
        return "mailiner.invalid".into();
    }
    d
}

fn generate_message_id(identity_email: &str) -> String {
    format!("<{}@{}>", Uuid::new_v4(), message_id_domain(identity_email))
}

fn format_rfc5322_date(dt: chrono::DateTime<Utc>) -> String {
    dt.format("%a, %d %b %Y %H:%M:%S +0000").to_string()
}

fn text_part(content_type: &str, body: &str) -> MimePart {
    MimePart {
        headers: vec![
            (
                "Content-Type".into(),
                format!("{content_type}; charset=UTF-8"),
            ),
            (
                "Content-Transfer-Encoding".into(),
                "quoted-printable".into(),
            ),
        ],
        body: MimeBody::Octets(qp_encode(body.as_bytes())),
    }
}

fn binary_part(
    content_type: &str,
    disposition: String,
    content_id: Option<&str>,
    bytes: &[u8],
) -> MimePart {
    let mut headers = vec![
        ("Content-Type".into(), content_type.to_string()),
        ("Content-Transfer-Encoding".into(), "base64".into()),
        ("Content-Disposition".into(), disposition),
    ];
    if let Some(cid) = content_id {
        headers.push(("Content-ID".into(), normalize_msg_id(cid)));
    }
    MimePart {
        headers,
        body: MimeBody::Octets(base64_encode(bytes)),
    }
}

fn attachment_bytes(data: &AttachmentData) -> Result<&[u8], PrepareSubmitError> {
    match data {
        AttachmentData::Bytes(b) => Ok(b),
        AttachmentData::Pending => Err(PrepareSubmitError::Serialize(
            "pending attachment reached export".into(),
        )),
    }
}

fn alternative_or_single(draft: &DraftDocument) -> MimePart {
    let use_html = draft.mode == BodyMode::Rich || !draft.html_body.trim().is_empty();
    if use_html {
        let plain = if draft.plain_cache_dirty || draft.plain_body.is_empty() {
            html_to_plain(&draft.html_body)
        } else {
            draft.plain_body.clone()
        };
        let html = sanitize_for_export(&draft.html_body);
        MimePart {
            headers: vec![],
            body: MimeBody::Multipart {
                subtype: "alternative".into(),
                boundary: generate_boundary(),
                parts: vec![
                    text_part("text/plain", &plain),
                    text_part("text/html", &html),
                ],
            },
        }
    } else {
        text_part("text/plain", &draft.plain_body)
    }
}

fn build_tree(draft: &DraftDocument) -> Result<MimePart, PrepareSubmitError> {
    let mut body = alternative_or_single(draft);

    if !draft.inline_images.is_empty() {
        let mut parts = vec![body];
        for img in &draft.inline_images {
            let bytes = attachment_bytes(&img.data)?;
            parts.push(binary_part(
                &img.content_type,
                format_disposition("inline", img.filename.as_deref()),
                Some(&img.content_id),
                bytes,
            ));
        }
        body = MimePart {
            headers: vec![],
            body: MimeBody::Multipart {
                subtype: "related".into(),
                boundary: generate_boundary(),
                parts,
            },
        };
    }

    if !draft.attachments.is_empty() {
        let mut parts = vec![body];
        for att in &draft.attachments {
            let bytes = attachment_bytes(&att.data)?;
            parts.push(binary_part(
                &att.content_type,
                format_disposition("attachment", Some(&att.filename)),
                None,
                bytes,
            ));
        }
        body = MimePart {
            headers: vec![],
            body: MimeBody::Multipart {
                subtype: "mixed".into(),
                boundary: generate_boundary(),
                parts,
            },
        };
    }

    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        AttachmentData, AttachmentId, ComposerAddress, DraftDocument, FileAttachment, InlineId,
        InlineImage,
    };

    fn identity() -> FromIdentity {
        FromIdentity::new("Me", "me@example.com")
    }

    fn minimal() -> DraftDocument {
        let mut d = DraftDocument::new_empty(&identity());
        d.mode = BodyMode::Plain;
        d.plain_body = "Hello".into();
        d.html_body.clear();
        d.to.push(ComposerAddress::email_only("you@example.com"));
        d.subject = "Hi".into();
        d
    }

    #[test]
    fn crlf_and_message_id_domain() {
        let prepared = prepare_submit(&minimal(), &identity()).unwrap();
        assert!(prepared.message_id.starts_with('<'));
        assert!(prepared.message_id.ends_with("@example.com>"));
        assert!(prepared.rfc822.windows(2).any(|w| w == b"\r\n"));
        let s = String::from_utf8_lossy(&prepared.rfc822);
        assert!(!s.contains('\n') || s.contains("\r\n"));
        // no bare LF
        for w in prepared.rfc822.windows(2) {
            if w[1] == b'\n' {
                assert_eq!(w[0], b'\r', "bare LF in message");
            }
        }
    }

    #[test]
    fn bcc_on_envelope_not_headers() {
        let mut d = minimal();
        d.bcc
            .push(ComposerAddress::email_only("secret@example.com"));
        let prepared = prepare_submit(&d, &identity()).unwrap();
        assert!(prepared
            .envelope
            .rcpt_to
            .iter()
            .any(|a| a == "secret@example.com"));
        let s = String::from_utf8_lossy(&prepared.rfc822);
        assert!(
            !s.to_ascii_lowercase().contains("bcc:"),
            "Bcc leaked into headers:\n{s}"
        );
        assert_eq!(prepared.bcc_header.as_deref(), Some("secret@example.com"));
        assert_eq!(prepared.envelope.mail_from, "me@example.com");
    }

    #[test]
    fn no_bcc_header_without_bcc() {
        let prepared = prepare_submit(&minimal(), &identity()).unwrap();
        assert!(prepared.bcc_header.is_none());
    }

    #[test]
    fn empty_to_is_validation() {
        let d = DraftDocument::new_empty(&identity());
        match prepare_submit(&d, &identity()) {
            Err(PrepareSubmitError::Validation(errs)) => {
                assert!(errs.contains(&DraftValidationError::EmptyTo));
            }
            other => panic!("expected validation, got {other:?}"),
        }
    }

    #[test]
    fn attachment_filename_rfc2231() {
        let mut d = minimal();
        d.attachments.push(FileAttachment {
            id: AttachmentId::new(),
            filename: "café.pdf".into(),
            content_type: "application/pdf".into(),
            size: 4,
            data: AttachmentData::Bytes(b"%PDF".to_vec()),
            source: None,
        });
        let prepared = prepare_submit(&d, &identity()).unwrap();
        let s = String::from_utf8_lossy(&prepared.rfc822);
        assert!(s.contains("multipart/mixed"));
        assert!(s.contains("filename*="));
        assert!(s.contains("caf%C3%A9.pdf"));
    }

    #[test]
    fn rich_alternative_and_related() {
        let mut d = minimal();
        d.mode = BodyMode::Rich;
        d.html_body = "<p>Hi <img src=\"cid:pic@mailiner\"></p>".into();
        d.plain_cache_dirty = true;
        d.inline_images.push(InlineImage {
            id: InlineId::new(),
            content_id: "pic@mailiner".into(),
            content_type: "image/png".into(),
            filename: Some("dot.png".into()),
            data: AttachmentData::Bytes(b"PNG".to_vec()),
            edit_url: None,
        });
        let prepared = prepare_submit(&d, &identity()).unwrap();
        let s = String::from_utf8_lossy(&prepared.rfc822);
        assert!(s.contains("multipart/related"));
        assert!(s.contains("multipart/alternative"));
        assert!(s.contains("Content-ID: <pic@mailiner>"));
    }

    #[test]
    fn plain_with_inlines_emits_related_cid() {
        let mut d = minimal();
        d.inline_images
            .push(crate::shell::attachment_list::inline_image(
                Some("dot.png".into()),
                "image/png",
                b"PNG".to_vec(),
            ));
        d.html_body = crate::shell::attachment_list::html_for_plain_with_inlines(
            &d.plain_body,
            &d.inline_images,
        );
        let prepared = prepare_submit(&d, &identity()).unwrap();
        let s = String::from_utf8_lossy(&prepared.rfc822);
        assert!(s.contains("multipart/related"));
        assert!(s.contains("Content-ID: <img-"));
        assert!(s.contains("cid:img-"));
        assert!(s.contains("Hello"));
    }

    #[test]
    fn localhost_email_uses_invalid_domain() {
        let id = FromIdentity::new("X", "root@127.0.0.1");
        let mut d = DraftDocument::new_empty(&id);
        d.mode = BodyMode::Plain;
        d.to.push(ComposerAddress::email_only("you@example.com"));
        let prepared = prepare_submit(&d, &id).unwrap();
        assert!(prepared.message_id.ends_with("@mailiner.invalid>"));
    }
}
