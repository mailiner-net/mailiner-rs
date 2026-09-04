use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use dioxus::prelude::*;
use mailiner_core::models::LoadedMessage;

use crate::account::{Account, AccountId};
use crate::components::virtual_scroll::SparseList;
use crate::connection::ConnectionState;
use crate::conversation::ConversationId;
use crate::download::{DownloadStatus, revoke_object_url};
use crate::layout::MobilePane;
use crate::mailbox::{MailboxId, MailboxNode};
use crate::message::{Message, MessageId};
use crate::message_loader::LoadedMessageCache;
use crate::outbox_store::OutboxListEntry;
use crate::selection::MessageSelection;
use crate::send::{ComposeSession, SendState};
use crate::toast::{Toast, ToastAction};
use crate::ui_prefs::{
    ComposePlacement, MailLayout, MessageListDensity, MessageListView, SavedSearch, SnoozedMessage,
};
use crate::unified_inbox::UnifiedInboxNote;

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
        account_id: AccountId,
        message_id: MessageId,
        loaded: Arc<LoadedMessage>,
    },
    Error {
        message_id: MessageId,
        message: String,
    },
}

/// Full RFC 5322 header block for the open “Show headers” dialog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MessageHeadersState {
    #[default]
    Closed,
    Loading {
        message_id: MessageId,
    },
    Ready {
        message_id: MessageId,
        text: String,
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

/// Full RFC 822 dump for the open “View source” dialog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MessageSourceState {
    #[default]
    Closed,
    Loading {
        account_id: AccountId,
        message_id: MessageId,
        request_id: u64,
    },
    Ready {
        account_id: AccountId,
        message_id: MessageId,
        request_id: u64,
        text: String,
    },
    Error {
        account_id: AccountId,
        message_id: MessageId,
        request_id: u64,
        message: String,
    },
}

#[derive(Clone)]
pub struct AppContext {
    /// UI accounts only (`id` / `name` / `email`) — **never** passwords or proxy tokens.
    pub accounts: Signal<HashMap<AccountId, Account>>,
    pub mailbox_nodes: Signal<HashMap<MailboxId, MailboxNode>>,
    pub mailbox_roots: Signal<Vec<MailboxId>>,
    /// Sparse cache of envelopes for the selected mailbox (order = [`Self::message_sort`]).
    pub messages: Signal<SparseList<Arc<Message>>>,
    /// Search-box draft (not applied until Enter / Search).
    pub list_text_filter: Signal<String>,
    /// Applied IMAP SEARCH query for the open folder (empty = whole folder).
    pub list_search_query: Signal<String>,
    /// Saved searches (virtual folders) for every account.
    pub saved_searches: Signal<Vec<SavedSearch>>,
    /// Selected virtual-folder id, if the list is showing a saved search.
    pub active_saved_search: Signal<Option<String>>,
    /// True while SELECT / EXISTS is in flight for the selected mailbox.
    pub messages_loading: Signal<bool>,
    /// Active list sort (may fall back if the server lacks IMAP SORT).
    pub message_sort: Signal<mailiner_core::MessageSort>,
    /// Virtualized message-list row density.
    pub message_list_density: Signal<MessageListDensity>,
    /// Flat list vs conversation grouping of loaded envelopes.
    pub message_list_view: Signal<MessageListView>,
    /// Expanded conversation ids in the open folder (session-only).
    pub expanded_conversations: Signal<HashSet<ConversationId>>,
    /// Quick list filters (Unread / Flagged via SEARCH; attachment chip is client-side).
    pub message_list_filter: Signal<mailiner_core::MessageListFilter>,
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
    /// Full header block dialog (`Closed` = hidden).
    pub message_headers: Signal<MessageHeadersState>,
    /// Full message source dialog (`Closed` = hidden).
    pub message_source: Signal<MessageSourceState>,
    /// Per-section attachment download progress (section path → status).
    pub download_status: Signal<HashMap<String, DownloadStatus>>,
    /// Object URLs from streamed attachments (section → blob URL) for preview reuse.
    pub attachment_blobs: Signal<HashMap<String, String>>,
    /// Open attachment preview (`None` = closed).
    pub attachment_preview: Signal<Option<AttachmentPreview>>,
    /// Stack of `message/rfc822` section paths being viewed (outer → inner).
    pub nested_rfc822: Signal<Vec<String>>,
    /// Per-account connection lifecycle (no secrets).
    pub connection_states: Signal<HashMap<AccountId, ConnectionState>>,
    /// Per-account composer send outcome (one SMTP op may run per account).
    pub send_status: Signal<HashMap<AccountId, SendState>>,
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
    /// Modal dialog vs in-flow bottom pane.
    pub compose_placement: Signal<ComposePlacement>,
    /// Stacked list-above-viewer vs classic three columns.
    pub mail_layout: Signal<MailLayout>,
    /// Full-screen pane on narrow viewports (CSS no-ops on desktop).
    pub mobile_pane: Signal<MobilePane>,
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
    /// Inbox desktop notifications (off until permission is granted).
    pub notify_inbox: Signal<bool>,
    /// Subscribe-folder manager dialog.
    pub folder_subscribe_open: Signal<bool>,
    /// Show unsubscribed folders in the tree and pickers.
    pub show_all_folders: Signal<bool>,
    /// Pinned IMAP UIDs for the open account+mailbox (local overlay, pin order).
    pub pinned_uids: Signal<Vec<String>>,
    /// Screen-reader live status (new mail). Empty string is silent.
    pub sr_status: Signal<String>,
    /// Snoozed rows for the open account+mailbox (local hide-until overlay).
    pub snoozed_messages: Signal<Vec<SnoozedMessage>>,
    /// Viewer snooze-preset menu (also opened by the **H** shortcut).
    pub snooze_picker_open: Signal<bool>,
    /// Inbox UNSEEN per account (selected tree + background STATUS cache).
    pub account_inbox_unread: Signal<HashMap<AccountId, u64>>,
    /// Muted notes for accounts served from cache or skipped in All inboxes.
    pub unified_inbox_notes: Signal<Vec<UnifiedInboxNote>>,
}

