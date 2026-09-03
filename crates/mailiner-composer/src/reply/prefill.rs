//! Reply / forward draft construction from structured envelope + loaded body.

use mailiner_core::{Envelope, LoadedMessage, MessageContent, MessagePart, PartKind};

use crate::identity::FromIdentity;
use crate::model::draft::{caps, BodyMode, ComposerAddress, DraftDocument};
use crate::model::recipients::{dedupe_addresses, exclude_self, flatten_addresses};
use crate::reply::quote::{attribution_line, quote_plain, subject_with_prefix};
use crate::shell::attachment_list::draft_payload_bytes;

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
/// Plain bodies use `>` quotes. HTML bodies are sanitized via
/// [`crate::sanitize::sanitize_for_edit`] and wrapped as a rich quote.
/// `cid:` images referenced by the quote are copied onto [`DraftDocument::inline_images`].
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

    if matches!(intent, ComposeIntent::Reply | ComposeIntent::ReplyAll) {
        apply_reply_threading(&mut draft, env);
    }

    let from_addr = env
        .from
        .as_ref()
        .map(flatten_addresses)
        .and_then(|v| v.into_iter().next());

    let attribution = attribution_line(env.date, from_addr.as_ref());

    match pick_body(loaded)? {
        BodyPick::Plain(text) => {
            draft.mode = BodyMode::Plain;
            draft.plain_body = quote_plain(&attribution, &text);
            draft.html_body.clear();
            draft.plain_cache_dirty = false;
        }
        BodyPick::HtmlOnly(html) => {
            apply_html_quote(&mut draft, &attribution, &html, &loaded.parts, None);
        }
        BodyPick::HtmlWithPlain { plain, html } => {
            apply_html_quote(&mut draft, &attribution, &html, &loaded.parts, Some(&plain));
        }
    }

    Ok(draft)
}

fn apply_html_quote(
    draft: &mut DraftDocument,
    attribution: &str,
    html: &str,
    parts: &[MessagePart],
    plain_override: Option<&str>,
) {
    let clean = crate::sanitize::sanitize_for_edit(html);
    // Reserve the quote wrapper + plain alternative before admitting inlines so
    // the finished draft cannot exceed MAX_DRAFT_BYTES.
    let html_est = wrap_html_quote(attribution, &clean);
    let plain_est = match plain_override {
        Some(plain) => quote_plain(attribution, plain),
        None => quote_plain(attribution, &crate::model::html_to_plain(&clean)),
    };
    let reserved = html_est.len() as u64 + plain_est.len() as u64;
    let remaining = caps::MAX_DRAFT_BYTES
        .saturating_sub(draft_payload_bytes(draft))
        .saturating_sub(reserved);
    let rehydrated =
        crate::reply::cid::rehydrate_cids(&clean, parts, draft.inline_images.len(), remaining);
    draft.mode = BodyMode::Rich;
    draft.html_body = wrap_html_quote(attribution, &rehydrated.html);
    draft.plain_body = match plain_override {
        Some(plain) => quote_plain(attribution, plain),
        None => quote_plain(attribution, &crate::model::html_to_plain(&rehydrated.html)),
    };
    draft.plain_cache_dirty = false;
    draft.inline_images = rehydrated.images;
    draft.prefill_warnings.extend(rehydrated.warnings);
}

fn wrap_html_quote(attribution: &str, sanitized_body: &str) -> String {
    let attr_esc = html_escape_text(attribution);
    // Strip untrusted classes/markers from the quote body so original mail cannot
    // spoof composer chrome (mlnr-compose-*, data-mlnr-quote).
    let body = strip_composer_chrome_attrs(sanitized_body);
    format!(
        concat!(
            r#"<div class="mlnr-compose-body">"#,
            r#"<div class="mlnr-compose-reply-editor"><br></div>"#,
            r#"<div class="mlnr-compose-quote" data-mlnr-quote="1">"#,
            r#"<p class="mlnr-attribution">{attr}</p>"#,
            r#"<blockquote class="mlnr-quote-body">{body}</blockquote>"#,
            r#"</div></div>"#
        ),
        attr = attr_esc,
        body = body
    )
}

