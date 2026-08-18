//! Plain-text compose overlay (v1 send).

use dioxus::prelude::*;

use super::icons::{Icon, IconButton, IconKind};

use mailiner_composer::identity::FromIdentity;
use mailiner_composer::model::draft::{ComposerAddress, DraftDocument, BodyMode};
use mailiner_composer::{build_draft, prepare_submit, ComposeIntent, PrepareSubmitError};

use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::send::{ComposeSession, OutboxDisplay, SendState};

fn apply_draft_fields(draft: &DraftDocument, to: &mut Signal<String>, subject: &mut Signal<String>, body: &mut Signal<String>) {
    to.set(
        draft
            .to
            .iter()
            .map(|a| a.email.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    );
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
        ctx.toast.set(Some("Select an account first.".into()));
        return;
    };
    let Some(account) = ctx.accounts.read().get(&account_id).cloned() else {
        ctx.toast.set(Some("Account not found.".into()));
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
        Err(e) => ctx.toast.set(Some(e.to_string())),
    }
}

#[component]
pub fn ComposeOverlay() -> Element {
    let ctx = use_context::<AppContext>();
    let core = use_coroutine_handle::<CoreEvent>();
    let mut to = use_signal(String::new);
    let mut subject = use_signal(String::new);
    let mut body = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let last_draft_id = use_signal(|| None::<String>);

    let sending = matches!(
        ctx.send_status.read().as_ref(),
        Some(SendState::Sending { .. })
    );
    let session = ctx.compose_draft.read().clone();
    let open = session.is_some();
    let title = session
        .as_ref()
        .map(|s| s.title.as_str())
        .unwrap_or("New message")
        .to_string();

    // Apply a newly opened draft once (do not clobber typing).
    {
        let ctx = ctx.clone();
        let mut last_draft_id = last_draft_id;
        use_effect(move || {
            match ctx.compose_draft.read().as_ref() {
                Some(session) => {
                    let id = session.draft.id.as_str().to_string();
                    if last_draft_id() != Some(id.clone()) {
                        last_draft_id.set(Some(id));
                        apply_draft_fields(&session.draft, &mut to, &mut subject, &mut body);
                        error.set(None);
                    }
                }
                None => last_draft_id.set(None),
            }
        });
    }

    {
        let mut ctx = ctx.clone();
        use_effect(move || {
            if let Some(SendState::Sent { .. }) = ctx.send_status.read().as_ref() {
                ctx.compose_draft.set(None);
                to.set(String::new());
                subject.set(String::new());
                body.set(String::new());
                error.set(None);
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
                            disabled: sending,
                            onclick: move |_| {
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
                                draft.to = to()
                                    .split(',')
                                    .map(|s| s.trim())
                                    .filter(|s| !s.is_empty())
                                    .map(ComposerAddress::email_only)
                                    .collect();
                                match prepare_submit(&draft, &identity) {
                                    Ok(prepared) => {
                                        let display = OutboxDisplay {
                                            subject: draft.subject.clone(),
                                            to_preview: prepared.envelope.rcpt_to.join(", "),
                                        };
                                        core.send(CoreEvent::SendMessage {
                                            account_id,
                                            request: mailiner_core::SubmitRequest {
                                                mail_from: prepared.envelope.mail_from,
                                                rcpt_to: prepared.envelope.rcpt_to,
                                                rfc822: prepared.rfc822,
                                                message_id: prepared.message_id,
                                            },
                                            display,
                                        });
                                    }
                                    Err(PrepareSubmitError::Validation(errs)) => {
                                        error.set(Some(format!("Cannot send: {errs:?}")));
                                    }
                                    Err(e) => error.set(Some(e.to_string())),
                                }
                            },
                            if sending { "Sending…" } else { "Send" }
                        }
                    }
                }
            }
        }
    }
}
