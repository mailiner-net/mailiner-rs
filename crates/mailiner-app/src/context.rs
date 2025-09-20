use std::collections::HashMap;
use std::sync::Arc;

use dioxus::prelude::*;

use crate::account::{Account, AccountId};
use crate::mailbox::{MailboxId, MailboxNode};
use crate::message::{Message, MessageId};

#[derive(Clone)]
pub struct AppContext {
    pub accounts: Signal<HashMap<AccountId, Account>>,
    pub mailbox_nodes: Signal<HashMap<MailboxId, MailboxNode>>,
    pub mailbox_roots: Signal<Vec<MailboxId>>,
    pub messages: Signal<Vec<Arc<Message>>>,

    pub selected_account: Signal<Option<AccountId>>,
    pub selected_mailbox: Signal<Option<MailboxId>>,
    pub selected_message: Signal<Option<MessageId>>,
}
