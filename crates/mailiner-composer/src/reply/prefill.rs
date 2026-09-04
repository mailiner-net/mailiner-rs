//! Reply / forward draft construction from structured envelope + loaded body.

use mailiner_core::{Envelope, LoadedMessage, MessageContent, MessagePart, PartKind};

use crate::identity::FromIdentity;
use crate::model::attachment::{AttachmentData, AttachmentId, AttachmentSource, FileAttachment};
use crate::model::draft::{caps, BodyMode, ComposerAddress, DraftDocument};
use crate::model::recipients::{dedupe_addresses, exclude_self, flatten_addresses};
use crate::reply::quote::{attribution_line, quote_plain, subject_with_prefix};
use crate::reply::signature::apply_plain_signature;
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
    /// Forward body and original non-inline file attachments.
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
///
/// `signature` is applied once to the initial plain body (after any quote).
/// Pass `None` when the account has no signature.
pub fn build_draft(
    intent: ComposeIntent,
    identity: &FromIdentity,
    envelope: Option<&Envelope>,
    loaded: Option<&LoadedMessage>,
    signature: Option<&str>,
) -> Result<DraftDocument, PrefillError> {
    let mut draft = match intent {
        ComposeIntent::New => DraftDocument::new_empty(identity),
        ComposeIntent::Reply | ComposeIntent::ReplyAll | ComposeIntent::Forward => {
            let env = envelope.ok_or(PrefillError::EnvelopeRequired)?;
            let loaded = loaded.ok_or(PrefillError::BodyNotLoaded)?;
            build_reply_like(intent, identity, env, loaded)?
        }
    };
    apply_plain_signature(&mut draft, signature);
    Ok(draft)
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

    if intent == ComposeIntent::Forward {
        apply_forward_attachments(&mut draft, loaded);
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

/// Drop the rich quote and its CID inlines when the caller will send plain text.
///
/// `build_draft` still attaches inlines for a rich export path. The v1 compose
/// overlay forces [`BodyMode::Plain`] and must call this so send does not emit
/// unused `multipart/related` parts.
pub fn discard_rich_quote(draft: &mut DraftDocument) {
    draft.mode = BodyMode::Plain;
    draft.html_body.clear();
    draft.inline_images.clear();
}

/// True for original parts that should be attached when forwarding.
///
/// Includes non-inline file attachments (`is_attachment && !is_hidden`).
/// Skips cid/inline images already represented in the quoted body.
pub fn is_forwardable_attachment(part: &MessagePart) -> bool {
    part.is_attachment && !part.is_hidden
}

fn apply_forward_attachments(draft: &mut DraftDocument, loaded: &LoadedMessage) {
    let mut used = draft_payload_bytes(draft);
    let mut skipped = 0usize;

    for part in loaded.parts.iter().filter(|p| is_forwardable_attachment(p)) {
        if draft.attachments.len() >= caps::MAX_ATTACHMENTS {
            skipped += 1;
            continue;
        }
        let (data, size) = part_attachment_data(part);
        if size > caps::MAX_FILE_BYTES {
            skipped += 1;
            continue;
        }
        if size > 0 && used.saturating_add(size) > caps::MAX_DRAFT_BYTES {
            skipped += 1;
            continue;
        }
        used = used.saturating_add(size);
        draft.attachments.push(FileAttachment {
            id: AttachmentId::new(),
            filename: forward_filename(part),
            content_type: forward_content_type(part),
            size,
            data,
            source: Some(AttachmentSource {
                message_id: loaded.envelope_id.clone(),
                section: part.section(),
                encoding: part.encoding,
            }),
        });
    }

    if skipped > 0 {
        draft.prefill_warnings.push(format!(
            "{skipped} original attachment(s) were skipped (size or file limit)."
        ));
    }
}

fn part_attachment_data(part: &MessagePart) -> (AttachmentData, u64) {
    match &part.content {
        MessageContent::Binary(b) => (AttachmentData::Bytes(b.clone()), b.len() as u64),
        MessageContent::Text(t) => {
            let b = t.as_bytes().to_vec();
            let n = b.len() as u64;
            (AttachmentData::Bytes(b), n)
        }
        MessageContent::Empty => {
            let size = if part.size > 0 {
                part.size
            } else {
                part.original_size.unwrap_or(0)
            };
            (AttachmentData::Pending, size)
        }
    }
}

fn forward_filename(part: &MessagePart) -> String {
    if let Some(f) = part
        .filename
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return f.to_string();
    }
    if let Some(d) = part
        .description
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return d.to_string();
    }
    let ext = match part.content_type.split(';').next().unwrap_or("").trim() {
        "application/pdf" => "pdf",
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "text/plain" => "txt",
        "message/rfc822" => "eml",
        _ => "bin",
    };
    format!("attachment.{ext}")
}

