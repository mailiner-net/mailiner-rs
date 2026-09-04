use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use dioxus::prelude::*;
use mailiner_core::models::LoadedMessage;

use crate::account::{Account, AccountId};
use crate::components::virtual_scroll::SparseList;
use crate::connection::ConnectionState;
use crate::download::DownloadStatus;
use crate::mailbox::{MailboxId, MailboxNode};
use crate::message::{Message, MessageId};
use crate::message_loader::LoadedMessageCache;
use crate::outbox_store::OutboxListEntry;
use crate::selection::MessageSelection;
use crate::send::{ComposeSession, SendState};
use crate::toast::{Toast, ToastAction};
use crate::ui_prefs::MessageListDensity;

/// KMail-style folder jumper: **J** goes to a mailbox, **M** moves, **Shift+C** copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailboxPickerMode {
    Jump,
    Move,
    Copy,
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
    /// Virtualized message-list row density.
    pub message_list_density: Signal<MessageListDensity>,
    /// Server advertised RFC 5256 `SORT` (Size / Sender; Date uses it when present).
    pub sort_supports_size_sender: Signal<bool>,
    /// STORAGE quota for the selected account (`None` if the server has no QUOTA).
    pub account_quota: Signal<Option<mailiner_core::MailboxQuota>>,

    pub selected_mailbox: Signal<Option<MailboxId>>,
    pub selected_account: Signal<Option<AccountId>>,
    /// Selected list rows. Single-select is a one-id set; the viewer shows `focus`.
    pub selection: Signal<MessageSelection>,
    /// Body viewer state (load / format inputs).
    pub message_view: Signal<MessageViewState>,
    /// Session-only decoded bodies (current + adjacent prefetch). Not a Signal:
    /// UI must not re-render on LRU inserts.
    pub message_bodies: Rc<RefCell<LoadedMessageCache>>,
    /// Per-section attachment download progress (section path → status).
    pub download_status: Signal<HashMap<String, DownloadStatus>>,
    /// Per-account connection lifecycle (no secrets).
    pub connection_states: Signal<HashMap<AccountId, ConnectionState>>,
    /// Composer send progress (at most one globally).
    pub send_status: Signal<Option<SendState>>,
    /// Form Test SMTP, keyed by ephemeral request id.
    pub smtp_test_status: Signal<HashMap<AccountId, SendState>>,
    /// Test SMTP request ids whose form unmounted before the result arrived.
    pub smtp_test_abandoned: Signal<HashSet<AccountId>>,
    /// Outbox list (no rfc822 / no passwords).
    pub outbox: Signal<Vec<OutboxListEntry>>,
    /// Ephemeral toast (e.g. “Sent”).
    pub toast: Signal<Option<Toast>>,
    /// Open compose session (`None` = closed).
    pub compose_draft: Signal<Option<ComposeSession>>,
    /// Folder jumper / move-or-copy dialog (`None` = closed).
    pub mailbox_picker: Signal<Option<MailboxPickerMode>>,
    /// Bumped after a successful `ClearLocalData` wipe.
    pub sign_out_epoch: Signal<u64>,
    /// True while a sign-out wipe is in flight (blocks accounts navigation).
    pub sign_out_pending: Signal<bool>,
    /// `sign_out_epoch` when the current wipe started.
    pub sign_out_started: Signal<u64>,
    /// Set when the wipe failed; the confirmation stays on screen.
    pub sign_out_error: Signal<Option<String>>,
    /// List → folder drag (`None` when idle).
    pub message_drag: Signal<Option<MessageDrag>>,
}

/// In-progress drag of list rows onto a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageDrag {
    pub message_ids: Vec<MessageId>,
    /// Folder the rows were dragged from (not whatever is selected at drop).
    pub source_mailbox: MailboxId,
    pub over: Option<MailboxId>,
}

impl AppContext {
    pub fn selected_ids(&self) -> Vec<MessageId> {
        self.selection.read().ids_vec()
    }

    pub fn show_toast(&self, action: ToastAction) {
        let mut toast = self.toast;
        let id = toast
            .peek()
            .as_ref()
            .map(|t| t.id.wrapping_add(1))
            .unwrap_or(1);
        toast.set(Some(Toast { id, action }));
    }

    /// Wipe session UI after a full local-data delete (onboarding is next).
    pub fn reset_after_sign_out(&mut self) {
        self.accounts.write().clear();
        self.selected_account.set(None);
        self.mailbox_nodes.write().clear();
        self.mailbox_roots.write().clear();
        self.messages.set(SparseList::new(0));
        self.messages_loading.set(false);
        self.message_sort.set(mailiner_core::MessageSort::default());
        self.sort_supports_size_sender.set(false);
        self.selected_mailbox.set(None);
        self.selection.set(MessageSelection::default());
        self.message_view.set(MessageViewState::Empty);
        self.download_status.write().clear();
        self.connection_states.write().clear();
        self.send_status.set(None);
        self.smtp_test_status.write().clear();
        self.smtp_test_abandoned.write().clear();
        self.outbox.write().clear();
        self.toast.set(None);
        self.compose_draft.set(None);
        self.mailbox_picker.set(None);
    }

    /// Record a Test SMTP UI state unless the form already unmounted.
    ///
    /// Returns `false` if `request_id` was abandoned; the caller must not
    /// leave a map entry or spawn further work for this id.
    pub fn set_smtp_test_status(&mut self, request_id: AccountId, state: SendState) -> bool {
        if self.smtp_test_abandoned.write().remove(&request_id) {
            return false;
        }
        self.smtp_test_status.write().insert(request_id, state);
        true
    }
}
