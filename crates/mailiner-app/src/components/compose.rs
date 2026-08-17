//! Plain-text compose overlay (v1 send).

use dioxus::prelude::*;

use mailiner_composer::identity::FromIdentity;
use mailiner_composer::model::draft::{ComposerAddress, DraftDocument, BodyMode};
use mailiner_composer::{prepare_submit, PrepareSubmitError};

use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::send::{OutboxDisplay, SendState};

#[component]
pub fn ComposeOverlay() -> Element {
    let ctx = use_context::<AppContext>();
    let core = use_coroutine_handle::<CoreEvent>();
    let mut open = use_signal(|| false);
    let mut to = use_signal(String::new);
    let mut subject = use_signal(String::new);
    let mut body = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);

    let sending = matches!(
        ctx.send_status.read().as_ref(),
        Some(SendState::Sending { .. })
    );

    // Close overlay after a successful persist/send start (status Sent or Idle after queue).
    use_effect(move || {
        if let Some(SendState::Sent { .. }) = ctx.send_status.read().as_ref() {
            open.set(false);
            to.set(String::new());
            subject.set(String::new());
            body.set(String::new());
            error.set(None);
        }
    });

    rsx! {
        button {
            class: "compose-fab",
            title: "Compose",
            disabled: ctx.selected_account.read().is_none(),
            onclick: move |_| open.set(true),
            "Compose"
        }

        if open() {
            div {
                class: "compose-backdrop",
                div {
                    class: "compose-dialog",
                    role: "dialog",
                    aria_label: "New message",
                    h2 { "New message" }
                    label {
                        class: "compose-field",
                        "To"
                        input {
                            r#type: "email",
                            value: to(),
                            disabled: sending,
                            oninput: move |e| to.set(e.value()),
                        }
                    }
                    label {
                        class: "compose-field",
                        "Subject"
                        input {
                            value: subject(),
                            disabled: sending,
                            oninput: move |e| subject.set(e.value()),
                        }
                    }
                    label {
                        class: "compose-field",
                        "Message"
                        textarea {
                            value: body(),
                            disabled: sending,
                            rows: 8,
                            oninput: move |e| body.set(e.value()),
                        }
                    }
                    if let Some(err) = error() {
                        p { class: "compose-error", "{err}" }
                    }
                    if let Some(SendState::Failed { message, .. }) = ctx.send_status.read().as_ref() {
                        p { class: "compose-error", "{message}" }
                    }
                    div {
                        class: "compose-actions",
                        button {
                            disabled: sending,
                            onclick: move |_| open.set(false),
                            "Cancel"
                        }
                        button {
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
                                let mut draft = DraftDocument::new_empty(&identity);
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