fn forward_content_type(part: &MessagePart) -> String {
    let ct = part.content_type.split(';').next().unwrap_or("").trim();
    if ct.is_empty() {
        "application/octet-stream".into()
    } else {
        ct.to_string()
    }
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
            snippet: None,
            auth_results: Default::default(),
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
        let d = build_draft(ComposeIntent::New, &id, None, None, None).unwrap();
        assert_eq!(d.mode, BodyMode::Rich);
        assert!(d.to.is_empty());
        assert_eq!(d.from.as_ref().unwrap().email, "me@example.com");
    }

    #[test]
    fn reply_plain_quotes() {
        let id = FromIdentity::new("Me", "me@example.com");
        let env = env_with_from("alice@example.com");
        let loaded = loaded_plain("Hi there");
        let d = build_draft(ComposeIntent::Reply, &id, Some(&env), Some(&loaded), None).unwrap();
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
        let d = build_draft(
            ComposeIntent::ReplyAll,
            &id,
            Some(&env),
            Some(&loaded),
            None,
        )
        .unwrap();
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
        let d = build_draft(ComposeIntent::Reply, &id, Some(&env), Some(&loaded), None).unwrap();
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
        let d = build_draft(ComposeIntent::Forward, &id, Some(&env), Some(&loaded), None).unwrap();
        assert!(d.in_reply_to.is_none());
        assert!(d.references.is_empty());
    }

    #[test]
    fn html_only_sanitized_rich_quote() {
        let id = FromIdentity::new("Me", "me@example.com");
        let env = env_with_from("alice@example.com");
        let loaded = loaded_html_only("<p>Hi</p><script>alert(1)</script>");
        let d = build_draft(ComposeIntent::Reply, &id, Some(&env), Some(&loaded), None).unwrap();
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
        let d = build_draft(ComposeIntent::Forward, &id, Some(&env), Some(&loaded), None).unwrap();
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
        let d = build_draft(ComposeIntent::Reply, &id, Some(&env), Some(&loaded), None).unwrap();
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
        let d = build_draft(ComposeIntent::Forward, &id, Some(&env), Some(&loaded), None).unwrap();
        assert_eq!(d.inline_images.len(), 1);
        assert_eq!(d.inline_images[0].content_id, "pic@mailiner");
        assert!(d.html_body.contains("cid:pic@mailiner"), "{}", d.html_body);
    }

    #[test]
    fn reply_strips_missing_cid_with_warning() {
        let id = FromIdentity::new("Me", "me@example.com");
        let env = env_with_from("alice@example.com");
        let loaded = loaded_html_only(r#"<p>Hi <img src="cid:gone@x"></p>"#);
        let d = build_draft(ComposeIntent::Reply, &id, Some(&env), Some(&loaded), None).unwrap();
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
        let d = build_draft(ComposeIntent::Reply, &id, Some(&env), Some(&loaded), None).unwrap();
        let prepared = crate::prepare_submit(&d, &id).unwrap();
        let s = String::from_utf8_lossy(&prepared.rfc822);
        assert!(s.contains("multipart/related"), "{s}");
        assert!(s.contains("Content-ID: <logo@x>"), "{s}");
        assert!(s.contains("cid:logo@x"), "{s}");
    }

    #[test]
    fn discard_rich_quote_clears_inlines() {
        let id = FromIdentity::new("Me", "me@example.com");
        let env = env_with_from("alice@example.com");
        let png = [1u8, 2, 3];
        let loaded = loaded_html_with_parts(
            r#"<p>Hi <img src="cid:logo@x"></p>"#,
            vec![cid_png("logo@x", &png)],
        );
        let mut d =
            build_draft(ComposeIntent::Reply, &id, Some(&env), Some(&loaded), None).unwrap();
        assert!(!d.inline_images.is_empty());
        assert!(!d.html_body.is_empty());
        discard_rich_quote(&mut d);
        assert_eq!(d.mode, BodyMode::Plain);
        assert!(d.html_body.is_empty());
        assert!(d.inline_images.is_empty());
    }

    #[test]
    fn draft_build_includes_signature_after_quote() {
        let id = FromIdentity::new("Me", "me@example.com");
        let env = env_with_from("alice@example.com");
        let loaded = loaded_plain("Hi there");
        let d = build_draft(
            ComposeIntent::Reply,
            &id,
            Some(&env),
            Some(&loaded),
            Some("Jane Doe"),
        )
        .unwrap();
        assert!(d.plain_body.contains("> Hi there"), "{}", d.plain_body);
        assert!(
            d.plain_body.ends_with("\n-- \nJane Doe"),
            "{}",
            d.plain_body
        );
    }

    fn loaded_parts(parts: Vec<MessagePart>) -> LoadedMessage {
        LoadedMessage {
            envelope_id: MessageId::new(FolderId::new("INBOX"), "1"),
            folder_id: FolderId::new("INBOX"),
            parts,
        }
    }

    struct Tp {
        id: &'static str,
        path: &'static str,
        kind: PartKind,
        content_type: &'static str,
        filename: Option<&'static str>,
        is_attachment: bool,
        is_hidden: bool,
        content: MessageContent,
        content_id: Option<&'static str>,
        size: u64,
    }

    impl Tp {
        fn body(
            path: &'static str,
            kind: PartKind,
            content_type: &'static str,
            text: &str,
        ) -> Self {
            Self {
                id: path,
                path,
                kind,
                content_type,
                filename: None,
                is_attachment: false,
                is_hidden: false,
                content: MessageContent::Text(text.into()),
                content_id: None,
                size: text.len() as u64,
            }
        }

        fn file(path: &'static str, content_type: &'static str, filename: &'static str) -> Self {
            Self {
                id: path,
                path,
                kind: PartKind::Attachment,
                content_type,
                filename: Some(filename),
                is_attachment: true,
                is_hidden: false,
                content: MessageContent::Empty,
                content_id: None,
                size: 50,
            }
        }

        fn hidden_cid(path: &'static str) -> Self {
            Self {
                id: path,
                path,
                kind: PartKind::Image,
                content_type: "image/png",
                filename: Some("logo.png"),
                is_attachment: true,
                is_hidden: true,
                content: MessageContent::Binary(vec![1, 2, 3]),
                content_id: Some("<logo@x>"),
                size: 3,
            }
        }

        fn size(mut self, size: u64) -> Self {
            self.size = size;
            self
        }

        fn bytes(mut self, data: Vec<u8>) -> Self {
            self.size = data.len() as u64;
            self.content = MessageContent::Binary(data);
            self
        }

        fn cid_image(path: &'static str, filename: &'static str) -> Self {
            Self {
                id: path,
                path,
                kind: PartKind::Image,
                content_type: "image/jpeg",
                filename: Some(filename),
                is_attachment: true,
                is_hidden: false,
                content: MessageContent::Empty,
                content_id: Some("<pic@x>"),
                size: 80,
            }
        }

        fn text_file(path: &'static str, filename: &'static str) -> Self {
            Self {
                id: path,
                path,
                kind: PartKind::TextPlain,
                content_type: "text/plain",
                filename: Some(filename),
                is_attachment: true,
                is_hidden: false,
                content: MessageContent::Empty,
                content_id: None,
                size: 20,
            }
        }

        fn nameless_pdf(path: &'static str) -> Self {
            Self {
                id: path,
                path,
                kind: PartKind::Attachment,
                content_type: "application/pdf",
                filename: None,
                is_attachment: true,
                is_hidden: false,
                content: MessageContent::Empty,
                content_id: None,
                size: 1,
            }
        }

        fn finish(self) -> MessagePart {
            MessagePart {
                id: MessagePartId::new(self.id),
                envelope_id: MessageId::new(FolderId::new("INBOX"), "1"),
                path: vec![self.path.into()],
                kind: self.kind,
                content_type: self.content_type.into(),
                charset: None,
                content_id: self.content_id.map(str::to_string),
                description: None,
                filename: self.filename.map(str::to_string),
                encoding: TransferEncoding::Base64,
                original_size: Some(self.size),
                size: self.size,
                is_attachment: self.is_attachment,
                is_hidden: self.is_hidden,
                content: self.content,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }
        }
    }

    fn selected_names(loaded: &LoadedMessage) -> Vec<String> {
        loaded
            .parts
            .iter()
            .filter(|p| is_forwardable_attachment(p))
            .map(forward_filename)
            .collect()
    }

    #[test]
    fn selects_non_inline_file_attachments() {
        let loaded = loaded_parts(vec![
            Tp::body("1", PartKind::TextPlain, "text/plain", "hi").finish(),
            Tp::file("2", "application/pdf", "report.pdf")
                .size(100)
                .finish(),
            Tp::text_file("3", "notes.txt").finish(),
        ]);
        assert_eq!(
            selected_names(&loaded),
            vec!["report.pdf".to_string(), "notes.txt".into()]
        );
    }

    #[test]
    fn draft_build_missing_signature_is_noop() {
        let id = FromIdentity::new("Me", "me@example.com");
        let env = env_with_from("alice@example.com");
        let loaded = loaded_plain("Hi there");
        let with = build_draft(
            ComposeIntent::Reply,
            &id,
            Some(&env),
            Some(&loaded),
            Some("  "),
        )
        .unwrap();
        let without =
            build_draft(ComposeIntent::Reply, &id, Some(&env), Some(&loaded), None).unwrap();
        assert_eq!(with.plain_body, without.plain_body);
        assert!(
            !without.plain_body.contains("-- "),
            "{}",
            without.plain_body
        );
    }

    #[test]
    fn new_draft_includes_signature() {
        let id = FromIdentity::new("Me", "me@example.com");
        let d = build_draft(ComposeIntent::New, &id, None, None, Some("Jane Doe")).unwrap();
        assert_eq!(d.plain_body, "\n-- \nJane Doe");
        assert_eq!(d.mode, BodyMode::Plain);
        assert!(d.html_body.is_empty());
    }

    #[test]
    fn html_reply_with_signature_is_plain_authoritative() {
        let id = FromIdentity::new("Me", "me@example.com");
        let env = env_with_from("alice@example.com");
        let loaded = loaded_html_only("<p>Hi</p>");
        let d = build_draft(
            ComposeIntent::Reply,
            &id,
            Some(&env),
            Some(&loaded),
            Some("Jane Doe"),
        )
        .unwrap();
        assert_eq!(d.mode, BodyMode::Plain);
        assert!(d.html_body.is_empty());
        assert!(
            d.plain_body.ends_with("\n-- \nJane Doe"),
            "{}",
            d.plain_body
        );
    }

    #[test]
    fn skips_hidden_cid_inline_images() {
        let loaded = loaded_parts(vec![
            Tp::body(
                "1",
                PartKind::TextHtml,
                "text/html",
                "<img src=\"cid:logo@x\">",
            )
            .finish(),
            Tp::hidden_cid("2").finish(),
            Tp::file("3", "application/pdf", "file.pdf").finish(),
        ]);
        assert_eq!(selected_names(&loaded), vec!["file.pdf".to_string()]);
        assert!(!is_forwardable_attachment(&loaded.parts[1]));
        assert!(is_forwardable_attachment(&loaded.parts[2]));
    }

    #[test]
    fn includes_cid_image_when_real_attachment() {
        // Parser leaves is_hidden=false when disposition is ATTACHMENT.
        let loaded = loaded_parts(vec![Tp::cid_image("2", "photo.jpg").finish()]);
        assert_eq!(selected_names(&loaded), vec!["photo.jpg".to_string()]);
    }

    #[test]
    fn skips_display_body_parts() {
        let loaded = loaded_parts(vec![
            Tp::body("1", PartKind::TextPlain, "text/plain", "body").finish(),
            Tp::body("2", PartKind::TextHtml, "text/html", "<p>body</p>").finish(),
        ]);
        assert!(selected_names(&loaded).is_empty());
    }

    #[test]
    fn reply_does_not_copy_attachments() {
        let id = FromIdentity::new("Me", "me@example.com");
        let env = env_with_from("alice@example.com");
        let loaded = loaded_parts(vec![
            Tp::body("1", PartKind::TextPlain, "text/plain", "hi").finish(),
            Tp::file("2", "application/pdf", "report.pdf")
                .bytes(b"%PDF".to_vec())
                .finish(),
        ]);
        let reply =
            build_draft(ComposeIntent::Reply, &id, Some(&env), Some(&loaded), None).unwrap();
        assert!(reply.attachments.is_empty());
        let fwd =
            build_draft(ComposeIntent::Forward, &id, Some(&env), Some(&loaded), None).unwrap();
        assert_eq!(fwd.attachments.len(), 1);
        assert_eq!(fwd.attachments[0].filename, "report.pdf");
    }

    #[test]
    fn forward_copies_loaded_bytes_and_pends_empty() {
        let id = FromIdentity::new("Me", "me@example.com");
        let env = env_with_from("alice@example.com");
        let loaded = loaded_parts(vec![
            Tp::body("1", PartKind::TextPlain, "text/plain", "hi").finish(),
            Tp::file("2", "application/pdf", "report.pdf")
                .bytes(b"%PDF".to_vec())
                .finish(),
            Tp::file("3", "application/zip", "data.zip")
                .size(200)
                .finish(),
        ]);
        let d = build_draft(ComposeIntent::Forward, &id, Some(&env), Some(&loaded), None).unwrap();
        assert_eq!(d.attachments.len(), 2);
        assert!(matches!(
            &d.attachments[0].data,
            AttachmentData::Bytes(b) if b == b"%PDF"
        ));
        assert_eq!(d.attachments[0].size, 4);
        assert!(matches!(d.attachments[1].data, AttachmentData::Pending));
        assert_eq!(d.attachments[1].size, 200);
        let src = d.attachments[1].source.as_ref().expect("source");
        assert_eq!(src.section, "3");
        assert_eq!(src.message_id.as_uid(), "1");
    }

    #[test]
    fn forward_skips_oversize_and_hidden() {
        let id = FromIdentity::new("Me", "me@example.com");
        let env = env_with_from("alice@example.com");
        let too_big = caps::MAX_FILE_BYTES + 1;
        let loaded = loaded_parts(vec![
            Tp::body("1", PartKind::TextPlain, "text/plain", "hi").finish(),
            Tp::hidden_cid("2").finish(),
            Tp::file("3", "application/zip", "huge.zip")
                .size(too_big)
                .finish(),
            Tp::file("4", "application/pdf", "ok.pdf").size(10).finish(),
        ]);
        let d = build_draft(ComposeIntent::Forward, &id, Some(&env), Some(&loaded), None).unwrap();
        assert_eq!(
            d.attachments
                .iter()
                .map(|a| a.filename.as_str())
                .collect::<Vec<_>>(),
            vec!["ok.pdf"]
        );
        assert_eq!(d.prefill_warnings.len(), 1);
        assert!(d.prefill_warnings[0].contains("skipped"));
    }

    #[test]
    fn forward_filename_falls_back_from_type() {
        let p = Tp::nameless_pdf("2").finish();
        assert_eq!(forward_filename(&p), "attachment.pdf");
        assert!(is_forwardable_attachment(&p));
    }
}
