use std::sync::Arc;

use chrono::{DateTime, Utc};
use dioxus::prelude::*;

use mailiner_composer::ComposeIntent;

use mailiner_core::MailboxRole;
use mailiner_core::models::PartKind;

use crate::components::attachments::AttachmentsFooter;
use crate::context::{AppContext, MessageViewState};
use crate::core_event::CoreEvent;
use crate::formatter::{FormatOptions, MessageFormatter};
use crate::mailbox::{MailboxId, flatten_mailboxes};
use crate::message::{Message, MessageId, preview_mailbox};

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
        // UA `body { margin: 8px }` and `cursor: auto` (I-beam over text) make
        // the pointer appear to jump when crossing the header → body edge,
        // especially on Linux cursor themes with off-center hotspots.
        if let Ok(reset) = document.create_element("style") {
            reset.set_text_content(Some(
                ":host { display: block; }\n\
                 html, body { margin: 0; cursor: default; overflow: visible; }\n",
            ));
            let _ = shadow.append_child(&reset);
        }
        let _ = shadow.append_child(&adopted);
    }
    #[cfg(not(feature = "web"))]
    {
        let _ = (host_id, html);
    }
}

const MESSAGE_CONTENT_ID: &str = "mailiner-message-content";

/// How far to move the message body. Line ≈ arrow-key scroll; page ≈ PageUp/Down.
#[derive(Clone, Copy)]
pub(crate) enum MessageScroll {
    Line,
    Page,
}

/// Scroll the message body. `down` is visually down (Right / PageDown).
///
/// Uses the browser's `behavior: smooth` easing. Repeated keys within
/// `SMOOTH_COALESCE_MS` accumulate on the animation target so we do not
/// restart from a mid-flight `scrollTop`.
pub(crate) fn scroll_message_view(down: bool, step: MessageScroll) {
    #[cfg(target_arch = "wasm32")]
    {
        use std::cell::Cell;
        use web_sys::{ScrollBehavior, ScrollToOptions};

        const SMOOTH_COALESCE_MS: f64 = 350.0;
        thread_local! {
            static PENDING: Cell<(f64, f64)> = const { Cell::new((f64::NAN, 0.0)) };
        }

        let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let Some(el) = doc.get_element_by_id(MESSAGE_CONTENT_ID) else {
            return;
        };
        let height = f64::from(el.client_height());
        let delta = message_scroll_delta(height, down, step);
        if delta == 0.0 {
            return;
        }
        let max = (f64::from(el.scroll_height()) - height).max(0.0);
        let current = f64::from(el.scroll_top());
        let now = js_sys::Date::now();
        let (pending_top, pending_at) = PENDING.with(|p| p.get());
        let next = next_smooth_target(
            current,
            pending_top,
            pending_at,
            now,
            delta,
            max,
            SMOOTH_COALESCE_MS,
        );
        PENDING.with(|p| p.set((next, now)));

        let opts = ScrollToOptions::new();
        opts.set_top(next);
        opts.set_behavior(ScrollBehavior::Smooth);
        el.scroll_to_with_scroll_to_options(&opts);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (down, step);
    }
}

fn next_smooth_target(
    current: f64,
    pending_top: f64,
    pending_at: f64,
    now: f64,
    delta: f64,
    max: f64,
    coalesce_ms: f64,
) -> f64 {
    let base = if pending_top.is_finite() && now - pending_at < coalesce_ms {
        pending_top
    } else {
        current
    };
    (base + delta).clamp(0.0, max)
}

