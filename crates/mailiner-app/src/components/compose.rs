//! Plain-text compose overlay (modal dialog or docked bottom pane).

use dioxus::html::Key;
use dioxus::prelude::*;

use super::icons::{Icon, IconButton, IconKind};
use super::recipient_field::RecipientField;

use wasm_bindgen::JsCast;

use mailiner_composer::editor::{SpellcheckField, spellcheck_attr};
use mailiner_composer::identity::FromIdentity;
use mailiner_composer::model::draft::{BodyMode, ComposerAddress, DraftDocument};
use mailiner_composer::shell::attachment_list::{
    draft_payload_bytes, file_attachment, html_for_plain_with_inlines, human_size, image_filename,
    inline_image, looks_like_inline_image, resolve_content_type, would_exceed_draft_cap,
};
use mailiner_composer::shell::recipient_field::commit_input;
use mailiner_composer::{
    AttachmentData, ComposeIntent, FileAttachment, InlineImage, PrepareSubmitError,
    SAFE_IMAGE_ACCEPT, build_draft, caps, discard_rich_quote, draft_from_stored_message,
    is_safe_image_content_type, is_valid_email_v1, plain_to_html, prepare_draft, prepare_submit,
};

use std::collections::HashMap;

use crate::account::{Account, AccountId};
use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::draft_store::{self, session_has_content};
use crate::recipient_suggest;
use crate::send::{
    ComposeSession, OutboxDisplay, SendState, composer_address_from_identity, from_account_label,
    identity_for_reply, identity_from_stored, list_from_choices, parse_from_choice_key,
    resolve_account_identity, resolve_compose_account_id, selected_from_choice,
    set_session_from_identity, strip_account_identities,
};
use crate::ui_prefs::{ComposeBodyMode, ComposePlacement};

fn looks_like_email(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty() && s.contains('@') && !s.contains(char::is_whitespace)
}

fn needs_quotes(name: &str) -> bool {
    name.contains([',', '<', '>', '"', '\\']) || looks_like_email(name)
}

fn quote_display_name(name: &str) -> String {
    format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
}

fn format_composer_address(addr: &ComposerAddress) -> String {
    match addr
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        Some(name) if needs_quotes(name) => {
            format!("{} <{}>", quote_display_name(name), addr.email)
        }
        Some(name) => format!("{name} <{}>", addr.email),
        None => addr.email.clone(),
    }
}

fn join_address_list(addrs: &[ComposerAddress]) -> String {
    addrs
        .iter()
        .map(format_composer_address)
        .collect::<Vec<_>>()
        .join(", ")
}

fn named_composer_address(name: &str, email: &str) -> Option<ComposerAddress> {
    let email = email.trim();
    if email.is_empty() {
        return None;
    }
    let name = name.trim();
    Some(ComposerAddress {
        name: if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        },
        email: email.to_string(),
    })
}

/// Apply the settings default. The overlay is a textarea; Rich sends an HTML
/// alternative of that text (reply HTML quotes are re-derived from the plain body).
fn apply_compose_body_mode(draft: &mut DraftDocument, mode: ComposeBodyMode) {
    match mode {
        ComposeBodyMode::Plain => {
            draft.mode = BodyMode::Plain;
            draft.html_body.clear();
            draft.plain_cache_dirty = false;
        }
        ComposeBodyMode::Rich => {
            draft.mode = BodyMode::Rich;
            draft.html_body = plain_to_html(&draft.plain_body);
            draft.plain_cache_dirty = false;
        }
    }
}

fn compose_body_mode_from_draft(draft: &DraftDocument) -> ComposeBodyMode {
    match draft.mode {
        BodyMode::Plain => ComposeBodyMode::Plain,
        BodyMode::Rich => ComposeBodyMode::Rich,
    }
}

fn find_unquoted(s: &str, needle: char) -> Option<usize> {
    let mut in_quotes = false;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_quotes => escaped = true,
            '"' => in_quotes = !in_quotes,
            c if c == needle && !in_quotes => return Some(i),
            _ => {}
        }
    }
    None
}

fn split_trailing_quoted(s: &str) -> Option<(&str, String)> {
    let s = s.trim();
    if !s.ends_with('"') {
        return None;
    }
    let mut in_quotes = false;
    let mut escaped = false;
    let mut quote_start = None;
    let mut decoded = String::new();
    for (i, c) in s.char_indices() {
        if escaped {
            decoded.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_quotes => escaped = true,
            '"' if !in_quotes => {
                in_quotes = true;
                quote_start = Some(i);
                decoded.clear();
            }
            '"' if in_quotes => in_quotes = false,
            _ if in_quotes => decoded.push(c),
            _ => {}
        }
    }
    if in_quotes {
        return None;
    }
    let start = quote_start?;
    let prefix = s[..start].trim().trim_end_matches(',').trim();
    Some((prefix, decoded))
}

/// Last comma-separated token before `<email>` is the display name.
/// Earlier email-like tokens are sibling recipients.
fn take_display_name(before: &str, out: &mut Vec<ComposerAddress>) -> String {
    let before = before.trim();
    if before.is_empty() {
        return String::new();
    }
    if let Some((bare, quoted)) = split_trailing_quoted(before) {
        // Only a comma (not whitespace) separates a sibling mailbox from this name.
        // `alice@example.com "Alice"` is one display name; `bob@example.com, "Alice"`
        // is a sibling plus a quoted name.
        let has_sibling_comma = find_unquoted(before, ',').is_some();
        let mut name_prefix = String::new();
        for part in bare.split(',') {
            if name_prefix.is_empty() && has_sibling_comma && looks_like_email(part) {
                out.push(ComposerAddress::email_only(part.trim()));
            } else if name_prefix.is_empty() {
                name_prefix = part.to_string();
            } else {
                name_prefix.push(',');
                name_prefix.push_str(part);
            }
        }
        let name_prefix = name_prefix.trim();
        if name_prefix.is_empty() {
            return quoted;
        }
        return format!("{name_prefix} {quoted}");
    }
    let parts: Vec<&str> = before.split(',').collect();
    let mut name_from = parts.len().saturating_sub(1);
    while name_from > 0 && !looks_like_email(parts[name_from - 1]) {
        name_from -= 1;
    }
    for part in &parts[..name_from] {
        let part = part.trim();
        if !part.is_empty() {
            out.push(ComposerAddress::email_only(part));
        }
    }
    parts[name_from..].join(",")
}

/// Parse a compose field. Named mailboxes (`Name <email>`) keep the display name.
fn parse_address_list(raw: &str) -> Vec<ComposerAddress> {
    let mut out = Vec::new();
    let mut rest = raw.trim();
    while !rest.is_empty() {
        if let Some(open) = find_unquoted(rest, '<')
            && let Some(close_rel) = rest[open..].find('>')
        {
            let close = open + close_rel;
            let name = take_display_name(&rest[..open], &mut out);
            if let Some(addr) = named_composer_address(&name, &rest[open + 1..close]) {
                out.push(addr);
            }
            rest = rest[close + 1..].trim_start_matches(',').trim();
            continue;
        }
        out.extend(
            rest.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ComposerAddress::email_only),
        );
        break;
    }
    out
}

#[derive(Clone, Copy)]
struct RecipientList {
    chips: Signal<Vec<ComposerAddress>>,
    draft: Signal<String>,
}

const DRAFT_SAVE_DEBOUNCE_MS: u32 = 300;

