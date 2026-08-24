use std::collections::HashMap;
use std::sync::Arc;

use dioxus::prelude::*;
use mailiner_core::models::LoadedMessage;

use crate::account::{Account, AccountId};
use crate::components::virtual_scroll::SparseList;
use crate::connection::ConnectionState;
use crate::download::DownloadStatus;
use crate::outbox_store::OutboxListEntry;
use crate::send::{ComposeSession, SendState};
use crate::mailbox::{MailboxId, MailboxNode};
use crate::message::{Message, MessageId};
use crate::toast::{Toast, ToastAction};

/// KMail-style folder jumper: **J** goes to a mailbox, **M** moves the current message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailboxPickerMode {
    Jump,
    Move,
}

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
    /// UI accounts only (`id` / `name` / `email`) — **never** passwords or proxy tokens.
    pub accounts: Signal<HashMap<AccountId, Account>>,
    pub mailbox_nodes: Signal<HashMap<MailboxId, MailboxNode>>,
    pub mailbox_roots: Signal<Vec<MailboxId>>,
    /// Sparse cache of envelopes for the selected mailbox (order = [`Self::message_sort`]).
    pub messages: Signal<SparseList<Arc<Message>>>,
    /// True while SELECT / EXISTS is in flight for the selected mailbox.
    pub messages_loading: Signal<bool>,
    /// Active list sort (may fall back if the server lacks IMAP SORT).
    pub message_sort: Signal<mailiner_core::MessageSort>,
    /// Server advertised RFC 5256 `SORT` (Size / Sender).
    pub sort_supports_size_sender: Signal<bool>,

    pub selected_mailbox: Signal<Option<MailboxId>>,
    pub selected_account: Signal<Option<AccountId>>,
    pub selected_message: Signal<Option<MessageId>>,
    /// Body viewer state (load / format inputs).
    pub message_view: Signal<MessageViewState>,
    /// Per-section attachment download progress (section path → status).
    pub download_status: Signal<HashMap<String, DownloadStatus>>,
    /// Per-account connection lifecycle (no secrets).
    pub connection_states: Signal<HashMap<AccountId, ConnectionState>>,
    /// Composer send progress (at most one globally).
    pub send_status: Signal<Option<SendState>>,
    /// Form Test SMTP, keyed by ephemeral request id.
    pub smtp_test_status: Signal<HashMap<AccountId, SendState>>,
    /// Outbox list (no rfc822 / no passwords).
    pub outbox: Signal<Vec<OutboxListEntry>>,
    /// Ephemeral toast (e.g. “Sent”).
    pub toast: Signal<Option<Toast>>,
    /// Open compose session (`None` = closed).
    pub compose_draft: Signal<Option<ComposeSession>>,
    /// Folder jumper / move-to-folder dialog (`None` = closed).
    pub mailbox_picker: Signal<Option<MailboxPickerMode>>,
}

impl AppContext {
    pub fn show_toast(&self, action: ToastAction) {
        let mut toast = self.toast;
        let id = toast
            .peek()
            .as_ref()
            .map(|t| t.id.wrapping_add(1))
            .unwrap_or(1);
        toast.set(Some(Toast { id, action }));
    }
}
