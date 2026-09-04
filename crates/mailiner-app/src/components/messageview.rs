use std::sync::Arc;

use chrono::{DateTime, Utc};
use dioxus::html::Key;
use dioxus::prelude::*;

use mailiner_composer::{ComposeIntent, ComposerAddress, try_composer_address};

use mailiner_core::MailboxRole;
use mailiner_core::models::{EmailAddr, EmailAddress, MessageContent, PartKind};

use crate::components::attachments::AttachmentsFooter;
use crate::components::icons::{IconButton, IconKind};
use crate::context::{AppContext, MailboxPickerMode, MessageHeadersState, MessageViewState};
use crate::core_event::CoreEvent;
use crate::download::{DownloadStatus, EML_DOWNLOAD_KEY, eml_filename};
use crate::formatter::quote::QUOTE_TOGGLE_CSS;
use crate::formatter::{FormatOptions, MessageFormatter, retain_reply_cid_payloads};
use crate::mailbox::{MailboxId, flatten_mailboxes};
use crate::message::{Message, MessageId, preview_mailbox};
use crate::phishing::{self, SenderCue};
use crate::print::{PrintError, PrintHeaders, build_print_document, open_print_document};
use crate::toast::ToastAction;

use super::compose::{open_new_message_to, open_reply_or_forward};

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
        //
        // Reading pane: inherit color-scheme from chrome (dark host + UA
        // defaults). Do not invert HTML mail — images and brand colors stay.
        // Plain text (`.mlnr-plain`) uses inherited foreground.
        if let Ok(reset) = document.create_element("style") {
            reset.set_text_content(Some(
                ":host { display: block; color-scheme: inherit; background: transparent; color: inherit; }\n\
                 html, body { margin: 0; cursor: default; overflow: visible; color-scheme: inherit; background: transparent; }\n\
                 .mlnr-plain, .mlnr-empty-body { color: inherit; background: transparent; }\n\
                 .mlnr-plain a { color: var(--item-accent); }\n",
            ));
            let _ = shadow.append_child(&reset);
        }
        let _ = shadow.append_child(&adopted);
        // After the document so email CSS cannot hide the quote toggle.
        if let Ok(quote) = document.create_element("style") {
            quote.set_text_content(Some(QUOTE_TOGGLE_CSS));
            let _ = shadow.append_child(&quote);
        }
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
    let mut allow_remote = use_signal(|| crate::ui_prefs::remote_image_decision(None).allowed());
    let mut prefer_plain = use_signal(|| false);
    let mut formatted_html = use_signal(|| String::new());
    let mut prevented_remote = use_signal(|| false);
    let mut had_remote = use_signal(|| false);
    let last_msg_key = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(None::<String>)));
    let html_cache =
        use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(InlinedHtmlCache::default())));

    // Format body + privacy flag when loaded message or allow_remote changes.
    // Mount into shadow DOM after formatting.
    {
        let mut ctx = ctx.clone();
        let last_msg_key = last_msg_key.clone();
        let html_cache = html_cache.clone();
        use_effect(move || {
            let view = ctx.message_view.read().clone();
            let allow = *allow_remote.read();
            let prefer = *prefer_plain.read();
            let key = view_message_key(&view);

            // Reset per-message toggles when the selected message changes.
            if *last_msg_key.borrow() != key {
                *last_msg_key.borrow_mut() = key.clone();
                html_cache.borrow_mut().reset();
                had_remote.set(false);
                let sender = match &view {
                    MessageViewState::Ready { message_id, .. }
                    | MessageViewState::Loading { message_id }
                    | MessageViewState::Error { message_id, .. } => find_envelope(&ctx, message_id)
                        .and_then(|m| m.sender_email().map(str::to_string)),
                    MessageViewState::Empty => None,
                };
                let desired = crate::ui_prefs::remote_image_decision(sender.as_deref()).allowed();
                if allow != desired || prefer {
                    allow_remote.set(desired);
                    prefer_plain.set(false);
                    // Effect will re-run after toggles clear.
                    return;
                }
            }

            let ready = match &view {
                MessageViewState::Ready { message_id, loaded } => {
                    Some((message_id.clone(), loaded.clone()))
                }
                MessageViewState::Empty
                | MessageViewState::Loading { .. }
                | MessageViewState::Error { .. } => {
                    prevented_remote.set(false);
                    formatted_html.set(String::new());
                    mount_shadow_html(MESSAGE_CONTENT_ID, "");
                    None
                }
            };
            let Some((message_id, loaded)) = ready else {
                return;
            };
            drop(view);

            if !prefer {
                let cache = html_cache.borrow();
                if let Some(html) = cache.html_for(&key, allow) {
                    if cache.prevented_remote {
                        had_remote.set(true);
                    }
                    prevented_remote.set(cache.prevented_remote && !allow);
                    mount_shadow_html(MESSAGE_CONTENT_ID, html);
                    return;
                }
            }

            let mut fmt = MessageFormatter::new(FormatOptions {
                allow_remote_resources: allow,
                prefer_plain: prefer,
            });
            if let Some(result) = fmt.format(&loaded.parts) {
                let inlined = result.inlined_part_ids.clone();
                let prevented = result.prevented_remote_resources;
                let html = result.html;

                if prevented {
                    had_remote.set(true);
                }
                prevented_remote.set(prevented && !allow);
                mount_shadow_html(MESSAGE_CONTENT_ID, &html);

                if !prefer && !inlined.is_empty() {
                    // Cache both remote-policy variants so Allow does not
                    // re-format. Referenced CID payloads stay on `loaded`.
                    let allowed_html = if prevented && !allow {
                        MessageFormatter::new(FormatOptions {
                            allow_remote_resources: true,
                            prefer_plain: false,
                        })
                        .format(&loaded.parts)
                        .map(|r| r.html)
                    } else {
                        None
                    };
                    *html_cache.borrow_mut() = InlinedHtmlCache {
                        message_key: key.clone(),
                        blocked: Some(html),
                        allowed: allowed_html,
                        prevented_remote: prevented,
                    };
                } else {
                    formatted_html.set(html);
                }

                drop(loaded);
                apply_cid_payload_retention(&mut ctx, &message_id, &inlined);
            } else {
                prevented_remote.set(false);
                let fallback =
                    "<p class=\"mlnr-empty-body\">No displayable content.</p>".to_string();
                formatted_html.set(fallback.clone());
                mount_shadow_html(MESSAGE_CONTENT_ID, &fallback);
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
    let from_email = envelope
        .as_ref()
        .and_then(|m| m.sender_email().map(str::to_string));
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
                        MessageHeader { message: env, prefer_plain, formatted_html }
                    }
                    div {
                        class: "message-view-loading",
                        "Loading…"
                    }
                },
                MessageViewState::Error { message, .. } => rsx! {
                    if let Some(env) = envelope {
                        MessageHeader { message: env, prefer_plain, formatted_html }
                    }
                    div {
                        class: "message-view-error",
                        "Failed to load message: {message}"
                    }
                },
                MessageViewState::Ready { .. } => rsx! {
                    if let Some(env) = envelope {
                        MessageHeader { message: env, prefer_plain, formatted_html }
                    }

                    RemotePrivacyBanner {
                        allow_remote,
                        prevented_remote: *prevented_remote.read(),
                        had_remote: *had_remote.read(),
                        from_email: from_email.clone(),
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

/// Cached HTML so Allow / plain toggles can re-render without reformatting.
#[derive(Default)]
struct InlinedHtmlCache {
    message_key: Option<String>,
    blocked: Option<String>,
    allowed: Option<String>,
    prevented_remote: bool,
}

impl InlinedHtmlCache {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn html_for(&self, key: &Option<String>, allow_remote: bool) -> Option<&str> {
        if self.message_key != *key {
            return None;
        }
        if allow_remote {
            self.allowed.as_deref().or(self.blocked.as_deref())
        } else if self.prevented_remote {
            // `blocked` is only stripped HTML when we formatted with allow=false.
            // An allow-first format stores allowed HTML there; miss so we re-strip.
            self.blocked.as_deref()
        } else {
            None
        }
    }
}

fn apply_cid_payload_retention(
    ctx: &mut AppContext,
    message_id: &MessageId,
    referenced: &[String],
) {
    let mut view = ctx.message_view.write();
    let MessageViewState::Ready {
        message_id: id,
        loaded,
    } = &mut *view
    else {
        return;
    };
    if id != message_id {
        return;
    }
    retain_reply_cid_payloads(&mut Arc::make_mut(loaded).parts, referenced);
}

fn has_decoded_text(part: &mailiner_core::models::MessagePart) -> bool {
    matches!(part.content, MessageContent::Text(_))
}

fn has_html_and_plain(parts: &[mailiner_core::models::MessagePart]) -> bool {
    let has_html = parts
        .iter()
        .any(|p| !p.is_hidden && p.kind == PartKind::TextHtml && has_decoded_text(p));
    let has_plain = parts
        .iter()
        .any(|p| !p.is_hidden && p.kind == PartKind::TextPlain && has_decoded_text(p));
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

fn print_loaded_message(ctx: &AppContext, message: &Message, body_html: &str) {
    let date = format_date(&message.date);
    let html = build_print_document(
        &PrintHeaders {
            from: &message.from,
            to: &message.to,
            cc: message.cc.as_deref(),
            subject: &message.subject,
            date: &date,
        },
        body_html,
    );
    let ctx = ctx.clone();
    open_print_document(&html, move |err| match err {
        PrintError::PopupBlocked => {
            ctx.show_toast(ToastAction::info(
                "Pop-up blocked. Allow pop-ups to print this message.",
            ));
        }
        PrintError::Failed => {
            ctx.show_toast(ToastAction::error("Could not open print preview."));
        }
    });
}

/// One mailbox shown in the viewer header.
#[derive(Clone, Debug, PartialEq, Eq)]
struct HeaderAddress {
    /// Visible text (display name, else email).
    label: String,
    /// Tooltip / accessible name (`Name <email>` when both exist).
    title: String,
    /// Prefill for compose; `None` when the mailbox is missing.
    compose_to: Option<ComposerAddress>,
}

/// Convert a viewer [`EmailAddr`] into a compose recipient.
///
/// Skips missing or empty email. Keeps a trimmed display name when present.
fn composer_address_from_email_addr(addr: &EmailAddr) -> Option<ComposerAddress> {
    try_composer_address(addr)
}

fn header_address(addr: &EmailAddr) -> Option<HeaderAddress> {
    let compose_to = composer_address_from_email_addr(addr);
    let name = addr
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());
    let label = match (name, compose_to.as_ref()) {
        (Some(n), _) => n.to_string(),
        (None, Some(c)) => c.email.clone(),
        (None, None) => return None,
    };
    Some(HeaderAddress {
        label,
        title: addr.to_string(),
        compose_to,
    })
}

fn header_addresses(addr: Option<&EmailAddress>) -> Vec<HeaderAddress> {
    match addr {
        None => Vec::new(),
        Some(EmailAddress::List(list)) => list.iter().filter_map(header_address).collect(),
        Some(EmailAddress::Group(groups)) => groups
            .iter()
            .flat_map(|g| g.members.iter())
            .filter_map(header_address)
            .collect(),
    }
}

fn resolve_header_addresses(parsed: Vec<HeaderAddress>, fallback: &str) -> Vec<HeaderAddress> {
    if !parsed.is_empty() {
        return parsed;
    }
    let fallback = fallback.trim();
    if fallback.is_empty() {
        return Vec::new();
    }
    vec![HeaderAddress {
        label: preview_mailbox(fallback).to_string(),
        title: fallback.to_string(),
        compose_to: None,
    }]
}

#[component]
fn HeaderAddressRow(
    label: &'static str,
    addresses: Vec<HeaderAddress>,
    fallback: String,
    always: bool,
) -> Element {
    let ctx = use_context::<AppContext>();
    let addresses = resolve_header_addresses(addresses, &fallback);
    if addresses.is_empty() && !always {
        return rsx! {};
    }
    let title = addresses
        .iter()
        .map(|a| a.title.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    rsx! {
        span {
            class: "message-view-meta-item",
            title: "{title}",
            span { class: "message-view-meta-k", "{label}" }
            " "
            for (i, addr) in addresses.into_iter().enumerate() {
                if i > 0 {
                    ", "
                }
                if let Some(to) = addr.compose_to.clone() {
                    button {
                        class: "message-view-addr",
                        r#type: "button",
                        title: "{addr.title}",
                        aria_label: "Compose to {addr.title}",
                        onclick: {
                            let mut ctx = ctx.clone();
                            move |_| open_new_message_to(&mut ctx, to.clone())
                        },
                        "{addr.label}"
                    }
                } else {
                    span { "{addr.label}" }
                }
            }
        }
    }
}

#[component]
fn RemotePrivacyBanner(
    mut allow_remote: Signal<bool>,
    prevented_remote: bool,
    had_remote: bool,
    from_email: Option<String>,
) -> Element {
    use crate::ui_prefs::{self, RemoteImagePref, RemoteImageSource};

    let decision = ui_prefs::remote_image_decision(from_email.as_deref());
    let domain = from_email.as_deref().and_then(ui_prefs::domain_of_email);
    let showing = allow_remote();
    let remembered_allow =
        showing && decision.allowed() && decision.source != RemoteImageSource::Global;
    let persist_this_message = showing && !decision.allowed() && had_remote;
    let blocked = !showing && prevented_remote;

    if !blocked && !persist_this_message && !remembered_allow {
        return rsx! {};
    }

    let sender_label = from_email
        .as_deref()
        .and_then(ui_prefs::normalize_email)
        .unwrap_or_default();
    let domain_label = domain
        .as_deref()
        .map(|d| format!("@{d}"))
        .unwrap_or_default();
    let from_forget = from_email.clone();
    let domain_forget = domain.clone();
    let from_save = from_email.clone();
    let domain_save = domain.clone();

    rsx! {
        div {
            class: if blocked {
                "message-privacy-banner"
            } else {
                "message-privacy-banner is-allowed"
            },
            role: "status",
            if blocked {
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
            } else if remembered_allow {
                span {
                    if decision.source == RemoteImageSource::Domain {
                        "Remote images are allowed for {domain_label}."
                    } else {
                        "Remote images are allowed for {sender_label}."
                    }
                }
                button {
                    class: "message-privacy-remember",
                    onclick: move |_| {
                        match decision.source {
                            RemoteImageSource::Address => {
                                if let Some(email) = from_forget.as_deref() {
                                    ui_prefs::clear_remote_image_address(email);
                                }
                            }
                            RemoteImageSource::Domain => {
                                if let Some(d) = domain_forget.as_deref() {
                                    ui_prefs::clear_remote_image_domain(d);
                                }
                            }
                            RemoteImageSource::Global => {}
                        }
                        let next = ui_prefs::remote_image_decision(from_forget.as_deref()).allowed();
                        allow_remote.set(next);
                    },
                    "Don't always allow"
                }
            } else {
                span {
                    "Remote resources are shown for this message."
                }
            }
            if (blocked || persist_this_message) && !sender_label.is_empty() {
                button {
                    class: "message-privacy-remember",
                    title: "Always allow remote images from {sender_label}",
                    onclick: move |_| {
                        if let Some(email) = from_save.as_deref() {
                            ui_prefs::save_remote_image_address(email, RemoteImagePref::Allow);
                        }
                        allow_remote.set(true);
                    },
                    "Always allow from this sender"
                }
            }
            if (blocked || persist_this_message) && !domain_label.is_empty() {
                button {
                    class: "message-privacy-remember",
                    title: "Always allow remote images from {domain_label}",
                    onclick: move |_| {
                        if let Some(d) = domain_save.as_deref() {
                            ui_prefs::save_remote_image_domain(d, RemoteImagePref::Allow);
                        }
                        allow_remote.set(true);
                    },
                    "Always allow from this domain"
                }
            }
        }
    }
}

#[component]
fn MessageHeader(
    message: Arc<Message>,
    mut prefer_plain: Signal<bool>,
    formatted_html: Signal<String>,
) -> Element {
    let ctx = use_context::<AppContext>();
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let date = format_date(&message.date);
    let reply_to = message.reply_to();
    let loaded = ready_loaded(&ctx, &message.id);
    let actions_ready = loaded.is_some();
    let show_plain_toggle = loaded
        .as_ref()
        .is_some_and(|loaded| has_html_and_plain(&loaded.parts));
    let account_id = ctx.selected_account.read().clone();
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
    let archive_id = crate::mailbox::find_archive_mailbox(&ctx.mailbox_nodes.read());
    let show_archive = archive_id
        .as_ref()
        .is_some_and(|id| mailbox_id.as_ref() != Some(id));
    let junk_id = crate::mailbox::find_junk_mailbox(&ctx.mailbox_nodes.read());
    let show_junk = junk_id
        .as_ref()
        .is_some_and(|id| mailbox_id.as_ref() != Some(id));
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
    let eml_busy = matches!(
        ctx.download_status.read().get(EML_DOWNLOAD_KEY),
        Some(DownloadStatus::InProgress { .. })
    );
    let all_selected_starred = {
        let list = ctx.messages.read();
        !selected_ids.is_empty()
            && selected_ids.iter().all(|id| {
                list.find(|m| m.id == *id)
                    .map(|m| m.is_starred)
                    .unwrap_or(message.is_starred)
            })
    };
    let all_selected_flagged = {
        let list = ctx.messages.read();
        !selected_ids.is_empty()
            && selected_ids.iter().all(|id| {
                list.find(|m| m.id == *id)
                    .map(|m| m.is_flagged)
                    .unwrap_or(message.is_flagged)
            })
    };
    let is_starred = if selected_n > 1 {
        all_selected_starred
    } else {
        message.is_starred
    };
    let is_flagged = if selected_n > 1 {
        all_selected_flagged
    } else {
        message.is_flagged
    };
    let own_email = account_id
        .as_ref()
        .and_then(|id| ctx.accounts.read().get(id).map(|a| a.email.clone()));
    let sender_cues = phishing::analyze_from(message.envelope.from.as_ref(), own_email.as_deref());
    let mut from_addresses = header_addresses(message.envelope.from.as_ref());
    if !sender_cues.is_empty() {
        for addr in &mut from_addresses {
            addr.label = addr.title.clone();
        }
    }

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
                    IconButton {
                        class: if is_starred {
                            "ui-btn ui-btn-secondary message-star-btn is-on"
                        } else {
                            "ui-btn ui-btn-secondary message-star-btn"
                        },
                        title: if is_starred { "Unstar" } else { "Star" },
                        size: 16,
                        icon: IconKind::Star,
                        aria_pressed: Some(is_starred),
                        onclick: {
                            let account_id = account_id.clone();
                            let mailbox_id = mailbox_id.clone();
                            let ids = selected_ids.clone();
                            move |_| {
                                let (Some(account_id), Some(mailbox_id)) =
                                    (account_id.clone(), mailbox_id.clone())
                                else {
                                    return;
                                };
                                if ids.is_empty() {
                                    return;
                                }
                                let _ = core_tx.send(CoreEvent::ToggleStar {
                                    account_id,
                                    mailbox_id,
                                    message_ids: ids.clone(),
                                });
                            }
                        },
                    }
                    IconButton {
                        class: if is_flagged {
                            "ui-btn ui-btn-secondary message-flag-btn is-on"
                        } else {
                            "ui-btn ui-btn-secondary message-flag-btn"
                        },
                        title: if is_flagged { "Unflag" } else { "Flag" },
                        size: 16,
                        icon: IconKind::Flag,
                        aria_pressed: Some(is_flagged),
                        onclick: {
                            let account_id = account_id.clone();
                            let mailbox_id = mailbox_id.clone();
                            let ids = selected_ids.clone();
                            move |_| {
                                let (Some(account_id), Some(mailbox_id)) =
                                    (account_id.clone(), mailbox_id.clone())
                                else {
                                    return;
                                };
                                if ids.is_empty() {
                                    return;
                                }
                                let _ = core_tx.send(CoreEvent::ToggleFlag {
                                    account_id,
                                    mailbox_id,
                                    message_ids: ids.clone(),
                                });
                            }
                        },
                    }
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
                        title: "Show full message headers",
                        onclick: {
                            let mailbox_id = mailbox_id.clone();
                            let message_id = message.id.clone();
                            let mut ctx = ctx.clone();
                            move |_| {
                                let Some(mailbox_id) = mailbox_id.clone() else {
                                    return;
                                };
                                let already_open = matches!(
                                    &*ctx.message_headers.peek(),
                                    MessageHeadersState::Loading { message_id: id }
                                        | MessageHeadersState::Ready { message_id: id, .. }
                                        if id == &message_id
                                );
                                if already_open {
                                    return;
                                }
                                ctx.message_headers.set(MessageHeadersState::Loading {
                                    message_id: message_id.clone(),
                                });
                                let _ = core_tx.send(CoreEvent::FetchMessageHeaders {
                                    mailbox_id,
                                    message_id: message_id.clone(),
                                });
                            }
                        },
                        "Show headers"
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
                        disabled: !actions_ready,
                        title: "Print",
                        onclick: {
                            let message = message.clone();
                            let ctx = ctx.clone();
                            move |_| {
                                if ready_loaded(&ctx, &message.id).is_none() {
                                    return;
                                }
                                print_loaded_message(&ctx, &message, &formatted_html.peek());
                            }
                        },
                        "Print"
                    }
                    button {
                        class: "ui-btn ui-btn-secondary",
                        disabled: eml_busy || mailbox_id.is_none() || ctx.selected_account.read().is_none(),
                        title: "Save as .eml",
                        onclick: {
                            let mailbox_id = mailbox_id.clone();
                            let message = message.clone();
                            let account_id = ctx.selected_account.read().clone();
                            move |_| {
                                let Some(mailbox_id) = mailbox_id.clone() else {
                                    return;
                                };
                                let Some(account_id) = account_id.clone() else {
                                    return;
                                };
                                let _ = core_tx.send(CoreEvent::SaveMessageEml {
                                    account_id,
                                    mailbox_id,
                                    message_id: message.id.clone(),
                                    filename: eml_filename(&message.subject),
                                    size_hint: message.envelope.size,
                                });
                            }
                        },
                        if eml_busy { "Saving…" } else { "Save as .eml" }
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
                    button {
                        class: "ui-btn ui-btn-secondary",
                        title: "Copy to folder",
                        onclick: {
                            let mut ctx = ctx.clone();
                            move |_| {
                                ctx.mailbox_picker.set(Some(MailboxPickerMode::Copy));
                            }
                        },
                        "Copy to…"
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
                    if show_archive {
                        button {
                            class: "ui-btn ui-btn-secondary",
                            title: "Move to Archive",
                            onclick: {
                                let account_id = account_id.clone();
                                let mailbox_id = mailbox_id.clone();
                                let ids = selected_ids.clone();
                                move |_| {
                                    let Some(account_id) = account_id.clone() else {
                                        return;
                                    };
                                    let Some(mailbox_id) = mailbox_id.clone() else {
                                        return;
                                    };
                                    if ids.is_empty() {
                                        return;
                                    }
                                    let _ = core_tx.send(CoreEvent::ArchiveMessages {
                                        account_id,
                                        mailbox_id,
                                        message_ids: ids.clone(),
                                    });
                                }
                            },
                            "Archive"
                        }
                    }
                    if show_junk {
                        button {
                            class: "ui-btn ui-btn-secondary",
                            title: "Move to Junk",
                            onclick: {
                                let account_id = account_id.clone();
                                let mailbox_id = mailbox_id.clone();
                                let ids = selected_ids.clone();
                                move |_| {
                                    let Some(account_id) = account_id.clone() else {
                                        return;
                                    };
                                    let Some(mailbox_id) = mailbox_id.clone() else {
                                        return;
                                    };
                                    if ids.is_empty() {
                                        return;
                                    }
                                    let _ = core_tx.send(CoreEvent::MoveToJunk {
                                        account_id,
                                        mailbox_id,
                                        message_ids: ids.clone(),
                                    });
                                }
                            },
                            "Junk"
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
                HeaderAddressRow {
                    label: "From",
                    addresses: from_addresses,
                    fallback: message.from.clone(),
                    always: true,
                }
                HeaderAddressRow {
                    label: "To",
                    addresses: header_addresses(message.envelope.to.as_ref()),
                    fallback: message.to.clone(),
                    always: false,
                }
                HeaderAddressRow {
                    label: "Cc",
                    addresses: header_addresses(message.envelope.cc.as_ref()),
                    fallback: message.cc.clone().unwrap_or_default(),
                    always: false,
                }
                HeaderAddressRow {
                    label: "Bcc",
                    addresses: header_addresses(message.envelope.bcc.as_ref()),
                    fallback: message.bcc.clone().unwrap_or_default(),
                    always: false,
                }
                HeaderAddressRow {
                    label: "Reply-To",
                    addresses: header_addresses(message.envelope.reply_to.as_ref()),
                    fallback: reply_to.unwrap_or_default(),
                    always: false,
                }
                span {
                    class: "message-view-meta-item",
                    span { class: "message-view-meta-k", "Date" }
                    " {date}"
                }
            }
            SenderCueBanner { cues: sender_cues }
        }
    }
}

#[component]
fn SenderCueBanner(cues: Vec<SenderCue>) -> Element {
    if cues.is_empty() {
        return rsx! {};
    }
    rsx! {
        div {
            class: "message-sender-cue",
            role: "status",
            for cue in cues.iter() {
                p {
                    class: "message-sender-cue-line",
                    "{cue.message()}"
                }
            }
        }
    }
}

pub(crate) fn find_envelope(ctx: &AppContext, message_id: &MessageId) -> Option<Arc<Message>> {
    let messages = ctx.messages.read();
    messages.find(|m| &m.id == message_id).cloned()
}

fn close_headers_dialog(ctx: &mut AppContext) {
    ctx.message_headers.set(MessageHeadersState::Closed);
}

fn copy_headers_to_clipboard(ctx: &AppContext, text: &str) {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        use wasm_bindgen_futures::JsFuture;
        let text = text.to_string();
        let ctx = ctx.clone();
        spawn(async move {
            let Some(window) = web_sys::window() else {
                ctx.show_toast(ToastAction::error("Could not copy headers"));
                return;
            };
            match JsFuture::from(window.navigator().clipboard().write_text(&text)).await {
                Ok(_) => ctx.show_toast(ToastAction::info("Headers copied")),
                Err(_) => ctx.show_toast(ToastAction::error("Could not copy headers")),
            }
        });
    }
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    {
        let _ = text;
        ctx.show_toast(ToastAction::error("Could not copy headers"));
    }
}

#[component]
pub fn MessageHeadersHost() -> Element {
    let ctx = use_context::<AppContext>();
    let state = ctx.message_headers.read().clone();
    if matches!(state, MessageHeadersState::Closed) {
        return rsx! {};
    }

    rsx! {
        MessageHeadersDialog { state }
    }
}

#[component]
fn MessageHeadersDialog(state: MessageHeadersState) -> Element {
    let ctx = use_context::<AppContext>();
    let ready_text = match &state {
        MessageHeadersState::Ready { text, .. } => Some(text.clone()),
        _ => None,
    };

    rsx! {
        div {
            class: "picker-backdrop headers-backdrop",
            onclick: {
                let mut ctx = ctx.clone();
                move |_| close_headers_dialog(&mut ctx)
            },
            div {
                class: "ui-dialog headers-dialog",
                role: "dialog",
                aria_modal: "true",
                aria_label: "Message headers",
                tabindex: "-1",
                onclick: move |evt| evt.stop_propagation(),
                onkeydown: {
                    let mut ctx = ctx.clone();
                    move |evt: KeyboardEvent| {
                        if evt.key() == Key::Escape {
                            evt.prevent_default();
                            close_headers_dialog(&mut ctx);
                        }
                    }
                },
                onmounted: move |evt| {
                    let data = evt.data();
                    spawn(async move {
                        let _ = data.set_focus(true).await;
                    });
                },

                div {
                    class: "ui-dialog-head",
                    h2 { class: "ui-dialog-title", "Message headers" }
                    IconButton {
                        class: "flat ui-icon-btn",
                        title: "Close",
                        size: 20,
                        icon: IconKind::XMark,
                        onclick: {
                            let mut ctx = ctx.clone();
                            move |_| close_headers_dialog(&mut ctx)
                        },
                    }
                }

                match &state {
                    MessageHeadersState::Loading { .. } => rsx! {
                        p { class: "headers-status", "Loading headers…" }
                    },
                    MessageHeadersState::Error { message, .. } => rsx! {
                        p { class: "ui-alert-error", "Failed to load headers: {message}" }
                    },
                    MessageHeadersState::Ready { text, .. } => rsx! {
                        pre { class: "headers-pre", "{text}" }
                    },
                    MessageHeadersState::Closed => rsx! {},
                }

                div {
                    class: "ui-dialog-actions",
                    if let Some(text) = ready_text {
                        button {
                            class: "ui-btn ui-btn-secondary",
                            title: "Copy headers",
                            onclick: {
                                let ctx = ctx.clone();
                                move |_| copy_headers_to_clipboard(&ctx, &text)
                            },
                            "Copy"
                        }
                    }
                    button {
                        class: "ui-btn ui-btn-secondary",
                        onclick: {
                            let mut ctx = ctx.clone();
                            move |_| close_headers_dialog(&mut ctx)
                        },
                        "Close"
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

    fn addr(name: Option<&str>, email: Option<&str>) -> EmailAddr {
        EmailAddr {
            name: name.map(str::to_string),
            email: email.map(str::to_string),
        }
    }

    #[test]
    fn composer_address_keeps_name_and_skips_empty_email() {
        let named =
            composer_address_from_email_addr(&addr(Some(" Ada "), Some(" ada@example.com ")))
                .expect("named mailbox");
        assert_eq!(named.name.as_deref(), Some("Ada"));
        assert_eq!(named.email, "ada@example.com");

        assert!(composer_address_from_email_addr(&addr(Some("No Mail"), None)).is_none());
        assert!(composer_address_from_email_addr(&addr(Some("No Mail"), Some("  "))).is_none());
        assert_eq!(
            composer_address_from_email_addr(&addr(None, Some("solo@example.com")))
                .map(|a| a.email),
            Some("solo@example.com".into())
        );
    }

    #[test]
    fn header_addresses_flatten_and_skip_blank() {
        let list = EmailAddress::List(vec![
            addr(Some("Ada"), Some("ada@example.com")),
            addr(Some("No Mail"), None),
            addr(None, Some("  ")),
            addr(None, Some("bob@example.com")),
        ]);
        let v = header_addresses(Some(&list));
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].label, "Ada");
        assert_eq!(v[0].title, "Ada <ada@example.com>");
        assert_eq!(
            v[0].compose_to.as_ref().map(|a| a.email.as_str()),
            Some("ada@example.com")
        );
        assert_eq!(v[1].label, "No Mail");
        assert!(v[1].compose_to.is_none());
        assert_eq!(v[2].label, "bob@example.com");
        assert_eq!(
            v[2].compose_to.as_ref().map(|a| a.email.as_str()),
            Some("bob@example.com")
        );

        let group = EmailAddress::Group(vec![mailiner_core::models::Group {
            name: Some("Team".into()),
            members: vec![addr(None, Some("t1@ex.com")), addr(None, Some("t2@ex.com"))],
        }]);
        assert_eq!(header_addresses(Some(&group)).len(), 2);
        assert!(header_addresses(None).is_empty());
    }

    #[test]
    fn resolve_header_addresses_falls_back_to_preview() {
        let fallback = resolve_header_addresses(Vec::new(), "Ada <ada@example.com>");
        assert_eq!(fallback.len(), 1);
        assert_eq!(fallback[0].label, "Ada");
        assert!(fallback[0].compose_to.is_none());
        assert!(resolve_header_addresses(Vec::new(), "  ").is_empty());
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

    #[test]
    fn html_cache_serves_blocked_and_allowed() {
        let key = Some("5:INBOX\u{1f}1".into());
        let cache = InlinedHtmlCache {
            message_key: key.clone(),
            blocked: Some("blocked".into()),
            allowed: Some("allowed".into()),
            prevented_remote: true,
        };
        assert_eq!(cache.html_for(&key, false), Some("blocked"));
        assert_eq!(cache.html_for(&key, true), Some("allowed"));
        assert_eq!(cache.html_for(&Some("other".into()), false), None);
    }

    #[test]
    fn html_cache_allowed_falls_back_to_blocked() {
        let key = Some("k".into());
        let cache = InlinedHtmlCache {
            message_key: key.clone(),
            blocked: Some("same".into()),
            allowed: None,
            prevented_remote: false,
        };
        assert_eq!(cache.html_for(&key, true), Some("same"));
    }

    #[test]
    fn html_cache_misses_block_when_only_allowed_html_was_stored() {
        let key = Some("k".into());
        let cache = InlinedHtmlCache {
            message_key: key.clone(),
            blocked: Some("<img src=\"https://tracker.example/pixel.png\">".into()),
            allowed: None,
            prevented_remote: false,
        };
        assert_eq!(cache.html_for(&key, false), None);
        assert_eq!(
            cache.html_for(&key, true),
            Some("<img src=\"https://tracker.example/pixel.png\">")
        );
    }
}
