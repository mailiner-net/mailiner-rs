//! Reply / forward draft construction from structured envelope + loaded body.

use mailiner_core::{Envelope, LoadedMessage, MessageContent, PartKind};

use crate::identity::FromIdentity;
use crate::model::draft::{BodyMode, ComposerAddress, DraftDocument};
use crate::model::recipients::{
    dedupe_addresses, exclude_self, flatten_addresses,
};
use crate::reply::quote::{attribution_line, quote_plain, subject_with_prefix};

/// How the user opened the composer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposeIntent {
    /// Blank new message.
    New,
    /// Reply to sender only.
    Reply,
    /// Reply to all participants.
    ReplyAll,
    /// Forward body (no auto file attachments in v1).
    Forward,
}

/// Prefill failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrefillError {
    /// Reply/Forward require a loaded message body.
    #[error("message body not loaded")]
    BodyNotLoaded,
    /// HTML-only body needs sanitize pipeline (PR 3) before rich prefill.
    #[error("HTML body requires sanitize before prefill")]
    HtmlBodyNeedsSanitize,
    /// Intent requires an envelope (Reply/ReplyAll/Forward).
    #[error("envelope required for this compose intent")]
    EnvelopeRequired,
}

/// Build a draft for the given intent.
///
/// PR2: plain-quote path only. If the best body is HTML-only (no plain part),
/// returns [`PrefillError::HtmlBodyNeedsSanitize`] for Reply/Forward so callers
/// wait for the shared sanitize module (PR 3). New compose always succeeds.
pub fn build_draft(
    intent: ComposeIntent,
    identity: &FromIdentity,
    envelope: Option<&Envelope>,
    loaded: Option<&LoadedMessage>,
) -> Result<DraftDocument, PrefillError> {
    match intent {
        ComposeIntent::New => Ok(DraftDocument::new_empty(identity)),
        ComposeIntent::Reply | ComposeIntent::ReplyAll | ComposeIntent::Forward => {
            let env = envelope.ok_or(PrefillError::EnvelopeRequired)?;
            let loaded = loaded.ok_or(PrefillError::BodyNotLoaded)?;
            build_reply_like(intent, identity, env, loaded)
        }
    }
}

fn build_reply_like(
    intent: ComposeIntent,
    identity: &FromIdentity,
    env: &Envelope,
    loaded: &LoadedMessage,
) -> Result<DraftDocument, PrefillError> {
    let mut draft = DraftDocument::new_empty(identity);

    // Headers
    match intent {
        ComposeIntent::Reply => {
            draft.to = reply_to_addresses(env);
            draft.subject = subject_with_prefix(env.subject.as_deref(), "Re:");
        }
        ComposeIntent::ReplyAll => {
            let (to, cc) = reply_all_addresses(env, &identity.email);
            draft.to = to;
            draft.cc = cc;
            draft.subject = subject_with_prefix(env.subject.as_deref(), "Re:");
        }
        ComposeIntent::Forward => {
            draft.subject = subject_with_prefix(env.subject.as_deref(), "Fwd:");
        }
        ComposeIntent::New => unreachable!(),
    }

    // Threading placeholders — filled when Envelope carries Message-ID (PR 8.5).
    // Fields remain empty until then.

    let from_addr = env
        .from
        .as_ref()
        .map(|a| flatten_addresses(a))
        .and_then(|v| v.into_iter().next());

    let attribution = attribution_line(env.date, from_addr.as_ref());

    match pick_body(loaded)? {
        BodyPick::Plain(text) => {
            draft.mode = BodyMode::Plain;
            draft.plain_body = quote_plain(&attribution, &text);
            draft.html_body.clear();
            draft.plain_cache_dirty = false;
        }
        BodyPick::HtmlOnly(_html) => {
            // PR2: do not inject unsanitized HTML.
            return Err(PrefillError::HtmlBodyNeedsSanitize);
        }
        BodyPick::HtmlWithPlain { plain } => {
            // Prefer plain quote in PR2 even when HTML exists — safe path.
            // HTML rich quote lands in PR 3 (sanitize) + PR 6 (cid).
            draft.mode = BodyMode::Plain;
            draft.plain_body = quote_plain(&attribution, &plain);
            draft.html_body.clear();
            draft.plain_cache_dirty = false;
        }
    }

    Ok(draft)
}

