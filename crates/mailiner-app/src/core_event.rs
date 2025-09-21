use dioxus::prelude::*;
use futures_util::StreamExt;
use mailiner_core::connector::EmailConnector;
use mailiner_imap_connector::ImapConnector;

use crate::account::AccountId;
use crate::context::AppContext;
use crate::mailbox::MailboxId;
use crate::message::MessageId;
use crate::websocket_stream::WebSocketStream;

pub enum CoreEvent {
    SelectAccount(AccountId),
    SelectMailbox(MailboxId),
    SelectMessage(MessageId),
}

pub async fn core_loop(mut core_rx: UnboundedReceiver<CoreEvent>, mut ctx: AppContext) {
    let password = env!("IMAP_PASSWORD").to_string();
    let websocket_stream = WebSocketStream::new("ws://localhost:8081/ws");
    let connector = ImapConnector::new(
        "localhost".to_string(),
        8081,
        "me@dvratil.cz".to_string(),
        password,
    );
    connector.connect(websocket_stream).await;

    while let Some(event) = core_rx.next().await {
        match event {
            CoreEvent::SelectAccount(account_id) => {
                ctx.selected_account.set(Some(account_id));
                let mboxes = connector.list().await;
                let (root_ids, mboxes) = build_mailbox_tree(mboxes);
                ctx.mailbox_nodes.set(mboxes);
                ctx.mailbox_roots.set(root_ids);
                ctx.selected_mailbox.set(None);
            }
            CoreEvent::SelectMailbox(mailbox_id) => {
                ctx.selected_mailbox.set(Some(mailbox_id));
            }
            CoreEvent::SelectMessage(message_id) => {
                ctx.selected_message.set(Some(message_id));
            }
        }
    }
}