fn join_address_emails(addrs: &[ComposerAddress]) -> String {
    addrs
        .iter()
        .map(|a| a.email.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

impl RecipientList {
    fn apply(&mut self, addrs: &[ComposerAddress]) {
        self.chips.set(addrs.to_vec());
        self.draft.set(String::new());
    }

    fn take_committed(mut self) -> Vec<ComposerAddress> {
        let (next, leftover) = commit_input(&self.chips.read(), &self.draft.read(), true);
        self.chips.set(next.clone());
        self.draft.set(leftover);
        next
    }
}

#[derive(Clone, Copy)]
struct ComposeForm {
    to: RecipientList,
    cc: RecipientList,
    bcc: RecipientList,
    subject: Signal<String>,
    body: Signal<String>,
}

fn apply_draft_fields(draft: &DraftDocument, form: &mut ComposeForm) {
    form.to.apply(&draft.to);
    form.cc.apply(&draft.cc);
    form.bcc.apply(&draft.bcc);
    form.subject.set(draft.subject.clone());
    form.body.set(draft.plain_body.clone());
}

fn compose_send_state(ctx: &AppContext) -> Option<SendState> {
    let account_id = ctx.compose_draft.read().as_ref()?.account_id.clone();
    ctx.send_status.read().get(&account_id).cloned()
}

fn persist_session(session: &ComposeSession) {
    if session_has_content(session) {
        draft_store::save_draft(&session.account_id, session);
    } else {
        draft_store::clear_draft(&session.account_id);
    }
}

fn apply_current_from(session: &mut ComposeSession, identity: &FromIdentity) {
    session.draft.from = Some(composer_address_from_identity(identity));
}

/// Open a new / reply / forward session. Replaces any existing compose draft.
pub fn open_compose(ctx: &mut AppContext, session: ComposeSession) {
    persist_session(&session);
    ctx.compose_draft.set(Some(session));
}

fn listed_compose_accounts(ctx: &AppContext) -> Vec<Account> {
    let mut listed: Vec<Account> = ctx.accounts.read().values().cloned().collect();
    listed.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.id.as_str().cmp(b.id.as_str()))
    });
    listed
}

fn resolve_compose_account(ctx: &AppContext, preferred: Option<&AccountId>) -> Option<Account> {
    let listed = listed_compose_accounts(ctx);
    let stored: Vec<AccountId> = listed.iter().map(|a| a.id.clone()).collect();
    let selected = ctx.selected_account.read().clone();
    let id = resolve_compose_account_id(preferred, selected.as_ref(), &stored)?;
    listed.into_iter().find(|a| a.id == id)
}

fn new_message_draft(ctx: &AppContext) -> Option<(Account, DraftDocument)> {
    let preferred = crate::ui_prefs::load_default_from_account();
    let account = resolve_compose_account(ctx, preferred.as_ref())?;
    let identity = identity_from_stored(&account.primary_identity());
    let mut draft = match build_draft(
        ComposeIntent::New,
        &identity,
        None,
        None,
        account.signature.as_deref(),
    ) {
        Ok(d) => d,
        Err(_) => return None,
    };
    apply_compose_body_mode(&mut draft, crate::ui_prefs::load_compose_body_mode());
    Some((account, draft))
}

fn open_new_draft(ctx: &mut AppContext, account: Account, draft: DraftDocument) {
    open_compose(
        ctx,
        ComposeSession {
            account_id: account.id,
            title: "New message".into(),
            draft,
            reply_source: None,
            imap_draft: None,
            stashed_originals: Vec::new(),
        },
    );
}

/// Open the saved draft for the compose account, or a blank compose.
pub fn open_new_message(ctx: &mut AppContext) {
    let preferred = crate::ui_prefs::load_default_from_account();
    if let Some(account) = resolve_compose_account(ctx, preferred.as_ref()) {
        if let Some(mut session) = draft_store::load_draft(&account.id) {
            session.account_id = account.id.clone();
            let identity = resolve_account_identity(&account, session.draft.from.as_ref());
            apply_current_from(&mut session, &identity_from_stored(&identity));
            ctx.compose_draft.set(Some(session));
            return;
        }
    }
    let Some((account, draft)) = new_message_draft(ctx) else {
        return;
    };
    open_new_draft(ctx, account, draft);
}

/// Open a new message with To prefilled from a viewer address.
pub fn open_new_message_to(ctx: &mut AppContext, to: ComposerAddress) {
    let Some((account, mut draft)) = new_message_draft(ctx) else {
        return;
    };
    draft.to.push(to);
    open_new_draft(ctx, account, draft);
}

/// Prefill Reply or Forward from a loaded message.
pub fn open_reply_or_forward(
    ctx: &mut AppContext,
    intent: ComposeIntent,
    envelope: &mailiner_core::Envelope,
    loaded: &mailiner_core::models::LoadedMessage,
) {
    let Some(account) = resolve_compose_account(ctx, Some(&envelope.account_id)) else {
        ctx.show_toast(crate::toast::ToastAction::error("Select an account first."));
        return;
    };
    let identity = identity_from_stored(&identity_for_reply(
        &account,
        envelope.to.as_ref(),
        envelope.cc.as_ref(),
    ));
    match build_draft(
        intent,
        &identity,
        Some(envelope),
        Some(loaded),
        account.signature.as_deref(),
    ) {
        Ok(mut draft) => {
            if matches!(intent, ComposeIntent::Reply | ComposeIntent::ReplyAll) {
                strip_account_identities(&mut draft, &account);
            }
            let mode = crate::ui_prefs::load_compose_body_mode();
            apply_compose_body_mode(&mut draft, mode);
            if mode == ComposeBodyMode::Plain {
                discard_rich_quote(&mut draft);
            }
            let title = match intent {
                ComposeIntent::Forward => "Forward",
                ComposeIntent::Reply | ComposeIntent::ReplyAll => "Reply",
                ComposeIntent::New => "New message",
            };
            let reply_source = matches!(intent, ComposeIntent::Reply | ComposeIntent::ReplyAll)
                .then(|| envelope.id.clone());
            open_compose(
                ctx,
                ComposeSession {
                    account_id: account.id,
                    title: title.into(),
                    draft,
                    reply_source,
                    imap_draft: None,
                    stashed_originals: Vec::new(),
                },
            );
        }
        Err(e) => ctx.show_toast(crate::toast::ToastAction::error(e.to_string())),
    }
}

/// Open a stored IMAP draft for editing. Replaces any existing compose draft.
pub fn open_imap_draft(
    ctx: &mut AppContext,
    envelope: &mailiner_core::Envelope,
    loaded: &mailiner_core::models::LoadedMessage,
) {
    let Some(account) = resolve_compose_account(ctx, Some(&envelope.account_id)) else {
        ctx.show_toast(crate::toast::ToastAction::error("Select an account first."));
        return;
    };
    let from = envelope.from.as_ref().and_then(flatten_from_for_identity);
    let identity = identity_from_stored(&resolve_account_identity(&account, from.as_ref()));
    match draft_from_stored_message(&identity, envelope, loaded) {
        Ok(mut draft) => {
            let mode = crate::ui_prefs::load_compose_body_mode();
            apply_compose_body_mode(&mut draft, mode);
            if mode == ComposeBodyMode::Plain {
                discard_rich_quote(&mut draft);
            }
            open_compose(
                ctx,
                ComposeSession {
                    account_id: account.id,
                    title: "Draft".into(),
                    draft,
                    reply_source: None,
                    imap_draft: Some(envelope.id.clone()),
                    stashed_originals: Vec::new(),
                },
            );
        }
        Err(e) => ctx.show_toast(crate::toast::ToastAction::error(e.to_string())),
    }
}

fn flatten_from_for_identity(
    from: &mailiner_core::EmailAddress,
) -> Option<mailiner_composer::ComposerAddress> {
    mailiner_composer::flatten_addresses(from)
        .into_iter()
        .next()
}

