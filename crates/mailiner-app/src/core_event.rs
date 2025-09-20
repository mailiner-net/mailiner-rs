use dioxus::prelude::*;
use futures_util::StreamExt;


use crate::context::AppContext;
use crate::mailbox::MailboxId;
use crate::message::MessageId;

pub enum CoreEvent {
    SelectMailbox(MailboxId),
    SelectMessage(MessageId),
}

pub async fn core_loop(mut core_rx: UnboundedReceiver<CoreEvent>, mut ctx: AppContext) {
    while let Some(event) = core_rx.next().await {
        match event {
            CoreEvent::SelectMailbox(mailbox_id) => {
                ctx.selected_mailbox.set(Some(mailbox_id));
            },
            CoreEvent::SelectMessage(message_id) => {
                ctx.selected_message.set(Some(message_id));
            },
        }
    }
}