fn strip_composer_chrome_attrs(html: &str) -> String {
    // Best-effort: drop class/data-mlnr-* from the untrusted fragment before wrapping.
    let re_class = regex::Regex::new(r#"(?i)\sclass\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+)"#).unwrap();
    let re_mlnr =
        regex::Regex::new(r#"(?i)\sdata-mlnr-[a-z0-9_-]+\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+)"#)
            .unwrap();
    let s = re_class.replace_all(html, "");
    re_mlnr.replace_all(&s, "").into_owned()
}

fn html_escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

fn apply_reply_threading(draft: &mut DraftDocument, env: &Envelope) {
    let Some(mid) = env
        .rfc_message_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    draft.in_reply_to = Some(mid.to_string());
    let mut refs = env.references.clone();
    if refs.is_empty() {
        if let Some(parent) = env
            .in_reply_to
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            refs.push(parent.to_string());
        }
    }
    if !refs.iter().any(|r| ids_equal(r, mid)) {
        refs.push(mid.to_string());
    }
    draft.references = refs;
}

fn ids_equal(a: &str, b: &str) -> bool {
    fn bare(s: &str) -> &str {
        s.trim().trim_start_matches('<').trim_end_matches('>')
    }
    bare(a).eq_ignore_ascii_case(bare(b))
}

fn reply_to_addresses(env: &Envelope) -> Vec<ComposerAddress> {
    env.reply_to
        .as_ref()
        .or(env.from.as_ref())
        .map(flatten_addresses)
        .unwrap_or_default()
}

fn reply_all_addresses(
    env: &Envelope,
    self_email: &str,
) -> (Vec<ComposerAddress>, Vec<ComposerAddress>) {
    let mut to = Vec::new();
    if let Some(reply_addr) = env.reply_to.as_ref().or(env.from.as_ref()) {
        to.extend(flatten_addresses(reply_addr));
    }
    if let Some(orig_to) = &env.to {
        to.extend(flatten_addresses(orig_to));
    }
    to = exclude_self(to, self_email);
    to = dedupe_addresses(to);

    let mut cc = env.cc.as_ref().map(flatten_addresses).unwrap_or_default();
    cc = exclude_self(cc, self_email);
    // Don't duplicate addresses already in To.
    cc.retain(|c| {
        !to.iter()
            .any(|t| crate::model::emails_equal(&t.email, &c.email))
    });
    cc = dedupe_addresses(cc);

    (to, cc)
}

enum BodyPick {
    Plain(String),
    HtmlOnly(String),
    /// Multipart alternative: rich HTML quote + plain alternative text.
    HtmlWithPlain {
        plain: String,
        html: String,
    },
}

fn pick_body(loaded: &LoadedMessage) -> Result<BodyPick, PrefillError> {
    let mut plain: Option<String> = None;
    let mut html: Option<String> = None;

    for part in &loaded.parts {
        if part.is_hidden || part.is_attachment {
            continue;
        }
        match part.kind {
            PartKind::TextPlain if plain.is_none() => {
                if let MessageContent::Text(t) = &part.content {
                    plain = Some(t.clone());
                }
            }
            PartKind::TextHtml if html.is_none() => {
                if let MessageContent::Text(t) = &part.content {
                    html = Some(t.clone());
                }
            }
            _ => {}
        }
    }

    match (plain, html) {
        (Some(p), Some(h)) => Ok(BodyPick::HtmlWithPlain { plain: p, html: h }),
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
            id: MessageId::new(FolderId::new("INBOX"), "1"),
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
            reply_to: None,
            rfc_message_id: None,
            in_reply_to: None,
            references: Vec::new(),
            date: Utc::now(),
            is_read: true,
            is_answered: false,
            is_starred: false,
            is_flagged: false,
            is_draft: false,
            is_deleted: false,
            has_attachments: false,
            size: None,
        }
    }

    fn loaded_plain(text: &str) -> LoadedMessage {
        LoadedMessage {
            envelope_id: MessageId::new(FolderId::new("INBOX"), "1"),
            folder_id: FolderId::new("INBOX"),
            parts: vec![MessagePart {
                id: MessagePartId::new("p1"),
                envelope_id: MessageId::new(FolderId::new("INBOX"), "1"),
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
                content: MessageContent::Text(text.into()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }],
        }
    }

    fn loaded_html_only(html: &str) -> LoadedMessage {
        loaded_html_with_parts(html, Vec::new())
    }

    fn html_part(html: &str) -> MessagePart {
        MessagePart {
            id: MessagePartId::new("h1"),
            envelope_id: MessageId::new(FolderId::new("INBOX"), "1"),
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
            content: MessageContent::Text(html.into()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn loaded_html_with_parts(html: &str, extra: Vec<MessagePart>) -> LoadedMessage {
        let mut parts = vec![html_part(html)];
        parts.extend(extra);
        LoadedMessage {
            envelope_id: MessageId::new(FolderId::new("INBOX"), "1"),
            folder_id: FolderId::new("INBOX"),
            parts,
        }
    }

    fn cid_png(cid: &str, bytes: &[u8]) -> MessagePart {
        MessagePart {
            id: MessagePartId::new("img"),
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
            created_at: Utc::now(),
            updated_at: Utc::now(),
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
    fn reply_prefers_reply_to_and_fills_threading() {
        let id = FromIdentity::new("Me", "me@example.com");
        let mut env = env_with_from("alice@example.com");
        env.reply_to = Some(EmailAddress::List(vec![EmailAddr {
            name: Some("Alice Reply".into()),
            email: Some("reply@example.com".into()),
        }]));
        env.rfc_message_id = Some("<mid@x>".into());
        env.in_reply_to = Some("<parent@x>".into());
        env.references = vec!["<root@x>".into(), "<parent@x>".into()];
        let loaded = loaded_plain("x");
        let d = build_draft(ComposeIntent::Reply, &id, Some(&env), Some(&loaded)).unwrap();
        assert_eq!(d.to[0].email, "reply@example.com");
        assert_eq!(d.in_reply_to.as_deref(), Some("<mid@x>"));
        assert_eq!(
            d.references,
            vec![
                "<root@x>".to_string(),
                "<parent@x>".into(),
                "<mid@x>".into()
            ]
        );
    }

    #[test]
    fn forward_does_not_set_threading() {
        let id = FromIdentity::new("Me", "me@example.com");
        let mut env = env_with_from("alice@example.com");
        env.rfc_message_id = Some("<mid@x>".into());
        let loaded = loaded_plain("body");
        let d = build_draft(ComposeIntent::Forward, &id, Some(&env), Some(&loaded)).unwrap();
        assert!(d.in_reply_to.is_none());
        assert!(d.references.is_empty());
    }

    #[test]
    fn html_only_sanitized_rich_quote() {
        let id = FromIdentity::new("Me", "me@example.com");
        let env = env_with_from("alice@example.com");
        let loaded = loaded_html_only("<p>Hi</p><script>alert(1)</script>");
        let d = build_draft(ComposeIntent::Reply, &id, Some(&env), Some(&loaded)).unwrap();
        assert_eq!(d.mode, BodyMode::Rich);
        assert!(d.html_body.contains("Hi"), "{}", d.html_body);
        assert!(!d.html_body.contains("script"), "{}", d.html_body);
        assert!(d.html_body.contains("data-mlnr-quote"), "{}", d.html_body);
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

    #[test]
    fn reply_html_rehydrates_cid_images() {
        let id = FromIdentity::new("Me", "me@example.com");
        let env = env_with_from("alice@example.com");
        let png = [0x89, b'P', b'N', b'G', 0, 1, 2, 3];
        let loaded = loaded_html_with_parts(
            r#"<p>Hi <img src="cid:logo@x" alt="logo"></p>"#,
            vec![cid_png("logo@x", &png)],
        );
        let d = build_draft(ComposeIntent::Reply, &id, Some(&env), Some(&loaded)).unwrap();
        assert_eq!(d.mode, BodyMode::Rich);
        assert_eq!(d.inline_images.len(), 1);
        assert_eq!(d.inline_images[0].content_id, "logo@x");
        assert!(d.html_body.contains("cid:logo@x"), "{}", d.html_body);
        assert!(!d.html_body.contains("data:"), "{}", d.html_body);
        assert!(d.prefill_warnings.is_empty());
    }

    #[test]
    fn forward_html_rehydrates_cid_images() {
        let id = FromIdentity::new("Me", "me@example.com");
        let env = env_with_from("alice@example.com");
        let png = [1u8, 2, 3, 4];
        let loaded = loaded_html_with_parts(
            r#"<img src="cid:pic@mailiner">"#,
            vec![cid_png("<pic@mailiner>", &png)],
        );
        let d = build_draft(ComposeIntent::Forward, &id, Some(&env), Some(&loaded)).unwrap();
        assert_eq!(d.inline_images.len(), 1);
        assert_eq!(d.inline_images[0].content_id, "pic@mailiner");
        assert!(d.html_body.contains("cid:pic@mailiner"), "{}", d.html_body);
    }

    #[test]
    fn reply_strips_missing_cid_with_warning() {
        let id = FromIdentity::new("Me", "me@example.com");
        let env = env_with_from("alice@example.com");
        let loaded = loaded_html_only(r#"<p>Hi <img src="cid:gone@x"></p>"#);
        let d = build_draft(ComposeIntent::Reply, &id, Some(&env), Some(&loaded)).unwrap();
        assert!(d.inline_images.is_empty());
        assert!(!d.html_body.contains("cid:gone@x"), "{}", d.html_body);
        assert!(d.prefill_warnings.iter().any(|w| w.contains("Missing")));
    }

    #[test]
    fn rehydrated_reply_exports_multipart_related() {
        let id = FromIdentity::new("Me", "me@example.com");
        let env = env_with_from("alice@example.com");
        let png = [0x89, b'P', b'N', b'G'];
        let loaded = loaded_html_with_parts(
            r#"<p>Hi <img src="cid:logo@x"></p>"#,
            vec![cid_png("<logo@x>", &png)],
        );
        let d = build_draft(ComposeIntent::Reply, &id, Some(&env), Some(&loaded)).unwrap();
        let prepared = crate::prepare_submit(&d, &id).unwrap();
        let s = String::from_utf8_lossy(&prepared.rfc822);
        assert!(s.contains("multipart/related"), "{s}");
        assert!(s.contains("Content-ID: <logo@x>"), "{s}");
        assert!(s.contains("cid:logo@x"), "{s}");
    }
}
