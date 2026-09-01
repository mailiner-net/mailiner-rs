//! Plain-text compose overlay (v1 send).

use dioxus::html::Key;
use dioxus::prelude::*;

use super::icons::{Icon, IconButton, IconKind};

use mailiner_composer::identity::FromIdentity;
use mailiner_composer::model::draft::{BodyMode, ComposerAddress, DraftDocument};
use mailiner_composer::shell::attachment_list::{
    draft_payload_bytes, file_attachment, human_size, resolve_content_type, would_exceed_draft_cap,
};
use mailiner_composer::{
    ComposeIntent, FileAttachment, PrepareSubmitError, build_draft, caps, prepare_submit,
};

use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::send::{ComposeSession, OutboxDisplay, SendState};

fn join_address_emails(addrs: &[ComposerAddress]) -> String {
    addrs
        .iter()
        .map(|a| a.email.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse_address_list(raw: &str) -> Vec<ComposerAddress> {
    raw.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(ComposerAddress::email_only)
        .collect()
}

fn apply_draft_fields(
    draft: &DraftDocument,
    to: &mut Signal<String>,
    cc: &mut Signal<String>,
    bcc: &mut Signal<String>,
    subject: &mut Signal<String>,
    body: &mut Signal<String>,
) {
    to.set(join_address_emails(&draft.to));
    cc.set(join_address_emails(&draft.cc));
    bcc.set(join_address_emails(&draft.bcc));
    subject.set(draft.subject.clone());
    body.set(draft.plain_body.clone());
}

/// Open a new / reply / forward session. Replaces any existing compose draft.
pub fn open_compose(ctx: &mut AppContext, session: ComposeSession) {
    ctx.compose_draft.set(Some(session));
}

/// Open a blank compose for the selected account.
pub fn open_new_message(ctx: &mut AppContext) {
    let Some(account_id) = ctx.selected_account.read().clone() else {
        return;
    };
    let Some(account) = ctx.accounts.read().get(&account_id).cloned() else {
        return;
    };
    let identity = FromIdentity::new(account.name, account.email);
    let mut draft = DraftDocument::new_empty(&identity);
    draft.mode = BodyMode::Plain;
    open_compose(
        ctx,
        ComposeSession {
            title: "New message".into(),
            draft,
        },
    );
}

/// Prefill Reply or Forward from a loaded message.
pub fn open_reply_or_forward(
    ctx: &mut AppContext,
    intent: ComposeIntent,
    envelope: &mailiner_core::Envelope,
    loaded: &mailiner_core::models::LoadedMessage,
) {
    let Some(account_id) = ctx.selected_account.read().clone() else {
        ctx.show_toast(crate::toast::ToastAction::error("Select an account first."));
        return;
    };
    let Some(account) = ctx.accounts.read().get(&account_id).cloned() else {
        ctx.show_toast(crate::toast::ToastAction::error("Account not found."));
        return;
    };
    let identity = FromIdentity::new(account.name, account.email);
    match build_draft(intent, &identity, Some(envelope), Some(loaded)) {
        Ok(mut draft) => {
            draft.mode = BodyMode::Plain;
            let title = match intent {
                ComposeIntent::Forward => "Forward",
                ComposeIntent::Reply | ComposeIntent::ReplyAll => "Reply",
                ComposeIntent::New => "New message",
            };
            open_compose(
                ctx,
                ComposeSession {
                    title: title.into(),
                    draft,
                },
            );
        }
        Err(e) => ctx.show_toast(crate::toast::ToastAction::error(e.to_string())),
    }
}

fn submit_compose(
    ctx: &AppContext,
    core: &Coroutine<CoreEvent>,
    to: Signal<String>,
    cc: Signal<String>,
    bcc: Signal<String>,
    subject: Signal<String>,
    body: Signal<String>,
    mut error: Signal<Option<String>>,
    mut submitting: Signal<bool>,
    mut submitted_id: Signal<Option<String>>,
    attaching: Signal<bool>,
) {
    if submitting() || attaching() {
        return;
    }
    error.set(None);
    let Some(account_id) = ctx.selected_account.read().clone() else {
        error.set(Some("Select an account first.".into()));
        return;
    };
    let Some(account) = ctx.accounts.read().get(&account_id).cloned() else {
        error.set(Some("Account not found.".into()));
        return;
    };
    let identity = FromIdentity::new(account.name.clone(), account.email.clone());
    let mut draft = ctx
        .compose_draft
        .read()
        .as_ref()
        .map(|s| s.draft.clone())
        .unwrap_or_else(|| DraftDocument::new_empty(&identity));
    draft.mode = BodyMode::Plain;
    draft.html_body.clear();
    draft.plain_body = body();
    draft.subject = subject();
    draft.to = parse_address_list(&to());
    draft.cc = parse_address_list(&cc());
    draft.bcc = parse_address_list(&bcc());
    match prepare_submit(&draft, &identity) {
        Ok(prepared) => {
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

fn live_payload_bytes(draft: &DraftDocument, live_plain_len: usize) -> u64 {
    draft_payload_bytes(draft)
        .saturating_sub(draft.plain_body.len() as u64)
        .saturating_add(live_plain_len as u64)
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
    if would_exceed_draft_cap(
        live_payload_bytes(&session.draft, body().len()),
        attachment.size,
    ) {
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
    if session.draft.attachments.len() != before {
        session.draft.touch();
    }
}

fn oversize_message(filename: &str) -> String {
    let max_mib = caps::MAX_FILE_BYTES / (1024 * 1024);
    format!("\"{filename}\" is larger than {max_mib} MiB.")
}

fn too_many_message() -> String {
    format!("You can attach at most {} files.", caps::MAX_ATTACHMENTS)
}

fn oversize_draft_message() -> String {
    let max_mib = caps::MAX_DRAFT_BYTES / (1024 * 1024);
    format!("Attachments would exceed the {max_mib} MiB draft limit.")
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

async fn attach_selected_files(
    ctx: AppContext,
    files: Vec<dioxus::html::FileData>,
    body: Signal<String>,
    error: Signal<Option<String>>,
) {
    let Some(draft_id) = open_draft_id(ctx.compose_draft) else {
        return;
    };
    let mut first_err = None::<String>;
    for file in files {
        let filename = file.name();
        let declared = file.size();
        if declared > caps::MAX_FILE_BYTES {
            first_err.get_or_insert_with(|| oversize_message(&filename));
            continue;
        }
        let live_plain_len = body().len();
        let Some((count, used)) = ctx
            .compose_draft
            .read()
            .as_ref()
            .filter(|s| s.draft.id.as_str() == draft_id)
            .map(|s| {
                (
                    s.draft.attachments.len(),
                    live_payload_bytes(&s.draft, live_plain_len),
                )
            })
        else {
            break;
        };
        if count >= caps::MAX_ATTACHMENTS {
            first_err.get_or_insert_with(too_many_message);
            break;
        }
        if would_exceed_draft_cap(used, declared) {
            first_err.get_or_insert_with(oversize_draft_message);
            continue;
        }
        let bytes = match file.read_bytes().await {
            Ok(b) => b,
            Err(_) => {
                first_err.get_or_insert_with(|| format!("Could not read \"{filename}\"."));
                continue;
            }
        };
        if bytes.len() as u64 > caps::MAX_FILE_BYTES {
            first_err.get_or_insert_with(|| oversize_message(&filename));
            continue;
        }
        let content_type = resolve_content_type(&filename, file.content_type().as_deref());
        let attachment = file_attachment(filename, content_type, bytes.to_vec());
        match push_attachment_on_draft(ctx.compose_draft, &draft_id, attachment, body) {
            PushAttachment::Added => {}
            PushAttachment::TooMany => {
                first_err.get_or_insert_with(too_many_message);
                break;
            }
            PushAttachment::Stale => break,
            PushAttachment::TooLarge => {
                first_err.get_or_insert_with(oversize_draft_message);
                continue;
            }
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
    let mut to = use_signal(String::new);
    let mut cc = use_signal(String::new);
    let mut bcc = use_signal(String::new);
    let mut subject = use_signal(String::new);
    let mut body = use_signal(String::new);
    let mut show_cc_bcc = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let last_draft_id = use_signal(|| None::<String>);
    // Local only: global `send_status` stays `Sending` after the dialog
    // closes (outbox drain) and must not disable a newly opened draft.
    let submitting = use_signal(|| false);
    let mut submitted_id = use_signal(|| None::<String>);
    let mut attaching = use_signal(|| false);
    let mut attach_gen = use_signal(|| 0u32);
    let mut attach_input_gen = use_signal(|| 0u32);

    let (open, title, attachments) = {
        let slot = ctx.compose_draft.read();
        match slot.as_ref() {
            Some(s) => (
                true,
                s.title.clone(),
                s.draft
                    .attachments
                    .iter()
                    .map(|a| (a.id.0.clone(), a.filename.clone(), a.size))
                    .collect::<Vec<_>>(),
            ),
            None => (false, "New message".to_string(), Vec::new()),
        }
    };
    let sending = submitting();
    let attaching_now = attaching();
    let busy = sending || attaching_now;

    // Apply a newly opened draft once (do not clobber typing).
    {
        let ctx = ctx.clone();
        let mut last_draft_id = last_draft_id;
        let mut submitting = submitting;
        let mut attaching = attaching;
        let mut attach_gen = attach_gen;
        use_effect(move || match ctx.compose_draft.read().as_ref() {
            Some(session) => {
                let id = session.draft.id.as_str().to_string();
                if last_draft_id() != Some(id.clone()) {
                    last_draft_id.set(Some(id));
                    apply_draft_fields(
                        &session.draft,
                        &mut to,
                        &mut cc,
                        &mut bcc,
                        &mut subject,
                        &mut body,
                    );
                    show_cc_bcc.set(!session.draft.cc.is_empty() || !session.draft.bcc.is_empty());
                    error.set(None);
                    submitting.set(false);
                    let next = *attach_gen.peek() + 1;
                    attach_gen.set(next);
                    attaching.set(false);
                    submitted_id.set(None);
                }
            }
            None => {
                last_draft_id.set(None);
                submitting.set(false);
                let next = *attach_gen.peek() + 1;
                attach_gen.set(next);
                attaching.set(false);
                submitted_id.set(None);
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
            if !matches!(
                ctx.send_status.read().as_ref(),
                Some(SendState::Failed { .. })
            ) {
                return;
            }
            let Some(open_id) = ctx
                .compose_draft
                .read()
                .as_ref()
                .map(|s| s.draft.id.as_str().to_string())
            else {
                return;
            };
            if submitted_id() == Some(open_id) {
                submitting.set(false);
            }
        });
    }

    let no_account = ctx.selected_account.read().is_none();
    let mut compose_draft = ctx.compose_draft;
    let close = move |_| {
        compose_draft.set(None);
    };

    rsx! {
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

        if open {
            div {
                class: "compose-backdrop",
                onclick: close,
                div {
                    class: "ui-dialog compose-dialog",
                    role: "dialog",
                    aria_label: "{title}",
                    onclick: move |evt| evt.stop_propagation(),
                    onkeydown: {
                        let ctx = ctx.clone();
                        move |evt: KeyboardEvent| {
                            if matches!(evt.key(), Key::Enter)
                                && (evt.modifiers().ctrl() || evt.modifiers().meta())
                            {
                                evt.prevent_default();
                                submit_compose(
                                    &ctx,
                                    &core,
                                    to,
                                    cc,
                                    bcc,
                                    subject,
                                    body,
                                    error,
                                    submitting,
                                    submitted_id,
                                    attaching,
                                );
                            }
                        }
                    },
                    div {
                        class: "ui-dialog-head",
                        h2 { class: "ui-dialog-title", "{title}" }
                        IconButton {
                            class: "flat ui-icon-btn",
                            title: "Close",
                            size: 20,
                            icon: IconKind::XMark,
                            onclick: close,
                        }
                    }
                    div {
                        class: "compose-to-row",
                        label {
                            class: "ui-field",
                            span { "To" }
                            input {
                                class: "ui-input",
                                r#type: "email",
                                value: to(),
                                disabled: sending,
                                placeholder: "name@example.com",
                                oninput: move |e| to.set(e.value()),
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
                            input {
                                class: "ui-input",
                                r#type: "email",
                                value: cc(),
                                disabled: sending,
                                placeholder: "name@example.com",
                                oninput: move |e| cc.set(e.value()),
                            }
                        }
                        label {
                            class: "ui-field",
                            span { "Bcc" }
                            input {
                                class: "ui-input",
                                r#type: "email",
                                value: bcc(),
                                disabled: sending,
                                placeholder: "name@example.com",
                                oninput: move |e| bcc.set(e.value()),
                            }
                        }
                    }
                    label {
                        class: "ui-field",
                        span { "Subject" }
                        input {
                            class: "ui-input",
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
                            value: body(),
                            disabled: sending,
                            rows: 10,
                            oninput: move |e| body.set(e.value()),
                        }
                    }
                    div {
                        class: "compose-attachments",
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
                                            attach_selected_files(ctx, files, body, error).await;
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
                        if !attachments.is_empty() {
                            ul {
                                class: "compose-attachment-list",
                                for (id, filename, size) in attachments {
                                    li {
                                        key: "{id}",
                                        class: "compose-attachment",
                                        span {
                                            class: "compose-attachment-name",
                                            title: "{filename}",
                                            "{filename}"
                                        }
                                        span {
                                            class: "compose-attachment-size",
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
                                                    remove_attachment(compose_draft, &id);
                                                }
                                            },
                                            Icon { size: 14, icon: IconKind::XMark }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if let Some(err) = error() {
                        p { class: "ui-alert-error", "{err}" }
                    }
                    if let Some(SendState::Failed { message, .. }) = ctx.send_status.read().as_ref() {
                        p { class: "ui-alert-error", "{message}" }
                    }
                    div {
                        class: "ui-dialog-actions",
                        button {
                            class: "ui-btn ui-btn-secondary",
                            disabled: sending,
                            onclick: close,
                            "Cancel"
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
                                        to,
                                        cc,
                                        bcc,
                                        subject,
                                        body,
                                        error,
                                        submitting,
                                        submitted_id,
                                        attaching,
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
