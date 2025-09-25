use std::collections::HashMap;
use std::sync::Arc;

use dioxus::prelude::*;
use dioxus::logger::tracing::{info, error};
use futures_util::StreamExt;
use mailiner_core::{Folder, FolderId};
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
        "dvratil.cz".to_string(),
        8081,
        "me@dvratil.cz".to_string(),
        password.clone(),
    );

    info!("Connecting to IMAP server...");
    connector.connect(websocket_stream).await.or_else(|e| {
        error!("Failed to connect to IMAP server: {}", e);
        Err(e)
    }).expect("Failed to connect to IMAP server");
    info!("Connected to IMAP server");

    connector.authenticate(password.as_str()).await.expect("Failed to authenticate with IMAP server");
    info!("Authenticated with IMAP server");
    

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
                ctx.messages.set(Vec::new());
                ctx.selected_mailbox.set(Some(mailbox_id.clone()));
                let folder_id = FolderId::new(mailbox_id.to_string());
                let messages = connector.list_envelopes(&folder_id).await.unwrap();
                ctx.messages.set(messages.into_iter().map(|e| Arc::new(e.into())).collect());
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
        mboxes.entry(mailbox_id.clone()).and_modify(|node| {
            node.parent = folder.parent_id.as_ref().map(|id| id.clone().into());
            node.name = folder.name.clone();
        }).or_insert(MailboxNode {
            id: mailbox_id.clone(),
            name: folder.name.clone(),
            parent: folder.parent_id.as_ref().map(|id| id.clone().into()),
            children: vec![],
            unread_count: 0,
            total_count: 0,
        });
        mboxes.insert(mailbox_id.clone(), folder.clone().into());
        if let Some(parent_id) = folder.parent_id.clone() {
            mboxes.entry(parent_id.clone().into()).or_insert(MailboxNode {
                id: parent_id.clone().into(),
                name: parent_id.to_string(),
                parent: None,
                children: vec![],
                unread_count: 0,
                total_count: 0,
            }).children.push(mailbox_id);
        } else {
            root_ids.push(mailbox_id);
        }
    }

    (root_ids, mboxes)
}