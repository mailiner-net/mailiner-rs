use std::sync::Arc;
use std::ops::Range;
use std::pin::Pin;

use dioxus::prelude::*;
use mailiner_core::connector::EmailConnector;

use crate::components::virtual_scroll::{VirtualScroll, VirtualScrollProps, prepend_message, VirtualScrollState};
use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::message::Message;

#[component]
pub fn VirtualMessageList() -> Element {
    let ctx = use_context::<AppContext>();
    let selected_mailbox = ctx.selected_mailbox.read();

    // For demo purposes, we'll simulate a large mailbox
    let total_messages = 10000;

    // Create a fetch function that returns messages for a given range
    let fetch_messages = async move |range: Range<usize>| -> Vec<Arc<Message>> {
        // Simulate async fetching with a small delay
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        // Generate mock messages for the range
        let mut messages = Vec::new();
        for i in range {
            let message = Arc::new(Message {
                id: format!("msg-{}", i).into(),
                subject: format!("Message {} - Important email about project updates", i),
                from: format!("sender{}@example.com", i % 10),
                to: "you@example.com".to_string(),
                cc: if i % 3 == 0 { Some("team@example.com".to_string()) } else { None },
                bcc: None,
            });
            messages.push(message);
        }
        messages
    };

    let render_item = move |args: (usize, &Arc<Message>)| -> Element {
        let (index, message) = args;
        let core_tx = use_coroutine_handle::<CoreEvent>();
        let ctx = use_context::<AppContext>();
        let selected_message = ctx.selected_message.read();
        let is_selected = selected_message.as_ref().map(|id| *id == message.id).unwrap_or(false);

        rsx! {
            div {
                class: "message-list-item",
                class: if is_selected { "selected" },
                onclick: move |_| {
                    let _ = core_tx.send(CoreEvent::SelectMessage(message.id.clone()));
                },

                div {
                    class: "message-list-item-content",
                    style: "padding: 12px; border-bottom: 1px solid #e0e0e0; cursor: pointer;",

                    div {
                        class: "message-from",
                        style: "font-weight: 600; margin-bottom: 4px;",
                        "{message.from}"
                    }

                    div {
                        class: "message-subject",
                        style: "color: #333; margin-bottom: 2px;",
                        "{message.subject}"
                    }

                    if let Some(cc) = &message.cc {
                        div {
                            class: "message-cc",
                            style: "font-size: 0.9em; color: #666;",
                            "CC: {cc}"
                        }
                    }
                }
            }
        }
    };

    rsx! {
        div {
            id: "virtual-messagelist",
            style: "height: 100%; width: 100%;",

            if selected_mailbox.is_some() {
                VirtualScroll {
                    total_items: total_messages,
                    item_height: 80.0,  // Approximate height of each message item
                    viewport_height: 600.0,  // This should be calculated from actual viewport
                    buffer_size: 10,  // Pre-render 10 items above/below viewport
                    fetch_threshold: 5,  // Start fetching when at least 5 items are missing
                    debounce_ms: Some(150),  // Debounce scrolling by 150ms
                    max_cached: Some(500),  // Keep max 500 messages in memory
                    on_fetch: Callback::new(fetch_messages),
                    render_item: Callback::new(render_item),
                }
            } else {
                div {
                    class: "no-mailbox-selected",
                    style: "display: flex; align-items: center; justify-content: center; height: 100%; color: #666;",
                    "Select a mailbox to view messages"
                }
            }
        }
    }
}

#[component]
pub fn MessageListWithRealData() -> Element {
    let ctx = use_context::<AppContext>();
    let selected_mailbox = ctx.selected_mailbox.read();
    let mut virtual_state = use_signal(|| None::<Signal<VirtualScrollState<Arc<Message>>>>);

    // This would connect to the real IMAP connector
    let fetch_messages = async move |range: Range<usize>| -> Vec<Arc<Message>> {
        let ctx = ctx.clone();
        let selected_mailbox = selected_mailbox.clone();

        if let Some(mailbox_id) = selected_mailbox.as_ref() {
            // Here you would call the IMAP connector's list_envelopes_range
            // For now, returning empty vec as placeholder
            vec![]
        } else {
            vec![]
        }
    };

    let render_item = move |args: (usize, &Arc<Message>)| -> Element {
        let (index, message) = args;
        let core_tx = use_coroutine_handle::<CoreEvent>();
        let ctx = use_context::<AppContext>();
        let selected_message = ctx.selected_message.read();
        let is_selected = selected_message.as_ref().map(|id| *id == message.id).unwrap_or(false);

        rsx! {
            div {
                class: "message-list-item",
                class: if is_selected { "selected" },
                style: "padding: 12px; border-bottom: 1px solid #e0e0e0; cursor: pointer; background-color: {if is_selected { \"#e3f2fd\" } else { \"white\" }};",

                onclick: move |_| {
                    let _ = core_tx.send(CoreEvent::SelectMessage(message.id.clone()));
                },

                div {
                    style: "display: flex; justify-content: space-between; align-items: start;",

                    div {
                        style: "flex: 1; min-width: 0;",

                        div {
                            style: "font-weight: 600; margin-bottom: 4px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
                            "{message.from}"
                        }

                        div {
                            style: "color: #333; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
                            "{message.subject}"
                        }
                    }
                }
            }
        }
    };

    // Listen for new messages and prepend them to the list
    use_effect(move || {
        // This would be triggered when a new message arrives
        // You'd call prepend_message(&mut virtual_state, new_message)
    });

    rsx! {
        div {
            id: "messagelist-real",
            style: "height: 100%; width: 100%;",

            if selected_mailbox.is_some() {
                VirtualScroll {
                    total_items: 1000,  // This would come from mailbox metadata
                    item_height: 72.0,
                    viewport_height: 600.0,
                    buffer_size: 15,
                    fetch_threshold: 5,
                    debounce_ms: Some(200),
                    max_cached: Some(1000),
                    on_fetch: Callback::new(fetch_messages),
                    render_item: Callback::new(render_item),
                }
            } else {
                div {
                    style: "display: flex; align-items: center; justify-content: center; height: 100%; color: #666;",
                    "Select a mailbox to view messages"
                }
            }
        }
    }
}