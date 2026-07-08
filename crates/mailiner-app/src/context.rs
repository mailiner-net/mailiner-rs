use std::collections::HashMap;
use std::sync::Arc;

use dioxus::prelude::*;

use crate::account::{Account, AccountId};
use crate::components::virtual_scroll::SparseList;
use crate::mailbox::{MailboxId, MailboxNode};
use crate::message::{Message, MessageId};

#[derive(Clone)]
pub struct AppContext {
    pub accounts: Signal<HashMap<AccountId, Account>>,
    pub mailbox_nodes: Signal<HashMap<MailboxId, MailboxNode>>,
    pub mailbox_roots: Signal<Vec<MailboxId>>,
    /// Sparse cache of envelopes for the selected mailbox (newest-first indices).
    pub messages: Signal<SparseList<Arc<Message>>>,
    /// True while SELECT / EXISTS is in flight for the selected mailbox.
    pub messages_loading: Signal<bool>,

    pub selected_mailbox: Signal<Option<MailboxId>>,
    pub selected_account: Signal<Option<AccountId>>,
    pub selected_message: Signal<Option<MessageId>>,
}