fn reply_to_addresses(env: &Envelope) -> Vec<ComposerAddress> {
    // Prefer Reply-To when present (PR 8.5); until then use From.
    env.from
        .as_ref()
        .map(flatten_addresses)
        .unwrap_or_default()
}

fn reply_all_addresses(env: &Envelope, self_email: &str) -> (Vec<ComposerAddress>, Vec<ComposerAddress>) {
    let mut to = Vec::new();
    if let Some(from) = &env.from {
        to.extend(flatten_addresses(from));
    }
    if let Some(orig_to) = &env.to {
        to.extend(flatten_addresses(orig_to));
    }
    to = exclude_self(to, self_email);
    to = dedupe_addresses(to);

    let mut cc = env
        .cc
        .as_ref()
        .map(flatten_addresses)
        .unwrap_or_default();
    cc = exclude_self(cc, self_email);
    // Don't duplicate addresses already in To.
    cc.retain(|c| !to.iter().any(|t| crate::model::emails_equal(&t.email, &c.email)));
    cc = dedupe_addresses(cc);

    (to, cc)
}

enum BodyPick {
    Plain(String),
    HtmlOnly(String),
    /// Multipart alternative: use plain for PR2 quote (HTML reserved for later).
    HtmlWithPlain { plain: String },
}