/// In-progress drag of list rows onto a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageDrag {
    pub message_ids: Vec<MessageId>,
    /// Folder the rows were dragged from (not whatever is selected at drop).
    pub source_mailbox: MailboxId,
    pub over: Option<MailboxId>,
}

/// In-place preview of a streamed attachment blob.
#[derive(Clone, Debug, PartialEq)]
pub struct AttachmentPreview {
    pub section: String,
    pub filename: String,
    pub content_type: String,
    pub object_url: String,
}

impl AppContext {
    pub fn selected_ids(&self) -> Vec<MessageId> {
        self.selection.read().ids_vec()
    }

    /// Show a preview, revoking the previously open dialog URL if it differs.
    pub fn open_attachment_preview(&self, preview: AttachmentPreview) {
        let mut preview_sig = self.attachment_preview;
        if let Some(prev) = preview_sig.peek().clone() {
            if prev.section != preview.section || prev.object_url != preview.object_url {
                let mut blobs = self.attachment_blobs;
                blobs.write().remove(&prev.section);
                revoke_object_url(&prev.object_url);
            }
        }
        preview_sig.set(Some(preview));
    }

    /// Open a nested `message/rfc822` in the viewer. Pushes onto the stack.
    pub fn open_nested_rfc822(&self, section: String) {
        if section.is_empty() {
            return;
        }
        let mut stack = self.nested_rfc822;
        let mut guard = stack.write();
        if guard.last() == Some(&section) {
            return;
        }
        guard.push(section);
    }

    /// Pop the innermost nested message.
    pub fn close_nested_rfc822(&self) {
        let mut stack = self.nested_rfc822;
        stack.write().pop();
    }

    pub fn clear_nested_rfc822(&self) {
        let mut stack = self.nested_rfc822;
        stack.write().clear();
    }

    /// Hide the preview dialog and revoke that attachment's object URL.
    pub fn close_attachment_preview(&self) {
        let mut preview_sig = self.attachment_preview;
        let Some(preview) = preview_sig.write().take() else {
            return;
        };
        let mut blobs = self.attachment_blobs;
        blobs.write().remove(&preview.section);
        revoke_object_url(&preview.object_url);
    }

    /// Drop download progress, close preview, and revoke held object URLs.
    pub fn clear_attachment_downloads(&self) {
        self.close_attachment_preview();
        let mut blobs = self.attachment_blobs;
        let urls: Vec<String> = blobs.write().drain().map(|(_, url)| url).collect();
        for url in urls {
            revoke_object_url(&url);
        }
        let mut download_status = self.download_status;
        download_status.set(HashMap::new());
    }

    /// Announce `message` to assistive tech via the mail-chrome live region.
    pub fn announce(&self, message: impl Into<String>) {
        let mut sr_status = self.sr_status;
        sr_status.set(message.into());
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
        self.reset_session_ui();
        self.mail_layout.set(MailLayout::default());
        self.saved_searches.set(Vec::new());
    }

    /// Drop in-memory mail/secrets after locking the vault. Prefs stay put.
    pub fn reset_after_lock(&mut self) {
        self.reset_session_ui();
    }

    fn reset_session_ui(&mut self) {
        self.accounts.write().clear();
        self.selected_account.set(None);
        self.mailbox_nodes.write().clear();
        self.mailbox_roots.write().clear();
        self.messages.set(SparseList::new(0));
        self.list_text_filter.set(String::new());
        self.list_search_query.set(String::new());
        self.active_saved_search.set(None);
        self.messages_loading.set(false);
        self.message_sort.set(mailiner_core::MessageSort::default());
        self.sort_supports_size_sender.set(false);
        self.selected_mailbox.set(None);
        self.selection.set(MessageSelection::default());
        self.message_view.set(MessageViewState::Empty);
        self.message_headers.set(MessageHeadersState::Closed);
        self.message_source.set(MessageSourceState::Closed);
        self.clear_nested_rfc822();
        self.download_status.write().clear();
        self.connection_states.write().clear();
        self.send_status.write().clear();
        self.smtp_test_status.write().clear();
        self.smtp_test_abandoned.write().clear();
        self.outbox.write().clear();
        self.toast.set(None);
        self.sr_status.set(String::new());
        self.compose_draft.set(None);
        self.mailbox_picker.set(None);
        self.pinned_uids.set(Vec::new());
        self.snoozed_messages.set(Vec::new());
        self.snooze_picker_open.set(false);
        self.expanded_conversations.write().clear();
        self.mobile_pane.set(MobilePane::default());
        self.account_inbox_unread.write().clear();
        self.unified_inbox_notes.set(Vec::new());
    }

    /// Show `pane` on narrow viewports. Desktop chrome ignores the class.
    pub fn set_mobile_pane(&self, pane: MobilePane) {
        let mut mobile_pane = self.mobile_pane;
        mobile_pane.set(pane);
    }

    /// Step back folders ← list ← viewer.
    pub fn mobile_back(&self) {
        let mut mobile_pane = self.mobile_pane;
        let next = mobile_pane.peek().back();
        mobile_pane.set(next);
    }

    /// Record composer send progress for one account without clobbering others.
    pub fn set_send_status(&mut self, account_id: AccountId, state: SendState) {
        match state {
            SendState::Idle => {
                self.send_status.write().remove(&account_id);
            }
            other => {
                self.send_status.write().insert(account_id, other);
            }
        }
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