fn submit_compose(
    ctx: &AppContext,
    core: &Coroutine<CoreEvent>,
    form: ComposeForm,
    mut error: Signal<Option<String>>,
    mut submitting: Signal<bool>,
    mut submitted_id: Signal<Option<String>>,
    attaching: Signal<bool>,
    forward_fetching: Signal<bool>,
) {
    if submitting() || attaching() || forward_fetching() {
        return;
    }
    error.set(None);
    let (account_id, mut draft, reply_source, imap_draft) = match ctx.compose_draft.read().as_ref()
    {
        Some(session) => (
            session.account_id.clone(),
            session.draft.clone(),
            session.reply_source.clone(),
            session.imap_draft.clone(),
        ),
        None => {
            error.set(Some("No draft open.".into()));
            return;
        }
    };
    let Some(account) = ctx.accounts.read().get(&account_id).cloned() else {
        error.set(Some("This draft's account is no longer available.".into()));
        return;
    };
    let identity = identity_from_stored(&resolve_account_identity(&account, draft.from.as_ref()));
    let mode = compose_body_mode_from_draft(&draft);
    if draft
        .from
        .as_ref()
        .is_none_or(|from| !is_valid_email_v1(&from.email))
    {
        draft.from = Some(composer_address_from_identity(&identity));
    }
    draft.plain_body = form.body.peek().clone();
    draft.subject = form.subject.peek().clone();
    apply_compose_body_mode(&mut draft, mode);
    if !draft.inline_images.is_empty() {
        draft.html_body = html_for_plain_with_inlines(&draft.plain_body, &draft.inline_images);
        draft.plain_cache_dirty = false;
    }
    draft.to = form.to.take_committed();
    draft.cc = form.cc.take_committed();
    draft.bcc = form.bcc.take_committed();
    if draft
        .attachments
        .iter()
        .any(|a| matches!(a.data, AttachmentData::Pending))
    {
        error.set(Some("Still loading original attachments.".into()));
        return;
    }
    match prepare_submit(&draft, &identity) {
        Ok(prepared) => {
            recipient_suggest::remember_recipients(
                draft
                    .to
                    .iter()
                    .chain(draft.cc.iter())
                    .chain(draft.bcc.iter()),
            );
            let display = OutboxDisplay {
                subject: draft.subject.clone(),
                to_preview: prepared.envelope.rcpt_to.join(", "),
            };
            let draft_id = draft.id.as_str().to_string();
            submitted_id.set(Some(draft_id.clone()));
            submitting.set(true);
            core.send(CoreEvent::SendMessage {
                account_id,
                request: mailiner_core::SubmitRequest {
                    mail_from: prepared.envelope.mail_from,
                    rcpt_to: prepared.envelope.rcpt_to,
                    rfc822: prepared.rfc822,
                    message_id: prepared.message_id,
                },
                display,
                draft_id,
                bcc_header: prepared.bcc_header,
                reply_source,
                imap_draft,
            });
        }
        Err(PrepareSubmitError::Validation(errs)) => {
            submitting.set(false);
            error.set(Some(format!("Cannot send: {errs:?}")));
        }
        Err(e) => {
            submitting.set(false);
            error.set(Some(e.to_string()));
        }
    }
}

fn open_draft_id(compose_draft: Signal<Option<ComposeSession>>) -> Option<String> {
    compose_draft
        .read()
        .as_ref()
        .map(|s| s.draft.id.as_str().to_string())
}

