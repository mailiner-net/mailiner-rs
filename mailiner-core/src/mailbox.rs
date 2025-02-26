use std::{collections::HashMap, sync::Arc};

use dioxus::prelude::*;
use tokio::sync::Mutex;

use crate::imap_connector::{self, Error, ImapConnector};

#[derive(Debug, Clone)]
pub struct Mailbox {
    id: String,
    name: String,

    total: u32,
    unread: u32,

    // internal data?
    children: Signal<Vec<Signal<Mailbox>>>,
}

impl From<&imap_connector::Mailbox> for Mailbox {
    fn from(mailbox: &imap_connector::Mailbox) -> Self {
        Mailbox {
            id: "INBOX_TODO".to_string(),
            name: "INBOX_TODO".to_string(),
            total: 0,
            unread: 0,
            children: use_signal(|| Vec::new()),
        }
    }
}

struct MailboxModel {
    root: Signal<Mailbox>,
    mailbox_lookup: HashMap<String, Signal<Mailbox>>,
    connector: Arc<Mutex<ImapConnector>>,
    separator: char,
}

impl MailboxModel {
    fn new(connector: Arc<Mutex<ImapConnector>>) -> MailboxModel {
        MailboxModel {
            root: use_signal(|| Mailbox {
                id: "root".to_string(),
                name: "root".to_string(),
                total: 0,
                unread: 0,
                children: use_signal(|| Vec::new()),
            }),
            mailbox_lookup: HashMap::new(),
            connector,
            separator: '.',
        }
    }

    pub async fn sync(&mut self) -> Result<(), Error> {
        let fetched_mailboxes = self.connector.lock().await.list_mailboxes().await?;
        if fetched_mailboxes.len() > 0 {
            self.separator = fetched_mailboxes[0].delimiter.unwrap_or('.');
        }

        /*
        let existing_mailboxes = self.mailbox_lookup.keys();

        // Insert/update existing mailboxes
        fetched_mailboxes
            .iter()
            .for_each(|mailbox| self.upsert_mailbox(mailbox));

        // Delete mailboxes that no longer exist
        for existing_mailbox in existing_mailboxes {
            if fetched_mailboxes.iter().any(|fetched| match fetched.name {
                imap_types::mailbox::Mailbox::Inbox => "INBOX" == *existing_mailbox,
                imap_types::mailbox::Mailbox::Other(other) => match other.inner() {
                    imap_types::core::AString::Atom(atom) => atom.as_ref() == *existing_mailbox,
                    imap_types::core::AString::String(str) => {
                        String::from_utf8_lossy(str.as_ref()) == *existing_mailbox
                    }
                },
            }) {
                continue;
            }

            self.remove_mailbox(existing_mailbox);
        }
        */

        Ok(())
    }

    pub fn remove_mailbox(&mut self, name: &str) {
        // Remove from lookup
        if self.mailbox_lookup.remove(name).is_none() {
            tracing::warn!(
                "Trying to remove mailbox {}, but it does not exist in the model",
                name
            );
            return;
        }

        // Get parent mailbox
        let mut parent = match name.rfind(self.separator).unwrap_or(0) {
            0 => self.root.clone(),
            pos => {
                let parent_name = &name[0..pos];
                match self.mailbox_lookup.get(parent_name) {
                    Some(parent) => parent.clone(),
                    None => {
                        tracing::warn!(
                            "Parent mailbox {} (for {}) not found, storage is inconsistent!",
                            parent_name,
                            name
                        );
                        return;
                    }
                }
            }
        };

        let pos = match parent
            .read()
            .children
            .read()
            .iter()
            .position(|child| child.read().name == name)
        {
            Some(pos) => pos,
            None => {
                tracing::warn!(
                    "Mailbox {} not found in parent mailbox children, storage is inconsistent!",
                    name
                );
                return;
            }
        };
        parent.write().children.write().remove(pos);
    }

    fn upsert_mailbox(&mut self, _mailbox: &Mailbox) {
        /*
        self.mailbox_lookup
            .entry(mailbox.id)
            .and_modify(|current| {
                current.write().total = mailbox.total;
                current.write().unread = mailbox.unread;
            })
            .or_insert(use_signal(|| (*mailbox).clone().into()));
        */
    }
}
