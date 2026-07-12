use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use dioxus::logger::tracing::{error, info};
use dioxus::prelude::*;
use futures_util::StreamExt;
use mailiner_core::connector::EmailConnector;
use mailiner_core::{Folder, FolderId, MessageId as CoreMessageId};
use mailiner_imap_connector::ImapConnector;

use crate::account::AccountId;
use crate::components::virtual_scroll::SparseList;
use crate::context::{AppContext, MessageViewState};
use crate::mailbox::{MailboxId, MailboxNode};
use crate::message::MessageId;
use crate::message_loader::load_message;
use crate::websocket_stream::WebSocketStream;

pub enum CoreEvent {
    SelectAccount(AccountId),
    SelectMailbox(MailboxId),
    /// Load envelopes for UI indices `[range.start, range.end)` into the sparse cache.
    FetchMessageRange {
        mailbox_id: MailboxId,
        range: Range<usize>,
    },
    SelectMessage(MessageId),
}

pub async fn core_loop(mut core_rx: UnboundedReceiver<CoreEvent>, mut ctx: AppContext) {
    let password = env!("IMAP_PASSWORD").to_string();
    let websocket_stream =
        WebSocketStream::new("ws://localhost:9400/proxy?token=testtoken&remote=dvratil.cz:993");
    let connector = ImapConnector::new(
        "dvratil.cz".to_string(),
        8081,
        "me@dvratil.cz".to_string(),
        password.clone(),
    );

    info!("Connecting to IMAP server...");
    connector
        .connect(websocket_stream)
        .await
        .or_else(|e| {
            error!("Failed to connect to IMAP server: {}", e);
            Err(e)
        })
        .expect("Failed to connect to IMAP server");
    info!("Connected to IMAP server");

    connector
        .authenticate(password.as_str())
        .await
        .expect("Failed to authenticate with IMAP server");
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
                ctx.messages.set(SparseList::new(0));
                ctx.messages_loading.set(false);
                ctx.selected_message.set(None);
                ctx.message_view.set(MessageViewState::Empty);
            }
            CoreEvent::SelectMailbox(mailbox_id) => {
                ctx.selected_message.set(None);
                ctx.message_view.set(MessageViewState::Empty);
                ctx.messages.set(SparseList::new(0));
                ctx.messages_loading.set(true);
                ctx.selected_mailbox.set(Some(mailbox_id.clone()));

                let folder_id = FolderId::new(mailbox_id.to_string());
                match connector.open_folder(&folder_id).await {
                    Ok(total) => {
                        info!(
                            "Opened mailbox {} with {} messages",
                            mailbox_id.to_string(),
                            total
                        );
                        // Only store the total count — envelopes load on demand as the
                        // virtual list discovers missing sparse ranges.
                        ctx.messages.set(SparseList::new(total));
                        ctx.messages_loading.set(false);
                        if let Some(node) = ctx.mailbox_nodes.write().get_mut(&mailbox_id) {
                            node.total_count = total;
                        }
                    }
                    Err(e) => {
                        error!("Failed to open mailbox: {}", e);
                        ctx.messages_loading.set(false);
                    }
                }
            }
            CoreEvent::FetchMessageRange { mailbox_id, range } => {
                // Ignore stale requests after the user switched mailboxes.
                if ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id) {
                    continue;
                }
                if range.start >= range.end {
                    continue;
                }

                // Skip indices we already have.
                let already = {
                    let messages = ctx.messages.read();
                    (range.start..range.end).all(|i| messages.has_item(i))
                };
                if already {
                    continue;
                }

                let folder_id = FolderId::new(mailbox_id.to_string());
                info!(
                    "Fetching messages {}..{} for {}",
                    range.start,
                    range.end,
                    mailbox_id.to_string()
                );
                match connector.list_envelopes_range(&folder_id, range.clone()).await {
                    Ok(envelopes) => {
                        // Re-check selection after the await.
                        if ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id) {
                            continue;
                        }
                        let batch: Vec<_> = envelopes
                            .into_iter()
                            .map(|e| Arc::new(e.into()))
                            .collect();
                        ctx.messages.write().insert_batch(range.start, batch);
                    }
                    Err(e) => {
                        error!(
                            "Failed to fetch message range {}..{}: {}",
                            range.start, range.end, e
                        );
                    }
                }
            }
            CoreEvent::SelectMessage(message_id) => {
                ctx.selected_message.set(Some(message_id.clone()));
                ctx.message_view.set(MessageViewState::Loading {
                    message_id: message_id.clone(),
                });

                let Some(mailbox_id) = ctx.selected_mailbox.read().clone() else {
                    ctx.message_view.set(MessageViewState::Error {
                        message_id: message_id.clone(),
                        message: "No mailbox selected".into(),
                    });
                    continue;
                };

                let folder_id = FolderId::new(mailbox_id.to_string());
                let core_id = CoreMessageId::new(message_id.to_string());
                info!(
                    "Loading message {} in {}",
                    message_id,
                    mailbox_id.to_string()
                );

                match load_message(&connector, &folder_id, &core_id).await {
                    Ok(loaded) => {
                        // Drop stale results if the user selected another message.
                        if ctx.selected_message.read().as_ref() != Some(&message_id) {
                            continue;
                        }
                        ctx.message_view.set(MessageViewState::Ready {
                            message_id,
                            loaded: Arc::new(loaded),
                        });
                    }
                    Err(e) => {
                        if ctx.selected_message.read().as_ref() != Some(&message_id) {
                            continue;
                        }
                        error!("Failed to load message {}: {}", message_id, e);
                        ctx.message_view.set(MessageViewState::Error {
                            message_id,
                            message: e.to_string(),
                        });
                    }
                }
            }
        }
    }
}

fn build_mailbox_tree(folders: Vec<Folder>) -> (Vec<MailboxId>, HashMap<MailboxId, MailboxNode>) {
    let mut root_ids = Vec::new();
    let mut mboxes = HashMap::<MailboxId, MailboxNode>::new();

    for folder in folders {
        let mailbox_id: MailboxId = folder.id.clone().into();
        mboxes
            .entry(mailbox_id.clone())
            .and_modify(|node| {
                node.parent = folder.parent_id.as_ref().map(|id| id.clone().into());
                node.name = folder.name.clone();
            })
            .or_insert(MailboxNode {
                id: mailbox_id.clone(),
                name: folder.name.clone(),
                parent: folder.parent_id.as_ref().map(|id| id.clone().into()),
                children: vec![],
                unread_count: 0,
                total_count: 0,
            });
        mboxes.insert(mailbox_id.clone(), folder.clone().into());
        if let Some(parent_id) = folder.parent_id.clone() {
            mboxes
                .entry(parent_id.clone().into())
                .or_insert(MailboxNode {
                    id: parent_id.clone().into(),
                    name: parent_id.to_string(),
                    parent: None,
                    children: vec![],
                    unread_count: 0,
                    total_count: 0,
                })
                .children
                .push(mailbox_id);
        } else {
            root_ids.push(mailbox_id);
        }
    }

    (root_ids, mboxes)
}
