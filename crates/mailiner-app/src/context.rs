use std::collections::HashMap;
use std::sync::Arc;

use dioxus::prelude::*;
use mailiner_core::models::LoadedMessage;

use crate::account::{Account, AccountId};
use crate::components::virtual_scroll::SparseList;
use crate::mailbox::{MailboxId, MailboxNode};
use crate::message::{Message, MessageId};

/// Viewer panel state driven by core_loop load pipeline.
#[derive(Clone, Debug)]
pub enum MessageViewState {
    Empty,
    Loading {
        message_id: MessageId,
    },
    Ready {
        message_id: MessageId,
        loaded: Arc<LoadedMessage>,
    },
    Error {
        message_id: MessageId,
        message: String,
    },
}

impl Default for MessageViewState {
    fn default() -> Self {
        Self::Empty
    }
}

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
    /// Body viewer state (load / format inputs).
    pub message_view: Signal<MessageViewState>,
}
