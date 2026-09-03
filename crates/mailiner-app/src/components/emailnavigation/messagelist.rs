use std::ops::Range;
use std::sync::Arc;

use dioxus::prelude::*;

use crate::components::emailnavigation::navigationheader::{Mode, NavigationHeader};
use crate::components::icons::{Icon, IconKind};
use crate::components::virtual_scroll::VirtualScroll;
use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::message::Message;
use chrono::{DateTime, Utc};

fn list_date(dt: &DateTime<Utc>) -> String {
    let now = Utc::now();
    if dt.date_naive() == now.date_naive() {
        dt.format("%H:%M").to_string()
    } else if dt.format("%Y").to_string() == now.format("%Y").to_string() {
        dt.format("%d %b").to_string()
    } else {
        dt.format("%d %b %Y").to_string()
    }
}

const BUFFER_SIZE: usize = 5;
const MAX_CACHED: usize = 500;

#[component]
pub fn MessageList() -> Element {
    let ctx = use_context::<AppContext>();
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let selected_mailbox = ctx.selected_mailbox.read().clone();
    let loading = *ctx.messages_loading.read();
    let total = ctx.messages.read().total_count();
    let density = *ctx.message_list_density.read();
    let cached = ctx.messages.read().cached_count();
    let selected_n = ctx.selection.read().len();
    let has_known = cached > 0;

    let on_need_range = move |range: Range<usize>| {
        if let Some(mailbox_id) = ctx.selected_mailbox.peek().clone() {
            let _ = core_tx.send(CoreEvent::FetchMessageRange { mailbox_id, range });
        }
    };

    let render_item = move |args: (usize, Arc<Message>)| -> Element {
        let (index, message) = args;
        let core_tx = use_coroutine_handle::<CoreEvent>();
        let ctx = use_context::<AppContext>();
        let selection = ctx.selection.read();
        let is_selected = selection.contains(&message.id);
        let is_focused = selection.focus() == Some(&message.id);
        let avatar = message.avatar_color();

        rsx! {
            div {
                class: "message-list-item",
                class: if is_selected { "selected" },
                class: if is_focused { "focused" },
                class: if !message.is_read { "unread" },
                aria_selected: if is_selected { "true" } else { "false" },

                onmousedown: move |evt: MouseEvent| {
                    if evt.modifiers().shift() || evt.modifiers().ctrl() || evt.modifiers().meta() {
                        evt.prevent_default();
                    }
                },

                onclick: move |evt: MouseEvent| {
                    evt.prevent_default();
                    let _ = core_tx.send(CoreEvent::SelectListClick {
                        message_id: message.id.clone(),
                        index,
                        extend: evt.modifiers().shift(),
                        toggle: evt.modifiers().ctrl() || evt.modifiers().meta(),
                    });
                },

                div {
                    class: "message-avatar",
                    style: "background-color: {avatar}",
                    aria_hidden: "true",
                }

                div {
                    class: "message-list-item-content",

                    div {
                        class: "message-list-item-top",
                        div {
                            class: "message-from",
                            "{message.from_preview()}"
                        }
                        div {
                            class: "message-list-item-meta",
                            if message.is_answered {
                                span {
                                    class: "message-answered-indicator",
                                    role: "img",
                                    aria_label: "Replied",
                                    title: "Replied",
                                    Icon { size: 14, icon: IconKind::ArrowUturnLeft }
                                }
                            }
                            if message.is_starred {
                                span {
                                    class: "message-star-indicator",
                                    role: "img",
                                    aria_label: "Starred",
                                    title: "Starred",
                                    Icon { size: 14, icon: IconKind::Star }
                                }
                            }
                            if message.is_flagged {
                                span {
                                    class: "message-flag-indicator",
                                    role: "img",
                                    aria_label: "Flagged",
                                    title: "Flagged",
                                    Icon { size: 14, icon: IconKind::Flag }
                                }
                            }
                            if message.has_attachments {
                                span {
                                    class: "message-attachment-indicator",
                                    role: "img",
                                    aria_label: "Has attachments",
                                    title: "Has attachments",
                                    Icon { size: 14, icon: IconKind::PaperClip }
                                }
                            }
                            div {
                                class: "message-date",
                                "{list_date(&message.date)}"
                            }
                        }
                    }

                    div {
                        class: "message-subject",
                        if message.subject.trim().is_empty() {
                            span { class: "message-subject-empty", "(no subject)" }
                        } else {
                            "{message.subject}"
                        }
                    }
                }
            }
        }
    };

    rsx! {
        section {
            id: "messagelist",
            class: "{density.css_class()}",

            NavigationHeader {
                mode: Mode::MessageList,
            }

            if selected_mailbox.is_some() && !loading && total > 0 {
                div {
                    class: "message-list-selection",
                    button {
                        r#type: "button",
                        class: "message-list-select-action",
                        title: "Select all loaded messages (Ctrl+A)",
                        disabled: !has_known,
                        onclick: move |_| {
                            let _ = core_tx.send(CoreEvent::SelectAllKnown);
                        },
                        "Select all"
                    }
                    button {
                        r#type: "button",
                        class: "message-list-select-action",
                        title: "Select unread loaded messages",
                        disabled: !has_known,
                        onclick: move |_| {
                            let _ = core_tx.send(CoreEvent::SelectUnreadKnown);
                        },
                        "Unread"
                    }
                    button {
                        r#type: "button",
                        class: "message-list-select-action",
                        title: "Invert selection among loaded messages",
                        disabled: !has_known,
                        onclick: move |_| {
                            let _ = core_tx.send(CoreEvent::InvertSelection);
                        },
                        "Invert"
                    }
                    if selected_n > 0 {
                        span {
                            class: "message-list-selected-count",
                            aria_live: "polite",
                            "{selected_n} selected"
                        }
                    }
                }
            }

            div {
                class: "message-list-body",

                if selected_mailbox.is_none() {
                    div {
                        class: "message-list-empty",
                        "Select a mailbox"
                    }
                } else if loading {
                    div {
                        class: "message-list-empty",
                        "Loading…"
                    }
                } else if total == 0 {
                    div {
                        class: "message-list-empty",
                        "No messages"
                    }
                } else {
                    VirtualScroll {
                        items: ctx.messages,
                        item_height: density.item_height(),
                        buffer_size: BUFFER_SIZE,
                        debounce_ms: Some(100),
                        max_cached: Some(MAX_CACHED),
                        reveal_index: ctx.selection.read().focus().and_then(|id| {
                            ctx.messages.read().position(|m| m.id == *id)
                        }),
                        on_need_range: on_need_range,
                        render_item: render_item,
                    }
                }
            }
        }
    }
}