/// Line step matches a typical browser arrow-key scroll (~3 lines).
/// Page step is almost one viewport, with a little overlap for context.
fn message_scroll_delta(client_height: f64, down: bool, step: MessageScroll) -> f64 {
    let amount = match step {
        MessageScroll::Line => 48.0,
        MessageScroll::Page => {
            if client_height <= 0.0 {
                return 0.0;
            }
            (client_height * 0.85).max(48.0)
        }
    };
    if down { amount } else { -amount }
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
    let mut prefer_plain = use_signal(|| false);
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
            let prefer = *prefer_plain.read();
            let key = view_message_key(&view);

            // Reset per-message toggles when the selected message changes.
            if *last_msg_key.borrow() != key {
                *last_msg_key.borrow_mut() = key.clone();
                if allow || prefer {
                    allow_remote.set(false);
                    prefer_plain.set(false);
                    // Effect will re-run after toggles clear.
                    return;
                }
            }

            match &view {
                MessageViewState::Ready { loaded, .. } => {
                    let mut fmt = MessageFormatter::new(FormatOptions {
                        allow_remote_resources: allow,
                        prefer_plain: prefer,
                    });
                    if let Some(result) = fmt.format(&loaded.parts) {
                        prevented_remote.set(result.prevented_remote_resources && !allow);
                        formatted_html.set(result.html.clone());
                        mount_shadow_html(MESSAGE_CONTENT_ID, &result.html);
                    } else {
                        prevented_remote.set(false);
                        let fallback =
                            "<p class=\"mlnr-empty-body\">No displayable content.</p>".to_string();
                        formatted_html.set(fallback.clone());
                        mount_shadow_html(MESSAGE_CONTENT_ID, &fallback);
                    }
                }
                MessageViewState::Empty
                | MessageViewState::Loading { .. }
                | MessageViewState::Error { .. } => {
                    prevented_remote.set(false);
                    formatted_html.set(String::new());
                    mount_shadow_html(MESSAGE_CONTENT_ID, "");
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
                        MessageHeader { message: env, prefer_plain }
                    }
                    div {
                        class: "message-view-loading",
                        "Loading…"
                    }
                },
                MessageViewState::Error { message, .. } => rsx! {
                    if let Some(env) = envelope {
                        MessageHeader { message: env, prefer_plain }
                    }
                    div {
                        class: "message-view-error",
                        "Failed to load message: {message}"
                    }
                },
                MessageViewState::Ready { .. } => rsx! {
                    if let Some(env) = envelope {
                        MessageHeader { message: env, prefer_plain }
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
                        id: MESSAGE_CONTENT_ID,
                        class: "mlnr-msg-host",
                        // Host stays empty in RSX — content is mounted into shadow DOM.
                    }

                    AttachmentsFooter {}
                },
            }
        }
    }
}

fn has_html_and_plain(parts: &[mailiner_core::models::MessagePart]) -> bool {
    let has_html = parts
        .iter()
        .any(|p| !p.is_hidden && p.kind == PartKind::TextHtml);
    let has_plain = parts
        .iter()
        .any(|p| !p.is_hidden && p.kind == PartKind::TextPlain);
    has_html && has_plain
}

