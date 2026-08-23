//! KMail-style folder jumper: **J** to go to a mailbox, **M** to move the current message.

use std::rc::Rc;

use dioxus::html::Key;
use dioxus::prelude::*;
use mailiner_core::MailboxRole;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use super::icons::{Icon, IconButton, IconKind};
use super::messageview::{MessageScroll, scroll_message_view};
use crate::context::{AppContext, MailboxPickerMode};
use crate::core_event::CoreEvent;
use crate::mailbox::{
    MailboxEntry, MailboxId, collect_mailbox_entries, filter_mailbox_entries,
};
use crate::toast::ToastAction;

fn role_icon(role: MailboxRole) -> IconKind {
    match role {
        MailboxRole::Inbox => IconKind::Inbox,
        MailboxRole::Drafts => IconKind::PencilSquare,
        MailboxRole::Sent => IconKind::PaperAirplane,
        MailboxRole::Outbox => IconKind::InboxStack,
        MailboxRole::Trash => IconKind::Trash,
        MailboxRole::Other => IconKind::Folder,
    }
}

fn claim_shortcut(evt: &web_sys::KeyboardEvent) {
    evt.prevent_default();
    evt.stop_propagation();
}

fn event_target_is_editable(evt: &web_sys::KeyboardEvent) -> bool {
    let Some(target) = evt.target() else {
        return false;
    };
    let Ok(el) = target.dyn_into::<web_sys::HtmlElement>() else {
        return false;
    };
    if el.is_content_editable() {
        return true;
    }
    matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT")
}

struct WindowKeydown {
    closure: Closure<dyn FnMut(web_sys::KeyboardEvent)>,
}

impl Drop for WindowKeydown {
    fn drop(&mut self) {
        if let Some(win) = web_sys::window() {
            let _ = win.remove_event_listener_with_callback_and_bool(
                "keydown",
                self.closure.as_ref().unchecked_ref(),
                true,
            );
        }
    }
}

#[component]
pub fn MailboxPickerHost() -> Element {
    let ctx = use_context::<AppContext>();
    let core = use_coroutine_handle::<CoreEvent>();
    let mode = *ctx.mailbox_picker.read();
    let mut open_gen = use_signal(|| 0u64);

    use_hook(|| {
        let mut ctx = ctx.clone();
        let mut open_gen = open_gen;
        let core = core;
        let closure = Closure::wrap(Box::new(move |evt: web_sys::KeyboardEvent| {
            if evt.ctrl_key() || evt.meta_key() || evt.alt_key() {
                return;
            }
            if event_target_is_editable(&evt) {
                return;
            }
            if ctx.compose_draft.peek().is_some() {
                return;
            }
            if ctx.mailbox_picker.peek().is_some() {
                return;
            }
            match evt.key().as_str() {
                "j" | "J" => {
                    claim_shortcut(&evt);
                    let next = open_gen() + 1;
                    open_gen.set(next);
                    ctx.mailbox_picker.set(Some(MailboxPickerMode::Jump));
                }
                "m" | "M" => {
                    if ctx.selected_message.peek().is_none() {
                        ctx.show_toast(ToastAction::info("Select a message first"));
                        return;
                    }
                    claim_shortcut(&evt);
                    let next = open_gen() + 1;
                    open_gen.set(next);
                    ctx.mailbox_picker.set(Some(MailboxPickerMode::Move));
                }
                "ArrowDown" => {
                    claim_shortcut(&evt);
                    let _ = core.send(CoreEvent::SelectAdjacent { delta: 1 });
                }
                "ArrowUp" => {
                    claim_shortcut(&evt);
                    let _ = core.send(CoreEvent::SelectAdjacent { delta: -1 });
                }
                "ArrowRight" => {
                    claim_shortcut(&evt);
                    scroll_message_view(true, MessageScroll::Line);
                }
                "ArrowLeft" => {
                    claim_shortcut(&evt);
                    scroll_message_view(false, MessageScroll::Line);
                }
                "PageDown" => {
                    claim_shortcut(&evt);
                    scroll_message_view(true, MessageScroll::Page);
                }
                "PageUp" => {
                    claim_shortcut(&evt);
                    scroll_message_view(false, MessageScroll::Page);
                }
                _ => {}
            }
        }) as Box<dyn FnMut(_)>);
        if let Some(win) = web_sys::window() {
            // Capture so PageUp/Down never become native scroll on the list.
            let _ = win.add_event_listener_with_callback_and_bool(
                "keydown",
                closure.as_ref().unchecked_ref(),
                true,
            );
        }
        Rc::new(WindowKeydown { closure })
    });

    // Host stays mounted, so this re-runs on every open/close. HTML `autofocus`
    // and in-keydown `focus()` only stick on the first insert; retry after paint.
    {
        let ctx = ctx.clone();
        use_effect(move || {
            if ctx.mailbox_picker.read().is_none() {
                return;
            }
            spawn(async move {
                for delay_ms in [0_u32, 16, 50, 100] {
                    sleep_ms(delay_ms).await;
                    if focus_picker_filter() {
                        return;
                    }
                }
            });
        });
    }

    let Some(mode) = mode else {
        return rsx! {};
    };

    rsx! {
        MailboxPicker { key: "{open_gen}", mode }
    }
}