fn pick_body(loaded: &LoadedMessage) -> Result<BodyPick, PrefillError> {
    let mut plain: Option<String> = None;
    let mut html: Option<String> = None;

    for part in &loaded.parts {
        if part.is_hidden || part.is_attachment {
            continue;
        }
        match part.kind {
            PartKind::TextPlain => {
                if plain.is_none() {
                    if let MessageContent::Text(t) = &part.content {
                        plain = Some(t.clone());
                    }
                }
            }
            PartKind::TextHtml => {
                if html.is_none() {
                    if let MessageContent::Text(t) = &part.content {
                        html = Some(t.clone());
                    }
                }
            }
            _ => {}
        }
    }

    match (plain, html) {
        (Some(p), Some(_h)) => Ok(BodyPick::HtmlWithPlain { plain: p }),
        (Some(p), None) => Ok(BodyPick::Plain(p)),
        (None, Some(h)) => Ok(BodyPick::HtmlOnly(h)),
        (None, None) => Ok(BodyPick::Plain(String::new())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use mailiner_core::{
        AccountId, EmailAddr, EmailAddress, FolderId, MessageContent, MessageId, MessagePart,
        MessagePartId, PartKind, TransferEncoding,
    };

    fn env_with_from(from: &str) -> Envelope {
        Envelope {
            id: MessageId::new("1"),
            account_id: AccountId::new("a"),
            folder_id: FolderId::new("INBOX"),
            subject: Some("Hello".into()),
            from: Some(EmailAddress::List(vec![EmailAddr {
                name: Some("Alice".into()),
                email: Some(from.into()),
            }])),
            to: Some(EmailAddress::List(vec![EmailAddr {
                name: None,
                email: Some("me@example.com".into()),
            }])),
            cc: Some(EmailAddress::List(vec![EmailAddr {
                name: None,
                email: Some("cc@example.com".into()),
            }])),
            bcc: None,
            date: Utc::now(),
            is_read: true,
            is_starred: false,
            is_flagged: false,
            is_draft: false,
            is_deleted: false,
            has_attachments: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn loaded_plain(text: &str) -> LoadedMessage {
        LoadedMessage {
            envelope_id: MessageId::new("1"),
            folder_id: FolderId::new("INBOX"),
            parts: vec![MessagePart {
                id: MessagePartId::new("p1"),
                envelope_id: MessageId::new("1"),
                path: vec!["1".into()],
                kind: PartKind::TextPlain,
                content_type: "text/plain".into(),
                charset: Some("utf-8".into()),
                content_id: None,
                description: None,
                filename: None,
                encoding: TransferEncoding::SevenBit,
                original_size: None,
                size: text.len() as u64,
                is_attachment: false,
                is_hidden: false,
                raw_content: None,
                content: MessageContent::Text(text.into()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }],
            structure: None,
        }
    }

    fn loaded_html_only(html: &str) -> LoadedMessage {
        LoadedMessage {
            envelope_id: MessageId::new("1"),
            folder_id: FolderId::new("INBOX"),
            parts: vec![MessagePart {
                id: MessagePartId::new("h1"),
                envelope_id: MessageId::new("1"),
                path: vec!["1".into()],
                kind: PartKind::TextHtml,
                content_type: "text/html".into(),
                charset: Some("utf-8".into()),
                content_id: None,
                description: None,
                filename: None,
                encoding: TransferEncoding::SevenBit,
                original_size: None,
                size: html.len() as u64,
                is_attachment: false,
                is_hidden: false,
                raw_content: None,
                content: MessageContent::Text(html.into()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }],
            structure: None,
        }
    }

    #[test]
    fn new_compose_rich_empty() {
        let id = FromIdentity::new("Me", "me@example.com");
        let d = build_draft(ComposeIntent::New, &id, None, None).unwrap();
        assert_eq!(d.mode, BodyMode::Rich);
        assert!(d.to.is_empty());
        assert_eq!(d.from.as_ref().unwrap().email, "me@example.com");
    }

    #[test]
    fn reply_plain_quotes() {
        let id = FromIdentity::new("Me", "me@example.com");
        let env = env_with_from("alice@example.com");
        let loaded = loaded_plain("Hi there");
        let d = build_draft(ComposeIntent::Reply, &id, Some(&env), Some(&loaded)).unwrap();
        assert_eq!(d.mode, BodyMode::Plain);
        assert_eq!(d.to[0].email, "alice@example.com");
        assert!(d.subject.starts_with("Re:"), "{}", d.subject);
        assert!(d.plain_body.contains("> Hi there"), "{}", d.plain_body);
        assert!(d.cc.is_empty());
    }

    #[test]
    fn reply_all_excludes_self() {
        let id = FromIdentity::new("Me", "me@example.com");
        let env = env_with_from("alice@example.com");
        let loaded = loaded_plain("x");
        let d = build_draft(ComposeIntent::ReplyAll, &id, Some(&env), Some(&loaded)).unwrap();
        assert!(d.to.iter().any(|a| a.email == "alice@example.com"));
        assert!(!d.to.iter().any(|a| a.email == "me@example.com"));
        assert!(d.cc.iter().any(|a| a.email == "cc@example.com"));
    }

    #[test]
    fn html_only_needs_sanitize() {
        let id = FromIdentity::new("Me", "me@example.com");
        let env = env_with_from("alice@example.com");
        let loaded = loaded_html_only("<p>Hi</p>");
        let err = build_draft(ComposeIntent::Reply, &id, Some(&env), Some(&loaded)).unwrap_err();
        assert_eq!(err, PrefillError::HtmlBodyNeedsSanitize);
    }

    #[test]
    fn forward_clears_recipients() {
        let id = FromIdentity::new("Me", "me@example.com");
        let env = env_with_from("alice@example.com");
        let loaded = loaded_plain("body");
        let d = build_draft(ComposeIntent::Forward, &id, Some(&env), Some(&loaded)).unwrap();
        assert!(d.to.is_empty());
        assert!(d.subject.starts_with("Fwd:"), "{}", d.subject);
    }
}
