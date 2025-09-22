use std::collections::HashMap;

use dioxus::prelude::*;
use futures_util::StreamExt;
use mailiner_core::Folder;
use mailiner_core::connector::EmailConnector;
use mailiner_imap_connector::ImapConnector;

use crate::account::AccountId;
use crate::context::AppContext;
use crate::mailbox::{MailboxId, MailboxNode};
use crate::message::MessageId;
use crate::websocket_stream::WebSocketStream;

pub enum CoreEvent {
    SelectAccount(AccountId),
    SelectMailbox(MailboxId),
    SelectMessage(MessageId),
}

pub async fn core_loop(mut core_rx: UnboundedReceiver<CoreEvent>, mut ctx: AppContext) {
    let password = env!("IMAP_PASSWORD").to_string();
    let websocket_stream = WebSocketStream::new("ws://localhost:9400/proxy?token=testtoken&remote=dvratil.cz:993");
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
                ctx.selected_account.set(Some(account_id.clone()));
                let mboxes = connector.list_folders(&account_id).await.unwrap();
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

fn build_mailbox_tree(folders: Vec<Folder>) -> (Vec<MailboxId>, HashMap<MailboxId, MailboxNode>) {
    let mut root_ids = Vec::new();
    let mut mboxes = HashMap::<MailboxId, MailboxNode>::new();

    for folder in folders {
        let mailbox_id: MailboxId = folder.id.clone().into();
        mboxes.insert(mailbox_id.clone(), folder.clone().into());
        if let Some(parent_id) = folder.parent_id.clone() {
            mboxes.get_mut(&parent_id.into()).unwrap().children.push(mailbox_id);
        } else {
            root_ids.push(mailbox_id);
        }
    }

    (root_ids, mboxes)
}