#[component]
fn MailboxPicker(mode: MailboxPickerMode) -> Element {
    let mut ctx = use_context::<AppContext>();
    let core = use_coroutine_handle::<CoreEvent>();
    let mut query = use_signal(String::new);
    let current = ctx.selected_mailbox.read().clone();
    let nodes = ctx.mailbox_nodes.read();
    let roots = ctx.mailbox_roots.read();
    let mut entries = collect_mailbox_entries(&roots, &nodes);
    if mode == MailboxPickerMode::Move {
        entries.retain(|e| current.as_ref() != Some(&e.id));
    }
    let start_hi = current
        .as_ref()
        .and_then(|id| entries.iter().position(|e| &e.id == id))
        .unwrap_or(0);
    let mut highlight = use_signal(|| start_hi);
    let filtered = filter_mailbox_entries(&entries, &query.read());
    let count = filtered.len();
    let hi = if count == 0 {
        0
    } else {
        (*highlight.read()).min(count - 1)
    };

    {
        use_effect(move || {
            let i = *highlight.read();
            scroll_picker_row(i);
        });
    }

    let title = match mode {
        MailboxPickerMode::Jump => "Go to folder",
        MailboxPickerMode::Move => "Move to folder",
    };

    let accept_id = {
        let ctx = ctx.clone();
        Rc::new(move |id: MailboxId| {
            let mut picker = ctx.mailbox_picker;
            match mode {
                MailboxPickerMode::Jump => {
                    let _ = core.send(CoreEvent::JumpToMailbox(id));
                }
                MailboxPickerMode::Move => {
                    let Some(mailbox_id) = ctx.selected_mailbox.peek().clone() else {
                        ctx.show_toast(ToastAction::error("No mailbox selected"));
                        picker.set(None);
                        return;
                    };
                    let Some(message_id) = ctx.selected_message.peek().clone() else {
                        ctx.show_toast(ToastAction::info("Select a message first"));
                        picker.set(None);
                        return;
                    };
                    let _ = core.send(CoreEvent::MoveMessages {
                        mailbox_id,
                        message_ids: vec![message_id],
                        dest_mailbox_id: id,
                    });
                }
            }
            picker.set(None);
        })
    };

    let mut close = move |_| {
        ctx.mailbox_picker.set(None);
    };
    let filtered_ids: Vec<MailboxId> = filtered.iter().map(|e| e.id.clone()).collect();

    rsx! {
        div {
            class: "picker-backdrop",
            onclick: move |_| close(()),
            div {
                class: "ui-dialog picker-dialog",
                role: "dialog",
                aria_modal: "true",
                aria_label: "{title}",
                onclick: move |evt| evt.stop_propagation(),
                onkeydown: {
                    let accept_id = accept_id.clone();
                    let filtered_ids = filtered_ids.clone();
                    move |evt: KeyboardEvent| {
                    match evt.key() {
                        Key::ArrowDown => {
                            evt.prevent_default();
                            if count > 0 {
                                highlight.set((hi + 1) % count);
                            }
                        }
                        Key::ArrowUp => {
                            evt.prevent_default();
                            if count > 0 {
                                highlight.set((hi + count - 1) % count);
                            }
                        }
                        Key::Enter => {
                            evt.prevent_default();
                            if let Some(id) = filtered_ids.get(hi).cloned() {
                                accept_id(id);
                            }
                        }
                        Key::Escape => {
                            evt.prevent_default();
                            ctx.mailbox_picker.set(None);
                        }
                        _ => {}
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
                        onclick: move |_| close(()),
                    }
                }

                input {
                    class: "ui-input picker-filter",
                    id: "mailbox-picker-filter",
                    r#type: "text",
                    value: "{query}",
                    placeholder: "Type to filter folders",
                    aria_label: "Filter folders",
                    onmounted: move |evt| {
                        let data = evt.data();
                        spawn(async move {
                            // `set_focus` runs `focus()` before the await. Yield
                            // first so we are not still inside the J/M keydown.
                            sleep_ms(0).await;
                            let _ = data.set_focus(true).await;
                        });
                    },
                    oninput: move |evt| {
                        query.set(evt.value());
                        highlight.set(0);
                    },
                }

                ul {
                    class: "picker-list",
                    role: "listbox",
                    if filtered.is_empty() {
                        li {
                            class: "picker-empty",
                            "No matching folders"
                        }
                    } else {
                        for (i, entry) in filtered.iter().enumerate() {
                            PickerRow {
                                entry: (*entry).clone(),
                                index: i,
                                active: i == hi,
                                show_path: !query.read().trim().is_empty(),
                                onclick: {
                                    let id = entry.id.clone();
                                    let accept_id = accept_id.clone();
                                    move |_| accept_id(id.clone())
                                },
                            }
                        }
                    }
                }

                p {
                    class: "picker-hint",
                    "↑↓ to move · Enter to select · Esc to close"
                }
            }
        }
    }
}

#[component]
fn PickerRow(
    entry: MailboxEntry,
    index: usize,
    active: bool,
    show_path: bool,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    let label = if show_path {
        entry.path.clone()
    } else {
        format!(
            "{}{}",
            "\u{00a0}\u{00a0}".repeat(entry.depth),
            entry.title
        )
    };
    rsx! {
        li {
            class: "picker-row",
            class: if active { "active" },
            role: "option",
            aria_selected: if active { "true" } else { "false" },
            "data-mailbox-picker-idx": "{index}",
            onclick: move |evt| onclick.call(evt),
            span {
                class: "picker-row-icon",
                Icon { size: 16, icon: role_icon(entry.role) }
            }
            span { class: "picker-row-label", "{label}" }
        }
    }
}

fn focus_picker_filter() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
            return false;
        };
        let Some(el) = doc.get_element_by_id("mailbox-picker-filter") else {
            return false;
        };
        let Ok(el) = el.dyn_into::<web_sys::HtmlElement>() else {
            return false;
        };
        el.focus().is_ok()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        false
    }
}

fn scroll_picker_row(index: usize) {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let Ok(Some(el)) = doc.query_selector(&format!("[data-mailbox-picker-idx='{index}']"))
        else {
            return;
        };
        el.scroll_into_view();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = index;
    }
}

async fn sleep_ms(ms: u32) {
    #[cfg(target_arch = "wasm32")]
    {
        gloo_timers::future::TimeoutFuture::new(ms).await;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = ms;
    }
}
