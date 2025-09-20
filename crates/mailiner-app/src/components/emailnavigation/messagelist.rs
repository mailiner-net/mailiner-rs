use std::sync::Arc;

use dioxus::prelude::*;

use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::message::Message;

#[component]
pub fn MessageList() -> Element {
    let ctx = use_context::<AppContext>();
    let messages = ctx.messages.read();

    rsx! {
        div {
            id: "messagelist",

            for message in messages.iter() {
                MessageListItem {
                    message: Arc::clone(&message),
                }
            }
        }
    }
}

#[derive(PartialEq, Clone, Props)]
pub struct MessageListItemProps {
    pub message: Arc<Message>,
}

#[component]
pub fn MessageListItem(props: MessageListItemProps) -> Element {
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let ctx = use_context::<AppContext>();
    let selected_message = ctx.selected_message.read();
    rsx! {
        div {
            class: "message-list-item",
            // This is a bit of a hack, but if-let chains don't seem to work in Dioxus despite using edition 2024.
            class: if selected_message.as_ref().map(|id| *id == props.message.id).unwrap_or(false) {
                "selected"
            },

            onclick: move |_| {
                let _ = core_tx.send(CoreEvent::SelectMessage(props.message.id.clone()));
            },

            "{props.message.subject}"
        }
    }
}