enum PushAttachment {
    Added,
    TooMany,
    Stale,
    TooLarge,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AttachKind {
    File,
    Inline,
}

fn live_payload_bytes(draft: &DraftDocument, live_plain: &str) -> u64 {
    let live_html_len = if draft.mode == BodyMode::Rich {
        plain_to_html(live_plain).len() as u64
    } else {
        0
    };
    draft_payload_bytes(draft)
        .saturating_sub(draft.plain_body.len() as u64)
        .saturating_sub(draft.html_body.len() as u64)
        .saturating_add(live_plain.len() as u64)
        .saturating_add(live_html_len)
}

fn push_attachment_on_draft(
    mut compose_draft: Signal<Option<ComposeSession>>,
    draft_id: &str,
    attachment: FileAttachment,
    body: Signal<String>,
) -> PushAttachment {
    let mut slot = compose_draft.write();
    let Some(session) = slot.as_mut() else {
        return PushAttachment::Stale;
    };
    if session.draft.id.as_str() != draft_id {
        return PushAttachment::Stale;
    }
    if session.draft.attachments.len() >= caps::MAX_ATTACHMENTS {
        return PushAttachment::TooMany;
    }
    if would_exceed_draft_cap(live_payload_bytes(&session.draft, &body()), attachment.size) {
        return PushAttachment::TooLarge;
    }
    session.draft.attachments.push(attachment);
    session.draft.touch();
    PushAttachment::Added
}

fn remove_attachment(mut compose_draft: Signal<Option<ComposeSession>>, id: &str) {
    let mut slot = compose_draft.write();
    let Some(session) = slot.as_mut() else {
        return;
    };
    let before = session.draft.attachments.len();
    session.draft.attachments.retain(|a| a.id.0 != id);
    session.stashed_originals.retain(|a| a.id.0 != id);
    if session.draft.attachments.len() != before {
        session.draft.touch();
    }
}

fn push_inline_on_draft(
    mut compose_draft: Signal<Option<ComposeSession>>,
    draft_id: &str,
    image: InlineImage,
    body: Signal<String>,
) -> PushAttachment {
    let mut slot = compose_draft.write();
    let Some(session) = slot.as_mut() else {
        return PushAttachment::Stale;
    };
    if session.draft.id.as_str() != draft_id {
        return PushAttachment::Stale;
    }
    if session.draft.inline_images.len() >= caps::MAX_INLINES {
        return PushAttachment::TooMany;
    }
    let extra = match &image.data {
        mailiner_composer::AttachmentData::Bytes(b) => b.len() as u64,
        mailiner_composer::AttachmentData::Pending => 0,
    };
    if would_exceed_draft_cap(live_payload_bytes(&session.draft, &body()), extra) {
        return PushAttachment::TooLarge;
    }
    session.draft.inline_images.push(image);
    session.draft.touch();
    PushAttachment::Added
}

fn remove_inline(mut compose_draft: Signal<Option<ComposeSession>>, id: &str) {
    let mut slot = compose_draft.write();
    let Some(session) = slot.as_mut() else {
        return;
    };
    let before = session.draft.inline_images.len();
    session.draft.inline_images.retain(|a| a.id.0 != id);
    if session.draft.inline_images.len() != before {
        session.draft.touch();
    }
}

fn toggle_original_attachments(
    mut compose_draft: Signal<Option<ComposeSession>>,
    include: bool,
    body: Signal<String>,
) {
    let mut slot = compose_draft.write();
    let Some(session) = slot.as_mut() else {
        return;
    };
    if include {
        if session.stashed_originals.is_empty() {
            return;
        }
        let mut used = live_payload_bytes(&session.draft, &body());
        let mut kept = Vec::new();
        let mut skipped = 0usize;
        for att in session.stashed_originals.drain(..) {
            let extra = match &att.data {
                AttachmentData::Bytes(b) => b.len() as u64,
                AttachmentData::Pending => att.size,
            };
            if session.draft.attachments.len() >= caps::MAX_ATTACHMENTS
                || (extra > 0 && would_exceed_draft_cap(used, extra))
            {
                skipped += 1;
                kept.push(att);
                continue;
            }
            used = used.saturating_add(extra);
            session.draft.attachments.push(att);
        }
        session.stashed_originals = kept;
        if skipped > 0 {
            session.draft.prefill_warnings.push(format!(
                "{skipped} original attachment(s) were skipped (size or file limit)."
            ));
        }
        session.draft.touch();
    } else {
        let (orig, rest): (Vec<_>, Vec<_>) = session
            .draft
            .attachments
            .drain(..)
            .partition(|a| a.source.is_some());
        if orig.is_empty() && session.stashed_originals.is_empty() {
            session.draft.attachments = rest;
            return;
        }
        session.draft.attachments = rest;
        session.stashed_originals.extend(orig);
        session.draft.touch();
    }
}

fn session_with_live_fields(
    session: &ComposeSession,
    to: &[ComposerAddress],
    to_draft: &str,
    cc: &[ComposerAddress],
    cc_draft: &str,
    bcc: &[ComposerAddress],
    bcc_draft: &str,
    subject: &str,
    body: &str,
) -> ComposeSession {
    let mut session = session.clone();
    session.draft.to = commit_input(to, to_draft, true).0;
    session.draft.cc = commit_input(cc, cc_draft, true).0;
    session.draft.bcc = commit_input(bcc, bcc_draft, true).0;
    session.draft.subject = subject.to_string();
    session.draft.plain_body = body.to_string();
    session.draft.touch();
    session
}

fn persist_live_draft(
    compose_draft: Signal<Option<ComposeSession>>,
    to: Signal<Vec<ComposerAddress>>,
    to_draft: Signal<String>,
    cc: Signal<Vec<ComposerAddress>>,
    cc_draft: Signal<String>,
    bcc: Signal<Vec<ComposerAddress>>,
    bcc_draft: Signal<String>,
    subject: Signal<String>,
    body: Signal<String>,
) -> Option<ComposeSession> {
    let session = compose_draft.peek().clone()?;
    let live = session_with_live_fields(
        &session,
        &to.peek(),
        &to_draft.peek(),
        &cc.peek(),
        &cc_draft.peek(),
        &bcc.peek(),
        &bcc_draft.peek(),
        &subject.peek(),
        &body.peek(),
    );
    persist_session(&live);
    Some(live)
}

fn queue_imap_draft_save(
    accounts: Signal<HashMap<AccountId, Account>>,
    core: &Coroutine<CoreEvent>,
    session: &ComposeSession,
) {
    if session
        .draft
        .attachments
        .iter()
        .any(|a| matches!(a.data, AttachmentData::Pending))
        || session
            .draft
            .inline_images
            .iter()
            .any(|img| matches!(img.data, AttachmentData::Pending))
    {
        return;
    }
    if !session_has_content(session) {
        if let Some(message_id) = session.imap_draft.clone() {
            core.send(CoreEvent::DeleteImapDraft {
                account_id: session.account_id.clone(),
                message_id,
            });
        }
        return;
    }
    let Some(account) = accounts.read().get(&session.account_id).cloned() else {
        return;
    };
    let identity = identity_from_stored(&resolve_account_identity(
        &account,
        session.draft.from.as_ref(),
    ));
    match prepare_draft(&session.draft, &identity) {
        Ok(prepared) => {
            core.send(CoreEvent::SaveImapDraft {
                account_id: session.account_id.clone(),
                draft_id: session.draft.id.as_str().to_string(),
                rfc822: prepared.rfc822,
                replace: session.imap_draft.clone(),
            });
        }
        Err(_) => {}
    }
}

fn close_keeping_draft(
    mut save_gen: Signal<u32>,
    mut compose_draft: Signal<Option<ComposeSession>>,
    to: Signal<Vec<ComposerAddress>>,
    to_draft: Signal<String>,
    cc: Signal<Vec<ComposerAddress>>,
    cc_draft: Signal<String>,
    bcc: Signal<Vec<ComposerAddress>>,
    bcc_draft: Signal<String>,
    subject: Signal<String>,
    body: Signal<String>,
    accounts: Signal<HashMap<AccountId, Account>>,
    core: &Coroutine<CoreEvent>,
) {
    let next = *save_gen.peek() + 1;
    save_gen.set(next);
    if let Some(live) = persist_live_draft(
        compose_draft,
        to,
        to_draft,
        cc,
        cc_draft,
        bcc,
        bcc_draft,
        subject,
        body,
    ) {
        queue_imap_draft_save(accounts, core, &live);
    }
    compose_draft.set(None);
}

fn discard_draft(
    mut save_gen: Signal<u32>,
    mut compose_draft: Signal<Option<ComposeSession>>,
    core: &Coroutine<CoreEvent>,
) {
    let next = *save_gen.peek() + 1;
    save_gen.set(next);
    if let Some(session) = compose_draft.peek().as_ref() {
        if let Some(message_id) = session.imap_draft.clone() {
            core.send(CoreEvent::DeleteImapDraft {
                account_id: session.account_id.clone(),
                message_id,
            });
        }
        draft_store::clear_draft(&session.account_id);
    }
    compose_draft.set(None);
}

async fn sleep_ms(ms: u32) {
    #[cfg(target_arch = "wasm32")]
    {
        gloo_timers::future::TimeoutFuture::new(ms).await;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        tokio::time::sleep(std::time::Duration::from_millis(ms as u64)).await;
    }
}

fn has_original_attachments(session: &ComposeSession) -> bool {
    session.draft.attachments.iter().any(|a| a.source.is_some())
        || !session.stashed_originals.is_empty()
}

fn has_pending_forward_fetch(session: &ComposeSession) -> bool {
    session
        .draft
        .attachments
        .iter()
        .any(|a| matches!(a.data, AttachmentData::Pending) && a.source.is_some())
}

fn oversize_message(filename: &str) -> String {
    let max_mib = caps::MAX_FILE_BYTES / (1024 * 1024);
    format!("\"{filename}\" is larger than {max_mib} MiB.")
}

fn too_many_message() -> String {
    format!("You can attach at most {} files.", caps::MAX_ATTACHMENTS)
}

fn too_many_inlines_message() -> String {
    format!("You can insert at most {} images.", caps::MAX_INLINES)
}

fn oversize_draft_message() -> String {
    let max_mib = caps::MAX_DRAFT_BYTES / (1024 * 1024);
    format!("Attachments would exceed the {max_mib} MiB draft limit.")
}

fn not_an_image_message(filename: &str) -> String {
    format!("\"{filename}\" is not a PNG, JPEG, GIF, WebP, or BMP image.")
}

fn set_attach_error_if_current(
    compose_draft: Signal<Option<ComposeSession>>,
    draft_id: &str,
    mut error: Signal<Option<String>>,
    msg: String,
) {
    if open_draft_id(compose_draft).as_deref() == Some(draft_id) {
        error.set(Some(msg));
    }
}

struct IncomingBytes {
    filename: String,
    content_type: String,
    bytes: Vec<u8>,
}

fn draft_slot_counts(
    compose_draft: Signal<Option<ComposeSession>>,
    draft_id: &str,
    live_plain: &str,
) -> Option<(usize, usize, u64)> {
    compose_draft
        .read()
        .as_ref()
        .filter(|s| s.draft.id.as_str() == draft_id)
        .map(|s| {
            (
                s.draft.attachments.len(),
                s.draft.inline_images.len(),
                live_payload_bytes(&s.draft, live_plain),
            )
        })
}

fn same_open_draft(compose_draft: Signal<Option<ComposeSession>>, draft_id: &str) -> bool {
    open_draft_id(compose_draft).as_deref() == Some(draft_id)
}

enum PreRead {
    Read,
    Skip,
    Stop,
}

fn pre_read_fits(
    compose_draft: Signal<Option<ComposeSession>>,
    draft_id: &str,
    body: Signal<String>,
    kind: AttachKind,
    declared: u64,
    first_err: &mut Option<String>,
) -> PreRead {
    let (_, max_count, too_many) = kind_limits(kind);
    let Some((file_count, inline_count, used)) =
        draft_slot_counts(compose_draft, draft_id, &body())
    else {
        return PreRead::Stop;
    };
    let count = match kind {
        AttachKind::File => file_count,
        AttachKind::Inline => inline_count,
    };
    if count >= max_count {
        first_err.get_or_insert_with(too_many);
        return PreRead::Stop;
    }
    if would_exceed_draft_cap(used, declared) {
        first_err.get_or_insert_with(oversize_draft_message);
        return PreRead::Skip;
    }
    PreRead::Read
}

fn kind_limits(kind: AttachKind) -> (u64, usize, fn() -> String) {
    match kind {
        AttachKind::File => (
            caps::MAX_FILE_BYTES,
            caps::MAX_ATTACHMENTS,
            too_many_message,
        ),
        AttachKind::Inline => (
            caps::MAX_INLINE_BYTES,
            caps::MAX_INLINES,
            too_many_inlines_message,
        ),
    }
}

enum AttachStep {
    Continue,
    Stop,
}

fn attach_one_bytes(
    compose_draft: Signal<Option<ComposeSession>>,
    draft_id: &str,
    mut item: IncomingBytes,
    body: Signal<String>,
    kind: AttachKind,
    inline_index: &mut usize,
    first_err: &mut Option<String>,
) -> AttachStep {
    if !same_open_draft(compose_draft, draft_id) {
        return AttachStep::Stop;
    }
    if kind == AttachKind::Inline {
        item.filename = image_filename(&item.filename, &item.content_type, *inline_index);
        *inline_index += 1;
        if !looks_like_inline_image(&item.filename, Some(&item.content_type)) {
            first_err.get_or_insert_with(|| not_an_image_message(&item.filename));
            return AttachStep::Continue;
        }
    }
    let size = item.bytes.len() as u64;
    let (max_one, max_count, too_many) = kind_limits(kind);
    if size > max_one {
        first_err.get_or_insert_with(|| oversize_message(&item.filename));
        return AttachStep::Continue;
    }
    let Some((file_count, inline_count, used)) =
        draft_slot_counts(compose_draft, draft_id, &body())
    else {
        return AttachStep::Stop;
    };
    let count = match kind {
        AttachKind::File => file_count,
        AttachKind::Inline => inline_count,
    };
    if count >= max_count {
        first_err.get_or_insert_with(too_many);
        return AttachStep::Stop;
    }
    if would_exceed_draft_cap(used, size) {
        first_err.get_or_insert_with(oversize_draft_message);
        return AttachStep::Continue;
    }
    let pushed = match kind {
        AttachKind::File => {
            let attachment = file_attachment(item.filename, item.content_type, item.bytes);
            push_attachment_on_draft(compose_draft, draft_id, attachment, body)
        }
        AttachKind::Inline => {
            let image = inline_image(Some(item.filename), item.content_type, item.bytes);
            push_inline_on_draft(compose_draft, draft_id, image, body)
        }
    };
    match pushed {
        PushAttachment::Added => AttachStep::Continue,
        PushAttachment::TooMany => {
            first_err.get_or_insert_with(too_many);
            AttachStep::Stop
        }
        PushAttachment::Stale => AttachStep::Stop,
        PushAttachment::TooLarge => {
            first_err.get_or_insert_with(oversize_draft_message);
            AttachStep::Continue
        }
    }
}

async fn attach_selected_files(
    ctx: AppContext,
    files: Vec<dioxus::html::FileData>,
    body: Signal<String>,
    error: Signal<Option<String>>,
    kind: AttachKind,
) {
    let Some(draft_id) = open_draft_id(ctx.compose_draft) else {
        return;
    };
    let mut first_err = None::<String>;
    let mut inline_index = 0usize;
    let (max_one, _, _) = kind_limits(kind);
    for file in files {
        if !same_open_draft(ctx.compose_draft, &draft_id) {
            return;
        }
        let filename = file.name();
        let declared = file.size();
        if declared > max_one {
            first_err.get_or_insert_with(|| oversize_message(&filename));
            continue;
        }
        match pre_read_fits(
            ctx.compose_draft,
            &draft_id,
            body,
            kind,
            declared,
            &mut first_err,
        ) {
            PreRead::Read => {}
            PreRead::Skip => continue,
            PreRead::Stop => break,
        }
        let bytes = match file.read_bytes().await {
            Ok(b) => b,
            Err(_) => {
                first_err.get_or_insert_with(|| format!("Could not read \"{filename}\"."));
                continue;
            }
        };
        if !same_open_draft(ctx.compose_draft, &draft_id) {
            return;
        }
        if bytes.len() as u64 > max_one {
            first_err.get_or_insert_with(|| oversize_message(&filename));
            continue;
        }
        let content_type = resolve_content_type(&filename, file.content_type().as_deref());
        match attach_one_bytes(
            ctx.compose_draft,
            &draft_id,
            IncomingBytes {
                filename,
                content_type,
                bytes: bytes.to_vec(),
            },
            body,
            kind,
            &mut inline_index,
            &mut first_err,
        ) {
            AttachStep::Continue => {}
            AttachStep::Stop => break,
        }
    }
    if let Some(msg) = first_err {
        set_attach_error_if_current(ctx.compose_draft, &draft_id, error, msg);
    }
}

fn clipboard_data_transfer(evt: &dioxus::html::ClipboardEvent) -> Option<web_sys::DataTransfer> {
    let raw = evt.data().downcast::<web_sys::Event>()?.clone();
    raw.dyn_into::<web_sys::ClipboardEvent>()
        .ok()?
        .clipboard_data()
}

fn clipboard_has_text(dt: &web_sys::DataTransfer) -> bool {
    if dt
        .get_data("text/plain")
        .ok()
        .is_some_and(|s| !s.trim().is_empty())
    {
        return true;
    }
    let types = dt.types();
    (0..types.length()).any(|i| {
        types.get(i).as_string().is_some_and(|t| {
            t.eq_ignore_ascii_case("text/plain") || t.eq_ignore_ascii_case("text/html")
        })
    })
}

fn is_clipboard_image(file: &web_sys::File) -> bool {
    let ty = file.type_();
    if is_safe_image_content_type(&ty) {
        return true;
    }
    ty.is_empty() && looks_like_inline_image(&file.name(), None)
}

fn collect_clipboard_images(dt: &web_sys::DataTransfer) -> Vec<web_sys::File> {
    let mut out = Vec::new();
    if let Some(list) = dt.files() {
        for i in 0..list.length() {
            if let Some(file) = list.item(i) {
                if is_clipboard_image(&file) {
                    out.push(file);
                }
            }
        }
    }
    if !out.is_empty() {
        return out;
    }
    let items = dt.items();
    for i in 0..items.length() {
        let Some(item) = items.get(i) else {
            continue;
        };
        if item.kind() != "file" || !is_safe_image_content_type(&item.type_()) {
            continue;
        }
        if let Ok(Some(file)) = item.get_as_file() {
            out.push(file);
        }
    }
    out
}

async fn read_web_file_bytes(file: &web_sys::File) -> Result<Vec<u8>, ()> {
    let blob: web_sys::Blob = file.clone().into();
    let buf = wasm_bindgen_futures::JsFuture::from(blob.array_buffer())
        .await
        .map_err(|_| ())?;
    Ok(js_sys::Uint8Array::new(&buf).to_vec())
}

async fn attach_web_images(
    ctx: AppContext,
    files: Vec<web_sys::File>,
    body: Signal<String>,
    error: Signal<Option<String>>,
) {
    let Some(draft_id) = open_draft_id(ctx.compose_draft) else {
        return;
    };
    let mut first_err = None::<String>;
    let mut inline_index = 0usize;
    for file in files {
        if !same_open_draft(ctx.compose_draft, &draft_id) {
            return;
        }
        let filename = file.name();
        let declared = file.size() as u64;
        if declared > caps::MAX_INLINE_BYTES {
            first_err.get_or_insert_with(|| oversize_message(&filename));
            continue;
        }
        match pre_read_fits(
            ctx.compose_draft,
            &draft_id,
            body,
            AttachKind::Inline,
            declared,
            &mut first_err,
        ) {
            PreRead::Read => {}
            PreRead::Skip => continue,
            PreRead::Stop => break,
        }
        let bytes = match read_web_file_bytes(&file).await {
            Ok(b) => b,
            Err(()) => {
                first_err.get_or_insert_with(|| format!("Could not read \"{filename}\"."));
                continue;
            }
        };
        if !same_open_draft(ctx.compose_draft, &draft_id) {
            return;
        }
        if bytes.len() as u64 > caps::MAX_INLINE_BYTES {
            first_err.get_or_insert_with(|| oversize_message(&filename));
            continue;
        }
        let reported = file.type_();
        let content_type =
            resolve_content_type(&filename, Some(reported.as_str()).filter(|s| !s.is_empty()));
        match attach_one_bytes(
            ctx.compose_draft,
            &draft_id,
            IncomingBytes {
                filename,
                content_type,
                bytes,
            },
            body,
            AttachKind::Inline,
            &mut inline_index,
            &mut first_err,
        ) {
            AttachStep::Continue => {}
            AttachStep::Stop => break,
        }
    }
    if let Some(msg) = first_err {
        set_attach_error_if_current(ctx.compose_draft, &draft_id, error, msg);
    }
}

#[component]
pub fn ComposeOverlay() -> Element {
    let ctx = use_context::<AppContext>();
    let core = use_coroutine_handle::<CoreEvent>();
    let to = use_signal(Vec::<ComposerAddress>::new);
    let to_draft = use_signal(String::new);
    let cc = use_signal(Vec::<ComposerAddress>::new);
    let cc_draft = use_signal(String::new);
    let bcc = use_signal(Vec::<ComposerAddress>::new);
    let bcc_draft = use_signal(String::new);
    let mut subject = use_signal(String::new);
    let mut body = use_signal(String::new);
    let mut show_cc_bcc = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let last_draft_id = use_signal(|| None::<String>);
    let save_gen = use_signal(|| 0u32);
    // Local only: per-account `send_status` stays `Sending` after the dialog
    // closes (outbox drain) and must not disable a newly opened draft.
    let submitting = use_signal(|| false);
    let mut submitted_id = use_signal(|| None::<String>);
    let mut attaching = use_signal(|| false);
    let forward_fetching = use_signal(|| false);
    let mut attach_gen = use_signal(|| 0u32);
    let mut attach_input_gen = use_signal(|| 0u32);
    let mut image_input_gen = use_signal(|| 0u32);
    let mut form = ComposeForm {
        to: RecipientList {
            chips: to,
            draft: to_draft,
        },
        cc: RecipientList {
            chips: cc,
            draft: cc_draft,
        },
        bcc: RecipientList {
            chips: bcc,
            draft: bcc_draft,
        },
        subject,
        body,
    };
    let mut include_original = use_signal(|| true);

    let (open, title, listed_files, from_account_id, from_addr, has_originals, prefill_warnings) = {
        let slot = ctx.compose_draft.read();
        match slot.as_ref() {
            Some(s) => {
                let mut listed = s
                    .draft
                    .attachments
                    .iter()
                    .map(|a| {
                        (
                            format!("att-{}", a.id.0),
                            a.id.0.clone(),
                            a.filename.clone(),
                            a.size,
                            false,
                        )
                    })
                    .collect::<Vec<_>>();
                listed.extend(s.draft.inline_images.iter().map(|img| {
                    let size = match &img.data {
                        mailiner_composer::AttachmentData::Bytes(b) => b.len() as u64,
                        mailiner_composer::AttachmentData::Pending => 0,
                    };
                    (
                        format!("img-{}", img.id.0),
                        img.id.0.clone(),
                        img.filename.clone().unwrap_or_else(|| "image".into()),
                        size,
                        true,
                    )
                }));
                (
                    true,
                    s.title.clone(),
                    listed,
                    Some(s.account_id.clone()),
                    s.draft.from.clone(),
                    has_original_attachments(s),
                    s.draft.prefill_warnings.clone(),
                )
            }
            None => (
                false,
                "New message".to_string(),
                Vec::new(),
                None,
                None,
                false,
                Vec::new(),
            ),
        }
    };
    let from_accounts = listed_compose_accounts(&ctx);
    let from_choices = list_from_choices(&from_accounts);
    let selected_choice =
        selected_from_choice(&from_choices, from_account_id.as_ref(), from_addr.as_ref());
    let from_label = selected_choice
        .map(|c| from_account_label(&c.identity.display_name, &c.identity.email))
        .unwrap_or_default();
    let selected_from_key = selected_choice
        .map(crate::send::FromChoice::key)
        .unwrap_or_default();
    let multi_from = from_choices.len() > 1;
    let sending = submitting();
    let attaching_now = attaching() || forward_fetching();
    let busy = sending || attaching_now;

    // Apply a newly opened draft once (do not clobber typing).
    {
        let ctx = ctx.clone();
        let core = core;
        let mut last_draft_id = last_draft_id;
        let mut submitting = submitting;
        let mut attaching = attaching;
        let mut forward_fetching = forward_fetching;
        let mut attach_gen = attach_gen;
        let mut include_original = include_original;
        use_effect(move || match ctx.compose_draft.read().as_ref() {
            Some(session) => {
                let id = session.draft.id.as_str().to_string();
                if last_draft_id() != Some(id.clone()) {
                    last_draft_id.set(Some(id.clone()));
                    apply_draft_fields(&session.draft, &mut form);
                    show_cc_bcc.set(!session.draft.cc.is_empty() || !session.draft.bcc.is_empty());
                    error.set(None);
                    submitting.set(false);
                    let next = *attach_gen.peek() + 1;
                    attach_gen.set(next);
                    include_original.set(true);
                    submitted_id.set(None);
                    attaching.set(false);
                    if has_pending_forward_fetch(session) {
                        forward_fetching.set(true);
                        core.send(CoreEvent::FetchComposeAttachments {
                            draft_id: id,
                            account_id: session.account_id.clone(),
                        });
                    } else {
                        forward_fetching.set(false);
                    }
                }
            }
            None => {
                last_draft_id.set(None);
                submitting.set(false);
                let next = *attach_gen.peek() + 1;
                attach_gen.set(next);
                attaching.set(false);
                forward_fetching.set(false);
                submitted_id.set(None);
            }
        });
    }

    // Debounced autosave of the open draft (text + attachments).
    {
        let ctx = ctx.clone();
        let mut save_gen = save_gen;
        use_effect(move || {
            let Some(session) = ctx.compose_draft.read().as_ref().cloned() else {
                return;
            };
            if last_draft_id() != Some(session.draft.id.as_str().to_string()) {
                return;
            }
            let _ = (
                to(),
                to_draft(),
                cc(),
                cc_draft(),
                bcc(),
                bcc_draft(),
                subject(),
                body(),
            );
            let generation = *save_gen.peek() + 1;
            save_gen.set(generation);
            let draft_id = session.draft.id.as_str().to_string();
            let compose_draft = ctx.compose_draft;
            spawn(async move {
                sleep_ms(DRAFT_SAVE_DEBOUNCE_MS).await;
                if save_gen() != generation {
                    return;
                }
                if open_draft_id(compose_draft).as_deref() != Some(draft_id.as_str()) {
                    return;
                }
                let _ = persist_live_draft(
                    compose_draft,
                    to,
                    to_draft,
                    cc,
                    cc_draft,
                    bcc,
                    bcc_draft,
                    subject,
                    body,
                );
            });
        });
    }

    {
        let ctx = ctx.clone();
        let mut forward_fetching = forward_fetching;
        use_effect(move || {
            if !forward_fetching() {
                return;
            }
            let still = ctx
                .compose_draft
                .read()
                .as_ref()
                .is_some_and(has_pending_forward_fetch);
            if !still {
                forward_fetching.set(false);
            }
        });
    }

    // Persist failed for *this* draft — allow retry. Ignore stale Failed
    // from an older send so a second click cannot enqueue a duplicate.
    {
        let ctx = ctx.clone();
        let mut submitting = submitting;
        let submitted_id = submitted_id;
        use_effect(move || {
            if !matches!(compose_send_state(&ctx), Some(SendState::Failed { .. })) {
                return;
            }
            let open_id = ctx
                .compose_draft
                .read()
                .as_ref()
                .map(|s| s.draft.id.as_str().to_string());
            if submitted_id() == open_id {
                submitting.set(false);
            }
        });
    }

    let no_account = ctx.accounts.read().is_empty();
    let mut compose_draft = ctx.compose_draft;
    let mut compose_placement = ctx.compose_placement;
    let docked = *compose_placement.read() == ComposePlacement::Docked;
    let run_close = move || {
        close_keeping_draft(
            save_gen,
            compose_draft,
            to,
            to_draft,
            cc,
            cc_draft,
            bcc,
            bcc_draft,
            subject,
            body,
            ctx.accounts,
            &core,
        );
    };
    let close = move |_| run_close();
    let discard = move |_| {
        discard_draft(save_gen, compose_draft, &core);
    };

    rsx! {
        if !open {
            button {
                class: "compose-fab",
                title: "Compose",
                disabled: no_account,
                onclick: {
                    let mut ctx = ctx.clone();
                    move |_| open_new_message(&mut ctx)
                },
                Icon { size: 18, icon: IconKind::PencilSquare }
                "Compose"
            }
        }

        if open {
            div {
                class: if docked { "compose-dock" } else { "compose-backdrop" },
                onclick: move |_| {
                    if !docked {
                        run_close();
                    }
                },
                div {
                    class: if docked {
                        "ui-dialog compose-dialog compose-dialog-docked"
                    } else {
                        "ui-dialog compose-dialog"
                    },
                    role: if docked { "region" } else { "dialog" },
                    aria_modal: if docked { "false" } else { "true" },
                    aria_label: "{title}",
                    onclick: move |evt| evt.stop_propagation(),
                    onkeydown: {
                        let ctx = ctx.clone();
                        move |evt: KeyboardEvent| {
                            if matches!(evt.key(), Key::Escape) && !docked {
                                evt.prevent_default();
                                run_close();
                                return;
                            }
                            if matches!(evt.key(), Key::Enter)
                                && (evt.modifiers().ctrl() || evt.modifiers().meta())
                            {
                                evt.prevent_default();
                                submit_compose(
                                    &ctx,
                                    &core,
                                    form,
                                    error,
                                    submitting,
                                    submitted_id,
                                    attaching,
                                    forward_fetching,
                                );
                            }
                        }
                    },
                    onpaste: {
                        let ctx = ctx.clone();
                        move |evt: ClipboardEvent| {
                            if sending || attaching() {
                                return;
                            }
                            let Some(dt) = clipboard_data_transfer(&evt) else {
                                return;
                            };
                            let files = collect_clipboard_images(&dt);
                            if files.is_empty() {
                                return;
                            }
                            if !clipboard_has_text(&dt) {
                                evt.prevent_default();
                            }
                            error.set(None);
                            let generation = attach_gen() + 1;
                            attach_gen.set(generation);
                            attaching.set(true);
                            let ctx = ctx.clone();
                            let mut attaching = attaching;
                            spawn(async move {
                                attach_web_images(ctx, files, body, error).await;
                                if attach_gen() == generation {
                                    attaching.set(false);
                                }
                            });
                        }
                    },
                    div {
                        class: "ui-dialog-head",
                        h2 { class: "ui-dialog-title", "{title}" }
                        div {
                            class: "compose-head-actions",
                            IconButton {
                                class: "flat ui-icon-btn",
                                title: if docked { "Open as dialog" } else { "Dock to bottom" },
                                size: 20,
                                icon: if docked {
                                    IconKind::ArrowsPointingOut
                                } else {
                                    IconKind::ArrowsPointingIn
                                },
                                aria_pressed: Some(docked),
                                onclick: move |_| {
                                    let next = match *compose_placement.peek() {
                                        ComposePlacement::Modal => ComposePlacement::Docked,
                                        ComposePlacement::Docked => ComposePlacement::Modal,
                                    };
                                    compose_placement.set(next);
                                    crate::ui_prefs::save_compose_placement(next);
                                },
                            }
                            IconButton {
                                class: "flat ui-icon-btn",
                                title: "Close",
                                size: 20,
                                icon: IconKind::XMark,
                                onclick: close,
                            }
                        }
                    }
                    label {
                        class: "ui-field",
                        span { "From" }
                        if multi_from {
                            select {
                                class: "ui-input",
                                value: "{selected_from_key}",
                                disabled: sending,
                                aria_label: "From",
                                onchange: {
                                    let ctx = ctx.clone();
                                    let mut compose_draft = ctx.compose_draft;
                                    let from_choices = from_choices.clone();
                                    move |evt: FormEvent| {
                                        if sending {
                                            return;
                                        }
                                        let Some((account_id, index)) =
                                            parse_from_choice_key(&evt.value())
                                        else {
                                            return;
                                        };
                                        let Some(choice) = from_choices
                                            .iter()
                                            .find(|c| c.account_id == account_id && c.index == index)
                                            .cloned()
                                        else {
                                            return;
                                        };
                                        let mut slot = compose_draft.write();
                                        if let Some(session) = slot.as_mut() {
                                            set_session_from_identity(
                                                session,
                                                choice.account_id,
                                                &choice.identity,
                                            );
                                        }
                                    }
                                },
                                for choice in from_choices.iter() {
                                    option {
                                        value: "{choice.key()}",
                                        selected: selected_from_key == choice.key(),
                                        "{from_account_label(&choice.identity.display_name, &choice.identity.email)}"
                                    }
                                }
                            }
                        } else {
                            input {
                                class: "ui-input",
                                value: "{from_label}",
                                disabled: true,
                                readonly: true,
                                aria_label: "From",
                            }
                        }
                    }
                    div {
                        class: "compose-to-row",
                        label {
                            class: "ui-field",
                            span { "To" }
                            RecipientField {
                                label: "To",
                                chips: to,
                                draft: to_draft,
                                disabled: sending,
                            }
                        }
                        button {
                            class: "compose-cc-toggle",
                            r#type: "button",
                            title: if show_cc_bcc() { "Hide Cc/Bcc" } else { "Show Cc/Bcc" },
                            aria_expanded: if show_cc_bcc() { "true" } else { "false" },
                            onclick: move |_| show_cc_bcc.set(!show_cc_bcc()),
                            "Cc/Bcc"
                        }
                    }
                    if show_cc_bcc() {
                        label {
                            class: "ui-field",
                            span { "Cc" }
                            RecipientField {
                                label: "Cc",
                                chips: cc,
                                draft: cc_draft,
                                disabled: sending,
                            }
                        }
                        label {
                            class: "ui-field",
                            span { "Bcc" }
                            RecipientField {
                                label: "Bcc",
                                chips: bcc,
                                draft: bcc_draft,
                                disabled: sending,
                            }
                        }
                    }
                    label {
                        class: "ui-field",
                        span { "Subject" }
                        input {
                            class: "ui-input",
                            spellcheck: spellcheck_attr(SpellcheckField::Subject),
                            value: subject(),
                            disabled: sending,
                            oninput: move |e| subject.set(e.value()),
                        }
                    }
                    label {
                        class: "ui-field ui-field-grow",
                        span { "Message" }
                        textarea {
                            class: "ui-input",
                            spellcheck: spellcheck_attr(SpellcheckField::Body),
                            value: body(),
                            disabled: sending,
                            rows: 10,
                            oninput: move |e| body.set(e.value()),
                        }
                    }
                    div {
                        class: "compose-attachments",
                        div {
                            class: "compose-attach-actions",
                            label {
                                class: if busy { "compose-attach is-disabled" } else { "compose-attach" },
                                title: "Attach files",
                                input {
                                    key: "{attach_input_gen()}",
                                    class: "compose-attach-input",
                                    r#type: "file",
                                    multiple: true,
                                    disabled: busy,
                                    aria_label: "Attach files",
                                    onchange: {
                                        let ctx = ctx.clone();
                                        move |evt: FormEvent| {
                                            if sending || attaching() {
                                                return;
                                            }
                                            let files = evt.files();
                                            attach_input_gen.set(attach_input_gen() + 1);
                                            if files.is_empty() {
                                                return;
                                            }
                                            error.set(None);
                                            let generation = attach_gen() + 1;
                                            attach_gen.set(generation);
                                            attaching.set(true);
                                            let ctx = ctx.clone();
                                            let mut attaching = attaching;
                                            spawn(async move {
                                                attach_selected_files(
                                                    ctx,
                                                    files,
                                                    body,
                                                    error,
                                                    AttachKind::File,
                                                )
                                                .await;
                                                if attach_gen() == generation {
                                                    attaching.set(false);
                                                }
                                            });
                                        }
                                    },
                                }
                                Icon { size: 16, icon: IconKind::PaperClip }
                                "Attach"
                            }
                            label {
                                class: if busy { "compose-attach is-disabled" } else { "compose-attach" },
                                title: "Insert image",
                                input {
                                    key: "{image_input_gen()}",
                                    class: "compose-attach-input",
                                    r#type: "file",
                                    multiple: true,
                                    accept: SAFE_IMAGE_ACCEPT,
                                    disabled: busy,
                                    aria_label: "Insert image",
                                    onchange: {
                                        let ctx = ctx.clone();
                                        move |evt: FormEvent| {
                                            if sending || attaching() {
                                                return;
                                            }
                                            let files = evt.files();
                                            image_input_gen.set(image_input_gen() + 1);
                                            if files.is_empty() {
                                                return;
                                            }
                                            error.set(None);
                                            let generation = attach_gen() + 1;
                                            attach_gen.set(generation);
                                            attaching.set(true);
                                            let ctx = ctx.clone();
                                            let mut attaching = attaching;
                                            spawn(async move {
                                                attach_selected_files(
                                                    ctx,
                                                    files,
                                                    body,
                                                    error,
                                                    AttachKind::Inline,
                                                )
                                                .await;
                                                if attach_gen() == generation {
                                                    attaching.set(false);
                                                }
                                            });
                                        }
                                    },
                                }
                                Icon { size: 16, icon: IconKind::Photo }
                                "Insert image"
                            }
                        }
                        if has_originals {
                            label {
                                class: "compose-include-original",
                                input {
                                    r#type: "checkbox",
                                    checked: include_original(),
                                    disabled: sending,
                                    onchange: move |e| {
                                        let on = e.checked();
                                        include_original.set(on);
                                        toggle_original_attachments(compose_draft, on, body);
                                    },
                                }
                                "Include original attachments"
                            }
                        }
                        if !listed_files.is_empty() {
                            ul {
                                class: "compose-attachment-list",
                                for (key, id, filename, size, is_inline) in listed_files {
                                    li {
                                        key: "{key}",
                                        class: "compose-attachment",
                                        span {
                                            class: "compose-attachment-name",
                                            title: "{filename}",
                                            "{filename}"
                                        }
                                        span {
                                            class: "compose-attachment-size",
                                            if is_inline {
                                                "image · "
                                            }
                                            "{human_size(size)}"
                                        }
                                        button {
                                            class: "compose-attachment-remove",
                                            r#type: "button",
                                            title: "Remove",
                                            disabled: sending,
                                            onclick: {
                                                let id = id.clone();
                                                move |_| {
                                                    if sending {
                                                        return;
                                                    }
                                                    if is_inline {
                                                        remove_inline(compose_draft, &id);
                                                    } else {
                                                        remove_attachment(compose_draft, &id);
                                                    }
                                                }
                                            },
                                            Icon { size: 14, icon: IconKind::XMark }
                                        }
                                    }
                                }
                            }
                        }
                        for warn in prefill_warnings {
                            p {
                                class: "compose-prefill-note",
                                "{warn}"
                            }
                        }
                    }
                    if let Some(err) = error() {
                        p { class: "ui-alert-error", "{err}" }
                    }
                    if let Some(SendState::Failed { message, .. }) = compose_send_state(&ctx) {
                        p { class: "ui-alert-error", "{message}" }
                    }
                    div {
                        class: "ui-dialog-actions",
                        button {
                            class: "ui-btn ui-btn-danger",
                            disabled: sending,
                            onclick: discard,
                            "Discard"
                        }
                        button {
                            class: "ui-btn ui-btn-secondary",
                            disabled: sending,
                            onclick: close,
                            "Close"
                        }
                        button {
                            class: "ui-btn ui-btn-primary",
                            disabled: busy,
                            onclick: {
                                let ctx = ctx.clone();
                                move |_| {
                                    submit_compose(
                                        &ctx,
                                        &core,
                                        form,
                                        error,
                                        submitting,
                                        submitted_id,
                                        attaching,
                                        forward_fetching,
                                    );
                                }
                            },
                            if sending {
                                "Sending…"
                            } else if attaching_now {
                                "Attaching…"
                            } else {
                                "Send"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_composer_address_keeps_name() {
        assert_eq!(
            format_composer_address(&ComposerAddress {
                name: Some("Ada".into()),
                email: "ada@example.com".into(),
            }),
            "Ada <ada@example.com>"
        );
        assert_eq!(
            format_composer_address(&ComposerAddress::email_only("solo@example.com")),
            "solo@example.com"
        );
        assert_eq!(
            format_composer_address(&ComposerAddress {
                name: Some("alice@example.com".into()),
                email: "alice@work.com".into(),
            }),
            "\"alice@example.com\" <alice@work.com>"
        );
    }

    #[test]
    fn parse_address_list_roundtrips_named_and_bare() {
        let parsed = parse_address_list(
            "Ada Lovelace <ada@example.com>, bob@example.com, \"Cc\" <cc@example.com>",
        );
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].name.as_deref(), Some("Ada Lovelace"));
        assert_eq!(parsed[0].email, "ada@example.com");
        assert_eq!(parsed[1].name, None);
        assert_eq!(parsed[1].email, "bob@example.com");
        assert_eq!(parsed[2].name.as_deref(), Some("Cc"));
        assert_eq!(parsed[2].email, "cc@example.com");

        let comma_name = parse_address_list("Lovelace, Ada <ada@example.com>");
        assert_eq!(comma_name.len(), 1);
        assert_eq!(comma_name[0].name.as_deref(), Some("Lovelace, Ada"));
        assert_eq!(comma_name[0].email, "ada@example.com");

        let joined = join_address_list(&parsed);
        assert_eq!(
            joined,
            "Ada Lovelace <ada@example.com>, bob@example.com, Cc <cc@example.com>"
        );
        assert_eq!(parse_address_list(&joined), parsed);
    }

    #[test]
    fn parse_address_list_keeps_email_like_display_name() {
        let unquoted = parse_address_list("alice@example.com <alice@work.com>");
        assert_eq!(unquoted.len(), 1);
        assert_eq!(unquoted[0].name.as_deref(), Some("alice@example.com"));
        assert_eq!(unquoted[0].email, "alice@work.com");

        let quoted = parse_address_list("\"alice@example.com\" <alice@work.com>");
        assert_eq!(quoted, unquoted);

        let mixed = parse_address_list("bob@example.com, \"alice@example.com\" <alice@work.com>");
        assert_eq!(mixed.len(), 2);
        assert_eq!(mixed[0].email, "bob@example.com");
        assert_eq!(mixed[1].name.as_deref(), Some("alice@example.com"));
        assert_eq!(mixed[1].email, "alice@work.com");

        assert_eq!(parse_address_list(&join_address_list(&unquoted)), unquoted);

        let nickname = parse_address_list(r#"John "Johnny" <john@example.com>"#);
        assert_eq!(nickname.len(), 1);
        assert_eq!(nickname[0].name.as_deref(), Some("John Johnny"));
        assert_eq!(nickname[0].email, "john@example.com");

        // Email-like display name + quoted nickname is one mailbox, not two.
        let email_nickname = parse_address_list(r#"alice@example.com "Alice" <alice@work.com>"#);
        assert_eq!(email_nickname.len(), 1);
        assert_eq!(
            email_nickname[0].name.as_deref(),
            Some("alice@example.com Alice")
        );
        assert_eq!(email_nickname[0].email, "alice@work.com");
    }

    #[test]
    fn parse_address_list_skips_empty_mailbox() {
        assert!(parse_address_list("No Mail <>").is_empty());
        assert!(parse_address_list("  ,  ").is_empty());
    }

    fn empty_draft() -> DraftDocument {
        DraftDocument::new_empty(&FromIdentity::new("Me", "me@example.com"))
    }

    #[test]
    fn apply_plain_clears_html() {
        let mut draft = empty_draft();
        draft.plain_body = "Hello".into();
        draft.html_body = "<p>old</p>".into();
        apply_compose_body_mode(&mut draft, ComposeBodyMode::Plain);
        assert_eq!(draft.mode, BodyMode::Plain);
        assert!(draft.html_body.is_empty());
        assert_eq!(draft.plain_body, "Hello");
    }

    #[test]
    fn apply_rich_builds_html_from_plain() {
        let mut draft = empty_draft();
        draft.plain_body = "Hello\n\nWorld".into();
        apply_compose_body_mode(&mut draft, ComposeBodyMode::Rich);
        assert_eq!(draft.mode, BodyMode::Rich);
        assert_eq!(draft.html_body, plain_to_html("Hello\n\nWorld"));
        assert!(draft.html_body.contains("<p>Hello</p>"));
        assert!(draft.html_body.contains("<p>World</p>"));
    }

    #[test]
    fn submit_uses_draft_mode_not_current_pref() {
        let mut draft = empty_draft();
        draft.plain_body = "Hi".into();
        apply_compose_body_mode(&mut draft, ComposeBodyMode::Rich);
        let mode = compose_body_mode_from_draft(&draft);
        draft.plain_body = "Hi there".into();
        apply_compose_body_mode(&mut draft, mode);
        assert_eq!(draft.mode, BodyMode::Rich);
        assert_eq!(draft.html_body, plain_to_html("Hi there"));
    }

    #[test]
    fn live_payload_counts_current_rich_html() {
        let mut draft = empty_draft();
        draft.plain_body = "Hi".into();
        apply_compose_body_mode(&mut draft, ComposeBodyMode::Rich);
        let opened = draft_payload_bytes(&draft);
        let live = live_payload_bytes(&draft, "Hi there");
        let expected_html = plain_to_html("Hi there").len() as u64;
        assert!(live > opened);
        assert_eq!(live, "Hi there".len() as u64 + expected_html);
    }
}