pub(crate) fn ready_loaded(
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
fn MessageHeader(message: Arc<Message>, mut prefer_plain: Signal<bool>) -> Element {
    let ctx = use_context::<AppContext>();
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let date = format_date(&message.date);
    let reply_to = message.reply_to();
    let loaded = ready_loaded(&ctx, &message.id);
    let actions_ready = loaded.is_some();
    let show_plain_toggle = loaded
        .as_ref()
        .is_some_and(|loaded| has_html_and_plain(&loaded.parts));
    let mailbox_id = ctx.selected_mailbox.read().clone();
    let in_trash = mailbox_id
        .as_ref()
        .and_then(|id| {
            ctx.mailbox_nodes
                .read()
                .get(id)
                .map(|n| n.role == MailboxRole::Trash)
        })
        .unwrap_or(false);
    let move_targets = {
        let nodes = ctx.mailbox_nodes.read();
        let roots = ctx.mailbox_roots.read();
        flatten_mailboxes(&roots, &nodes)
            .into_iter()
            .filter(|(id, _)| mailbox_id.as_ref() != Some(id))
            .collect::<Vec<_>>()
    };
    let mut move_seq = use_signal(|| 0u32);
    let selected_ids = ctx.selected_ids();
    let selected_n = selected_ids.len();
    let all_selected_read = {
        let list = ctx.messages.read();
        !selected_ids.is_empty()
            && selected_ids.iter().all(|id| {
                list.find(|m| m.id == *id)
                    .map(|m| m.is_read)
                    .unwrap_or(message.is_read)
            })
    };
    let is_read = if selected_n > 1 {
        all_selected_read
    } else {
        message.is_read
    };

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
                    if show_plain_toggle {
                        button {
                            class: "ui-btn ui-btn-secondary",
                            title: if prefer_plain() { "Show HTML" } else { "Plain text" },
                            onclick: move |_| prefer_plain.set(!prefer_plain()),
                            if prefer_plain() { "Show HTML" } else { "Plain text" }
                        }
                    }
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
                        title: "Reply all",
                        onclick: {
                            let message = message.clone();
                            let mut ctx = ctx.clone();
                            move |_| {
                                if let Some(loaded) = ready_loaded(&ctx, &message.id) {
                                    open_reply_or_forward(
                                        &mut ctx,
                                        ComposeIntent::ReplyAll,
                                        &message.envelope,
                                        &loaded,
                                    );
                                }
                            }
                        },
                        "Reply All"
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
                    button {
                        class: "ui-btn ui-btn-secondary",
                        title: if is_read { "Mark as unread" } else { "Mark as read" },
                        onclick: {
                            let mailbox_id = mailbox_id.clone();
                            let ids = selected_ids.clone();
                            move |_| {
                                let Some(mailbox_id) = mailbox_id.clone() else {
                                    return;
                                };
                                if ids.is_empty() {
                                    return;
                                }
                                let _ = core_tx.send(CoreEvent::MarkRead {
                                    mailbox_id,
                                    message_ids: ids.clone(),
                                    is_read: !is_read,
                                });
                            }
                        },
                        if is_read { "Mark unread" } else { "Mark read" }
                    }
                    select {
                        key: "{move_seq}",
                        class: "ui-btn ui-btn-secondary message-move-select",
                        title: "Move to folder",
                        aria_label: "Move to folder",
                        value: "",
                        onchange: {
                            let mailbox_id = mailbox_id.clone();
                            let ids = selected_ids.clone();
                            move |evt: FormEvent| {
                                let dest = evt.value();
                                move_seq.set(move_seq() + 1);
                                if dest.is_empty() {
                                    return;
                                }
                                let Some(mailbox_id) = mailbox_id.clone() else {
                                    return;
                                };
                                if ids.is_empty() {
                                    return;
                                }
                                let _ = core_tx.send(CoreEvent::MoveMessages {
                                    mailbox_id,
                                    message_ids: ids.clone(),
                                    dest_mailbox_id: MailboxId::from(dest),
                                });
                            }
                        },
                        option {
                            value: "",
                            disabled: true,
                            selected: true,
                            "Move to…"
                        }
                        for (id, title) in move_targets {
                            option {
                                value: "{id.to_string()}",
                                "{title}"
                            }
                        }
                    }
                    button {
                        class: "ui-btn ui-btn-secondary",
                        title: if in_trash { "Delete permanently" } else { "Move to Trash" },
                        onclick: {
                            let mailbox_id = mailbox_id.clone();
                            let ids = selected_ids.clone();
                            let n = selected_n;
                            move |_| {
                                let Some(mailbox_id) = mailbox_id.clone() else {
                                    return;
                                };
                                if ids.is_empty() {
                                    return;
                                }
                                #[cfg(feature = "web")]
                                if in_trash {
                                    let Some(window) = web_sys::window() else {
                                        return;
                                    };
                                    let prompt = if n == 1 {
                                        "Delete this message permanently?".to_string()
                                    } else {
                                        format!("Delete {n} messages permanently?")
                                    };
                                    match window.confirm_with_message(&prompt) {
                                        Ok(true) => {}
                                        _ => return,
                                    }
                                }
                                let _ = core_tx.send(CoreEvent::MoveToTrash {
                                    mailbox_id,
                                    message_ids: ids.clone(),
                                });
                            }
                        },
                        if in_trash { "Delete" } else { "Trash" }
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
                if let Some(cc) = message.cc.as_deref().filter(|s| !s.trim().is_empty()) {
                    span {
                        class: "message-view-meta-item",
                        title: "{cc}",
                        span { class: "message-view-meta-k", "Cc" }
                        " {message.cc_preview()}"
                    }
                }
                if let Some(bcc) = message.bcc.as_deref().filter(|s| !s.trim().is_empty()) {
                    span {
                        class: "message-view-meta-item",
                        title: "{bcc}",
                        span { class: "message-view-meta-k", "Bcc" }
                        " {message.bcc_preview()}"
                    }
                }
                if let Some(reply_to) = reply_to.as_deref() {
                    span {
                        class: "message-view-meta-item",
                        title: "{reply_to}",
                        span { class: "message-view-meta-k", "Reply-To" }
                        " {preview_mailbox(reply_to)}"
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

pub(crate) fn find_envelope(ctx: &AppContext, message_id: &MessageId) -> Option<Arc<Message>> {
    let messages = ctx.messages.read();
    messages.find(|m| &m.id == message_id).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_step_is_small_and_signed() {
        assert_eq!(message_scroll_delta(400.0, true, MessageScroll::Line), 48.0);
        assert_eq!(
            message_scroll_delta(400.0, false, MessageScroll::Line),
            -48.0
        );
    }

    #[test]
    fn page_step_uses_viewport_with_overlap() {
        assert_eq!(
            message_scroll_delta(400.0, true, MessageScroll::Page),
            340.0
        );
        assert_eq!(
            message_scroll_delta(400.0, false, MessageScroll::Page),
            -340.0
        );
        assert_eq!(message_scroll_delta(0.0, true, MessageScroll::Page), 0.0);
    }

    #[test]
    fn rapid_keys_extend_the_smooth_target() {
        let first = next_smooth_target(0.0, f64::NAN, 0.0, 1000.0, 48.0, 1000.0, 350.0);
        assert_eq!(first, 48.0);
        let second = next_smooth_target(12.0, first, 1000.0, 1100.0, 48.0, 1000.0, 350.0);
        assert_eq!(second, 96.0);
        let after_pause = next_smooth_target(96.0, second, 1100.0, 1600.0, 48.0, 1000.0, 350.0);
        assert_eq!(after_pause, 144.0);
    }
}
