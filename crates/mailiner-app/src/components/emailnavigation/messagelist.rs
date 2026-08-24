use std::ops::Range;
use std::sync::Arc;

use dioxus::prelude::*;

use crate::components::emailnavigation::navigationheader::{Mode, NavigationHeader};
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

/// Must match `#messagelist .message-list-item` padding + avatar size.
const ITEM_HEIGHT: f64 = 52.0;
const BUFFER_SIZE: usize = 5;
const MAX_CACHED: usize = 500;

#[component]
pub fn MessageList() -> Element {
    let ctx = use_context::<AppContext>();
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let selected_mailbox = ctx.selected_mailbox.read().clone();
    let loading = *ctx.messages_loading.read();
    let total = ctx.messages.read().total_count();

    let on_need_range = move |range: Range<usize>| {
        if let Some(mailbox_id) = ctx.selected_mailbox.peek().clone() {
            let _ = core_tx.send(CoreEvent::FetchMessageRange { mailbox_id, range });
        }
    };

    let render_item = move |args: (usize, Arc<Message>)| -> Element {
        let (_index, message) = args;
        let core_tx = use_coroutine_handle::<CoreEvent>();
        let ctx = use_context::<AppContext>();
        let selected_message = ctx.selected_message.read();
        let is_selected = selected_message
            .as_ref()
            .map(|id| *id == message.id)
            .unwrap_or(false);
        let avatar = message.avatar_color();

        rsx! {
            div {
                class: "message-list-item",
                class: if is_selected { "selected" },
                class: if !message.is_read { "unread" },

                onclick: move |_| {
                    let _ = core_tx.send(CoreEvent::SelectMessage(message.id.clone()));
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
                            class: "message-date",
                            "{list_date(&message.date)}"
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

            NavigationHeader {
                mode: Mode::MessageList,
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
                        item_height: ITEM_HEIGHT,
                        buffer_size: BUFFER_SIZE,
                        debounce_ms: Some(100),
                        max_cached: Some(MAX_CACHED),
                        reveal_index: ctx.selected_message.read().as_ref().and_then(|id| {
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
