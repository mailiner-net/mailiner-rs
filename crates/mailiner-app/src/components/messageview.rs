use std::sync::Arc;

use chrono::{DateTime, Utc};
use dioxus::prelude::*;

use crate::context::{AppContext, MessageViewState};
use crate::formatter::{FormatOptions, MessageFormatter};
use crate::message::{Message, MessageId};

/// Format a UTC date for the message header.
fn format_date(dt: &DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M UTC").to_string()
}

fn mount_shadow_html(host_id: &str, html: &str) {
    #[cfg(feature = "web")]
    {
        use web_sys::{ShadowRootInit, ShadowRootMode};
        let Some(window) = web_sys::window() else {
            return;
        };
        let Some(document) = window.document() else {
            return;
        };
        let Some(host) = document.get_element_by_id(host_id) else {
            return;
        };
        let shadow = match host.shadow_root() {
            Some(s) => s,
            None => {
                let init = ShadowRootInit::new(ShadowRootMode::Open);
                match host.attach_shadow(&init) {
                    Ok(s) => s,
                    Err(_) => return,
                }
            }
        };
        shadow.set_inner_html(html);
    }
    #[cfg(not(feature = "web"))]
    {
        let _ = (host_id, html);
    }
}

fn view_message_key(state: &MessageViewState) -> Option<String> {
    match state {
        MessageViewState::Ready { message_id, .. }
        | MessageViewState::Loading { message_id }
        | MessageViewState::Error { message_id, .. } => Some(message_id.to_string()),
        MessageViewState::Empty => None,
    }
}

#[component]
pub fn MessageView() -> Element {
    let ctx = use_context::<AppContext>();
    let mut allow_remote = use_signal(|| false);
    let mut formatted_html = use_signal(|| String::new());
    let mut prevented_remote = use_signal(|| false);
    let last_msg_key = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(None::<String>)));

    // Format body + privacy flag when loaded message or allow_remote changes.
    // Mount into shadow DOM after formatting.
    {
        let ctx = ctx.clone();
        let last_msg_key = last_msg_key.clone();
        use_effect(move || {
            let view = ctx.message_view.read().clone();
            let allow = *allow_remote.read();
            let key = view_message_key(&view);

            // Reset privacy toggle when the selected message changes.
            if *last_msg_key.borrow() != key {
                *last_msg_key.borrow_mut() = key.clone();
                if allow {
                    allow_remote.set(false);
                    // Effect will re-run after allow_remote clears.
                    return;
                }
            }

            match &view {
                MessageViewState::Ready { loaded, .. } => {
                    let mut fmt = MessageFormatter::new(FormatOptions {
                        allow_remote_resources: allow,
                    });
                    if let Some(result) = fmt.format(&loaded.parts) {
                        prevented_remote.set(result.prevented_remote_resources && !allow);
                        formatted_html.set(result.html.clone());
                        mount_shadow_html("mailiner-message-content", &result.html);
                    } else {
                        prevented_remote.set(false);
                        let fallback =
                            "<p class=\"mlnr-empty-body\">No displayable content.</p>".to_string();
                        formatted_html.set(fallback.clone());
                        mount_shadow_html("mailiner-message-content", &fallback);
                    }
                }
                MessageViewState::Empty
                | MessageViewState::Loading { .. }
                | MessageViewState::Error { .. } => {
                    prevented_remote.set(false);
                    formatted_html.set(String::new());
                    mount_shadow_html("mailiner-message-content", "");
                }
            }
        });
    }

    let view = ctx.message_view.read().clone();
    let envelope = match &view {
        MessageViewState::Ready { message_id, .. }
        | MessageViewState::Loading { message_id }
        | MessageViewState::Error { message_id, .. } => find_envelope(&ctx, message_id),
        MessageViewState::Empty => None,
    };

    rsx! {
        section {
            id: "messageview",

            match &view {
                MessageViewState::Empty => rsx! {
                    div {
                        class: "message-view-empty",
                        "Select a message to read"
                    }
                },
                MessageViewState::Loading { .. } => rsx! {
                    if let Some(env) = envelope {
                        MessageHeader { message: env }
                    }
                    div {
                        class: "message-view-loading",
                        "Loading…"
                    }
                },
                MessageViewState::Error { message, .. } => rsx! {
                    if let Some(env) = envelope {
                        MessageHeader { message: env }
                    }
                    div {
                        class: "message-view-error",
                        "Failed to load message: {message}"
                    }
                },
                MessageViewState::Ready { .. } => rsx! {
                    if let Some(env) = envelope {
                        MessageHeader { message: env }
                    }

                    if *prevented_remote.read() {
                        div {
                            class: "message-privacy-banner",
                            role: "status",
                            span {
                                "To protect your privacy, remote resources (images, styles) were blocked."
                            }
                            button {
                                class: "message-privacy-allow",
                                onclick: move |_| {
                                    allow_remote.set(true);
                                },
                                "Allow"
                            }
                        }
                    }

                    div {
                        id: "mailiner-message-content",
                        class: "mlnr-msg-host",
                        // Host stays empty in RSX — content is mounted into shadow DOM.
                    }
                },
            }
        }
    }
}

#[component]
fn MessageHeader(message: Arc<Message>) -> Element {
    let date = format_date(&message.date);
    rsx! {
        header {
            class: "message-view-header",
            h2 {
                class: "message-view-subject",
                "{message.subject}"
            }
            ul {
                class: "message-view-meta",
                li {
                    strong { "From: " }
                    "{message.from}"
                }
                li {
                    strong { "Date: " }
                    "{date}"
                }
            }
        }
    }
}

fn find_envelope(ctx: &AppContext, message_id: &MessageId) -> Option<Arc<Message>> {
    let messages = ctx.messages.read();
    messages.find(|m| &m.id == message_id).cloned()
}
