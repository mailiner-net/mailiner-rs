use std::sync::Arc;

use chrono::{DateTime, Utc};
use dioxus::prelude::*;

use mailiner_composer::ComposeIntent;

use crate::components::attachments::AttachmentsFooter;
use crate::context::{AppContext, MessageViewState};
use crate::formatter::{FormatOptions, MessageFormatter};
use crate::message::{Message, MessageId};

use super::compose::open_reply_or_forward;

/// Format a UTC date for the message header.
fn format_date(dt: &DateTime<Utc>) -> String {
    dt.format("%d %b %Y, %H:%M").to_string()
}

/// Mount sanitized message HTML as a standalone document inside an open
/// shadow root.
///
/// `DOMParser` (`text/html`) rebuilds a real `<html>` / `<head>` / `<body>`
/// tree (same as the old TypeScript viewer). Adopting that document element
/// into the shadow root:
/// - lets email CSS targeting `html` / `body` match real elements
/// - keeps those rules from restyling Mailiner chrome (shadow isolation)
fn mount_shadow_html(host_id: &str, html: &str) {
    #[cfg(feature = "web")]
    {
        use web_sys::{DomParser, ShadowRootInit, ShadowRootMode, SupportedType};
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
        shadow.set_inner_html("");
        if html.is_empty() {
            return;
        }
        let Ok(parser) = DomParser::new() else {
            return;
        };
        let Ok(parsed) = parser.parse_from_string(html, SupportedType::TextHtml) else {
            return;
        };
        let Some(root) = parsed.document_element() else {
            return;
        };
        let Ok(adopted) = document.adopt_node(&root) else {
            return;
        };
        let _ = shadow.append_child(&adopted);
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
                        "Select a message"
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

                    AttachmentsFooter {}
                },
            }
        }
    }
}

fn ready_loaded(
    ctx: &AppContext,
    message_id: &MessageId,
) -> Option<Arc<mailiner_core::models::LoadedMessage>> {
    match &*ctx.message_view.read() {
        MessageViewState::Ready {
            message_id: loaded_id,
            loaded,
        } if loaded_id == message_id => Some(loaded.clone()),
        _ => None,
    }
}

#[component]
fn MessageHeader(message: Arc<Message>) -> Element {
    let ctx = use_context::<AppContext>();
    let date = format_date(&message.date);
    let actions_ready = ready_loaded(&ctx, &message.id).is_some();

    rsx! {
        header {
            class: "message-view-header",
            div {
                class: "message-view-headline",
                h2 {
                    class: "message-view-subject",
                    title: "{message.subject}",
                    if message.subject.trim().is_empty() {
                        span { class: "message-subject-empty", "(no subject)" }
                    } else {
                        "{message.subject}"
                    }
                }
                div {
                    class: "message-view-actions",
                    button {
                        class: "ui-btn ui-btn-secondary",
                        disabled: !actions_ready,
                        title: "Reply",
                        onclick: {
                            let message = message.clone();
                            let mut ctx = ctx.clone();
                            move |_| {
                                if let Some(loaded) = ready_loaded(&ctx, &message.id) {
                                    open_reply_or_forward(
                                        &mut ctx,
                                        ComposeIntent::Reply,
                                        &message.envelope,
                                        &loaded,
                                    );
                                }
                            }
                        },
                        "Reply"
                    }
                    button {
                        class: "ui-btn ui-btn-secondary",
                        disabled: !actions_ready,
                        title: "Forward",
                        onclick: {
                            let message = message.clone();
                            let mut ctx = ctx.clone();
                            move |_| {
                                if let Some(loaded) = ready_loaded(&ctx, &message.id) {
                                    open_reply_or_forward(
                                        &mut ctx,
                                        ComposeIntent::Forward,
                                        &message.envelope,
                                        &loaded,
                                    );
                                }
                            }
                        },
                        "Forward"
                    }
                }
            }
            div {
                class: "message-view-meta",
                span {
                    class: "message-view-meta-item",
                    title: "{message.from}",
                    span { class: "message-view-meta-k", "From" }
                    " {message.from_preview()}"
                }
                if !message.to.trim().is_empty() {
                    span {
                        class: "message-view-meta-item",
                        title: "{message.to}",
                        span { class: "message-view-meta-k", "To" }
                        " {message.to_preview()}"
                    }
                }
                span {
                    class: "message-view-meta-item",
                    span { class: "message-view-meta-k", "Date" }
                    " {date}"
                }
            }
        }
    }
}

fn find_envelope(ctx: &AppContext, message_id: &MessageId) -> Option<Arc<Message>> {
    let messages = ctx.messages.read();
    messages.find(|m| &m.id == message_id).cloned()
}
