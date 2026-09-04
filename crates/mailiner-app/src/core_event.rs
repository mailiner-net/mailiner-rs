use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use dioxus::logger::tracing::{error, info, warn};
use dioxus::prelude::*;
use futures_channel::mpsc::{
    TryRecvError, UnboundedReceiver as SmtpUnboundedReceiver, UnboundedSender,
};
use futures_util::StreamExt;
use futures_util::future::{Either, select, select_all};
use gloo_timers::future::TimeoutFuture;
use mailiner_composer::{AttachmentData, caps, prepare_submit};
use mailiner_core::connector::EmailConnector;
use mailiner_core::models::TransferEncoding;
use mailiner_core::submit::{SendErrorKind, SubmitRequest};
use mailiner_core::{
    EnvelopeFlag, FolderCounts, FolderId, ImapKeyword, MailboxRole, MessageListFilter, MessageSort,
};
use mailiner_mime::decode_transfer_encoding;

use crate::account::AccountId;
use crate::account_config::AccountConfig;
use crate::account_store::AccountStore;
use crate::components::virtual_scroll::{
    SparseList, UnreadScan, adjacent_index, index_after_removal, next_unread_index,
    unread_scan_from, unread_scan_resume,
};
use crate::connection::{
    AccountConnectionManager, ConnectErrorKind, ConnectionState, EnsureConnectedMode,
    set_connection_state,
};
use crate::context::{
    AppContext, AttachmentPreview, MessageHeadersState, MessageSourceState, MessageViewState,
};
use crate::conversation::{
    ConversationRow, flatten_conversations, group_conversations, row_index_for_message,
};
use crate::download::{
    DownloadStatus, EML_DOWNLOAD_KEY, FinishedAttachment, MAIL_EXPORT_KEY, MAIL_IMPORT_KEY,
    MAX_DOWNLOAD_BYTES, StreamingBlobDownload, is_previewable_content_type, save_bytes_download,
};
use crate::layout::MobilePane;
use crate::mail_cache::{
    CachedFolderTree, CachedMessageList, HydratedAccount, MailCache, contiguous_envelope_prefix,
    hydrate_account, load_cached_loaded_message, persist_loaded_parts,
};
use crate::mail_file::{
    ExportMessageItem, MAX_EXPORT_MESSAGES, MailExportFormat, Rfc822Message, pack_export_named,
};
use crate::mailbox::{MailboxId, apply_live_folder_state, live_refresh_end};
use crate::message::{Message, MessageId, next_flag_value};
use crate::message_list_filter::message_matches_filter;
use crate::message_loader::{adjacent_neighbor_indices, load_message};
use crate::outbox_store::{
    MAX_OUTBOX_AUTO_ATTEMPTS, OutboxId, OutboxItem, OutboxItemState, OutboxListEntry, OutboxStore,
    pick_oldest_queued,
};
use crate::reconnect::reconnect_backoff_ms;
use crate::send::{
    ComposeSession, OutboxDisplay, SendPhase, SendState, identity_for_reply, identity_from_stored,
};
use crate::smtp_inflight::{InFlightSmtp, SmtpInflight};
use crate::smtp_session::{SEND_TIMEOUT_MS, SmtpOutcome, preflight, spawn_submit, spawn_test};
use crate::snippet::{SNIPPET_FETCH_OCTETS, clean_snippet};
use crate::snooze::SnoozePreset;
use crate::toast::{DismissCommit, MoveUndo, RemovedMessage, SnoozeUndo, ToastAction, UndoRequest};
use crate::ui_prefs::{MessageListView, SnoozedMessage};
use crate::unified_inbox::{
    AccountInboxPrefix, PrefixSource, UNIFIED_INBOX_PREFIX, batch_open_target, inbox_folder_id,
    inbox_unread_from_status, inbox_unread_from_tree, is_unified_mailbox, merge_inbox_prefixes,
    notes_from_prefixes, open_target, unified_mailbox_id,
};
use chrono::Utc;

thread_local! {
    static SMTP_TX: RefCell<Option<UnboundedSender<CoreEvent>>> = const { RefCell::new(None) };
}

fn bind_smtp_tx(tx: UnboundedSender<CoreEvent>) {
    SMTP_TX.with(|cell| *cell.borrow_mut() = Some(tx));
}

fn queue_core_event(event: CoreEvent) {
    SMTP_TX.with(|cell| {
        if let Some(tx) = cell.borrow().as_ref() {
            let _ = tx.unbounded_send(event);
        }
    });
}

pub enum CoreEvent {
    // —— mail ops ——
    SelectMailbox(MailboxId),
    /// Open a mailbox and select the newest message (keyboard jump).
    JumpToMailbox(MailboxId),
    /// Rebuild the current folder's list in a new order.
    SetMessageSort(MessageSort),
    /// Flip the given list-filter flags (processed in order so rapid clicks compose).
    ToggleMessageListFilter {
        unread: bool,
        flagged: bool,
        has_attachment: bool,
    },
    /// Run IMAP SEARCH for the open folder (`query` empty = clear).
    ApplyMailboxSearch {
        query: String,
    },
    /// Persist the current folder's search as a virtual folder.
    SaveMailboxSearch {
        name: String,
        query: String,
    },
    /// Open a saved search (select its folder and re-run the query).
    OpenSavedSearch {
        id: String,
    },
    RenameSavedSearch {
        id: String,
        name: String,
    },
    DeleteSavedSearch {
        id: String,
    },
    /// Load envelopes for UI indices `[range.start, range.end)` into the sparse cache.
    FetchMessageRange {
        mailbox_id: MailboxId,
        range: Range<usize>,
    },
    SelectMessage(MessageId),
    /// Click in the message list (plain / Ctrl / Shift).
    SelectListClick {
        message_id: MessageId,
        index: usize,
        extend: bool,
        toggle: bool,
    },
    /// Move the list focus by `delta` rows (↓ = +1, ↑ = −1). `extend` is Shift+arrow.
    SelectAdjacent {
        delta: i32,
        extend: bool,
    },
    /// Select every currently loaded row in the open folder.
    SelectAllKnown,
    /// Select loaded unread rows in the open folder.
    SelectUnreadKnown,
    /// Invert membership over currently loaded rows.
    InvertSelection,
    /// Jump to the next (`delta > 0`) or previous unread row, fetching if needed.
    SelectAdjacentUnread {
        delta: i32,
    },
    MarkRead {
        mailbox_id: MailboxId,
        message_ids: Vec<MessageId>,
        is_read: bool,
    },
    /// Toggle `\Starred` (custom) on the given messages.
    ToggleStar {
        account_id: AccountId,
        mailbox_id: MailboxId,
        message_ids: Vec<MessageId>,
    },
    /// Toggle `\Flagged` on the given messages.
    ToggleFlag {
        account_id: AccountId,
        mailbox_id: MailboxId,
        message_ids: Vec<MessageId>,
    },
    /// Toggle the local pin overlay for the given messages (keep at top).
    TogglePin {
        account_id: AccountId,
        mailbox_id: MailboxId,
        message_ids: Vec<MessageId>,
    },
    /// Hide the given messages until a preset time (local overlay).
    SnoozeMessages {
        account_id: AccountId,
        mailbox_id: MailboxId,
        message_ids: Vec<MessageId>,
        preset: SnoozePreset,
    },
    /// Drop expired snoozes, notify, and refresh the open folder if needed.
    SweepSnooze,
    /// Toggle a built-in custom IMAP keyword on the given messages.
    ToggleKeyword {
        account_id: AccountId,
        mailbox_id: MailboxId,
        message_ids: Vec<MessageId>,
        keyword: ImapKeyword,
    },
    MoveMessages {
        mailbox_id: MailboxId,
        message_ids: Vec<MessageId>,
        dest_mailbox_id: MailboxId,
    },
    /// Copy to another folder; source messages stay.
    CopyMessages {
        account_id: AccountId,
        mailbox_id: MailboxId,
        message_ids: Vec<MessageId>,
        dest_mailbox_id: MailboxId,
    },
    /// Move to the Archive special-use folder when one exists.
    ArchiveMessages {
        account_id: AccountId,
        mailbox_id: MailboxId,
        message_ids: Vec<MessageId>,
    },
    /// Move to the Trash special-use folder, or permanently delete when already there.
    MoveToTrash {
        mailbox_id: MailboxId,
        message_ids: Vec<MessageId>,
    },
    /// Move to the Junk special-use folder when one exists.
    MoveToJunk {
        account_id: AccountId,
        mailbox_id: MailboxId,
        message_ids: Vec<MessageId>,
    },
    /// Permanently delete (IMAP on toast dismiss unless undone).
    DeleteMessages {
        mailbox_id: MailboxId,
        message_ids: Vec<MessageId>,
    },
    /// Permanently delete every message in the Trash special-use folder.
    EmptyTrash {
        account_id: AccountId,
        mailbox_id: MailboxId,
    },
    /// CREATE a mailbox under `parent_id` (or at the root).
    CreateFolder {
        account_id: AccountId,
        parent_id: Option<MailboxId>,
        name: String,
    },
    /// RENAME `mailbox_id` so its last path segment is `new_name`.
    RenameFolder {
        account_id: AccountId,
        mailbox_id: MailboxId,
        new_name: String,
    },
    /// DELETE `mailbox_id` (and its descendants, deepest first). Inbox is refused.
    DeleteFolder {
        account_id: AccountId,
        mailbox_id: MailboxId,
    },
    /// IMAP SUBSCRIBE / UNSUBSCRIBE for one folder.
    SetFolderSubscribed {
        account_id: AccountId,
        mailbox_id: MailboxId,
        subscribed: bool,
    },
    /// Inverse of a toasted action (central undo).
    Undo(UndoRequest),
    /// Work held until a toast dismissed without Undo (permanent delete).
    CommitDismissed(DismissCommit),
    /// FETCH `BODY.PEEK[HEADER]` and open the headers dialog.
    FetchMessageHeaders {
        mailbox_id: MailboxId,
        message_id: MessageId,
    },
    /// FETCH `BODY.PEEK[]` and open the source dialog.
    FetchMessageSource {
        account_id: AccountId,
        mailbox_id: MailboxId,
        message_id: MessageId,
        request_id: u64,
    },
    /// Stream a single attachment part and save to disk (browser download).
    DownloadAttachment {
        account_id: AccountId,
        mailbox_id: MailboxId,
        message_id: MessageId,
        section: String,
        filename: String,
        content_type: String,
        encoding: TransferEncoding,
        size_hint: Option<u64>,
    },
    /// FETCH the raw RFC 822 message and save it as `.eml`.
    SaveMessageEml {
        account_id: AccountId,
        mailbox_id: MailboxId,
        message_id: MessageId,
        filename: String,
        size_hint: Option<u64>,
    },
    /// FETCH selected messages and download them as a zip of `.eml` files or one mbox.
    ExportMessages {
        account_id: AccountId,
        mailbox_id: MailboxId,
        items: Vec<crate::mail_file::ExportMessageItem>,
        format: crate::mail_file::MailExportFormat,
        folder_label: String,
    },
    /// IMAP APPEND parsed `.eml` / mbox messages into the current folder.
    ImportMessages {
        account_id: AccountId,
        mailbox_id: MailboxId,
        messages: Vec<crate::mail_file::Rfc822Message>,
    },
    /// Fetch pending forwarded file bytes into the open compose draft.
    FetchComposeAttachments {
        draft_id: String,
        account_id: AccountId,
    },
    /// Stream a previewable attachment and open an inline preview dialog.
    PreviewAttachment {
        account_id: AccountId,
        mailbox_id: MailboxId,
        message_id: MessageId,
        section: String,
        filename: String,
        content_type: String,
        encoding: TransferEncoding,
        size_hint: Option<u64>,
    },

    /// Select account for UI + ensure connector + list folders.
    /// Loads config from store/cache by id.
    SelectAccount(AccountId),

    /// After store open: seed manager awareness; connect active if present.
    Bootstrap {
        active: Option<AccountId>,
    },

    /// Unsaved form: connect, report state under ephemeral key, disconnect, do not persist.
    TestConnection {
        /// Ephemeral id for ConnectionState map / UI correlation (not stored).
        request_id: AccountId,
        config: AccountConfig,
    },

    /// First-run / add-account commit (connect-before-persist).
    /// On Ready only: store.upsert, set_active_id, keep connector, list folders.
    CommitNewAccount {
        config: AccountConfig,
    },

    /// Account already in store (cold start, switch). Load via store, connect, list folders.
    ConnectExisting {
        account_id: AccountId,
    },

    Reconnect {
        account_id: AccountId,
    },

    /// WebSocket closed or IMAP I/O failed on a live session.
    SessionDropped {
        account_id: AccountId,
    },

    /// Timer-fired auto-reconnect (generation must still match).
    AutoReconnect {
        account_id: AccountId,
        generation: u64,
    },

    DisconnectAccount(AccountId),

    /// UI mutated store (edit/delete). Manager drops deleted connectors; does not auto-connect.
    AccountsChanged,

    /// Sign-out: drop every connector and cached config (secrets).
    ClearLocalData,

    /// Forget the vault session key and drop in-memory secrets / connectors.
    LockSecrets,

    SendMessage {
        account_id: AccountId,
        request: SubmitRequest,
        display: OutboxDisplay,
        /// Compose session id; only this draft is closed after persist.
        draft_id: String,
        /// Formatted Bcc for the Sent copy. `None` when there is no Bcc.
        bcc_header: Option<String>,
        /// Source message to mark `\Answered` after a successful Reply / Reply All.
        reply_source: Option<MessageId>,
        /// IMAP Drafts message to delete after the send is queued.
        imap_draft: Option<MessageId>,
    },
    TestSmtpConnection {
        request_id: AccountId,
        config: AccountConfig,
    },
    SmtpFinished {
        generation: u64,
        outcome: SmtpOutcome,
    },
    DrainOutbox,
    RetryOutboxItem {
        id: crate::outbox_store::OutboxId,
    },
    DeleteOutboxItem {
        id: crate::outbox_store::OutboxId,
    },
    /// Best-effort IMAP APPEND to Sent after SMTP success. Failure does not unsend.
    ArchiveSent {
        account_id: AccountId,
        rfc822: Vec<u8>,
    },
    /// Best-effort IMAP STORE `\Answered` on the source after a successful reply.
    MarkAnswered {
        account_id: AccountId,
        message_id: MessageId,
    },
    /// Best-effort IMAP APPEND to Drafts (`\Draft`). Replaces `replace` when set.
    SaveImapDraft {
        account_id: AccountId,
        draft_id: String,
        rfc822: Vec<u8>,
        replace: Option<MessageId>,
    },
    /// Best-effort IMAP delete of a Drafts message (discard / send / empty close).
    DeleteImapDraft {
        account_id: AccountId,
        message_id: MessageId,
    },
    /// Selected mailbox changed on the server (IDLE / NOOP).
    MailboxActivity {
        account_id: AccountId,
        mailbox_id: MailboxId,
    },
    /// Periodic `STATUS` of non-selected connected accounts (unread badges).
    BackgroundStatusPoll,
    /// Virtual All-inboxes view: merge Inbox prefixes from every account.
    SelectUnifiedInbox,
}

/// Background BODY.PEEK of list neighbors after the focused message is Ready.
struct PrefetchJob {
    around: MessageId,
    mailbox_id: MailboxId,
    account_id: AccountId,
    remaining: Vec<MessageId>,
    /// Adjacent ids when the job was queued. Relocate/move/delete changes this.
    neighbors: Vec<MessageId>,
}

/// Cold-start prelude for [`core_loop`]: skip connect, or run bootstrap with an active id.
///
/// Prefer this over `Option<Option<AccountId>>` so call sites are not ambiguous.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitialBootstrap {
    /// Store open/list failed — do not connect; event loop stays idle until a later event.
    Skip,
    /// Run [`CoreEvent::Bootstrap`] once with the resolved active account (or `None`).
    Run { active: Option<AccountId> },
}

/// RFC 2177: re-issue IDLE before typical 30-minute server inactivity timeouts.
const IDLE_REISSUE_MS: u32 = 25 * 60 * 1000;
/// Poll interval when the server does not advertise IDLE.
const NOOP_INTERVAL_MS: u32 = 30_000;

/// Application core task: handles mail ops and account connection lifecycle.
///
/// **Serial event processing (v1):** each handler is fully awaited before the next
/// event is taken from the channel. Connect attempts therefore do not run concurrently;
/// generation debounce in the manager is defensive (stale-result guard if connect is
/// later made concurrent) rather than a mid-flight cancel of an in-progress connect.
///
/// `initial_bootstrap`: App opens the store and passes [`InitialBootstrap::Run`] with
/// the resolved active id, or [`InitialBootstrap::Skip`] on store failure.
pub async fn core_loop(
    mut core_rx: UnboundedReceiver<CoreEvent>,
    mut smtp_rx: SmtpUnboundedReceiver<CoreEvent>,
    smtp_tx: UnboundedSender<CoreEvent>,
    mut ctx: AppContext,
    store: Rc<dyn AccountStore>,
    outbox: Rc<dyn OutboxStore>,
    cache: Rc<dyn MailCache>,
    initial_bootstrap: InitialBootstrap,
) {
    let mut manager = AccountConnectionManager::new(store, cache);
    let mut inflight = SmtpInflight::new();
    let mut pending_event: Option<CoreEvent> = None;
    let mut pending_prefetch: Option<PrefetchJob> = None;
    bind_smtp_tx(smtp_tx.clone());

    if let InitialBootstrap::Run { active } = initial_bootstrap {
        handle_bootstrap(&mut manager, &mut ctx, active).await;
        recover_outbox(outbox.as_ref(), &mut ctx).await;
        drain_outbox(
            &mut manager,
            &mut ctx,
            outbox.as_ref(),
            &smtp_tx,
            &mut inflight,
        )
        .await;
        queue_adjacent_prefetch(&ctx, &mut pending_prefetch);
    }

    schedule_background_status(smtp_tx.clone());

    loop {
        if pending_event.is_none() {
            match core_rx.try_recv() {
                Ok(ev) => pending_event = Some(ev),
                Err(TryRecvError::Closed) => break,
                Err(TryRecvError::Empty) => match smtp_rx.try_recv() {
                    Ok(ev) => pending_event = Some(ev),
                    Err(TryRecvError::Closed | TryRecvError::Empty) => {}
                },
            }
        }

        if pending_event.is_none() && run_one_prefetch(&manager, &ctx, &mut pending_prefetch).await
        {
            continue;
        }

        let watches = manager.death_watches();
        let event = if let Some(ev) = pending_event.take() {
            ev
        } else {
            match recv_next_or_watch(&mut core_rx, &mut smtp_rx, watches, &manager, &ctx).await {
                RecvOutcome::Event { event, follow_up } => {
                    if let Some(extra) = follow_up {
                        pending_event = Some(extra);
                    }
                    event
                }
                RecvOutcome::Continue => continue,
                RecvOutcome::Closed => break,
            }
        };
        match event {
            CoreEvent::Bootstrap { active } => {
                handle_bootstrap(&mut manager, &mut ctx, active).await;
            }
            CoreEvent::SelectAccount(account_id) => {
                handle_select_account(&mut manager, &mut ctx, account_id).await;
            }
            CoreEvent::ConnectExisting { account_id } => {
                handle_select_account(&mut manager, &mut ctx, account_id).await;
            }
            CoreEvent::Reconnect { account_id } => {
                handle_reconnect(&mut manager, &mut ctx, &smtp_tx, account_id).await;
            }
            CoreEvent::SessionDropped { account_id } => {
                handle_session_dropped(&mut manager, &mut ctx, &smtp_tx, account_id).await;
            }
            CoreEvent::AutoReconnect {
                account_id,
                generation,
            } => {
                handle_auto_reconnect(&mut manager, &mut ctx, &smtp_tx, account_id, generation)
                    .await;
            }
            CoreEvent::DisconnectAccount(account_id) => {
                inflight.cancel_for_account(&account_id);
                manager.disconnect_account(&account_id, &mut ctx).await;
                if ctx.selected_account.read().as_ref() == Some(&account_id) {
                    clear_mailbox_ui(&mut ctx);
                    ctx.selected_account.set(None);
                }
            }
            CoreEvent::TestConnection { request_id, config } => {
                let _ = manager
                    .test_connection(&request_id, &config, &mut ctx)
                    .await;
            }
            CoreEvent::CommitNewAccount { config } => {
                handle_commit_new_account(&mut manager, &mut ctx, config).await;
            }
            CoreEvent::AccountsChanged => {
                handle_accounts_changed(&mut manager, &mut ctx).await;
                purge_missing_accounts(&manager, outbox.as_ref(), &mut ctx, &mut inflight).await;
            }
            CoreEvent::ClearLocalData => {
                // Drop every SMTP slot now. A later SmtpFinished must not persist
                // (would recreate outbox/cache keys after the wipe).
                inflight.take_all();
                handle_clear_local_data(&mut manager, &mut ctx, outbox.as_ref()).await;
            }
            CoreEvent::LockSecrets => {
                inflight.take_all();
                handle_lock_secrets(&mut manager, &mut ctx).await;
            }
            CoreEvent::SelectMailbox(mailbox_id) => {
                handle_select_mailbox(&manager, &mut ctx, mailbox_id, true).await;
            }
            CoreEvent::JumpToMailbox(mailbox_id) => {
                handle_select_mailbox(&manager, &mut ctx, mailbox_id, true).await;
            }
            CoreEvent::SetMessageSort(sort) => {
                handle_set_message_sort(&manager, &mut ctx, sort).await;
            }
            CoreEvent::ToggleMessageListFilter {
                unread,
                flagged,
                has_attachment,
            } => {
                handle_toggle_message_list_filter(
                    &manager,
                    &mut ctx,
                    unread,
                    flagged,
                    has_attachment,
                )
                .await;
            }
            CoreEvent::ApplyMailboxSearch { query } => {
                handle_apply_mailbox_search(&manager, &mut ctx, query).await;
            }
            CoreEvent::SaveMailboxSearch { name, query } => {
                handle_save_mailbox_search(&manager, &mut ctx, name, query).await;
            }
            CoreEvent::OpenSavedSearch { id } => {
                handle_open_saved_search(&manager, &mut ctx, id).await;
            }
            CoreEvent::RenameSavedSearch { id, name } => {
                handle_rename_saved_search(&mut ctx, id, name);
            }
            CoreEvent::DeleteSavedSearch { id } => {
                handle_delete_saved_search(&mut ctx, id);
            }
            CoreEvent::FetchMessageRange { mailbox_id, range } => {
                handle_fetch_message_range(&manager, &mut ctx, mailbox_id, range).await;
            }
            CoreEvent::SelectMessage(message_id) => {
                ctx.set_mobile_pane(MobilePane::after_select_message());
                handle_select_message(&manager, &mut ctx, message_id, true, true).await;
            }
            CoreEvent::SelectListClick {
                message_id,
                index,
                extend,
                toggle,
            } => {
                ctx.set_mobile_pane(MobilePane::after_select_message());
                handle_select_list_click(&manager, &mut ctx, message_id, index, extend, toggle)
                    .await;
            }
            CoreEvent::SelectAdjacent { delta, extend } => {
                ctx.set_mobile_pane(MobilePane::after_select_message());
                handle_select_adjacent(&manager, &mut ctx, delta, extend).await;
            }
            CoreEvent::SelectAllKnown => {
                handle_select_known(&manager, &mut ctx, KnownSelect::All).await;
            }
            CoreEvent::SelectUnreadKnown => {
                handle_select_known(&manager, &mut ctx, KnownSelect::Unread).await;
            }
            CoreEvent::InvertSelection => {
                handle_select_known(&manager, &mut ctx, KnownSelect::Invert).await;
            }
            CoreEvent::SelectAdjacentUnread { delta } => {
                ctx.set_mobile_pane(MobilePane::after_select_message());
                handle_select_adjacent_unread(&manager, &mut ctx, delta).await;
            }
            CoreEvent::MarkRead {
                mailbox_id,
                message_ids,
                is_read,
            } => {
                handle_mark_read(&manager, &mut ctx, mailbox_id, message_ids, is_read).await;
            }
            CoreEvent::ToggleStar {
                account_id,
                mailbox_id,
                message_ids,
            } => {
                handle_toggle_flag(
                    &manager,
                    &mut ctx,
                    account_id,
                    mailbox_id,
                    message_ids,
                    EnvelopeFlag::Starred,
                )
                .await;
            }
            CoreEvent::ToggleFlag {
                account_id,
                mailbox_id,
                message_ids,
            } => {
                handle_toggle_flag(
                    &manager,
                    &mut ctx,
                    account_id,
                    mailbox_id,
                    message_ids,
                    EnvelopeFlag::Flagged,
                )
                .await;
            }
            CoreEvent::TogglePin {
                account_id,
                mailbox_id,
                message_ids,
            } => {
                handle_toggle_pin(&mut ctx, account_id, mailbox_id, message_ids);
            }
            CoreEvent::SnoozeMessages {
                account_id,
                mailbox_id,
                message_ids,
                preset,
            } => {
                handle_snooze_messages(
                    &manager,
                    &mut ctx,
                    account_id,
                    mailbox_id,
                    message_ids,
                    preset,
                )
                .await;
            }
            CoreEvent::SweepSnooze => {
                handle_sweep_snooze(&manager, &mut ctx).await;
            }
            CoreEvent::ToggleKeyword {
                account_id,
                mailbox_id,
                message_ids,
                keyword,
            } => {
                handle_toggle_flag(
                    &manager,
                    &mut ctx,
                    account_id,
                    mailbox_id,
                    message_ids,
                    EnvelopeFlag::Keyword(keyword),
                )
                .await;
            }
            CoreEvent::MoveMessages {
                mailbox_id,
                message_ids,
                dest_mailbox_id,
            } => {
                let account_id = if is_unified_mailbox(&mailbox_id) {
                    match batch_target_for(&ctx, &message_ids) {
                        Some(target) => target.account_id,
                        None => {
                            ctx.show_toast(ToastAction::info("Select messages from one account"));
                            continue;
                        }
                    }
                } else {
                    let Some(account_id) = ctx.selected_account.read().clone() else {
                        ctx.show_toast(ToastAction::error("No account selected"));
                        continue;
                    };
                    account_id
                };
                let mailbox_id = if is_unified_mailbox(&mailbox_id) {
                    match batch_target_for(&ctx, &message_ids) {
                        Some(target) => target.mailbox_id,
                        None => mailbox_id,
                    }
                } else {
                    mailbox_id
                };
                handle_move_messages(
                    &manager,
                    &mut ctx,
                    account_id,
                    mailbox_id,
                    message_ids,
                    dest_mailbox_id,
                )
                .await;
            }
            CoreEvent::CopyMessages {
                account_id,
                mailbox_id,
                message_ids,
                dest_mailbox_id,
            } => {
                handle_copy_messages(
                    &manager,
                    &mut ctx,
                    account_id,
                    mailbox_id,
                    message_ids,
                    dest_mailbox_id,
                )
                .await;
            }
            CoreEvent::ArchiveMessages {
                account_id,
                mailbox_id,
                message_ids,
            } => {
                handle_archive_messages(&manager, &mut ctx, account_id, mailbox_id, message_ids)
                    .await;
            }
            CoreEvent::MoveToTrash {
                mailbox_id,
                message_ids,
            } => {
                handle_move_to_trash(&manager, &mut ctx, mailbox_id, message_ids).await;
            }
            CoreEvent::MoveToJunk {
                account_id,
                mailbox_id,
                message_ids,
            } => {
                handle_move_to_junk(&manager, &mut ctx, account_id, mailbox_id, message_ids).await;
            }
            CoreEvent::DeleteMessages {
                mailbox_id,
                message_ids,
            } => {
                handle_delete_messages(&manager, &mut ctx, mailbox_id, message_ids).await;
            }
            CoreEvent::EmptyTrash {
                account_id,
                mailbox_id,
            } => {
                handle_empty_trash(&manager, &mut ctx, account_id, mailbox_id).await;
            }
            CoreEvent::CreateFolder {
                account_id,
                parent_id,
                name,
            } => {
                handle_create_folder(&manager, &mut ctx, account_id, parent_id, name).await;
            }
            CoreEvent::RenameFolder {
                account_id,
                mailbox_id,
                new_name,
            } => {
                handle_rename_folder(&manager, &mut ctx, account_id, mailbox_id, new_name).await;
            }
            CoreEvent::DeleteFolder {
                account_id,
                mailbox_id,
            } => {
                handle_delete_folder(&manager, &mut ctx, account_id, mailbox_id).await;
            }
            CoreEvent::SetFolderSubscribed {
                account_id,
                mailbox_id,
                subscribed,
            } => {
                handle_set_folder_subscribed(
                    &manager, &mut ctx, account_id, mailbox_id, subscribed,
                )
                .await;
            }
            CoreEvent::Undo(undo) => {
                handle_undo(&manager, &mut ctx, undo).await;
            }
            CoreEvent::CommitDismissed(commit) => {
                handle_commit_dismissed(&manager, &mut ctx, commit).await;
            }
            CoreEvent::FetchMessageHeaders {
                mailbox_id,
                message_id,
            } => {
                handle_fetch_message_headers(&manager, &mut ctx, mailbox_id, message_id).await;
            }
            CoreEvent::FetchMessageSource {
                account_id,
                mailbox_id,
                message_id,
                request_id,
            } => {
                handle_fetch_message_source(
                    &manager, &mut ctx, account_id, mailbox_id, message_id, request_id,
                )
                .await;
            }
            CoreEvent::DownloadAttachment {
                account_id,
                mailbox_id,
                message_id,
                section,
                filename,
                content_type,
                encoding,
                size_hint,
            } => {
                handle_download_attachment(
                    &manager,
                    &mut ctx,
                    account_id,
                    mailbox_id,
                    message_id,
                    section,
                    filename,
                    content_type,
                    encoding,
                    size_hint,
                )
                .await;
            }
            CoreEvent::SaveMessageEml {
                account_id,
                mailbox_id,
                message_id,
                filename,
                size_hint,
            } => {
                handle_save_message_eml(
                    &manager, &mut ctx, account_id, mailbox_id, message_id, filename, size_hint,
                )
                .await;
            }
            CoreEvent::ExportMessages {
                account_id,
                mailbox_id,
                items,
                format,
                folder_label,
            } => {
                handle_export_messages(
                    &manager,
                    &mut ctx,
                    account_id,
                    mailbox_id,
                    items,
                    format,
                    folder_label,
                )
                .await;
            }
            CoreEvent::ImportMessages {
                account_id,
                mailbox_id,
                messages,
            } => {
                handle_import_messages(&manager, &mut ctx, account_id, mailbox_id, messages).await;
            }
            CoreEvent::FetchComposeAttachments {
                draft_id,
                account_id,
            } => {
                handle_fetch_compose_attachments(&manager, &mut ctx, draft_id, account_id).await;
            }
            CoreEvent::PreviewAttachment {
                account_id,
                mailbox_id,
                message_id,
                section,
                filename,
                content_type,
                encoding,
                size_hint,
            } => {
                handle_preview_attachment(
                    &manager,
                    &mut ctx,
                    account_id,
                    mailbox_id,
                    message_id,
                    section,
                    filename,
                    content_type,
                    encoding,
                    size_hint,
                )
                .await;
            }
            CoreEvent::SendMessage {
                account_id,
                request,
                display,
                draft_id,
                bcc_header,
                reply_source,
                imap_draft,
            } => {
                handle_send_message(
                    &mut manager,
                    &mut ctx,
                    outbox.as_ref(),
                    &smtp_tx,
                    &mut inflight,
                    account_id,
                    request,
                    display,
                    draft_id,
                    bcc_header,
                    reply_source,
                    imap_draft,
                )
                .await;
            }
            CoreEvent::TestSmtpConnection { request_id, config } => {
                handle_test_smtp(&mut ctx, &smtp_tx, &mut inflight, request_id, config).await;
            }
            CoreEvent::SmtpFinished {
                generation,
                outcome,
            } => {
                handle_smtp_finished(
                    &mut manager,
                    &mut ctx,
                    outbox.as_ref(),
                    &smtp_tx,
                    &mut inflight,
                    generation,
                    outcome,
                )
                .await;
            }
            CoreEvent::DrainOutbox => {
                drain_outbox(
                    &mut manager,
                    &mut ctx,
                    outbox.as_ref(),
                    &smtp_tx,
                    &mut inflight,
                )
                .await;
            }
            CoreEvent::RetryOutboxItem { id } => {
                handle_retry_outbox(
                    &mut manager,
                    &mut ctx,
                    outbox.as_ref(),
                    &smtp_tx,
                    &mut inflight,
                    id,
                )
                .await;
            }
            CoreEvent::DeleteOutboxItem { id } => {
                inflight.cancel_by_outbox_id(&id);
                let _ = outbox.delete(&id).await;
                refresh_outbox_signal(outbox.as_ref(), &mut ctx).await;
            }
            CoreEvent::ArchiveSent { account_id, rfc822 } => {
                handle_archive_sent(&manager, &mut ctx, account_id, rfc822).await;
            }
            CoreEvent::MarkAnswered {
                account_id,
                message_id,
            } => {
                handle_mark_answered(&manager, &mut ctx, account_id, message_id).await;
            }
            CoreEvent::SaveImapDraft {
                account_id,
                draft_id,
                rfc822,
                replace,
            } => {
                handle_save_imap_draft(&manager, &mut ctx, account_id, draft_id, rfc822, replace)
                    .await;
            }
            CoreEvent::DeleteImapDraft {
                account_id,
                message_id,
            } => {
                handle_delete_imap_draft(&manager, &mut ctx, account_id, message_id).await;
            }
            CoreEvent::MailboxActivity {
                account_id,
                mailbox_id,
            } => {
                handle_mailbox_activity(&manager, &mut ctx, account_id, mailbox_id).await;
            }
            CoreEvent::BackgroundStatusPoll => {
                handle_background_status_poll(&manager, &mut ctx).await;
                schedule_background_status(smtp_tx.clone());
            }
            CoreEvent::SelectUnifiedInbox => {
                handle_select_unified_inbox(&manager, &mut ctx).await;
            }
        }

        for id in manager.take_session_deaths() {
            handle_session_dropped(&mut manager, &mut ctx, &smtp_tx, id).await;
        }
        queue_adjacent_prefetch(&ctx, &mut pending_prefetch);
    }
}

async fn handle_bootstrap(
    manager: &mut AccountConnectionManager,
    ctx: &mut AppContext,
    active: Option<AccountId>,
) {
    let Some(account_id) = active else {
        // No active account: still sync UI map from store (and any prior memory-only).
        refresh_ui_accounts(manager, ctx).await;
        info!("Bootstrap: no active account");
        return;
    };

    let Some(config) = manager.resolve_config(&account_id).await else {
        warn!("Bootstrap: no config for active account {}", account_id);
        refresh_ui_accounts(manager, ctx).await;
        return;
    };

    // Cache config **before** refresh so memory-only entries are already registered
    // when `refresh_ui_accounts` rebuilds the UI map. Otherwise Ready can briefly
    // observe `accounts == {}` across store-get awaits.
    let is_memory_only = manager
        .store()
        .get(&account_id)
        .await
        .ok()
        .flatten()
        .is_none();
    if is_memory_only {
        manager.cache_config_memory_only(config.clone());
    } else {
        manager.cache_config(config.clone());
    }

    // Refresh UI accounts from store (no secrets). Merge memory-only configs.
    refresh_ui_accounts(manager, ctx).await;
    hydrate_inbox_unread_map(manager, ctx).await;

    // Ensure UI has the active account even if refresh missed it.
    {
        let mut accounts = ctx.accounts.write();
        accounts
            .entry(config.id.clone())
            .or_insert_with(|| config.to_ui_account());
    }

    ctx.selected_account.set(Some(account_id.clone()));
    // Re-hydrate if bootstrap skipped it (e.g. later Bootstrap event).
    if ctx.mailbox_roots.read().is_empty() {
        hydrate_account_into(manager.cache(), ctx, &account_id).await;
    }
    if crate::local_data::e2e_skip_connect() {
        info!(
            "Bootstrap: skipping IMAP connect ({})",
            crate::local_data::E2E_SKIP_CONNECT_KEY
        );
        return;
    }
    match manager
        .ensure_connected(&config, ctx, EnsureConnectedMode::Switch)
        .await
    {
        Ok(()) => {
            list_folders_soft(manager, ctx, &account_id).await;
        }
        Err(e) => {
            error!(
                "Bootstrap connect failed for {}: {} ({:?})",
                account_id, e.message, e.kind
            );
            // Keep a cache hit visible while disconnected.
            if ctx.mailbox_roots.read().is_empty() {
                clear_mailbox_ui(ctx);
            }
        }
    }
}

async fn handle_select_account(
    manager: &mut AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
) {
    ctx.selected_account.set(Some(account_id.clone()));
    // UI may already have written `selected_account` before this event.
    manager.cancel_pending_reconnects(Some(&account_id), ctx);
    // Drop the previous account's selection / body before hydrate so a
    // cache hit cannot leave the old message view painted over the new tree.
    clear_mailbox_ui(ctx);
    hydrate_account_into(manager.cache(), ctx, &account_id).await;

    let Some(config) = manager.resolve_config(&account_id).await else {
        error!("SelectAccount: unknown account {}", account_id);
        set_connection_state(
            ctx,
            &account_id,
            ConnectionState::Error {
                message: "Account configuration not found.".into(),
                kind: ConnectErrorKind::Internal,
                retryable: false,
            },
        );
        return;
    };

    match manager
        .ensure_connected(&config, ctx, EnsureConnectedMode::Switch)
        .await
    {
        Ok(()) => {
            list_folders_soft(manager, ctx, &account_id).await;
        }
        Err(e) => {
            error!(
                "SelectAccount connect failed for {}: {} ({:?})",
                account_id, e.message, e.kind
            );
        }
    }
}

async fn handle_reconnect(
    manager: &mut AccountConnectionManager,
    ctx: &mut AppContext,
    event_tx: &UnboundedSender<CoreEvent>,
    account_id: AccountId,
) {
    // Manual Retry resets backoff and cancels any pending auto-reconnect.
    manager.reset_reconnect_attempts(&account_id);
    // Snapshot config **before** disconnect: `disconnect_account` drops the
    // manager cache (including memory-only configs). Store-backed accounts can
    // re-resolve after disconnect; memory-only ones cannot.
    let Some(config) = manager.resolve_config(&account_id).await else {
        error!("Reconnect: unknown account {}", account_id);
        set_connection_state(
            ctx,
            &account_id,
            ConnectionState::Error {
                message: "Account configuration not found.".into(),
                kind: ConnectErrorKind::Internal,
                retryable: false,
            },
        );
        return;
    };
    let was_memory_only = manager.memory_only_ids().contains(&account_id);

    manager.disconnect_account(&account_id, ctx).await;

    if was_memory_only {
        manager.cache_config_memory_only(config.clone());
    } else {
        manager.cache_config(config.clone());
    }

    match reconnect_and_restore(manager, ctx, &config).await {
        Ok(()) => {}
        Err(e) => {
            error!(
                "Reconnect failed for {}: {} ({:?})",
                account_id, e.message, e.kind
            );
            if e.retryable && e.kind != ConnectErrorKind::Cancelled {
                manager.bump_reconnect_attempts(&account_id);
                start_auto_reconnect(manager, ctx, event_tx, account_id).await;
            }
        }
    }
}

async fn handle_session_dropped(
    manager: &mut AccountConnectionManager,
    ctx: &mut AppContext,
    event_tx: &UnboundedSender<CoreEvent>,
    account_id: AccountId,
) {
    let busy = ctx
        .connection_states
        .read()
        .get(&account_id)
        .is_some_and(|s| {
            matches!(
                s,
                ConnectionState::Connecting
                    | ConnectionState::Authenticating
                    | ConnectionState::Reconnecting { .. }
            )
        });
    if busy {
        // Don't bump generation (would cancel an in-flight connect / timer).
        manager.remove_ws_watch(&account_id);
        return;
    }
    let is_ready = ctx
        .connection_states
        .read()
        .get(&account_id)
        .is_some_and(|s| matches!(s, ConnectionState::Ready));
    if !is_ready {
        manager.remove_ws_watch(&account_id);
        return;
    }

    warn!("IMAP session dropped for {account_id}");
    manager.drop_dead_connector(&account_id);
    if ctx.selected_account.read().as_ref() != Some(&account_id) {
        set_connection_state(ctx, &account_id, ConnectionState::Disconnected);
        return;
    }
    start_auto_reconnect(manager, ctx, event_tx, account_id).await;
}

async fn handle_auto_reconnect(
    manager: &mut AccountConnectionManager,
    ctx: &mut AppContext,
    event_tx: &UnboundedSender<CoreEvent>,
    account_id: AccountId,
    generation: u64,
) {
    if manager.current_generation(&account_id) != generation {
        return;
    }
    if ctx.selected_account.read().as_ref() != Some(&account_id) {
        return;
    }

    let Some(config) = manager.resolve_config(&account_id).await else {
        error!("AutoReconnect: unknown account {}", account_id);
        set_connection_state(
            ctx,
            &account_id,
            ConnectionState::Error {
                message: "Account configuration not found.".into(),
                kind: ConnectErrorKind::Internal,
                retryable: false,
            },
        );
        return;
    };

    info!(
        "Auto-reconnect attempt {} for {}",
        manager.reconnect_attempts(&account_id).saturating_add(1),
        account_id
    );

    match reconnect_and_restore(manager, ctx, &config).await {
        Ok(()) => {
            manager.reset_reconnect_attempts(&account_id);
        }
        Err(e) => {
            error!(
                "Auto-reconnect failed for {}: {} ({:?})",
                account_id, e.message, e.kind
            );
            if e.kind == ConnectErrorKind::Cancelled {
                return;
            }
            if e.kind == ConnectErrorKind::Auth || !e.retryable {
                manager.reset_reconnect_attempts(&account_id);
                return;
            }
            manager.bump_reconnect_attempts(&account_id);
            start_auto_reconnect(manager, ctx, event_tx, account_id).await;
        }
    }
}

async fn start_auto_reconnect(
    manager: &mut AccountConnectionManager,
    ctx: &mut AppContext,
    event_tx: &UnboundedSender<CoreEvent>,
    account_id: AccountId,
) {
    let failed = manager.reconnect_attempts(&account_id);
    let Some(delay_ms) = reconnect_backoff_ms(failed) else {
        manager.reset_reconnect_attempts(&account_id);
        set_connection_state(
            ctx,
            &account_id,
            ConnectionState::Error {
                message: "Lost connection to the mail server. Automatic reconnect gave up.".into(),
                kind: ConnectErrorKind::NetworkOrProxy,
                retryable: true,
            },
        );
        return;
    };

    set_connection_state(
        ctx,
        &account_id,
        ConnectionState::Reconnecting {
            failed_attempts: failed,
            delay_ms,
        },
    );

    let generation = manager.current_generation(&account_id);
    schedule_auto_reconnect(event_tx.clone(), account_id, generation, delay_ms);
}

fn schedule_auto_reconnect(
    event_tx: UnboundedSender<CoreEvent>,
    account_id: AccountId,
    generation: u64,
    delay_ms: u32,
) {
    spawn_reconnect_timer(async move {
        TimeoutFuture::new(delay_ms).await;
        let _ = event_tx.unbounded_send(CoreEvent::AutoReconnect {
            account_id,
            generation,
        });
    });
}

fn schedule_background_status(event_tx: UnboundedSender<CoreEvent>) {
    spawn_reconnect_timer(async move {
        TimeoutFuture::new(crate::background_sync::BACKGROUND_STATUS_INTERVAL_MS).await;
        let _ = event_tx.unbounded_send(CoreEvent::BackgroundStatusPoll);
    });
}

fn spawn_reconnect_timer(fut: impl std::future::Future<Output = ()> + 'static) {
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(fut);
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Host cargo-check cannot run WASM timers.
        drop(fut);
    }
}

async fn reconnect_and_restore(
    manager: &mut AccountConnectionManager,
    ctx: &mut AppContext,
    config: &crate::account_config::AccountConfig,
) -> Result<(), crate::connection::ConnectError> {
    manager
        .ensure_connected(config, ctx, EnsureConnectedMode::Switch)
        .await?;
    if ctx.selected_account.read().as_ref() == Some(&config.id) {
        list_folders_soft(manager, ctx, &config.id).await;
    }
    Ok(())
}

fn note_selected_imap_error(
    manager: &AccountConnectionManager,
    ctx: &AppContext,
    err: &mailiner_core::MailinerError,
) {
    if let Some(id) = ctx.selected_account.read().as_ref() {
        manager.note_imap_error(id, err);
    }
}

async fn handle_commit_new_account(
    manager: &mut AccountConnectionManager,
    ctx: &mut AppContext,
    config: AccountConfig,
) {
    let account_id = config.id.clone();

    // Existing store entry ⇒ credential edit (or re-save); absent ⇒ first-time add.
    let was_in_store = manager
        .store()
        .get(&account_id)
        .await
        .ok()
        .flatten()
        .is_some();
    let was_selected = ctx.selected_account.read().as_ref() == Some(&account_id);
    let had_connector = manager.get(&account_id).is_some();

    // Force a fresh connect so credential edits re-verify (ensure_connected would
    // otherwise short-circuit when a Ready connector already exists for this id).
    // Drop connector only; store entry is left intact until Ready + upsert.
    if had_connector {
        manager.disconnect_account(&account_id, ctx).await;
    }

    // KeepActiveUntilReady: prior *other* active session stays up through connect **and**
    // store writes. Apply the connection cap only after full commit success when we activate.
    match manager
        .ensure_connected(&config, ctx, EnsureConnectedMode::KeepActiveUntilReady)
        .await
    {
        Ok(()) => {
            // Demote Ready → Connecting across the upsert await so the UI does not treat
            // connect-Ready as “commit finished” (edit navigate-before-persist race).
            set_connection_state(ctx, &account_id, ConnectionState::Connecting);

            // Connect-before-persist: only write store after a successful connect.
            if let Err(e) = manager.store().upsert(&config).await {
                error!("CommitNewAccount: store upsert failed: {}", e);
                manager.disconnect_account(&account_id, ctx).await;
                let err_state = ConnectionState::Error {
                    message: format!("Connected, but failed to save account: {e}"),
                    kind: ConnectErrorKind::Internal,
                    retryable: true,
                };
                set_connection_state(ctx, &account_id, err_state.clone());
                // Restore prior live session for mail, then re-surface Error for the form
                // (restore sets Ready and would otherwise look like commit success).
                if was_in_store && was_selected {
                    restore_store_session(manager, ctx, &account_id).await;
                    set_connection_state(ctx, &account_id, err_state);
                }
                return;
            }

            manager.cache_config(config.clone());
            refresh_ui_accounts(manager, ctx).await;

            // New account, or edit of the already-selected account → make/keep active.
            // Edit of a background account: persist only; do not steal the active session.
            let should_activate = !was_in_store || was_selected;
            if should_activate {
                if let Err(e) = manager.store().set_active_id(Some(&account_id)).await {
                    error!("CommitNewAccount: set_active_id failed: {}", e);
                    manager.disconnect_account(&account_id, ctx).await;
                    let err_state = ConnectionState::Error {
                        message: format!(
                            "Connected and saved, but failed to set active account: {e}. \
                             The account may already be saved — reload the page or try again."
                        ),
                        kind: ConnectErrorKind::Internal,
                        retryable: true,
                    };
                    set_connection_state(ctx, &account_id, err_state.clone());
                    if was_in_store && was_selected {
                        restore_store_session(manager, ctx, &account_id).await;
                        set_connection_state(ctx, &account_id, err_state);
                    }
                    // Account is in store from upsert; UI accounts already refreshed.
                    return;
                }

                manager.touch_recency(&account_id);
                manager.evict_over_cap(&account_id, ctx).await;
                ctx.selected_account.set(Some(account_id.clone()));
                set_connection_state(ctx, &account_id, ConnectionState::Ready);
                list_folders_soft(manager, ctx, &account_id).await;
            } else {
                // Background edit: drop the trial connector; leave the prior active session.
                manager.disconnect_account(&account_id, ctx).await;
                // Keep store-backed config cached for a later switch (no live connector).
                manager.cache_config(config.clone());
                // Disconnected (not Ready): no connector. Edit form confirms via store
                // `updated_at` bump after upsert, not connection Ready alone.
                set_connection_state(ctx, &account_id, ConnectionState::Disconnected);
            }
        }
        Err(e) => {
            // No store write on failure. Restore selected store-backed session if we
            // tore it down for re-verify (BUG-3). Re-surface Error after restore so the
            // form does not treat restore-Ready as commit success.
            error!(
                "CommitNewAccount connect failed: {} ({:?})",
                e.message, e.kind
            );
            let err_state = e.to_state();
            if was_in_store {
                if was_selected {
                    restore_store_session(manager, ctx, &account_id).await;
                    set_connection_state(ctx, &account_id, err_state);
                } else if let Some(store_cfg) =
                    manager.store().get(&account_id).await.ok().flatten()
                {
                    // Replace draft cache with store config; leave disconnected.
                    manager.cache_config(store_cfg);
                }
            }
        }
    }
}

/// Re-resolve store config and reconnect when this account is still selected.
async fn restore_store_session(
    manager: &mut AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: &AccountId,
) {
    let Some(store_cfg) = manager.store().get(account_id).await.ok().flatten() else {
        warn!(
            "restore_store_session: no store config for {} — cannot restore",
            account_id
        );
        return;
    };
    manager.cache_config(store_cfg.clone());
    if ctx.selected_account.read().as_ref() != Some(account_id) {
        set_connection_state(ctx, account_id, ConnectionState::Disconnected);
        return;
    }
    match manager
        .ensure_connected(&store_cfg, ctx, EnsureConnectedMode::Switch)
        .await
    {
        Ok(()) => {
            list_folders_soft(manager, ctx, account_id).await;
        }
        Err(re) => {
            error!(
                "restore_store_session reconnect failed for {}: {} ({:?})",
                account_id, re.message, re.kind
            );
        }
    }
}

async fn handle_accounts_changed(manager: &mut AccountConnectionManager, ctx: &mut AppContext) {
    // Store list is authoritative for persisted accounts; memory_only is explicit.
    // On list **error**, do not treat as empty (would mass-disconnect all connectors).
    let store_ids: std::collections::HashSet<AccountId> = match manager.store().list().await {
        Ok(list) => list.into_iter().map(|c| c.id).collect(),
        Err(e) => {
            warn!(
                "AccountsChanged: failed to list store (skipping orphan teardown and UI wipe): {}",
                e
            );
            return;
        }
    };
    let known: std::collections::HashSet<AccountId> = store_ids
        .iter()
        .cloned()
        .chain(manager.memory_only_ids().iter().cloned())
        .collect();
    let orphaned: Vec<AccountId> = manager
        .connector_account_ids()
        .into_iter()
        .filter(|id| !known.contains(id))
        .collect();
    for id in orphaned {
        manager.disconnect_account(&id, ctx).await;
    }

    refresh_ui_accounts(manager, ctx).await;
    hydrate_inbox_unread_map(manager, ctx).await;
    crate::ui_prefs::retain_last_mailboxes(&known);
    crate::ui_prefs::retain_ack_unread(&known);
    crate::ui_prefs::retain_saved_searches(&known);
    ctx.saved_searches
        .set(crate::ui_prefs::load_saved_searches());
    crate::ui_prefs::retain_pinned_messages(&known);
    crate::ui_prefs::retain_snoozed_messages(&known);
    crate::mail_rules::retain_mail_rules(&known);
    crate::vacation::retain_vacation(&known);
    crate::draft_store::retain_drafts(&known);
    if let Err(e) = manager.cache().retain_accounts(&known).await {
        warn!("mail cache retain_accounts failed: {e}");
    }
}

/// Rebuild UI accounts from the store plus explicitly memory-only configs.
///
/// Does **not** re-insert the previous `ctx.accounts` map (that resurrected deleted
/// accounts). Only store list + `manager.memory_only` are authoritative.
///
/// On `store.list()` **error**, leaves current UI accounts unchanged (does not treat
/// the failure as an empty store).
async fn refresh_ui_accounts(manager: &AccountConnectionManager, ctx: &mut AppContext) {
    let mut map = HashMap::new();
    match manager.store().list().await {
        Ok(list) => {
            for cfg in list {
                map.insert(cfg.id.clone(), cfg.to_ui_account());
            }
        }
        Err(e) => {
            warn!(
                "Failed to list accounts from store (leaving UI accounts unchanged): {}",
                e
            );
            return;
        }
    }
    // Explicit memory-only configs — not the previous UI map.
    for id in manager.memory_only_ids() {
        if let Some(cfg) = manager.config(id) {
            map.entry(id.clone()).or_insert_with(|| cfg.to_ui_account());
        }
    }
    ctx.accounts.set(map);
}

async fn list_folders_soft(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: &AccountId,
) {
    let Some(connector) = manager.get(account_id) else {
        error!("list_folders: no connector for {}", account_id);
        return;
    };

    match connector.list_folders(account_id).await {
        Ok(mboxes) => {
            let folder_ids: Vec<FolderId> = mboxes
                .iter()
                .filter(|f| f.selectable && f.subscribed)
                .map(|f| f.id.clone())
                .collect();
            let (root_ids, nodes) = crate::mailbox::build_mailbox_tree(mboxes);
            ctx.mailbox_nodes.set(nodes);
            ctx.mailbox_roots.set(root_ids);

            // STATUS the folder we will open first so its badge is ready before SELECT.
            let startup = {
                let nodes = ctx.mailbox_nodes.read();
                let roots = ctx.mailbox_roots.read();
                let saved = crate::ui_prefs::load_last_mailbox(account_id);
                let show_all = *ctx.show_all_folders.read();
                crate::mailbox::resolve_startup_mailbox(saved.as_ref(), &nodes, &roots, show_all)
            };
            if let Some(startup_id) = startup.as_ref() {
                let one = [FolderId::new(startup_id.to_string())];
                if let Ok(counts) = connector.folder_counts(&one).await {
                    let ack = crate::ui_prefs::load_ack_unread(account_id);
                    {
                        let mut nodes = ctx.mailbox_nodes.write();
                        crate::mailbox::apply_folder_counts(&mut nodes, &counts);
                        crate::mailbox::apply_unread_new_state(&mut nodes, &counts, &ack);
                    }
                    observe_remote_counts(ctx, &counts);
                }
            }

            restore_mailbox(manager, ctx, account_id).await;

            // Remaining folders. Skip the selected one so later STATUS cannot
            // overwrite local read/unread adjustments.
            let selected = ctx.selected_mailbox.read().clone();
            let rest: Vec<FolderId> = folder_ids
                .into_iter()
                .filter(|id| {
                    selected
                        .as_ref()
                        .is_none_or(|sel| sel.as_str() != id.as_str())
                })
                .collect();
            let ack = crate::ui_prefs::load_ack_unread(account_id);
            for id in rest {
                match connector.folder_counts(std::slice::from_ref(&id)).await {
                    Ok(counts) if !counts.is_empty() => {
                        {
                            let mut nodes = ctx.mailbox_nodes.write();
                            crate::mailbox::apply_folder_counts(&mut nodes, &counts);
                            crate::mailbox::apply_unread_new_state(&mut nodes, &counts, &ack);
                        }
                        observe_remote_counts(ctx, &counts);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        warn!("folder_counts {} failed: {}", id, e);
                        manager.note_imap_error(account_id, &e);
                    }
                }
            }
            persist_folder_tree(manager.cache(), ctx, account_id).await;
            sync_selected_inbox_unread(ctx);
        }
        Err(e) => {
            error!("Failed to list folders for {}: {}", account_id, e);
            manager.note_imap_error(account_id, &e);
            // Keep a cached tree if we already painted one.
            if ctx.mailbox_roots.read().is_empty() {
                ctx.mailbox_nodes.set(HashMap::new());
                ctx.mailbox_roots.set(Vec::new());
            }
        }
    }
}

async fn fetch_quota_soft(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: &AccountId,
    folder_id: &FolderId,
) {
    if !selected_account_is(ctx, account_id)
        || ctx.selected_mailbox.read().as_ref().map(|id| id.as_str()) != Some(folder_id.as_str())
    {
        return;
    }
    let Some(connector) = manager.get(account_id) else {
        return;
    };
    match connector.folder_quota(folder_id).await {
        Ok(quota) => {
            if selected_account_is(ctx, account_id)
                && ctx.selected_mailbox.read().as_ref().map(|id| id.as_str())
                    == Some(folder_id.as_str())
            {
                ctx.account_quota.set(quota);
            }
        }
        Err(e) => {
            warn!("folder_quota failed for {account_id}: {e}");
        }
    }
}

/// Open the last folder for this account, or Inbox / first root when none is saved.
async fn restore_mailbox(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: &AccountId,
) {
    if is_unified_selected(ctx) {
        return;
    }
    let to_open = {
        let nodes = ctx.mailbox_nodes.read();
        let roots = ctx.mailbox_roots.read();
        let saved = crate::ui_prefs::load_last_mailbox(account_id);
        let show_all = *ctx.show_all_folders.read();
        crate::mailbox::resolve_startup_mailbox(saved.as_ref(), &nodes, &roots, show_all)
    };
    let Some(mailbox_id) = to_open else {
        return;
    };
    handle_select_mailbox(manager, ctx, mailbox_id, true).await;
}

fn clear_mailbox_ui(ctx: &mut AppContext) {
    ctx.selected_mailbox.set(None);
    ctx.list_text_filter.set(String::new());
    ctx.list_search_query.set(String::new());
    ctx.active_saved_search.set(None);
    ctx.messages.set(SparseList::new(0));
    ctx.messages_loading.set(false);
    ctx.selection.write().clear();
    ctx.message_view.set(MessageViewState::Empty);
    ctx.message_bodies.borrow_mut().clear();
    ctx.message_headers.set(MessageHeadersState::Closed);
    ctx.message_source.set(MessageSourceState::Closed);
    ctx.clear_nested_rfc822();
    ctx.download_status.set(HashMap::new());
    ctx.clear_attachment_downloads();
    ctx.mailbox_nodes.set(HashMap::new());
    ctx.mailbox_roots.set(Vec::new());
    ctx.account_quota.set(None);
    ctx.unified_inbox_notes.set(Vec::new());
    crate::notifications::reset_inbox_unread_baseline();
}

/// Paint a [`HydratedAccount`] onto UI signals (cache hit, no IMAP).
pub(crate) fn apply_hydrated(ctx: &mut AppContext, hydrated: HydratedAccount) {
    ctx.mailbox_nodes.set(hydrated.nodes);
    ctx.mailbox_roots.set(hydrated.roots);
    sync_selected_inbox_unread(ctx);
    if let Some(mailbox_id) = hydrated.selected_mailbox {
        ctx.selected_mailbox.set(Some(mailbox_id));
    }
    sync_list_overlays(ctx);
    match hydrated.messages {
        Some(msgs) => {
            let mut list = SparseList::new(msgs.total);
            list.insert_batch(0, msgs.prefix);
            ctx.messages.set(list);
            apply_list_overlays(ctx);
            ctx.messages_loading.set(false);
        }
        None => {
            if ctx.selected_mailbox.read().is_some() {
                ctx.messages.set(SparseList::new(0));
                ctx.messages_loading.set(true);
            }
        }
    }
}

fn apply_cached_message_list(ctx: &mut AppContext, cached: &CachedMessageList) {
    let ui = cached.to_ui_prefix();
    let mut list = SparseList::new(ui.total);
    list.insert_batch(0, ui.prefix);
    ctx.messages.set(list);
    apply_list_overlays(ctx);
    ctx.messages_loading.set(false);
}

/// `true` when a folder tree was applied from cache.
pub(crate) async fn hydrate_account_into(
    cache: &dyn MailCache,
    ctx: &mut AppContext,
    account_id: &AccountId,
) -> bool {
    let sort = *ctx.message_sort.peek();
    let saved = crate::ui_prefs::load_last_mailbox(account_id);
    let ack = crate::ui_prefs::load_ack_unread(account_id);
    match hydrate_account(cache, account_id, sort, saved.as_ref(), &ack).await {
        Ok(Some(hydrated)) => {
            apply_hydrated(ctx, hydrated);
            true
        }
        Ok(None) => false,
        Err(e) => {
            warn!("mail cache hydrate failed for {account_id}: {e}");
            false
        }
    }
}

async fn handle_background_status_poll(manager: &AccountConnectionManager, ctx: &mut AppContext) {
    let selected = ctx.selected_account.read().clone();
    let ids: Vec<AccountId> = manager
        .connector_account_ids()
        .into_iter()
        .filter(|id| Some(id) != selected.as_ref())
        .filter(|id| {
            ctx.connection_states
                .read()
                .get(id)
                .is_some_and(|s| matches!(s, ConnectionState::Ready))
        })
        .collect();
    for id in ids {
        poll_background_account(manager, ctx, &id).await;
    }
}

/// `STATUS` subscribed folders on a non-selected account; persist unread only.
async fn poll_background_account(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: &AccountId,
) {
    let Some(connector) = manager.get(account_id) else {
        return;
    };
    let targets = match manager.cache().load_folders(account_id).await {
        Ok(Some(tree)) => crate::background_sync::background_status_targets(&tree),
        Ok(None) => vec![FolderId::new("INBOX")],
        Err(e) => {
            warn!("background status: cache load failed for {account_id}: {e}");
            vec![FolderId::new("INBOX")]
        }
    };
    match connector.folder_counts(&targets).await {
        Ok(counts) if !counts.is_empty() => {
            if let Some(unread) = inbox_unread_from_status(&counts) {
                set_account_inbox_unread(ctx, account_id, unread);
            }
            if let Err(e) = crate::background_sync::merge_status_into_cache(
                manager.cache(),
                account_id,
                &counts,
            )
            .await
            {
                warn!("background status: cache save failed for {account_id}: {e}");
            }
        }
        Ok(_) => {}
        Err(e) => {
            warn!("background STATUS failed for {account_id}: {e}");
            manager.note_imap_error(account_id, &e);
        }
    }
}

async fn persist_folder_tree(cache: &dyn MailCache, ctx: &AppContext, account_id: &AccountId) {
    let tree = {
        let nodes = ctx.mailbox_nodes.read();
        CachedFolderTree::from_nodes(account_id, &nodes)
    };
    if tree.folders.is_empty() {
        return;
    }
    if let Err(e) = cache.save_folders(account_id, &tree).await {
        warn!("mail cache save folders failed: {e}");
    }
}

async fn persist_selected_messages(
    cache: &dyn MailCache,
    ctx: &AppContext,
    account_id: &AccountId,
) {
    if ctx.selected_account.read().as_ref() != Some(account_id) {
        return;
    }
    let Some(mailbox_id) = ctx.selected_mailbox.read().clone() else {
        return;
    };
    if is_unified_mailbox(&mailbox_id) {
        return;
    }
    if ctx.message_list_filter.peek().imap_search_query().is_some()
        || mailiner_core::mailbox_search_is_active(ctx.list_search_query.peek().as_str())
    {
        return;
    }
    let sort = *ctx.message_sort.peek();
    let (total, unread, prefix) = {
        let list = ctx.messages.read();
        let total = list.total_count();
        let unread = ctx
            .mailbox_nodes
            .read()
            .get(&mailbox_id)
            .map(|n| n.unread_count);
        let prefix = contiguous_envelope_prefix(|i| list.get(i).map(|m| m.envelope.clone()), total);
        (total, unread, prefix)
    };
    if total > 0 && prefix.is_empty() {
        // Don't clobber a previous prefix with an empty hole at index 0.
        return;
    }
    let snapshot = CachedMessageList::from_prefix(&mailbox_id, sort, total, unread, prefix);
    if let Err(e) = cache.save_messages(account_id, &snapshot).await {
        warn!("mail cache save messages failed: {e}");
    }
}

/// Keep already-fetched snippets when a live envelope batch replaces a cache hit.
fn messages_from_envelopes(
    envelopes: Vec<mailiner_core::Envelope>,
    existing: &SparseList<Arc<Message>>,
) -> Vec<Arc<Message>> {
    envelopes
        .into_iter()
        .map(|envelope| {
            let mut msg = Message::from(envelope);
            if msg.snippet.is_none()
                && let Some(prev) = existing.find(|m| m.id == msg.id)
                && let Some(snippet) = prev.snippet.clone()
            {
                msg.snippet = Some(snippet.clone());
                msg.envelope.snippet = Some(snippet);
            }
            Arc::new(msg)
        })
        .collect()
}

fn apply_snippets(
    ctx: &mut AppContext,
    raw: HashMap<mailiner_core::MessageId, mailiner_core::TextPrefix>,
) {
    for msg in ctx.messages.write().iter_mut() {
        if msg.snippet.is_some() {
            continue;
        }
        let Some(prefix) = raw.get(&msg.id) else {
            // Peek/structure failed — leave unset so a later load can retry.
            continue;
        };
        let cleaned = clean_snippet(&prefix.text, prefix.is_html);
        let mut next = (**msg).clone();
        next.snippet = Some(cleaned.clone());
        next.envelope.snippet = Some(cleaned);
        *msg = Arc::new(next);
    }
}

/// Peek first-text-part prefixes for loaded rows that still lack a snippet.
///
/// List rows are already on screen; this must not run before the envelope batch
/// is written. Failures leave `snippet` unset so a later page load can retry.
/// `persist` is true only when the contiguous cached prefix can grow/change.
async fn fetch_and_apply_snippets(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: &AccountId,
    mailbox_id: &MailboxId,
    persist: bool,
) {
    if ctx.selected_mailbox.read().as_ref() != Some(mailbox_id)
        || !selected_account_is(ctx, account_id)
    {
        return;
    }
    let Some(connector) = manager.get(account_id) else {
        return;
    };
    let needed: Vec<MessageId> = {
        let list = ctx.messages.read();
        (0..list.total_count())
            .filter_map(|i| list.get(i))
            .filter(|m| m.snippet.is_none())
            .map(|m| m.id.clone())
            .collect()
    };
    if needed.is_empty() {
        return;
    }
    let folder_id = FolderId::new(mailbox_id.to_string());
    match connector
        .fetch_text_prefixes(&folder_id, &needed, SNIPPET_FETCH_OCTETS)
        .await
    {
        Ok(raw) => {
            if ctx.selected_mailbox.read().as_ref() != Some(mailbox_id)
                || !selected_account_is(ctx, account_id)
            {
                return;
            }
            apply_snippets(ctx, raw);
            if persist {
                persist_selected_messages(manager.cache(), ctx, account_id).await;
            }
        }
        Err(e) => {
            warn!("snippet fetch failed for {}: {e}", mailbox_id.as_str());
        }
    }
}

fn contiguous_loaded_prefix_len<T: Clone>(list: &SparseList<T>) -> usize {
    let cap = list.total_count();
    let mut n = 0;
    while n < cap && list.has_item(n) {
        n += 1;
    }
    n
}

fn selected_account_is(ctx: &AppContext, account_id: &AccountId) -> bool {
    ctx.selected_account.read().as_ref() == Some(account_id)
}

async fn invalidate_mailbox_messages(
    cache: &dyn MailCache,
    account_id: &AccountId,
    mailbox_id: &MailboxId,
) {
    if let Err(e) = cache.invalidate_messages(account_id, mailbox_id).await {
        warn!("mail cache invalidate {} failed: {e}", mailbox_id.as_str());
    }
}

/// After a MOVE that finished on a different account/mailbox, keep folder totals
/// and unread badges in sync without mutating the now-current UI list.
async fn persist_stale_move_counts(
    cache: &dyn MailCache,
    ctx: &mut AppContext,
    account_id: &AccountId,
    source: &MailboxId,
    dest: &MailboxId,
    moved: usize,
    unread: i32,
) {
    if moved == 0 && unread == 0 {
        return;
    }
    let dest_name = ctx.mailbox_nodes.read().get(dest).map(|n| n.name.clone());
    // Gmail All Mail already contains every message; do not double-count dest.
    let dest_is_all_mail = mailbox_is_all_mail(dest, dest_name.as_deref());
    if selected_account_is(ctx, account_id) {
        bump_mailbox_total(ctx, source, -(moved as i32));
        if !dest_is_all_mail {
            bump_mailbox_total(ctx, dest, moved as i32);
        }
        if unread != 0 {
            bump_mailbox_unread(ctx, source, -unread, true);
            if !dest_is_all_mail {
                bump_mailbox_unread(ctx, dest, unread, false);
            }
        }
        persist_folder_tree(cache, ctx, account_id).await;
        return;
    }
    let Ok(Some(mut tree)) = cache.load_folders(account_id).await else {
        return;
    };
    let moved = moved as u64;
    let unread = unread.max(0) as u64;
    if let Some(src) = tree.counts.get_mut(source.as_str()) {
        src.total_messages = src.total_messages.saturating_sub(moved);
        src.unread_messages = src.unread_messages.saturating_sub(unread);
    }
    if !dest_is_all_mail {
        if let Some(dst) = tree.counts.get_mut(dest.as_str()) {
            dst.total_messages = dst.total_messages.saturating_add(moved);
            dst.unread_messages = dst.unread_messages.saturating_add(unread);
        }
    }
    if let Err(e) = cache.save_folders(account_id, &tree).await {
        warn!("mail cache adjust folder totals failed: {e}");
    }
}

fn bump_mailbox_total(ctx: &mut AppContext, mailbox_id: &MailboxId, delta: i32) {
    if delta == 0 {
        return;
    }
    if let Some(node) = ctx.mailbox_nodes.write().get_mut(mailbox_id) {
        node.total_count = (node.total_count as i32 + delta).max(0) as usize;
    }
}

async fn handle_select_mailbox(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    mailbox_id: MailboxId,
    select_first: bool,
) {
    if is_unified_mailbox(&mailbox_id) {
        handle_select_unified_inbox(manager, ctx).await;
        return;
    }
    ctx.unified_inbox_notes.set(Vec::new());
    select_mailbox(manager, ctx, mailbox_id, select_first, false).await;
}

async fn handle_select_unified_inbox(manager: &AccountConnectionManager, ctx: &mut AppContext) {
    ctx.set_mobile_pane(MobilePane::after_select_mailbox());
    ctx.active_saved_search.set(None);
    ctx.list_text_filter.set(String::new());
    ctx.list_search_query.set(String::new());
    ctx.selection.write().clear();
    ctx.message_view.set(MessageViewState::Empty);
    ctx.message_headers.set(MessageHeadersState::Closed);
    ctx.message_source.set(MessageSourceState::Closed);
    ctx.clear_nested_rfc822();
    ctx.download_status.set(HashMap::new());
    ctx.clear_attachment_downloads();
    ctx.selected_mailbox.set(Some(unified_mailbox_id()));
    ctx.messages.set(SparseList::new(0));
    ctx.messages_loading.set(true);
    ctx.unified_inbox_notes.set(Vec::new());

    let mut account_ids: Vec<AccountId> = ctx.accounts.read().keys().cloned().collect();
    account_ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));

    let mut prefixes = Vec::with_capacity(account_ids.len());
    for account_id in account_ids {
        let prefix = fetch_account_inbox_prefix(manager, &account_id).await;
        if let Some(unread) = prefix.unread {
            ctx.account_inbox_unread
                .write()
                .insert(account_id.clone(), unread);
        }
        prefixes.push(prefix);
        if !is_unified_selected(ctx) {
            return;
        }
    }

    if !is_unified_selected(ctx) {
        return;
    }

    let notes = notes_from_prefixes(&prefixes);
    let merged = merge_inbox_prefixes(prefixes);
    let mut list = SparseList::new(merged.len());
    list.insert_batch(0, merged.into_iter().map(Arc::new).collect());
    ctx.messages.set(list);
    ctx.unified_inbox_notes.set(notes);
    ctx.messages_loading.set(false);
}

async fn fetch_account_inbox_prefix(
    manager: &AccountConnectionManager,
    account_id: &AccountId,
) -> AccountInboxPrefix {
    let tree = manager
        .cache()
        .load_folders(account_id)
        .await
        .ok()
        .flatten();
    let folder_id = inbox_folder_id(tree.as_ref());
    let mailbox_id = MailboxId::from(folder_id.clone());
    let cached_unread = tree.as_ref().and_then(inbox_unread_from_tree);

    if let Some(connector) = manager.get(account_id) {
        match connector
            .prepare_folder_list(
                &folder_id,
                mailiner_core::MessageSort::Date,
                MessageListFilter::default(),
                "",
            )
            .await
        {
            Ok(state) => {
                let end = state.total.min(UNIFIED_INBOX_PREFIX);
                let envelopes = if end > 0 {
                    match connector.list_envelopes_range(&folder_id, 0..end).await {
                        Ok(envs) => envs,
                        Err(e) => {
                            warn!("unified inbox FETCH failed for {account_id}: {e}");
                            manager.note_imap_error(account_id, &e);
                            return cached_inbox_prefix(
                                manager,
                                account_id,
                                folder_id,
                                mailbox_id,
                                cached_unread,
                                PrefixSource::Failed,
                            )
                            .await;
                        }
                    }
                } else {
                    Vec::new()
                };
                let unread = state.unread.map(|n| n as u64).or(cached_unread);
                let snapshot = CachedMessageList::from_prefix(
                    &mailbox_id,
                    mailiner_core::MessageSort::Date,
                    state.total,
                    state.unread,
                    envelopes.clone(),
                );
                if let Err(e) = manager.cache().save_messages(account_id, &snapshot).await {
                    warn!("unified inbox cache save failed for {account_id}: {e}");
                }
                return AccountInboxPrefix {
                    account_id: account_id.clone(),
                    folder_id,
                    envelopes,
                    unread,
                    source: PrefixSource::Live,
                };
            }
            Err(e) => {
                warn!("unified inbox SELECT failed for {account_id}: {e}");
                manager.note_imap_error(account_id, &e);
                return cached_inbox_prefix(
                    manager,
                    account_id,
                    folder_id,
                    mailbox_id,
                    cached_unread,
                    PrefixSource::Failed,
                )
                .await;
            }
        }
    }

    cached_inbox_prefix(
        manager,
        account_id,
        folder_id,
        mailbox_id,
        cached_unread,
        PrefixSource::Skipped,
    )
    .await
}

async fn cached_inbox_prefix(
    manager: &AccountConnectionManager,
    account_id: &AccountId,
    folder_id: FolderId,
    mailbox_id: MailboxId,
    cached_unread: Option<u64>,
    miss: PrefixSource,
) -> AccountInboxPrefix {
    for sort in mailiner_core::MessageSort::ALL {
        match manager
            .cache()
            .load_messages(account_id, &mailbox_id, sort)
            .await
        {
            Ok(Some(cached)) => {
                return AccountInboxPrefix {
                    account_id: account_id.clone(),
                    folder_id,
                    envelopes: cached.envelopes,
                    unread: cached.unread.map(|n| n as u64).or(cached_unread),
                    source: PrefixSource::Cache,
                };
            }
            Ok(None) => {}
            Err(e) => {
                warn!("unified inbox cache load failed for {account_id}: {e}");
                break;
            }
        }
    }
    AccountInboxPrefix {
        account_id: account_id.clone(),
        folder_id,
        envelopes: Vec::new(),
        unread: cached_unread,
        source: miss,
    }
}

fn is_unified_selected(ctx: &AppContext) -> bool {
    ctx.selected_mailbox
        .read()
        .as_ref()
        .is_some_and(is_unified_mailbox)
}

/// True when this mailbox action belongs to the open list (including All inboxes).
fn mailbox_action_applies(ctx: &AppContext, mailbox_id: &MailboxId) -> bool {
    if is_unified_mailbox(mailbox_id) {
        return false;
    }
    if is_unified_selected(ctx) {
        return true;
    }
    ctx.selected_mailbox.read().as_ref() == Some(mailbox_id)
}

fn action_account_for(ctx: &AppContext, message_ids: &[MessageId]) -> Option<AccountId> {
    if is_unified_selected(ctx) {
        return batch_target_for(ctx, message_ids).map(|t| t.account_id);
    }
    ctx.selected_account.read().clone()
}

fn set_account_inbox_unread(ctx: &AppContext, account_id: &AccountId, unread: u64) {
    let mut map = ctx.account_inbox_unread;
    map.write().insert(account_id.clone(), unread);
}

fn sync_selected_inbox_unread(ctx: &AppContext) {
    let Some(account_id) = ctx.selected_account.read().clone() else {
        return;
    };
    let unread = crate::notifications::inbox_unread(&ctx.mailbox_nodes.read()).map(|(_, n)| n);
    if let Some(unread) = unread {
        set_account_inbox_unread(ctx, &account_id, unread as u64);
    }
}

fn bump_account_inbox_unread(ctx: &AppContext, account_id: &AccountId, delta: i32) {
    if delta == 0 {
        return;
    }
    let mut map = ctx.account_inbox_unread;
    let mut guard = map.write();
    let entry = guard.entry(account_id.clone()).or_insert(0);
    *entry = (*entry as i64 + i64::from(delta)).max(0) as u64;
}

async fn hydrate_inbox_unread_map(manager: &AccountConnectionManager, ctx: &mut AppContext) {
    let ids: Vec<AccountId> = ctx.accounts.read().keys().cloned().collect();
    for id in ids {
        match manager.cache().load_folders(&id).await {
            Ok(Some(tree)) => {
                if let Some(unread) = inbox_unread_from_tree(&tree) {
                    set_account_inbox_unread(ctx, &id, unread);
                }
            }
            Ok(None) => {}
            Err(e) => warn!("inbox unread hydrate failed for {id}: {e}"),
        }
    }
    sync_selected_inbox_unread(ctx);
}

fn list_message_by_id(ctx: &AppContext, message_id: &MessageId) -> Option<Arc<Message>> {
    if let Some(idx) = ctx.selection.read().focus_at_index()
        && let Some(msg) = ctx.messages.read().get(idx)
        && msg.id == *message_id
    {
        return Some(msg.clone());
    }
    ctx.messages.read().find(|m| m.id == *message_id).cloned()
}

fn open_target_for(
    ctx: &AppContext,
    message_id: &MessageId,
) -> Option<crate::unified_inbox::OpenTarget> {
    list_message_by_id(ctx, message_id).map(|m| open_target(m.as_ref()))
}

fn batch_target_for(
    ctx: &AppContext,
    message_ids: &[MessageId],
) -> Option<crate::unified_inbox::OpenTarget> {
    let list = ctx.messages.read();
    let rows: Vec<Arc<Message>> = message_ids
        .iter()
        .filter_map(|id| list.find(|m| m.id == *id).cloned())
        .collect();
    if rows.len() != message_ids.len() {
        return None;
    }
    batch_open_target(rows.iter().map(|m| m.as_ref()))
}

async fn select_mailbox(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    mailbox_id: MailboxId,
    select_first: bool,
    from_saved_search: bool,
) {
    if ctx
        .mailbox_nodes
        .read()
        .get(&mailbox_id)
        .is_some_and(|n| !n.selectable)
    {
        return;
    }

    // Folder open stays on the list even when the first row is auto-selected.
    ctx.set_mobile_pane(MobilePane::after_select_mailbox());

    let mailbox_changed = ctx.selected_mailbox.peek().as_ref() != Some(&mailbox_id);
    if mailbox_changed {
        ctx.expanded_conversations.write().clear();
    }
    let leaving_saved = !from_saved_search && ctx.active_saved_search.peek().is_some();
    if !from_saved_search {
        ctx.active_saved_search.set(None);
        if mailbox_changed || leaving_saved {
            ctx.list_text_filter.set(String::new());
            ctx.list_search_query.set(String::new());
        }
    }

    let already_showing = !leaving_saved
        && ctx.selected_mailbox.read().as_ref() == Some(&mailbox_id)
        && ctx.messages.read().cached_count() > 0;
    if !already_showing {
        ctx.selection.write().clear();
        ctx.message_view.set(MessageViewState::Empty);
        ctx.message_headers.set(MessageHeadersState::Closed);
        ctx.message_source.set(MessageSourceState::Closed);
        ctx.clear_nested_rfc822();
        ctx.download_status.set(HashMap::new());
        ctx.clear_attachment_downloads();
        ctx.selected_mailbox.set(Some(mailbox_id.clone()));
        sync_list_overlays(ctx);
        let sort = *ctx.message_sort.peek();
        let account = ctx.selected_account.read().clone();
        // Cached prefixes are unfiltered; skip them when SEARCH is narrowing the folder.
        let use_cache = ctx.message_list_filter.peek().imap_search_query().is_none()
            && !mailiner_core::mailbox_search_is_active(ctx.list_search_query.peek().as_str());
        let hydrated = if use_cache {
            match account {
                Some(account_id) => manager
                    .cache()
                    .load_messages(&account_id, &mailbox_id, sort)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            }
        } else {
            None
        };
        if let Some(cached) = hydrated {
            apply_cached_message_list(ctx, &cached);
        } else {
            ctx.messages.set(SparseList::new(0));
            ctx.messages_loading.set(true);
        }
    } else {
        ctx.selected_mailbox.set(Some(mailbox_id.clone()));
        sync_list_overlays(ctx);
        apply_list_overlays(ctx);
        ctx.messages_loading.set(false);
    }

    let Some(account_id) = ctx.selected_account.read().clone() else {
        error!("SelectMailbox: no account selected");
        ctx.messages_loading.set(false);
        return;
    };
    let Some(connector) = manager.get(&account_id) else {
        // Offline / still connecting: keep the cache visible.
        ctx.messages_loading.set(false);
        return;
    };

    let folder_id = FolderId::new(mailbox_id.to_string());
    let requested = *ctx.message_sort.peek();
    let filter = *ctx.message_list_filter.peek();
    let search = ctx.list_search_query.peek().clone();
    match connector
        .prepare_folder_list(&folder_id, requested, filter, &search)
        .await
    {
        Ok(state) => {
            info!(
                "Opened mailbox {} with {} messages (sort={:?})",
                mailbox_id.to_string(),
                state.total,
                state.sort
            );
            ctx.sort_supports_size_sender
                .set(state.supports_size_sender);
            ctx.message_sort.set(state.sort);
            crate::ui_prefs::save_last_mailbox(&account_id, &mailbox_id);
            acknowledge_mailbox_open(
                ctx,
                &account_id,
                &mailbox_id,
                state.folder_total,
                state.unread,
            );

            // Fetch the first page, then swap the list atomically so a cache
            // hit stays on screen until live envelopes arrive.
            let end = state.total.min(20);
            let live = if end > 0 {
                connector.list_envelopes_range(&folder_id, 0..end).await
            } else {
                Ok(Vec::new())
            };
            if ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id)
                || !selected_account_is(ctx, &account_id)
            {
                return;
            }
            match live {
                Ok(envelopes) => {
                    let mut list = SparseList::new(state.total);
                    let batch = messages_from_envelopes(envelopes, &ctx.messages.read());
                    list.insert_batch(0, batch);
                    ctx.messages.set(list);
                    apply_list_overlays(ctx);
                    ctx.messages_loading.set(false);
                    apply_vacation_auto_reply(ctx, &account_id, &mailbox_id);
                    apply_incoming_mail_rules(manager, ctx, &account_id, &mailbox_id).await;
                    persist_selected_messages(manager.cache(), ctx, &account_id).await;
                    persist_folder_tree(manager.cache(), ctx, &account_id).await;
                    fetch_and_apply_snippets(manager, ctx, &account_id, &mailbox_id, true).await;
                }
                Err(e) => {
                    error!(
                        "Failed to fetch first page of {}: {}",
                        mailbox_id.as_str(),
                        e
                    );
                    note_selected_imap_error(manager, ctx, &e);
                    ctx.messages_loading.set(false);
                    {
                        let mut list = ctx.messages.write();
                        if list.cached_count() == 0 {
                            *list = SparseList::new(state.total);
                        } else if list.total_count() != state.total {
                            list.set_total_count(state.total);
                        }
                    }
                    if ctx.messages.read().cached_count() > 0 {
                        persist_selected_messages(manager.cache(), ctx, &account_id).await;
                    }
                    fetch_and_apply_snippets(manager, ctx, &account_id, &mailbox_id, true).await;
                }
            }

            if select_first && state.total > 0 {
                let first_id = ctx.messages.read().get(0).map(|m| m.id.clone());
                if let Some(id) = first_id {
                    // Unread-first / unread filter: selecting the top row would
                    // immediately consume the message the user just asked to see.
                    let auto_mark = state.sort != MessageSort::Unread
                        && !filter.unread
                        && !mailiner_core::MailboxSearch::parse(&search).has_unread();
                    handle_select_message(manager, ctx, id, auto_mark, true).await;
                }
            }
        }
        Err(e) => {
            error!("Failed to open mailbox: {}", e);
            note_selected_imap_error(manager, ctx, &e);
            ctx.messages_loading.set(false);
        }
    }
    fetch_quota_soft(manager, ctx, &account_id, &folder_id).await;
}

async fn handle_mailbox_activity(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    mailbox_id: MailboxId,
) {
    if !selected_account_is(ctx, &account_id)
        || ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id)
    {
        return;
    }
    let Some(connector) = manager.get(&account_id) else {
        return;
    };

    let folder_id = FolderId::new(mailbox_id.to_string());
    let requested = *ctx.message_sort.peek();
    let filter = *ctx.message_list_filter.peek();
    let search = ctx.list_search_query.peek().clone();
    let state = match connector
        .prepare_folder_list(&folder_id, requested, filter, &search)
        .await
    {
        Ok(state) => state,
        Err(e) => {
            warn!(
                "live mailbox refresh failed for {}: {e}",
                mailbox_id.as_str()
            );
            note_selected_imap_error(manager, ctx, &e);
            return;
        }
    };

    if !selected_account_is(ctx, &account_id)
        || ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id)
    {
        return;
    }

    ctx.sort_supports_size_sender
        .set(state.supports_size_sender);
    ctx.message_sort.set(state.sort);
    apply_live_counts(
        ctx,
        &account_id,
        &mailbox_id,
        state.folder_total,
        state.unread,
    );

    let loaded_hi = ctx
        .messages
        .read()
        .iter_indexed()
        .map(|(i, _)| i.saturating_add(1))
        .max()
        .unwrap_or(0);
    let end = live_refresh_end(state.total, loaded_hi);
    let old_ids: HashSet<MessageId> = ctx
        .messages
        .read()
        .iter_indexed()
        .filter(|(i, _)| *i < end)
        .map(|(_, m)| m.id.clone())
        .collect();

    let live = if end > 0 {
        connector.list_envelopes_range(&folder_id, 0..end).await
    } else {
        Ok(Vec::new())
    };
    if !selected_account_is(ctx, &account_id)
        || ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id)
    {
        return;
    }
    match live {
        Ok(envelopes) => {
            let new_ids: HashSet<MessageId> = envelopes.iter().map(|e| e.id.clone()).collect();
            let mut list = SparseList::new(state.total);
            let batch = messages_from_envelopes(envelopes, &ctx.messages.read());
            list.insert_batch(0, batch);
            ctx.messages.set(list);
            apply_list_overlays(ctx);
            prune_selection_after_refresh(ctx, &old_ids, &new_ids);
            apply_vacation_auto_reply(ctx, &account_id, &mailbox_id);
            apply_incoming_mail_rules(manager, ctx, &account_id, &mailbox_id).await;
            persist_selected_messages(manager.cache(), ctx, &account_id).await;
            persist_folder_tree(manager.cache(), ctx, &account_id).await;
            fetch_and_apply_snippets(manager, ctx, &account_id, &mailbox_id, true).await;
        }
        Err(e) => {
            warn!(
                "live mailbox list fetch failed for {}: {e}",
                mailbox_id.as_str()
            );
            note_selected_imap_error(manager, ctx, &e);
            let mut list = ctx.messages.write();
            if list.total_count() != state.total {
                list.set_total_count(state.total);
            }
        }
    }

    refresh_other_folder_badges(manager, ctx, &account_id, Some(&mailbox_id)).await;
}

/// Queue local vacation auto-replies for newly arrived incoming mail.
///
/// Client-side only (not ManageSieve). Same folder skip list as mail rules.
/// Replies go through [`CoreEvent::SendMessage`] / the SMTP outbox.
fn apply_vacation_auto_reply(ctx: &AppContext, account_id: &AccountId, mailbox_id: &MailboxId) {
    if !same_mail_session(ctx, account_id, mailbox_id) {
        return;
    }
    let role = ctx
        .mailbox_nodes
        .read()
        .get(mailbox_id)
        .map(|n| n.role)
        .unwrap_or(MailboxRole::Other);
    if crate::vacation::folder_skips_vacation(role) {
        return;
    }
    let mut settings = crate::vacation::load_settings(account_id);
    let now = Utc::now();
    if settings.enabled && settings.armed_at.is_none() {
        settings.armed_at = Some(now);
        crate::vacation::save_settings(account_id.clone(), settings.clone());
    }
    if !settings.is_active(now) {
        return;
    }
    let Some(account) = ctx.accounts.read().get(account_id).cloned() else {
        return;
    };
    let own: Vec<String> = account
        .all_identities()
        .into_iter()
        .map(|id| id.email)
        .collect();
    let period = settings.period_key();
    let replied = crate::vacation::load_replied(account_id, &period);
    let envelopes: Vec<mailiner_core::Envelope> = ctx
        .messages
        .read()
        .iter()
        .map(|m| m.envelope.clone())
        .collect();
    let hits = crate::vacation::plan_vacation_hits(&settings, now, &envelopes, &own, &replied);
    if hits.is_empty() {
        return;
    }

    let mut sent_to = Vec::new();
    for hit in hits {
        let Some(envelope) = envelopes.iter().find(|e| e.id.as_uid() == hit.uid) else {
            continue;
        };
        let identity = identity_from_stored(&identity_for_reply(
            &account,
            envelope.to.as_ref(),
            envelope.cc.as_ref(),
        ));
        let draft =
            crate::vacation::build_vacation_draft(&identity, &settings, envelope, &hit.sender);
        let prepared = match prepare_submit(&draft, &identity) {
            Ok(prepared) => prepared,
            Err(e) => {
                warn!("vacation auto-reply prepare failed for {}: {e}", hit.sender);
                continue;
            }
        };
        queue_core_event(CoreEvent::SendMessage {
            account_id: account_id.clone(),
            request: SubmitRequest {
                mail_from: prepared.envelope.mail_from,
                rcpt_to: prepared.envelope.rcpt_to,
                rfc822: prepared.rfc822,
                message_id: prepared.message_id,
                dsn: None,
            },
            display: OutboxDisplay {
                subject: draft.subject,
                to_preview: hit.sender.clone(),
            },
            draft_id: format!("vacation-{}", draft.id.as_str()),
            bcc_header: None,
            reply_source: Some(envelope.id.clone()),
            imap_draft: None,
        });
        sent_to.push(hit.sender);
    }
    crate::vacation::mark_replied(account_id, &period, &sent_to);
}

/// Apply local incoming-mail filters to loaded envelopes (not ManageSieve).
///
/// First matching enabled rule wins. Each UID is recorded after a successful
/// attempt so reopening the folder does not re-move. Sent/Drafts/Trash/Junk/
/// Outbox are skipped.
async fn apply_incoming_mail_rules(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: &AccountId,
    mailbox_id: &MailboxId,
) {
    if !same_mail_session(ctx, account_id, mailbox_id) {
        return;
    }
    let role = ctx
        .mailbox_nodes
        .read()
        .get(mailbox_id)
        .map(|n| n.role)
        .unwrap_or(MailboxRole::Other);
    if crate::mail_rules::folder_skips_rules(role) {
        return;
    }
    let rules = crate::mail_rules::load_rules(account_id);
    if rules.iter().all(|r| !r.enabled) {
        return;
    }
    let applied = crate::mail_rules::load_applied_uids(account_id, mailbox_id);
    let envelopes: Vec<mailiner_core::Envelope> = ctx
        .messages
        .read()
        .iter()
        .map(|m| m.envelope.clone())
        .collect();
    let hits = crate::mail_rules::plan_rule_hits(&rules, &envelopes, &applied);
    if hits.is_empty() {
        return;
    }
    let Some(connector) = manager.get(account_id) else {
        return;
    };

    let planned: Vec<(crate::mail_rules::MailRule, MessageId)> = {
        let list = ctx.messages.read();
        hits.iter()
            .filter_map(|hit| {
                let msg = list.find(|m| m.id.as_uid() == hit.uid)?;
                Some((hit.rule.clone(), msg.id.clone()))
            })
            .collect()
    };
    if planned.is_empty() {
        return;
    }

    let folder_id = FolderId::new(mailbox_id.to_string());
    let mut mark_read: Vec<MessageId> = Vec::new();
    let mut star: Vec<MessageId> = Vec::new();
    let mut flag: Vec<MessageId> = Vec::new();
    let mut keywords: HashMap<ImapKeyword, Vec<MessageId>> = HashMap::new();
    let mut moves: HashMap<String, Vec<MessageId>> = HashMap::new();
    let mut done: HashSet<String> = HashSet::new();

    for (rule, id) in &planned {
        if rule.action_mark_read {
            mark_read.push(id.clone());
        }
        if rule.action_star {
            star.push(id.clone());
        }
        if rule.action_flag {
            flag.push(id.clone());
        }
        if let Some(keyword) = rule.add_keyword() {
            keywords.entry(keyword).or_default().push(id.clone());
        }
        if let Some(dest) = rule.move_mailbox()
            && dest.as_str() != mailbox_id.as_str()
        {
            moves
                .entry(dest.as_str().to_string())
                .or_default()
                .push(id.clone());
        }
        done.insert(id.as_uid().to_string());
    }

    if let Err(e) = store_rule_flags(connector, &folder_id, &mark_read, EnvelopeFlag::Read).await {
        warn!("mail rule mark-read failed: {e}");
    } else if !mark_read.is_empty() {
        apply_read_flag(ctx, &mark_read, true);
    }
    if let Err(e) = store_rule_flags(connector, &folder_id, &star, EnvelopeFlag::Starred).await {
        warn!("mail rule star failed: {e}");
    } else if !star.is_empty() {
        apply_toggleable_flag(ctx, &star, EnvelopeFlag::Starred, true);
    }
    if let Err(e) = store_rule_flags(connector, &folder_id, &flag, EnvelopeFlag::Flagged).await {
        warn!("mail rule flag failed: {e}");
    } else if !flag.is_empty() {
        apply_toggleable_flag(ctx, &flag, EnvelopeFlag::Flagged, true);
    }
    for (keyword, ids) in keywords {
        if let Err(e) =
            store_rule_flags(connector, &folder_id, &ids, EnvelopeFlag::Keyword(keyword)).await
        {
            warn!("mail rule keyword {} failed: {e}", keyword.atom());
        } else {
            apply_toggleable_flag(ctx, &ids, EnvelopeFlag::Keyword(keyword), true);
        }
    }

    let mut moved_n = 0usize;
    let mut removed_sel = None;
    for (dest, ids) in moves {
        if ids.is_empty() {
            continue;
        }
        let dest_id = MailboxId::from(dest);
        let dest_exists = ctx.mailbox_nodes.read().contains_key(&dest_id);
        if !dest_exists {
            warn!(
                "mail rule dest {} missing; leaving {} message(s)",
                dest_id.as_str(),
                ids.len()
            );
            for id in &ids {
                done.remove(id.as_uid());
            }
            continue;
        }
        let dest_folder = FolderId::new(dest_id.to_string());
        let core_ids = core_message_ids(&ids);
        match connector
            .move_messages(&folder_id, &core_ids, &dest_folder)
            .await
        {
            Ok(_) => {
                if !same_mail_session(ctx, account_id, mailbox_id) {
                    invalidate_mailbox_messages(manager.cache(), account_id, mailbox_id).await;
                    invalidate_mailbox_messages(manager.cache(), account_id, &dest_id).await;
                    continue;
                }
                let unread_n = unread_among(ctx, &ids);
                let dest_is_all_mail = ctx
                    .mailbox_nodes
                    .read()
                    .get(&dest_id)
                    .is_some_and(|n| mailbox_is_all_mail(&dest_id, Some(n.name.as_str())));
                let (_, sel) = take_messages_from_ui(ctx, &ids);
                if removed_sel.is_none() {
                    removed_sel = sel;
                }
                if unread_n != 0 && !dest_is_all_mail {
                    bump_mailbox_unread(ctx, &dest_id, unread_n, false);
                }
                if let Some(node) = ctx.mailbox_nodes.write().get_mut(&dest_id) {
                    node.total_count = node.total_count.saturating_add(ids.len());
                }
                moved_n += ids.len();
                invalidate_mailbox_messages(manager.cache(), account_id, &dest_id).await;
            }
            Err(mailiner_core::MailinerError::PartialMove { message, .. }) => {
                error!("mail rule partial move: {message}");
            }
            Err(e) => {
                warn!("mail rule move to {} failed: {e}", dest_id.as_str());
                for id in &ids {
                    done.remove(id.as_uid());
                }
            }
        }
    }

    let applied_uids: Vec<String> = done.into_iter().collect();
    crate::mail_rules::mark_applied(account_id, mailbox_id, &applied_uids);

    if !same_mail_session(ctx, account_id, mailbox_id) {
        return;
    }
    if moved_n > 0 {
        apply_list_overlays(ctx);
        select_after_removed_row(manager, ctx, removed_sel).await;
        let label = if moved_n == 1 {
            "Filed 1 message".to_string()
        } else {
            format!("Filed {moved_n} messages")
        };
        ctx.show_toast(ToastAction::info(label));
    }
}

async fn store_rule_flags(
    connector: &dyn EmailConnector,
    folder_id: &FolderId,
    ids: &[MessageId],
    flag: EnvelopeFlag,
) -> Result<(), mailiner_core::MailinerError> {
    if ids.is_empty() {
        return Ok(());
    }
    connector
        .update_envelope_flags(folder_id, &core_message_ids(ids), &[(flag, true)])
        .await
}

fn apply_live_counts(
    ctx: &mut AppContext,
    account_id: &AccountId,
    mailbox_id: &MailboxId,
    folder_total: usize,
    unread: Option<usize>,
) {
    let ack = crate::ui_prefs::load_ack_unread(account_id)
        .get(mailbox_id)
        .copied()
        .unwrap_or(0);
    {
        let mut nodes = ctx.mailbox_nodes.write();
        if let Some(node) = nodes.get_mut(mailbox_id) {
            apply_live_folder_state(node, folder_total, unread, ack);
        }
    }
    if ctx
        .mailbox_nodes
        .read()
        .get(mailbox_id)
        .is_some_and(|n| n.role == MailboxRole::Inbox)
    {
        sync_inbox_unread(ctx, crate::notifications::InboxCountEvent::Remote);
    }
}

fn prune_selection_after_refresh(
    ctx: &mut AppContext,
    old_ids_in_range: &HashSet<MessageId>,
    new_ids: &HashSet<MessageId>,
) {
    let gone: HashSet<MessageId> = old_ids_in_range.difference(new_ids).cloned().collect();
    if !gone.is_empty() {
        let focus_gone = ctx
            .selection
            .read()
            .focus()
            .is_some_and(|id| gone.contains(id));
        ctx.selection.write().remove_ids(&gone);
        if focus_gone {
            ctx.message_view.set(MessageViewState::Empty);
            ctx.message_headers.set(MessageHeadersState::Closed);
            ctx.message_source.set(MessageSourceState::Closed);
        }
    }
    let focus = ctx.selection.read().focus().cloned();
    if let Some(id) = focus
        && let Some(idx) = ctx.messages.read().position(|m| m.id == id)
    {
        ctx.selection.write().note_focus(id, Some(idx));
    }
}

async fn refresh_other_folder_badges(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: &AccountId,
    skip: Option<&MailboxId>,
) {
    let Some(connector) = manager.get(account_id) else {
        return;
    };
    let rest: Vec<FolderId> = ctx
        .mailbox_nodes
        .read()
        .iter()
        .filter(|(id, node)| node.selectable && node.subscribed && skip.is_none_or(|s| s != *id))
        .map(|(id, _)| FolderId::new(id.to_string()))
        .collect();
    if rest.is_empty() {
        return;
    }
    let ack = crate::ui_prefs::load_ack_unread(account_id);
    for id in rest {
        match connector.folder_counts(std::slice::from_ref(&id)).await {
            Ok(counts) if !counts.is_empty() => {
                {
                    let mut nodes = ctx.mailbox_nodes.write();
                    crate::mailbox::apply_folder_counts(&mut nodes, &counts);
                    crate::mailbox::apply_unread_new_state(&mut nodes, &counts, &ack);
                }
                observe_remote_counts(ctx, &counts);
            }
            Ok(_) => {}
            Err(e) => {
                warn!("folder_counts {} failed: {}", id, e);
                manager.note_imap_error(account_id, &e);
            }
        }
    }
    persist_folder_tree(manager.cache(), ctx, account_id).await;
}

async fn handle_set_message_sort(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    sort: MessageSort,
) {
    crate::ui_prefs::save_message_sort(sort);
    ctx.message_sort.set(sort);
    let Some(mailbox_id) = ctx.selected_mailbox.read().clone() else {
        return;
    };
    // Drop the previous sort's rows so we don't treat them as a cache hit.
    ctx.messages.set(SparseList::new(0));
    ctx.messages_loading.set(true);
    handle_select_mailbox(manager, ctx, mailbox_id, true).await;
}

async fn handle_toggle_message_list_filter(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    unread: bool,
    flagged: bool,
    has_attachment: bool,
) {
    let prev = *ctx.message_list_filter.peek();
    let mut filter = prev;
    if unread {
        filter.unread = !filter.unread;
    }
    if flagged {
        filter.flagged = !filter.flagged;
    }
    if has_attachment {
        filter.has_attachment = !filter.has_attachment;
    }
    crate::ui_prefs::save_message_list_filter(filter);
    ctx.message_list_filter.set(filter);
    if prev.imap_search_query() == filter.imap_search_query() {
        return;
    }
    let Some(mailbox_id) = ctx.selected_mailbox.read().clone() else {
        return;
    };
    // Drop the previous SEARCH result so we don't treat it as a cache hit.
    ctx.messages.set(SparseList::new(0));
    ctx.messages_loading.set(true);
    handle_select_mailbox(manager, ctx, mailbox_id, true).await;
}

async fn handle_apply_mailbox_search(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    query: String,
) {
    let query = query.trim().to_string();
    ctx.list_text_filter.set(query.clone());
    ctx.active_saved_search.set(None);
    if ctx.list_search_query.peek().as_str() == query {
        return;
    }
    ctx.list_search_query.set(query);
    let Some(mailbox_id) = ctx.selected_mailbox.read().clone() else {
        return;
    };
    ctx.messages.set(SparseList::new(0));
    ctx.messages_loading.set(true);
    select_mailbox(manager, ctx, mailbox_id, true, false).await;
}

async fn handle_save_mailbox_search(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    name: String,
    query: String,
) {
    let Some(account_id) = ctx.selected_account.read().clone() else {
        ctx.show_toast(ToastAction::error("No account selected"));
        return;
    };
    let Some(mailbox_id) = ctx.selected_mailbox.read().clone() else {
        ctx.show_toast(ToastAction::error("No folder selected"));
        return;
    };
    match crate::ui_prefs::add_saved_search(&name, &query, account_id, &mailbox_id) {
        Ok(saved) => {
            ctx.saved_searches
                .set(crate::ui_prefs::load_saved_searches());
            ctx.active_saved_search.set(Some(saved.id.clone()));
            ctx.show_toast(ToastAction::info(format!("Saved \"{}\"", saved.name)));
            if ctx.list_search_query.peek().as_str() != saved.query {
                handle_apply_mailbox_search(manager, ctx, saved.query).await;
                ctx.active_saved_search.set(Some(saved.id));
            }
        }
        Err(crate::ui_prefs::SaveSearchError::EmptyQuery) => {
            ctx.show_toast(ToastAction::error("Nothing to save"));
        }
    }
}

async fn handle_open_saved_search(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    id: String,
) {
    let Some(search) = ctx
        .saved_searches
        .peek()
        .iter()
        .find(|s| s.id == id)
        .cloned()
    else {
        ctx.show_toast(ToastAction::error("Saved search not found"));
        return;
    };
    let mailbox_id = search.mailbox();
    if ctx
        .mailbox_nodes
        .read()
        .get(&mailbox_id)
        .is_none_or(|n| !n.selectable)
    {
        ctx.show_toast(ToastAction::error(format!(
            "Folder for \"{}\" is no longer available",
            search.name
        )));
        return;
    }
    ctx.active_saved_search.set(Some(search.id.clone()));
    ctx.list_text_filter.set(search.query.clone());
    if ctx.list_search_query.peek().as_str() == search.query
        && ctx.selected_mailbox.peek().as_ref() == Some(&mailbox_id)
    {
        return;
    }
    ctx.list_search_query.set(search.query);
    ctx.messages.set(SparseList::new(0));
    ctx.messages_loading.set(true);
    select_mailbox(manager, ctx, mailbox_id, true, true).await;
}

fn handle_rename_saved_search(ctx: &mut AppContext, id: String, name: String) {
    if crate::ui_prefs::rename_saved_search(&id, &name).is_none() {
        ctx.show_toast(ToastAction::error("Saved search not found"));
        return;
    }
    ctx.saved_searches
        .set(crate::ui_prefs::load_saved_searches());
}

fn handle_delete_saved_search(ctx: &mut AppContext, id: String) {
    if !crate::ui_prefs::remove_saved_search(&id) {
        ctx.show_toast(ToastAction::error("Saved search not found"));
        return;
    }
    if ctx.active_saved_search.peek().as_deref() == Some(id.as_str()) {
        ctx.active_saved_search.set(None);
    }
    ctx.saved_searches
        .set(crate::ui_prefs::load_saved_searches());
}

async fn handle_fetch_message_range(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    mailbox_id: MailboxId,
    range: Range<usize>,
) {
    if is_unified_mailbox(&mailbox_id) {
        return;
    }
    if ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id) {
        return;
    }
    let total = ctx.messages.read().total_count();
    let start = range.start.min(total);
    let end = range.end.min(total);
    if start >= end {
        return;
    }
    let range = start..end;

    let already = {
        let messages = ctx.messages.read();
        (range.start..range.end).all(|i| messages.has_item(i))
    };
    if already {
        return;
    }

    let Some(account_id) = ctx.selected_account.read().clone() else {
        return;
    };
    let Some(connector) = manager.get(&account_id) else {
        error!("FetchMessageRange: no connector for {}", account_id);
        return;
    };

    let folder_id = FolderId::new(mailbox_id.to_string());
    info!(
        "Fetching messages {}..{} for {}",
        range.start,
        range.end,
        mailbox_id.to_string()
    );
    match connector
        .list_envelopes_range(&folder_id, range.clone())
        .await
    {
        Ok(envelopes) => {
            if ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id)
                || !selected_account_is(ctx, &account_id)
            {
                return;
            }
            let prefix_before = contiguous_loaded_prefix_len(&ctx.messages.read());
            let batch = messages_from_envelopes(envelopes, &ctx.messages.read());
            ctx.messages.write().insert_batch(range.start, batch);
            apply_list_overlays(ctx);
            // Only rewrite localStorage when the contiguous cached prefix grew.
            let prefix_grew = contiguous_loaded_prefix_len(&ctx.messages.read()) > prefix_before;
            if prefix_grew {
                persist_selected_messages(manager.cache(), ctx, &account_id).await;
            }
            fetch_and_apply_snippets(manager, ctx, &account_id, &mailbox_id, prefix_grew).await;
        }
        Err(e) => {
            error!(
                "Failed to fetch message range {}..{}: {}",
                range.start, range.end, e
            );
            note_selected_imap_error(manager, ctx, &e);
        }
    }
}

fn current_list_index(ctx: &AppContext) -> Option<usize> {
    ctx.selection.read().focus_at_index().or_else(|| {
        ctx.selection
            .read()
            .focus()
            .cloned()
            .and_then(|id| ctx.messages.read().position(|m| m.id == id))
    })
}

fn unread_scan_start(ctx: &AppContext, delta: i32) -> Option<usize> {
    let stored = ctx.selection.read().focus_at_index();
    let live = ctx
        .selection
        .read()
        .focus()
        .cloned()
        .and_then(|id| ctx.messages.read().position(|m| m.id == id));
    unread_scan_from(stored, live, delta)
}

/// Same window `select_list_index` fetches when a keyboard move lands on a hole.
fn adjacent_fetch_range(index: usize, total: usize) -> Range<usize> {
    index.saturating_sub(5)..(index + 15).min(total)
}

fn conversation_rows_from_ctx(ctx: &AppContext) -> Vec<ConversationRow> {
    let filter = *ctx.message_list_filter.peek();
    let loaded: Vec<Arc<Message>> = ctx
        .messages
        .read()
        .iter()
        .filter(|m| !filter.has_attachment || message_matches_filter(m, filter))
        .cloned()
        .collect();
    let conversations = group_conversations(loaded, ctx.pinned_uids.peek().as_slice());
    flatten_conversations(&conversations, &ctx.expanded_conversations.peek())
}

async fn handle_select_adjacent_conversation(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    delta: i32,
    extend: bool,
) {
    let rows = conversation_rows_from_ctx(ctx);
    if rows.is_empty() {
        return;
    }
    let current = ctx
        .selection
        .read()
        .focus()
        .and_then(|id| row_index_for_message(&rows, id));
    let Some(index) = adjacent_index(rows.len(), current, delta) else {
        return;
    };
    let message_id = rows[index].select_target().id.clone();
    if extend {
        let anchor = ctx
            .selection
            .read()
            .anchor_index()
            .and_then(|i| ctx.messages.read().get(i).map(|m| m.id.clone()))
            .or_else(|| ctx.selection.read().focus().cloned())
            .and_then(|id| row_index_for_message(&rows, &id))
            .unwrap_or(index);
        let (lo, hi) = if anchor <= index {
            (anchor, index)
        } else {
            (index, anchor)
        };
        let ids: Vec<MessageId> = rows[lo..=hi]
            .iter()
            .flat_map(ConversationRow::selected_ids)
            .collect();
        let source_index = ctx.messages.read().position(|m| m.id == message_id);
        ctx.selection
            .write()
            .set_range(ids, message_id.clone(), source_index);
        snapshot_selection_unread(ctx);
        handle_select_message(manager, ctx, message_id, false, false).await;
        return;
    }
    handle_select_message(manager, ctx, message_id, true, true).await;
}

async fn handle_select_adjacent_unread_conversation(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    delta: i32,
) {
    let rows = conversation_rows_from_ctx(ctx);
    if rows.is_empty() {
        ctx.show_toast(ToastAction::info(if delta > 0 {
            "No next unread message"
        } else {
            "No previous unread message"
        }));
        return;
    }
    let current = ctx
        .selection
        .read()
        .focus()
        .and_then(|id| row_index_for_message(&rows, id));
    let mut from = current;
    loop {
        let Some(index) = adjacent_index(rows.len(), from, delta) else {
            ctx.show_toast(ToastAction::info(if delta > 0 {
                "No next unread message"
            } else {
                "No previous unread message"
            }));
            return;
        };
        if rows[index].is_unread() {
            let message_id = rows[index].select_target().id.clone();
            handle_select_message(manager, ctx, message_id, true, true).await;
            return;
        }
        from = Some(index);
    }
}

async fn handle_select_adjacent(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    delta: i32,
    extend: bool,
) {
    if ctx.selected_mailbox.peek().is_none() {
        return;
    }
    if *ctx.messages_loading.peek() {
        return;
    }
    if *ctx.message_list_view.peek() == MessageListView::Conversations {
        handle_select_adjacent_conversation(manager, ctx, delta, extend).await;
        return;
    }
    // Server SEARCH already narrowed the list; only attachment stays client-side.
    let total = ctx.messages.read().total_count();
    let filter = *ctx.message_list_filter.peek();
    let current = current_list_index(ctx);
    let Some(mut index) = adjacent_index(total, current, delta) else {
        return;
    };
    if !filter.is_empty() {
        loop {
            if ctx.messages.read().get(index).is_none() {
                let Some(mailbox_id) = ctx.selected_mailbox.read().clone() else {
                    return;
                };
                let start = index.saturating_sub(5);
                let end = (index + 15).min(total);
                handle_fetch_message_range(manager, ctx, mailbox_id, start..end).await;
                if ctx.messages.read().get(index).is_none() {
                    return;
                }
            }
            let matches = ctx
                .messages
                .read()
                .get(index)
                .is_some_and(|m| filter.matches(m.is_read, m.is_flagged, m.has_attachments));
            if matches {
                break;
            }
            let Some(next) = adjacent_index(total, Some(index), delta) else {
                return;
            };
            index = next;
        }
    }
    if extend {
        apply_index_range_selection(manager, ctx, index).await;
    }
    select_list_index(manager, ctx, index, !extend, !extend).await;
}

enum KnownSelect {
    All,
    Unread,
    Invert,
}

/// Select / invert over cached list rows only (virtual list may have holes).
async fn handle_select_known(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    mode: KnownSelect,
) {
    if ctx.selected_mailbox.peek().is_none() {
        return;
    }
    if *ctx.messages_loading.peek() {
        return;
    }

    let rows: Vec<(MessageId, bool)> = matching_source_unread(ctx);
    if rows.is_empty() {
        return;
    }

    {
        let mut sel = ctx.selection.write();
        match mode {
            KnownSelect::All => {
                sel.select_all(rows.iter().map(|(id, _)| id.clone()));
            }
            KnownSelect::Unread => {
                sel.select_unread(
                    rows.iter()
                        .filter(|(_, unread)| *unread)
                        .map(|(id, _)| id.clone()),
                );
            }
            KnownSelect::Invert => {
                sel.invert(rows.iter().map(|(id, _)| id.clone()));
            }
        }
    }

    let Some(focus) = ctx.selection.read().focus().cloned() else {
        ctx.message_view.set(MessageViewState::Empty);
        ctx.clear_nested_rfc822();
        ctx.download_status.set(HashMap::new());
        return;
    };
    // Multi-select must not consume unread (`should_auto_mark_read`).
    handle_select_message(manager, ctx, focus, false, false).await;
    // `note_focus` has the current (post-relocate) index; use it as the range start.
    ctx.selection.write().reset_range_anchor();
}

async fn handle_select_adjacent_unread(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    delta: i32,
) {
    if ctx.selected_mailbox.peek().is_none() {
        return;
    }
    if *ctx.messages_loading.peek() {
        return;
    }
    if *ctx.message_list_view.peek() == MessageListView::Conversations {
        handle_select_adjacent_unread_conversation(manager, ctx, delta).await;
        return;
    }
    let mut from = unread_scan_start(ctx, delta);
    loop {
        let total = ctx.messages.read().total_count();
        let scan = {
            let messages = ctx.messages.read();
            next_unread_index(total, from, delta, |i| messages.get(i).map(|m| !m.is_read))
        };
        match scan {
            UnreadScan::Found(index) => {
                select_list_index(manager, ctx, index, true, true).await;
                return;
            }
            UnreadScan::Hole(index) => {
                let Some(mailbox_id) = ctx.selected_mailbox.read().clone() else {
                    return;
                };
                let range = adjacent_fetch_range(index, total);
                handle_fetch_message_range(manager, ctx, mailbox_id, range).await;
                if ctx.messages.read().has_item(index) {
                    from = unread_scan_resume(index, delta);
                } else {
                    // Advance past this hole so a failed fetch cannot stall.
                    from = Some(index);
                }
            }
            UnreadScan::None => {
                let message = if delta > 0 {
                    "No next unread message"
                } else {
                    "No previous unread message"
                };
                ctx.show_toast(ToastAction::info(message));
                return;
            }
        }
    }
}

async fn handle_select_list_click(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    message_id: MessageId,
    index: usize,
    extend: bool,
    toggle: bool,
) {
    if *ctx.message_list_view.peek() == MessageListView::Conversations && extend {
        let rows = conversation_rows_from_ctx(ctx);
        let Some(end) = row_index_for_message(&rows, &message_id) else {
            handle_select_message(manager, ctx, message_id, false, false).await;
            return;
        };
        let anchor = ctx
            .selection
            .read()
            .anchor_index()
            .and_then(|i| ctx.messages.read().get(i).map(|m| m.id.clone()))
            .or_else(|| ctx.selection.read().focus().cloned())
            .and_then(|id| row_index_for_message(&rows, &id))
            .unwrap_or(end);
        let (lo, hi) = if anchor <= end {
            (anchor, end)
        } else {
            (end, anchor)
        };
        let ids: Vec<MessageId> = rows[lo..=hi]
            .iter()
            .flat_map(ConversationRow::selected_ids)
            .collect();
        ctx.selection
            .write()
            .set_range(ids, message_id.clone(), Some(index));
        snapshot_selection_unread(ctx);
        handle_select_message(manager, ctx, message_id, false, false).await;
        return;
    }
    if extend {
        apply_index_range_selection(manager, ctx, index).await;
        handle_select_message(manager, ctx, message_id, false, false).await;
        return;
    }
    if toggle {
        ctx.selection
            .write()
            .toggle(message_id.clone(), Some(index));
        snapshot_selection_unread(ctx);
        let Some(focus) = ctx.selection.read().focus().cloned() else {
            return;
        };
        handle_select_message(manager, ctx, focus, false, false).await;
        return;
    }
    handle_select_message(manager, ctx, message_id, true, true).await;
}

async fn apply_index_range_selection(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    end_index: usize,
) {
    let anchor = ctx.selection.read().anchor_index().unwrap_or(end_index);
    let start = anchor.min(end_index);
    let end = anchor.max(end_index);
    let mailbox_id = ctx.selected_mailbox.read().clone();
    if let Some(mailbox_id) = mailbox_id {
        let missing = ctx.messages.read().missing_ranges(start, end + 1);
        for range in missing {
            handle_fetch_message_range(manager, ctx, mailbox_id.clone(), range).await;
        }
    }
    let ids = cached_ids_in_range(ctx, start, end);
    let focus = ctx.messages.read().get(end_index).map(|m| m.id.clone());
    if let Some(focus) = focus {
        ctx.selection.write().set_range(ids, focus, Some(end_index));
        snapshot_selection_unread(ctx);
    }
}

fn matching_source_unread(ctx: &AppContext) -> Vec<(MessageId, bool)> {
    ctx.messages
        .read()
        .iter()
        .map(|m| (m.id.clone(), !m.is_read))
        .collect()
}

fn cached_ids_in_range(ctx: &AppContext, start: usize, end_inclusive: usize) -> Vec<MessageId> {
    let list = ctx.messages.read();
    (start..=end_inclusive)
        .filter_map(|i| list.get(i).map(|m| m.id.clone()))
        .collect()
}

async fn select_list_index(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    index: usize,
    replace_selection: bool,
    auto_mark: bool,
) {
    let total = ctx.messages.read().total_count();
    if ctx.messages.read().get(index).is_none() {
        let Some(mailbox_id) = ctx.selected_mailbox.read().clone() else {
            return;
        };
        handle_fetch_message_range(manager, ctx, mailbox_id, adjacent_fetch_range(index, total))
            .await;
    }
    let Some(message_id) = ctx.messages.read().get(index).map(|m| m.id.clone()) else {
        return;
    };
    handle_select_message(manager, ctx, message_id, auto_mark, replace_selection).await;
}

async fn select_after_removed_row(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    removed_index: Option<usize>,
) {
    select_after_removed_row_mark(manager, ctx, removed_index, true).await;
}

async fn select_after_removed_row_mark(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    removed_index: Option<usize>,
    auto_mark: bool,
) {
    let Some(removed_index) = removed_index else {
        return;
    };
    let filter = *ctx.message_list_filter.peek();
    let total = ctx.messages.read().total_count();
    let index = if filter.is_empty() {
        index_after_removal(total, removed_index)
    } else {
        next_cached_filter_index(ctx, removed_index, filter)
    };
    let Some(index) = index else {
        return;
    };
    select_list_index(manager, ctx, index, true, auto_mark).await;
}

fn next_cached_filter_index(
    ctx: &AppContext,
    removed_index: usize,
    filter: MessageListFilter,
) -> Option<usize> {
    let list = ctx.messages.read();
    let total = list.total_count();
    let matches = |i: usize| {
        list.get(i)
            .is_some_and(|m| filter.matches(m.is_read, m.is_flagged, m.has_attachments))
    };
    (removed_index..total)
        .find(|&i| matches(i))
        .or_else(|| (0..removed_index).rev().find(|&i| matches(i)))
}

fn neighbor_ids(ctx: &AppContext, focus: &MessageId) -> Vec<MessageId> {
    let list = ctx.messages.read();
    let Some(index) = list.position(|m| m.id == *focus) else {
        return Vec::new();
    };
    adjacent_neighbor_indices(index, list.total_count())
        .into_iter()
        .flatten()
        .filter_map(|i| list.get(i).map(|m| m.id.clone()))
        .collect()
}

fn prefetch_job_stale(ctx: &AppContext, job: &PrefetchJob) -> bool {
    ctx.selection.read().focus() != Some(&job.around)
        || ctx.selected_mailbox.read().as_ref() != Some(&job.mailbox_id)
        || ctx.selected_account.read().as_ref() != Some(&job.account_id)
        || neighbor_ids(ctx, &job.around) != job.neighbors
}

fn queue_adjacent_prefetch(ctx: &AppContext, pending: &mut Option<PrefetchJob>) {
    if is_unified_selected(ctx) {
        *pending = None;
        return;
    }
    let MessageViewState::Ready { message_id, .. } = &*ctx.message_view.read() else {
        *pending = None;
        return;
    };
    if pending
        .as_ref()
        .is_some_and(|job| &job.around == message_id && !prefetch_job_stale(ctx, job))
    {
        return;
    }
    let Some(account_id) = ctx.selected_account.read().clone() else {
        *pending = None;
        return;
    };
    let Some(mailbox_id) = ctx.selected_mailbox.read().clone() else {
        *pending = None;
        return;
    };
    let neighbors = neighbor_ids(ctx, message_id);
    let remaining: Vec<MessageId> = neighbors
        .iter()
        .filter(|id| !ctx.message_bodies.borrow().contains(*id))
        .cloned()
        .collect();
    if remaining.is_empty() {
        *pending = None;
        return;
    }
    *pending = Some(PrefetchJob {
        around: message_id.clone(),
        mailbox_id,
        account_id,
        remaining,
        neighbors,
    });
}

/// Load one neighbor via BODY.PEEK. Does not update the viewer or `\Seen`.
///
/// Returns `true` when the caller should loop (work done or skipped) instead of
/// waiting for the next UI event.
async fn run_one_prefetch(
    manager: &AccountConnectionManager,
    ctx: &AppContext,
    pending: &mut Option<PrefetchJob>,
) -> bool {
    let Some(job) = pending.as_mut() else {
        return false;
    };
    if prefetch_job_stale(ctx, job) {
        *pending = None;
        return false;
    }
    if job.remaining.is_empty() {
        *pending = None;
        return false;
    }
    let id = job.remaining.remove(0);
    if ctx.message_bodies.borrow().contains(&id) {
        return true;
    }
    let Some(connector) = manager.get(&job.account_id) else {
        *pending = None;
        return false;
    };
    let folder_id = FolderId::new(job.mailbox_id.to_string());
    match load_message(connector, &folder_id, &id).await {
        Ok(loaded) => {
            let loaded = Arc::new(loaded);
            persist_loaded_parts(manager.cache(), &job.account_id, &loaded).await;
            ctx.message_bodies.borrow_mut().insert(id, loaded);
        }
        Err(e) => {
            warn!("prefetch {id} failed: {e}");
        }
    }
    true
}

async fn handle_select_message(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    message_id: MessageId,
    auto_mark_read: bool,
    replace_selection: bool,
) {
    let index = ctx.messages.read().position(|m| m.id == message_id);
    if replace_selection {
        ctx.selection.write().replace(message_id.clone(), index);
    } else {
        ctx.selection.write().note_focus(message_id.clone(), index);
    }
    snapshot_selection_unread(ctx);
    ctx.download_status.set(HashMap::new());
    ctx.message_headers.set(MessageHeadersState::Closed);
    ctx.message_source.set(MessageSourceState::Closed);
    ctx.clear_nested_rfc822();

    let target = open_target_for(ctx, &message_id);
    let cached = ctx.message_bodies.borrow_mut().get(&message_id);
    if let Some(loaded) = cached {
        let Some(account_id) = target
            .as_ref()
            .map(|t| t.account_id.clone())
            .or_else(|| ctx.selected_account.read().clone())
        else {
            ctx.message_view.set(MessageViewState::Error {
                message_id: message_id.clone(),
                message: "No account selected".into(),
            });
            return;
        };
        ctx.message_view.set(MessageViewState::Ready {
            account_id,
            message_id: message_id.clone(),
            loaded,
        });
        maybe_auto_mark_read(manager, ctx, &message_id, auto_mark_read).await;
        return;
    }

    let Some(target) = target.or_else(|| {
        let mailbox_id = ctx.selected_mailbox.read().clone()?;
        if is_unified_mailbox(&mailbox_id) {
            return None;
        }
        Some(crate::unified_inbox::OpenTarget {
            account_id: ctx.selected_account.read().clone()?,
            mailbox_id,
            message_id: message_id.clone(),
        })
    }) else {
        ctx.message_view.set(MessageViewState::Error {
            message_id: message_id.clone(),
            message: "No mailbox selected".into(),
        });
        return;
    };
    let account_id = target.account_id.clone();
    let mailbox_id = target.mailbox_id.clone();

    let persisted = load_cached_loaded_message(manager.cache(), &account_id, &message_id).await;
    if let Some(loaded) = persisted.clone() {
        let loaded = Arc::new(loaded);
        ctx.message_bodies
            .borrow_mut()
            .insert(message_id.clone(), loaded.clone());
        ctx.message_view.set(MessageViewState::Ready {
            account_id: account_id.clone(),
            message_id: message_id.clone(),
            loaded,
        });
        if manager.get(&account_id).is_none() {
            return;
        }
        maybe_auto_mark_read(manager, ctx, &message_id, auto_mark_read).await;
    } else {
        ctx.clear_attachment_downloads();
        ctx.message_view.set(MessageViewState::Loading {
            message_id: message_id.clone(),
        });
    }

    let Some(connector) = manager.get(&account_id) else {
        if persisted.is_none() {
            ctx.message_view.set(MessageViewState::Error {
                message_id: message_id.clone(),
                message: "Not connected".into(),
            });
        }
        return;
    };

    let folder_id = FolderId::new(mailbox_id.to_string());
    info!(
        "Loading message {} in {} ({})",
        message_id,
        mailbox_id.to_string(),
        account_id
    );

    match load_message(connector, &folder_id, &message_id).await {
        Ok(loaded) => {
            let loaded = Arc::new(loaded);
            persist_loaded_parts(manager.cache(), &account_id, &loaded).await;
            ctx.message_bodies
                .borrow_mut()
                .insert(message_id.clone(), loaded.clone());
            if ctx.selection.read().focus() != Some(&message_id) {
                return;
            }
            if !is_unified_selected(ctx) && !selected_account_is(ctx, &account_id) {
                return;
            }
            ctx.message_view.set(MessageViewState::Ready {
                account_id: account_id.clone(),
                message_id: message_id.clone(),
                loaded,
            });
            maybe_auto_mark_read(manager, ctx, &message_id, auto_mark_read).await;
        }
        Err(e) => {
            if persisted.is_some() {
                return;
            }
            if ctx.selection.read().focus() != Some(&message_id) {
                return;
            }
            error!("Failed to load message {}: {}", message_id, e);
            note_selected_imap_error(manager, ctx, &e);
            ctx.message_view.set(MessageViewState::Error {
                message_id,
                message: e.to_string(),
            });
        }
    }
}

fn snapshot_selection_unread(ctx: &mut AppContext) {
    let list = ctx.messages.read();
    let mut sel = ctx.selection.write();
    for id in sel.ids_vec() {
        if let Some(m) = list.find(|m| m.id == id) {
            sel.note_unread(&id, !m.is_read);
        }
    }
}

fn unread_in_ids(ctx: &AppContext, ids: &[MessageId]) -> usize {
    let from_sel = ctx.selection.read().unread_among(ids);
    let list = ctx.messages.read();
    let from_list = ids
        .iter()
        .filter(|id| list.find(|m| m.id == **id).is_some_and(|m| !m.is_read))
        .count();
    from_sel.max(from_list)
}

async fn maybe_auto_mark_read(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    message_id: &MessageId,
    auto_mark_read: bool,
) {
    let Some(target) = open_target_for(ctx, message_id).or_else(|| {
        let mailbox_id = ctx.selected_mailbox.read().clone()?;
        if is_unified_mailbox(&mailbox_id) {
            return None;
        }
        Some(crate::unified_inbox::OpenTarget {
            account_id: ctx.selected_account.read().clone()?,
            mailbox_id,
            message_id: message_id.clone(),
        })
    }) else {
        return;
    };
    let account_id = target.account_id;
    let mailbox_id = target.mailbox_id;
    let Some(connector) = manager.get(&account_id) else {
        return;
    };
    let was_unread = ctx
        .messages
        .read()
        .find(|m| m.id == *message_id)
        .is_some_and(|m| !m.is_read);
    let is_multi = ctx.selection.read().is_multi();
    if !was_unread || !crate::selection::should_auto_mark_read(auto_mark_read, is_multi) {
        return;
    }
    let folder_id = FolderId::new(mailbox_id.to_string());
    apply_read_flag(ctx, std::slice::from_ref(message_id), true);
    if let Err(e) = connector
        .update_envelope_flags(
            &folder_id,
            std::slice::from_ref(message_id),
            &[(EnvelopeFlag::Read, true)],
        )
        .await
    {
        warn!("Auto-mark as read failed for {}: {}", message_id, e);
        note_selected_imap_error(manager, ctx, &e);
        apply_read_flag(ctx, std::slice::from_ref(message_id), false);
    } else {
        relocate_unread_sort_rows(connector, ctx, std::slice::from_ref(message_id), true).await;
        persist_selected_messages(manager.cache(), ctx, &account_id).await;
        persist_folder_tree(manager.cache(), ctx, &account_id).await;
    }
}

fn apply_read_flag(ctx: &mut AppContext, ids: &[MessageId], is_read: bool) {
    {
        let mut sel = ctx.selection.write();
        for id in ids {
            sel.note_unread(id, !is_read);
        }
    }
    let idset: std::collections::HashSet<&MessageId> = ids.iter().collect();
    let mut unread_delta: i32 = 0;
    let mut per_account: HashMap<AccountId, i32> = HashMap::new();
    for msg in ctx.messages.write().iter_mut() {
        if idset.contains(&msg.id) && msg.is_read != is_read {
            let account_id = msg.envelope.account_id.clone();
            let mut next = (**msg).clone();
            next.is_read = is_read;
            next.envelope.is_read = is_read;
            *msg = Arc::new(next);
            let step = if is_read { -1 } else { 1 };
            unread_delta += step;
            *per_account.entry(account_id).or_insert(0) += step;
        }
    }
    if unread_delta != 0 {
        if per_account.is_empty() {
            let account_id = ctx.selected_account.read().clone();
            if let Some(account_id) = account_id {
                bump_account_inbox_unread(ctx, &account_id, unread_delta);
            }
        } else {
            for (account_id, delta) in per_account {
                bump_account_inbox_unread(ctx, &account_id, delta);
            }
        }
        let mailbox_id = ctx.selected_mailbox.read().clone();
        if let Some(mailbox_id) = mailbox_id
            && !is_unified_mailbox(&mailbox_id)
        {
            bump_mailbox_unread(ctx, &mailbox_id, unread_delta, true);
        }
    }
}

fn acknowledge_mailbox_open(
    ctx: &mut AppContext,
    account_id: &AccountId,
    mailbox_id: &MailboxId,
    total: usize,
    unread: Option<usize>,
) {
    let unread = {
        let mut nodes = ctx.mailbox_nodes.write();
        let Some(node) = nodes.get_mut(mailbox_id) else {
            return;
        };
        node.total_count = total;
        if let Some(unread) = unread {
            node.unread_count = unread;
        }
        node.has_new = false;
        node.unread_count
    };
    crate::ui_prefs::save_ack_unread(account_id, mailbox_id, unread);
    observe_local_mailbox(ctx, mailbox_id);
}

fn bump_mailbox_unread(
    ctx: &mut AppContext,
    mailbox_id: &MailboxId,
    delta: i32,
    acknowledge: bool,
) {
    if delta == 0 {
        return;
    }
    let account_id = ctx.selected_account.read().clone();
    let unread = {
        let mut nodes = ctx.mailbox_nodes.write();
        let Some(node) = nodes.get_mut(mailbox_id) else {
            return;
        };
        let next = (node.unread_count as i32 + delta).max(0) as usize;
        node.unread_count = next;
        if acknowledge {
            node.has_new = false;
        } else if let Some(account_id) = account_id.as_ref() {
            let ack = crate::ui_prefs::load_ack_unread(account_id)
                .get(mailbox_id)
                .copied()
                .unwrap_or(0);
            node.has_new = crate::mailbox::unread_badge_is_new(next, ack);
        } else {
            node.has_new = next > 0;
        }
        next
    };
    if acknowledge {
        if let Some(account_id) = account_id.as_ref() {
            crate::ui_prefs::save_ack_unread(account_id, mailbox_id, unread);
        }
    }
    observe_local_mailbox(ctx, mailbox_id);
}

fn observe_remote_counts(ctx: &AppContext, counts: &HashMap<FolderId, FolderCounts>) {
    let Some((inbox_id, _)) = crate::notifications::inbox_unread(&ctx.mailbox_nodes.read()) else {
        return;
    };
    if !counts.contains_key(&FolderId::new(inbox_id.to_string())) {
        return;
    }
    sync_inbox_unread(ctx, crate::notifications::InboxCountEvent::Remote);
}

fn observe_local_mailbox(ctx: &AppContext, mailbox_id: &MailboxId) {
    let is_inbox = ctx
        .mailbox_nodes
        .read()
        .get(mailbox_id)
        .is_some_and(|n| n.role == MailboxRole::Inbox);
    if is_inbox {
        sync_inbox_unread(ctx, crate::notifications::InboxCountEvent::Local);
    }
}

fn sync_inbox_unread(ctx: &AppContext, event: crate::notifications::InboxCountEvent) {
    let Some(account_id) = ctx.selected_account.read().clone() else {
        return;
    };
    let Some((inbox_id, unread)) = crate::notifications::inbox_unread(&ctx.mailbox_nodes.read())
    else {
        return;
    };
    set_account_inbox_unread(ctx, &account_id, unread as u64);
    let ack = crate::ui_prefs::load_ack_unread(&account_id)
        .get(&inbox_id)
        .copied()
        .unwrap_or(0);
    let Some(added) =
        crate::notifications::observe_inbox_unread(&account_id, &inbox_id, unread, ack, event)
    else {
        return;
    };
    ctx.announce(crate::notifications::notification_body(added));
    let viewing_inbox = ctx.selected_mailbox.read().as_ref() == Some(&inbox_id);
    if crate::notifications::should_show_desktop_notification(
        *ctx.notify_inbox.peek(),
        crate::notifications::current_permission(),
        crate::notifications::is_document_hidden(),
        viewing_inbox,
    ) {
        crate::notifications::show_inbox_notification(added);
    }
}

fn unread_in_removed(snapshots: &[RemovedMessage]) -> i32 {
    snapshots.iter().filter(|s| !s.message.is_read).count() as i32
}

fn unread_among(ctx: &AppContext, ids: &[MessageId]) -> i32 {
    let list = ctx.messages.read();
    ids.iter()
        .filter(|id| list.find(|m| &m.id == *id).is_some_and(|m| !m.is_read))
        .count() as i32
}

fn mailbox_is_all_mail(id: &MailboxId, name: Option<&str>) -> bool {
    if name.is_some_and(|n| n.eq_ignore_ascii_case("all mail")) {
        return true;
    }
    // Gmail uses `/`. Do not treat `.` as hierarchy — `Reports.All Mail` is a name.
    id.as_str()
        .rsplit_once('/')
        .map(|(_, leaf)| leaf)
        .unwrap_or(id.as_str())
        .eq_ignore_ascii_case("all mail")
}

/// Remove `ids` from the current list. If the selected row is among them,
/// returns its pre-removal index so the caller can select the next remaining row.
fn take_messages_from_ui(
    ctx: &mut AppContext,
    ids: &[MessageId],
) -> (Vec<RemovedMessage>, Option<usize>) {
    let idset: std::collections::HashSet<MessageId> = ids.iter().cloned().collect();
    let focus = ctx.selection.read().focus().cloned();
    let selected_removed_index = focus.as_ref().and_then(|id| {
        if !idset.contains(id) {
            return None;
        }
        // Prefer the index from when the row was focused. Unread-first
        // auto-mark-read relocates the message into the read section, which
        // would otherwise make "next" a random later read row.
        ctx.selection
            .read()
            .focus_at_index()
            .or_else(|| ctx.messages.read().position(|m| &m.id == id))
    });
    let taken = ctx
        .messages
        .write()
        .take_matching(|m| idset.contains(&m.id));
    if selected_removed_index.is_none() {
        let mut indices: Vec<usize> = taken.iter().map(|(i, _)| *i).collect();
        indices.sort_unstable();
        indices.reverse();
        let mut selection = ctx.selection.write();
        for i in indices {
            selection.note_removed_at(i);
        }
    }
    let n = taken.len();
    let unread_n =
        unread_in_ids(ctx, ids).max(taken.iter().filter(|(_, m)| !m.is_read).count()) as i32;
    let mb = ctx.selected_mailbox.read().clone();
    if let Some(mb) = mb {
        if let Some(node) = ctx.mailbox_nodes.write().get_mut(&mb) {
            node.total_count = node.total_count.saturating_sub(n);
        }
        if unread_n != 0 {
            bump_mailbox_unread(ctx, &mb, -unread_n, true);
        }
    }
    ctx.selection.write().remove_ids(&idset);
    ctx.message_bodies.borrow_mut().remove_many(ids);
    if selected_removed_index.is_some() {
        ctx.selection.write().clear();
        ctx.message_view.set(MessageViewState::Empty);
        ctx.message_headers.set(MessageHeadersState::Closed);
        ctx.message_source.set(MessageSourceState::Closed);
        ctx.clear_nested_rfc822();
        ctx.download_status.set(HashMap::new());
        ctx.clear_attachment_downloads();
    }
    let snapshots = taken
        .into_iter()
        .map(|(index, message)| RemovedMessage { index, message })
        .collect();
    (snapshots, selected_removed_index)
}

fn restore_snapshots(
    ctx: &mut AppContext,
    mailbox_id: &MailboxId,
    snapshots: Vec<RemovedMessage>,
    new_ids: Option<&[MessageId]>,
) {
    if ctx.selected_mailbox.read().as_ref() != Some(mailbox_id) {
        return;
    }
    let mut snapshots = snapshots;
    snapshots.sort_by_key(|s| s.index);
    if let Some(ids) = new_ids {
        if ids.len() == snapshots.len() {
            for (snap, id) in snapshots.iter_mut().zip(ids) {
                let mut next = (*snap.message).clone();
                next.id = id.clone();
                next.envelope.id = id.clone();
                snap.message = Arc::new(next);
            }
        }
    }
    {
        let unread_n = unread_in_removed(&snapshots);
        let mut list = ctx.messages.write();
        {
            let mut selection = ctx.selection.write();
            for snap in &snapshots {
                selection.note_inserted_at(snap.index);
            }
        }
        for snap in snapshots {
            list.insert_at(snap.index, snap.message);
        }
        if let Some(node) = ctx.mailbox_nodes.write().get_mut(mailbox_id) {
            node.total_count = list.total_count();
        }
        drop(list);
        if unread_n != 0 {
            let acknowledge = ctx.selected_mailbox.read().as_ref() == Some(mailbox_id);
            bump_mailbox_unread(ctx, mailbox_id, unread_n, acknowledge);
        }
    }
}

fn core_message_ids(ids: &[MessageId]) -> Vec<MessageId> {
    ids.to_vec()
}

async fn handle_mark_read(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    mailbox_id: MailboxId,
    message_ids: Vec<MessageId>,
    is_read: bool,
) {
    if message_ids.is_empty() {
        return;
    }
    if !mailbox_action_applies(ctx, &mailbox_id) {
        return;
    }
    let Some(account_id) = action_account_for(ctx, &message_ids) else {
        ctx.show_toast(ToastAction::error("No account selected"));
        return;
    };
    let Some(connector) = manager.get(&account_id) else {
        ctx.show_toast(ToastAction::error("Not connected"));
        return;
    };

    apply_read_flag(ctx, &message_ids, is_read);

    let folder_id = FolderId::new(mailbox_id.to_string());
    let core_ids = core_message_ids(&message_ids);
    if let Err(e) = connector
        .update_envelope_flags(&folder_id, &core_ids, &[(EnvelopeFlag::Read, is_read)])
        .await
    {
        error!("Failed to update read flag: {}", e);
        note_selected_imap_error(manager, ctx, &e);
        apply_read_flag(ctx, &message_ids, !is_read);
        ctx.show_toast(ToastAction::error(format!(
            "Could not update read state: {e}"
        )));
        return;
    }
    relocate_unread_sort_rows(connector, ctx, &message_ids, is_read).await;
    if filter_dropped_read_rows(ctx, is_read) {
        clear_selection_if_focus_gone(manager, ctx, &message_ids).await;
    }
    persist_selected_messages(manager.cache(), ctx, &account_id).await;
    persist_folder_tree(manager.cache(), ctx, &account_id).await;
}

async fn handle_toggle_flag(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    mailbox_id: MailboxId,
    message_ids: Vec<MessageId>,
    flag: EnvelopeFlag,
) {
    if message_ids.is_empty() {
        return;
    }
    if message_ids
        .iter()
        .any(|id| id.folder_id().as_str() != mailbox_id.as_str())
    {
        return;
    }
    if !same_mail_session(ctx, &account_id, &mailbox_id) {
        return;
    }
    let Some(connector) = manager.get(&account_id) else {
        ctx.show_toast(ToastAction::error("Not connected"));
        return;
    };

    let snapshot = snapshot_flag_values(ctx, &message_ids, flag);
    let value = next_flag_value(snapshot.iter().map(|(_, on)| *on));
    apply_toggleable_flag(ctx, &message_ids, flag, value);

    let folder_id = FolderId::new(mailbox_id.to_string());
    let core_ids = core_message_ids(&message_ids);
    let result = connector
        .update_envelope_flags(&folder_id, &core_ids, &[(flag, value)])
        .await;

    if !same_mail_session(ctx, &account_id, &mailbox_id) {
        if let Err(e) = result {
            error!("Failed to update {flag:?} flag after session change: {e}");
            ctx.show_toast(ToastAction::error(format!(
                "Could not update {}: {e}",
                flag_label(flag)
            )));
        }
        return;
    }
    if let Err(e) = result {
        error!("Failed to update {flag:?} flag: {e}");
        restore_flag_values(ctx, &snapshot, flag);
        ctx.show_toast(ToastAction::error(format!(
            "Could not update {}: {e}",
            flag_label(flag)
        )));
        return;
    }
    if flag == EnvelopeFlag::Flagged && filter_dropped_flagged_rows(ctx, value) {
        let idset: std::collections::HashSet<&MessageId> = message_ids.iter().collect();
        ctx.messages
            .write()
            .remove_matching(|m| idset.contains(&m.id));
        clear_selection_if_focus_gone(manager, ctx, &message_ids).await;
    }
    persist_selected_messages(manager.cache(), ctx, &account_id).await;
}

fn sync_pinned_uids(ctx: &mut AppContext) {
    let pins = match (
        ctx.selected_account.peek().as_ref(),
        ctx.selected_mailbox.peek().as_ref(),
    ) {
        (Some(account_id), Some(mailbox_id)) => {
            crate::ui_prefs::load_pinned_uids(account_id, mailbox_id)
        }
        _ => Vec::new(),
    };
    ctx.pinned_uids.set(pins);
}

fn sync_snoozed_messages(ctx: &mut AppContext) {
    let entries = match (
        ctx.selected_account.peek().as_ref(),
        ctx.selected_mailbox.peek().as_ref(),
    ) {
        (Some(account_id), Some(mailbox_id)) => {
            crate::ui_prefs::load_snoozed_messages(account_id, mailbox_id)
        }
        _ => Vec::new(),
    };
    ctx.snoozed_messages.set(entries);
}

fn sync_list_overlays(ctx: &mut AppContext) {
    sync_pinned_uids(ctx);
    sync_snoozed_messages(ctx);
}

fn apply_list_overlays(ctx: &mut AppContext) {
    apply_pins_to_messages(ctx);
    hide_active_snoozes(ctx);
}

fn hide_active_snoozes(ctx: &mut AppContext) {
    let now = Utc::now();
    let uids = crate::snooze::active_uids(&ctx.snoozed_messages.peek(), now);
    if uids.is_empty() {
        return;
    }
    hide_rows_locally(ctx, &uids);
}

/// Remove matching rows from the list without changing IMAP folder counts.
fn hide_rows_locally(ctx: &mut AppContext, uids: &[String]) -> Vec<RemovedMessage> {
    let uidset: HashSet<&str> = uids.iter().map(String::as_str).collect();
    if uidset.is_empty() {
        return Vec::new();
    }
    let focus = ctx.selection.read().focus().cloned();
    let focus_hidden = focus
        .as_ref()
        .is_some_and(|id| uidset.contains(id.as_uid()));
    let taken = ctx
        .messages
        .write()
        .take_matching(|m| uidset.contains(m.id.as_uid()));
    if taken.is_empty() {
        return Vec::new();
    }
    if !focus_hidden {
        let mut indices: Vec<usize> = taken.iter().map(|(i, _)| *i).collect();
        indices.sort_unstable();
        indices.reverse();
        let mut selection = ctx.selection.write();
        for i in indices {
            selection.note_removed_at(i);
        }
    }
    let gone: HashSet<MessageId> = taken.iter().map(|(_, m)| m.id.clone()).collect();
    ctx.selection.write().remove_ids(&gone);
    if focus_hidden {
        ctx.selection.write().clear();
        ctx.message_view.set(MessageViewState::Empty);
        ctx.message_headers.set(MessageHeadersState::Closed);
        ctx.message_source.set(MessageSourceState::Closed);
        ctx.clear_nested_rfc822();
        ctx.download_status.set(HashMap::new());
        ctx.clear_attachment_downloads();
    }
    taken
        .into_iter()
        .map(|(index, message)| RemovedMessage { index, message })
        .collect()
}

fn apply_pins_to_messages(ctx: &mut AppContext) {
    let pins = ctx.pinned_uids.peek().clone();
    if pins.is_empty() {
        return;
    }
    let focus = ctx.selection.read().focus().cloned();
    let prefix_len = contiguous_loaded_prefix_len(&ctx.messages.read());
    if prefix_len < 2 {
        return;
    }
    let mut prefix: Vec<Arc<Message>> = {
        let list = ctx.messages.read();
        (0..prefix_len)
            .filter_map(|i| list.get(i).cloned())
            .collect()
    };
    if prefix.len() != prefix_len {
        return;
    }
    if !crate::pin::sort_pinned_first(&mut prefix, &pins, |m| m.id.as_uid()) {
        return;
    }
    {
        let mut list = ctx.messages.write();
        for (i, msg) in prefix.into_iter().enumerate() {
            list.insert(i, msg);
        }
    }
    if let Some(id) = focus
        && let Some(idx) = ctx.messages.read().position(|m| m.id == id)
    {
        ctx.selection.write().note_focus(id, Some(idx));
    }
}

fn handle_toggle_pin(
    ctx: &mut AppContext,
    account_id: AccountId,
    mailbox_id: MailboxId,
    message_ids: Vec<MessageId>,
) {
    if message_ids.is_empty()
        || message_ids
            .iter()
            .any(|id| id.folder_id().as_str() != mailbox_id.as_str())
    {
        return;
    }
    if !same_mail_session(ctx, &account_id, &mailbox_id) {
        return;
    }
    let current = ctx.pinned_uids.peek().clone();
    let pin = !crate::pin::all_pinned(message_ids.iter().map(|id| id.as_uid()), &current);
    let mut ordered: Vec<String> = Vec::new();
    {
        let list = ctx.messages.read();
        for msg in list.iter() {
            if message_ids.iter().any(|id| id == &msg.id)
                && !ordered.iter().any(|uid| uid == msg.id.as_uid())
            {
                ordered.push(msg.id.as_uid().to_string());
            }
        }
    }
    for id in &message_ids {
        if !ordered.iter().any(|uid| uid == id.as_uid()) {
            ordered.push(id.as_uid().to_string());
        }
    }
    let next = crate::ui_prefs::toggle_pinned_uids(&account_id, &mailbox_id, &ordered, pin);
    ctx.pinned_uids.set(next);
    apply_list_overlays(ctx);
}

async fn handle_snooze_messages(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    mailbox_id: MailboxId,
    message_ids: Vec<MessageId>,
    preset: SnoozePreset,
) {
    if message_ids.is_empty()
        || message_ids
            .iter()
            .any(|id| id.folder_id().as_str() != mailbox_id.as_str())
    {
        return;
    }
    if !same_mail_session(ctx, &account_id, &mailbox_id) {
        return;
    }
    let until = preset.until(Utc::now());
    let mut entries = Vec::new();
    let mut ordered_ids: Vec<MessageId> = Vec::new();
    {
        let list = ctx.messages.read();
        for msg in list.iter() {
            if message_ids.iter().any(|id| id == &msg.id)
                && !ordered_ids.iter().any(|id| id == &msg.id)
            {
                ordered_ids.push(msg.id.clone());
                entries.push(SnoozedMessage {
                    uid: msg.id.as_uid().to_string(),
                    until,
                    subject: msg.subject.clone(),
                });
            }
        }
    }
    for id in &message_ids {
        if !ordered_ids.iter().any(|existing| existing == id) {
            ordered_ids.push(id.clone());
            entries.push(SnoozedMessage {
                uid: id.as_uid().to_string(),
                until,
                subject: String::new(),
            });
        }
    }
    let uids: Vec<String> = entries.iter().map(|e| e.uid.clone()).collect();
    let next = crate::ui_prefs::snooze_messages(&account_id, &mailbox_id, &entries);
    ctx.snoozed_messages.set(next);
    ctx.snooze_picker_open.set(false);
    let removed_sel = ctx.selection.read().focus().and_then(|id| {
        if !uids.iter().any(|uid| uid == id.as_uid()) {
            return None;
        }
        ctx.selection
            .read()
            .focus_at_index()
            .or_else(|| ctx.messages.read().position(|m| m.id == *id))
    });
    let snapshots = hide_rows_locally(ctx, &uids);
    ctx.show_toast(ToastAction::snoozed(
        crate::snooze::format_until(until),
        SnoozeUndo {
            account_id: account_id.clone(),
            mailbox_id: mailbox_id.clone(),
            uids: uids.clone(),
            snapshots,
        },
    ));
    select_after_removed_row(manager, ctx, removed_sel).await;
    persist_selected_messages(manager.cache(), ctx, &account_id).await;
    set_later_keyword(manager, ctx, &account_id, &mailbox_id, &ordered_ids, true).await;
}

async fn handle_sweep_snooze(manager: &AccountConnectionManager, ctx: &mut AppContext) {
    let expired = crate::ui_prefs::take_expired_snoozes(Utc::now());
    if expired.is_empty() {
        return;
    }
    sync_snoozed_messages(ctx);
    let current_account = ctx.selected_account.peek().clone();
    let current_mailbox = ctx.selected_mailbox.peek().clone();
    let refresh = expired.iter().any(|row| {
        current_account.as_ref() == Some(&row.account_id)
            && current_mailbox.as_ref() == Some(&row.mailbox_id)
    });
    let first = expired[0].clone();
    let extra = expired.len().saturating_sub(1);
    if *ctx.notify_inbox.peek()
        && crate::notifications::current_permission()
            == crate::notifications::NotifyPermission::Granted
    {
        crate::notifications::show_snooze_notification(&first.subject);
    }
    let subject = if extra == 0 {
        first.subject
    } else {
        format!("{} (+{extra} more)", subject_or_fallback(&first.subject))
    };
    ctx.show_toast(ToastAction::snooze_ended(
        first.account_id,
        first.mailbox_id,
        first.uid,
        subject,
    ));
    for row in &expired {
        if row.uid.is_empty() {
            continue;
        }
        let id = MessageId::new(FolderId::new(row.mailbox_id.to_string()), row.uid.clone());
        set_later_keyword(manager, ctx, &row.account_id, &row.mailbox_id, &[id], false).await;
    }
    if refresh {
        if let (Some(account_id), Some(mailbox_id)) = (current_account, current_mailbox) {
            handle_mailbox_activity(manager, ctx, account_id, mailbox_id).await;
        }
    }
}

fn subject_or_fallback(subject: &str) -> String {
    let trimmed = subject.trim();
    if trimmed.is_empty() {
        "Snoozed message".into()
    } else {
        trimmed.to_string()
    }
}

async fn handle_open_snoozed_message(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    mailbox_id: MailboxId,
    uid: String,
) {
    if uid.is_empty() {
        return;
    }
    if ctx.selected_account.peek().as_ref() != Some(&account_id) {
        ctx.show_toast(ToastAction::info(
            "Switch to that account to open the message",
        ));
        return;
    }
    if ctx.selected_mailbox.peek().as_ref() != Some(&mailbox_id) {
        handle_select_mailbox(manager, ctx, mailbox_id.clone(), true).await;
    }
    if ctx.selected_mailbox.peek().as_ref() != Some(&mailbox_id) {
        return;
    }
    let message_id = MessageId::new(FolderId::new(mailbox_id.to_string()), uid);
    ctx.set_mobile_pane(MobilePane::after_select_message());
    handle_select_message(manager, ctx, message_id, true, true).await;
}

async fn set_later_keyword(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: &AccountId,
    mailbox_id: &MailboxId,
    message_ids: &[MessageId],
    on: bool,
) {
    if message_ids.is_empty() {
        return;
    }
    let Some(connector) = manager.get(account_id) else {
        return;
    };
    let folder_id = FolderId::new(mailbox_id.to_string());
    let core_ids = core_message_ids(message_ids);
    if let Err(e) = connector
        .update_envelope_flags(
            &folder_id,
            &core_ids,
            &[(EnvelopeFlag::Keyword(ImapKeyword::Later), on)],
        )
        .await
    {
        warn!("optional $Later keyword update failed: {e}");
        note_selected_imap_error(manager, ctx, &e);
    }
}

fn same_mail_session(ctx: &AppContext, account_id: &AccountId, mailbox_id: &MailboxId) -> bool {
    if is_unified_selected(ctx) {
        return !is_unified_mailbox(mailbox_id);
    }
    selected_account_is(ctx, account_id) && ctx.selected_mailbox.read().as_ref() == Some(mailbox_id)
}

fn flag_label(flag: EnvelopeFlag) -> &'static str {
    match flag {
        EnvelopeFlag::Starred => "star",
        EnvelopeFlag::Flagged => "flag",
        EnvelopeFlag::Keyword(keyword) => keyword.label(),
        _ => "flag",
    }
}

fn message_has_flag(msg: &crate::message::Message, flag: EnvelopeFlag) -> bool {
    match flag {
        EnvelopeFlag::Starred => msg.is_starred,
        EnvelopeFlag::Flagged => msg.is_flagged,
        EnvelopeFlag::Read => msg.is_read,
        EnvelopeFlag::Answered => msg.is_answered,
        EnvelopeFlag::Draft => msg.envelope.is_draft,
        EnvelopeFlag::Deleted => msg.envelope.is_deleted,
        EnvelopeFlag::Keyword(keyword) => msg.envelope.has_keyword(keyword),
    }
}

fn set_message_flag(msg: &mut crate::message::Message, flag: EnvelopeFlag, value: bool) {
    match flag {
        EnvelopeFlag::Starred => {
            msg.is_starred = value;
            msg.envelope.is_starred = value;
        }
        EnvelopeFlag::Flagged => {
            msg.is_flagged = value;
            msg.envelope.is_flagged = value;
        }
        EnvelopeFlag::Read => {
            msg.is_read = value;
            msg.envelope.is_read = value;
        }
        EnvelopeFlag::Answered => {
            msg.is_answered = value;
            msg.envelope.is_answered = value;
        }
        EnvelopeFlag::Draft => msg.envelope.is_draft = value,
        EnvelopeFlag::Deleted => msg.envelope.is_deleted = value,
        EnvelopeFlag::Keyword(keyword) => msg.envelope.set_keyword(keyword, value),
    }
}

fn snapshot_flag_values(
    ctx: &AppContext,
    ids: &[MessageId],
    flag: EnvelopeFlag,
) -> Vec<(MessageId, bool)> {
    let list = ctx.messages.read();
    ids.iter()
        .filter_map(|id| {
            list.find(|m| m.id == *id)
                .map(|m| (id.clone(), message_has_flag(m, flag)))
        })
        .collect()
}

fn restore_flag_values(ctx: &mut AppContext, snapshot: &[(MessageId, bool)], flag: EnvelopeFlag) {
    let wanted: std::collections::HashMap<&MessageId, bool> =
        snapshot.iter().map(|(id, on)| (id, *on)).collect();
    for msg in ctx.messages.write().iter_mut() {
        let Some(&value) = wanted.get(&msg.id) else {
            continue;
        };
        if message_has_flag(msg, flag) == value {
            continue;
        }
        let mut next = (**msg).clone();
        set_message_flag(&mut next, flag, value);
        *msg = Arc::new(next);
    }
}

fn apply_toggleable_flag(ctx: &mut AppContext, ids: &[MessageId], flag: EnvelopeFlag, value: bool) {
    let idset: std::collections::HashSet<&MessageId> = ids.iter().collect();
    for msg in ctx.messages.write().iter_mut() {
        if !idset.contains(&msg.id) || message_has_flag(msg, flag) == value {
            continue;
        }
        let mut next = (**msg).clone();
        set_message_flag(&mut next, flag, value);
        *msg = Arc::new(next);
    }
}

fn active_mailbox_search(ctx: &AppContext) -> mailiner_core::MailboxSearch {
    mailiner_core::MailboxSearch::parse(ctx.list_search_query.peek().as_str())
}

fn filter_dropped_read_rows(ctx: &AppContext, now_read: bool) -> bool {
    active_mailbox_search(ctx).drops_on_read_change(*ctx.message_list_filter.peek(), now_read)
}

fn filter_dropped_flagged_rows(ctx: &AppContext, now_flagged: bool) -> bool {
    active_mailbox_search(ctx).drops_on_flagged_change(*ctx.message_list_filter.peek(), now_flagged)
}

async fn clear_selection_if_focus_gone(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    message_ids: &[MessageId],
) {
    let focus = ctx.selection.read().focus().cloned();
    let gone = focus
        .as_ref()
        .is_some_and(|id| ctx.messages.read().position(|m| m.id == *id).is_none());
    if gone {
        let idx = ctx.selection.read().focus_at_index();
        ctx.selection.write().clear();
        ctx.message_view.set(MessageViewState::Empty);
        ctx.download_status.set(HashMap::new());
        select_after_removed_row_mark(manager, ctx, idx, false).await;
    } else {
        let gone: HashSet<_> = message_ids.iter().cloned().collect();
        ctx.selection.write().remove_ids(&gone);
    }
}

/// Slide rows in the unread-first index without SELECT/SEARCH or a list rebuild.
async fn relocate_unread_sort_rows(
    connector: &mailiner_imap_connector::ImapConnector<crate::websocket_stream::WebSocketStream>,
    ctx: &mut AppContext,
    message_ids: &[MessageId],
    now_read: bool,
) {
    let filter = *ctx.message_list_filter.peek();
    let search = active_mailbox_search(ctx);
    if search.drops_on_read_change(filter, now_read) {
        let core_ids = core_message_ids(message_ids);
        if let Err(e) = connector.sync_unread_sort_index(&core_ids, now_read).await {
            warn!("search-filter index drop failed: {e}");
        }
        let idset: std::collections::HashSet<&MessageId> = message_ids.iter().collect();
        ctx.messages
            .write()
            .remove_matching(|m| idset.contains(&m.id));
        return;
    }
    if *ctx.message_sort.peek() != MessageSort::Unread || message_ids.is_empty() {
        return;
    }
    let core_ids = core_message_ids(message_ids);
    match connector.sync_unread_sort_index(&core_ids, now_read).await {
        Ok(moves) => {
            if moves.is_empty() {
                return;
            }
            let mut list = ctx.messages.write();
            for (from, to) in moves {
                list.relocate(from, to);
            }
            drop(list);
            apply_list_overlays(ctx);
        }
        Err(e) => {
            warn!("unread-sort relocate failed: {e}");
        }
    }
}

async fn handle_move_messages(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    mailbox_id: MailboxId,
    message_ids: Vec<MessageId>,
    dest_mailbox_id: MailboxId,
) {
    if message_ids.is_empty() || mailbox_id == dest_mailbox_id {
        return;
    }
    if !same_mail_session(ctx, &account_id, &mailbox_id) {
        return;
    }
    let Some(connector) = manager.get(&account_id) else {
        ctx.show_toast(ToastAction::error("Not connected"));
        return;
    };

    let folder_id = FolderId::new(mailbox_id.to_string());
    let dest_id = FolderId::new(dest_mailbox_id.to_string());
    let core_ids = core_message_ids(&message_ids);
    let unread_flags: Vec<bool> = {
        let from_sel = ctx.selection.read();
        let list = ctx.messages.read();
        message_ids
            .iter()
            .map(|id| {
                from_sel.unread_among(std::slice::from_ref(id)) > 0
                    || list.find(|m| m.id == *id).is_some_and(|m| !m.is_read)
            })
            .collect()
    };
    let unread_n = unread_flags.iter().filter(|u| **u).count();
    match connector
        .move_messages(&folder_id, &core_ids, &dest_id)
        .await
    {
        Ok(dest_uids) => {
            if !selected_account_is(ctx, &account_id)
                || ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id)
            {
                // COPYUID present → mapped count; empty → no mapping, assume all moved.
                let (moved, unread_n) = if dest_uids.is_empty() {
                    (message_ids.len(), unread_n)
                } else if dest_uids.len() >= message_ids.len() {
                    (dest_uids.len(), unread_n)
                } else {
                    // Same-order COPYUID: only the leading mapped ids moved.
                    (
                        dest_uids.len(),
                        unread_flags
                            .iter()
                            .take(dest_uids.len())
                            .filter(|u| **u)
                            .count(),
                    )
                };
                persist_stale_move_counts(
                    manager.cache(),
                    ctx,
                    &account_id,
                    &mailbox_id,
                    &dest_mailbox_id,
                    moved,
                    unread_n as i32,
                )
                .await;
                invalidate_mailbox_messages(manager.cache(), &account_id, &mailbox_id).await;
                invalidate_mailbox_messages(manager.cache(), &account_id, &dest_mailbox_id).await;
                return;
            }
            let (snapshots, removed_sel) = take_messages_from_ui(ctx, &message_ids);
            let unread_n = unread_in_removed(&snapshots);
            let dest_is_all_mail = ctx
                .mailbox_nodes
                .read()
                .get(&dest_mailbox_id)
                .is_some_and(|n| mailbox_is_all_mail(&dest_mailbox_id, Some(n.name.as_str())));
            if unread_n != 0 && !dest_is_all_mail {
                bump_mailbox_unread(ctx, &dest_mailbox_id, unread_n, false);
            }
            let dest_label = ctx
                .mailbox_nodes
                .read()
                .get(&dest_mailbox_id)
                .map(|n| n.title().to_string())
                .unwrap_or_else(|| dest_mailbox_id.to_string());
            let dest_ids = dest_uids;
            if dest_ids.len() == snapshots.len() && !dest_ids.is_empty() {
                ctx.show_toast(ToastAction::moved(
                    dest_label,
                    MoveUndo {
                        account_id: account_id.clone(),
                        from: dest_mailbox_id.clone(),
                        to: mailbox_id,
                        dest_ids,
                        snapshots,
                    },
                ));
            } else {
                ctx.show_toast(ToastAction::info(format!("Moved to {dest_label}")));
            }
            select_after_removed_row(manager, ctx, removed_sel).await;
            persist_selected_messages(manager.cache(), ctx, &account_id).await;
            persist_folder_tree(manager.cache(), ctx, &account_id).await;
            invalidate_mailbox_messages(manager.cache(), &account_id, &dest_mailbox_id).await;
        }
        Err(mailiner_core::MailinerError::PartialMove {
            dest_ids: _,
            message,
        }) => {
            error!("Partial move (copy ok, delete failed): {message}");
            ctx.show_toast(ToastAction::error(
                "Copied, but the originals are still here. Do not retry.",
            ));
        }
        Err(e) => {
            error!("Failed to move messages: {}", e);
            note_selected_imap_error(manager, ctx, &e);
            ctx.show_toast(ToastAction::error(format!("Could not move messages: {e}")));
        }
    }
}

async fn handle_copy_messages(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    mailbox_id: MailboxId,
    message_ids: Vec<MessageId>,
    dest_mailbox_id: MailboxId,
) {
    if message_ids.is_empty() || mailbox_id == dest_mailbox_id {
        return;
    }
    if !same_mail_session(ctx, &account_id, &mailbox_id) {
        return;
    }
    let Some(connector) = manager.get(&account_id) else {
        ctx.show_toast(ToastAction::error("Not connected"));
        return;
    };

    let folder_id = FolderId::new(mailbox_id.to_string());
    let dest_id = FolderId::new(dest_mailbox_id.to_string());
    let core_ids = core_message_ids(&message_ids);
    match connector
        .copy_messages(&folder_id, &core_ids, &dest_id)
        .await
    {
        Ok(_) => {
            let same_account = selected_account_is(ctx, &account_id);
            let same_mailbox = ctx.selected_mailbox.read().as_ref() == Some(&mailbox_id);
            if same_account && same_mailbox {
                let unread_n = unread_among(ctx, &message_ids);
                if let Some(node) = ctx.mailbox_nodes.write().get_mut(&dest_mailbox_id) {
                    node.total_count = node.total_count.saturating_add(message_ids.len());
                }
                if unread_n != 0 {
                    bump_mailbox_unread(ctx, &dest_mailbox_id, unread_n, false);
                }
                persist_folder_tree(manager.cache(), ctx, &account_id).await;
            }
            if selected_account_is(ctx, &account_id) {
                let dest_label = ctx
                    .mailbox_nodes
                    .read()
                    .get(&dest_mailbox_id)
                    .map(|n| n.title().to_string())
                    .unwrap_or_else(|| dest_mailbox_id.to_string());
                ctx.show_toast(ToastAction::info(format!("Copied to {dest_label}")));
            }
            invalidate_mailbox_messages(manager.cache(), &account_id, &dest_mailbox_id).await;
        }
        Err(e) => {
            error!("Failed to copy messages: {}", e);
            if selected_account_is(ctx, &account_id) {
                ctx.show_toast(ToastAction::error(format!("Could not copy messages: {e}")));
            }
        }
    }
}

async fn handle_archive_messages(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    mailbox_id: MailboxId,
    message_ids: Vec<MessageId>,
) {
    if message_ids.is_empty() {
        return;
    }
    if !same_mail_session(ctx, &account_id, &mailbox_id) {
        return;
    }
    if is_unified_selected(ctx) && !selected_account_is(ctx, &account_id) {
        ctx.show_toast(ToastAction::info("Switch to that account to archive"));
        return;
    }
    let archive_id = crate::mailbox::find_archive_mailbox(&ctx.mailbox_nodes.read());
    let Some(archive_id) = archive_id else {
        ctx.show_toast(ToastAction::error(
            "No Archive folder found on this account",
        ));
        return;
    };
    handle_move_messages(
        manager,
        ctx,
        account_id,
        mailbox_id,
        message_ids,
        archive_id,
    )
    .await;
}

async fn handle_move_to_junk(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    mailbox_id: MailboxId,
    message_ids: Vec<MessageId>,
) {
    if message_ids.is_empty() {
        return;
    }
    if !same_mail_session(ctx, &account_id, &mailbox_id) {
        return;
    }
    if is_unified_selected(ctx) && !selected_account_is(ctx, &account_id) {
        ctx.show_toast(ToastAction::info("Switch to that account to move to Junk"));
        return;
    }
    let junk_id = crate::mailbox::find_junk_mailbox(&ctx.mailbox_nodes.read());
    let Some(junk_id) = junk_id else {
        ctx.show_toast(ToastAction::error("No Junk folder found on this account"));
        return;
    };
    handle_move_messages(manager, ctx, account_id, mailbox_id, message_ids, junk_id).await;
}

async fn handle_move_to_trash(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    mailbox_id: MailboxId,
    message_ids: Vec<MessageId>,
) {
    if message_ids.is_empty() {
        return;
    }
    if !mailbox_action_applies(ctx, &mailbox_id) {
        return;
    }
    let Some(account_id) = action_account_for(ctx, &message_ids) else {
        ctx.show_toast(ToastAction::error("No account selected"));
        return;
    };
    let Some(connector) = manager.get(&account_id) else {
        ctx.show_toast(ToastAction::error("Not connected"));
        return;
    };

    let source_is_trash = ctx
        .mailbox_nodes
        .read()
        .get(&mailbox_id)
        .is_some_and(|n| n.role == MailboxRole::Trash);
    let trash_id =
        crate::mailbox::find_mailbox_with_role(&ctx.mailbox_nodes.read(), MailboxRole::Trash);

    let folder_id = FolderId::new(mailbox_id.to_string());
    let core_ids = core_message_ids(&message_ids);

    if source_is_trash || trash_id.as_ref() == Some(&mailbox_id) {
        schedule_permanent_delete(manager, ctx, account_id, mailbox_id, &message_ids).await;
        return;
    }

    let Some(trash_id) = trash_id else {
        ctx.show_toast(ToastAction::error("No Trash folder found on this account"));
        return;
    };

    let dest_id = FolderId::new(trash_id.to_string());
    match connector
        .move_messages(&folder_id, &core_ids, &dest_id)
        .await
    {
        Ok(dest_uids) => {
            let (snapshots, removed_sel) = take_messages_from_ui(ctx, &message_ids);
            let unread_n = unread_in_removed(&snapshots);
            if unread_n != 0 {
                bump_mailbox_unread(ctx, &trash_id, unread_n, false);
            }
            let dest_ids = dest_uids;
            if dest_ids.len() == snapshots.len() && !dest_ids.is_empty() {
                ctx.show_toast(ToastAction::trashed(MoveUndo {
                    account_id: account_id.clone(),
                    from: trash_id.clone(),
                    to: mailbox_id,
                    dest_ids,
                    snapshots,
                }));
            } else {
                ctx.show_toast(ToastAction::info("Moved to Trash"));
            }
            select_after_removed_row(manager, ctx, removed_sel).await;
            persist_selected_messages(manager.cache(), ctx, &account_id).await;
            persist_folder_tree(manager.cache(), ctx, &account_id).await;
            invalidate_mailbox_messages(manager.cache(), &account_id, &trash_id).await;
        }
        Err(mailiner_core::MailinerError::PartialMove {
            dest_ids: _,
            message,
        }) => {
            error!("Partial trash (copy ok, delete failed): {message}");
            ctx.show_toast(ToastAction::error(
                "Copied to Trash, but the originals are still here. Do not retry.",
            ));
        }
        Err(e) => {
            error!("Failed to move to trash: {}", e);
            note_selected_imap_error(manager, ctx, &e);
            ctx.show_toast(ToastAction::error(format!("Could not move to Trash: {e}")));
        }
    }
}

async fn handle_delete_messages(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    mailbox_id: MailboxId,
    message_ids: Vec<MessageId>,
) {
    if message_ids.is_empty() {
        return;
    }
    if !mailbox_action_applies(ctx, &mailbox_id) {
        return;
    }
    let Some(account_id) = action_account_for(ctx, &message_ids) else {
        ctx.show_toast(ToastAction::error("No account selected"));
        return;
    };
    if manager.get(&account_id).is_none() {
        ctx.show_toast(ToastAction::error("Not connected"));
        return;
    }
    schedule_permanent_delete(manager, ctx, account_id, mailbox_id, &message_ids).await;
}

async fn handle_empty_trash(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    mailbox_id: MailboxId,
) {
    if !selected_account_is(ctx, &account_id) {
        return;
    }
    if ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id) {
        return;
    }
    let is_trash = ctx
        .mailbox_nodes
        .read()
        .get(&mailbox_id)
        .is_some_and(crate::mailbox::can_empty_trash);
    if !is_trash {
        return;
    }
    let Some(connector) = manager.get(&account_id) else {
        ctx.show_toast(ToastAction::error("Not connected"));
        return;
    };

    let folder_id = FolderId::new(mailbox_id.to_string());
    match connector.empty_folder(&folder_id).await {
        Ok(()) => {
            ctx.messages.set(SparseList::new(0));
            ctx.selection.write().clear();
            ctx.message_view.set(MessageViewState::Empty);
            ctx.message_bodies.borrow_mut().clear();
            ctx.message_headers.set(MessageHeadersState::Closed);
            ctx.message_source.set(MessageSourceState::Closed);
            ctx.clear_nested_rfc822();
            ctx.download_status.set(HashMap::new());
            ctx.clear_attachment_downloads();
            if let Some(node) = ctx.mailbox_nodes.write().get_mut(&mailbox_id) {
                node.total_count = 0;
                node.unread_count = 0;
                node.has_new = false;
            }
            crate::ui_prefs::save_ack_unread(&account_id, &mailbox_id, 0);
            observe_local_mailbox(ctx, &mailbox_id);
            persist_selected_messages(manager.cache(), ctx, &account_id).await;
            persist_folder_tree(manager.cache(), ctx, &account_id).await;
            ctx.show_toast(ToastAction::info("Trash emptied"));
        }
        Err(e) => {
            error!("Failed to empty trash: {}", e);
            manager.note_imap_error(&account_id, &e);
            ctx.show_toast(ToastAction::error(format!("Could not empty Trash: {e}")));
        }
    }
}

async fn handle_create_folder(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    parent_id: Option<MailboxId>,
    name: String,
) {
    if !selected_account_is(ctx, &account_id) {
        return;
    }
    let Some(connector) = manager.get(&account_id) else {
        ctx.show_toast(ToastAction::error("Not connected"));
        return;
    };
    let parent = parent_id.as_ref().map(|id| FolderId::new(id.to_string()));
    match connector
        .create_folder(&account_id, &name, parent.as_ref())
        .await
    {
        Ok(folder) => {
            info!("Created folder {}", folder.id.as_str());
            list_folders_soft(manager, ctx, &account_id).await;
            ctx.show_toast(ToastAction::info(format!("Created folder {}", folder.name)));
        }
        Err(e) => {
            error!("Failed to create folder: {e}");
            ctx.show_toast(ToastAction::error(format!("Could not create folder: {e}")));
        }
    }
}

async fn handle_rename_folder(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    mailbox_id: MailboxId,
    new_name: String,
) {
    if !selected_account_is(ctx, &account_id) {
        return;
    }
    let can_rename = ctx
        .mailbox_nodes
        .read()
        .get(&mailbox_id)
        .is_some_and(crate::mailbox::can_manage_folder);
    if !can_rename {
        ctx.show_toast(ToastAction::error("Cannot rename Inbox"));
        return;
    }
    let Some(connector) = manager.get(&account_id) else {
        ctx.show_toast(ToastAction::error("Not connected"));
        return;
    };
    let folder_id = FolderId::new(mailbox_id.to_string());
    match connector.rename_folder(&folder_id, &new_name).await {
        Ok(folder) => {
            let new_id = MailboxId::from(folder.id.clone());
            let nodes = ctx.mailbox_nodes.read().clone();
            if let Some(next) = ctx.selected_mailbox.read().as_ref().and_then(|sel| {
                crate::mailbox::remap_renamed_mailbox(&mailbox_id, &new_id, sel, &nodes)
            }) {
                crate::ui_prefs::save_last_mailbox(&account_id, &next);
            }
            for id in crate::mailbox::mailbox_subtree_deepest_first(&mailbox_id, &nodes) {
                invalidate_mailbox_messages(manager.cache(), &account_id, &id).await;
            }
            list_folders_soft(manager, ctx, &account_id).await;
            ctx.show_toast(ToastAction::info(format!(
                "Renamed folder to {}",
                folder.name
            )));
        }
        Err(e) => {
            error!("Failed to rename folder: {e}");
            ctx.show_toast(ToastAction::error(format!("Could not rename folder: {e}")));
        }
    }
}

async fn handle_delete_folder(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    mailbox_id: MailboxId,
) {
    if !selected_account_is(ctx, &account_id) {
        return;
    }
    let can_delete = ctx
        .mailbox_nodes
        .read()
        .get(&mailbox_id)
        .is_some_and(crate::mailbox::can_manage_folder);
    if !can_delete {
        ctx.show_toast(ToastAction::error("Cannot delete Inbox"));
        return;
    }
    let Some(connector) = manager.get(&account_id) else {
        ctx.show_toast(ToastAction::error("Not connected"));
        return;
    };

    let selected = ctx.selected_mailbox.read().clone();
    let nodes = ctx.mailbox_nodes.read().clone();
    let to_delete: Vec<MailboxId> =
        crate::mailbox::mailbox_subtree_deepest_first(&mailbox_id, &nodes)
            .into_iter()
            .filter(|id| nodes.get(id).is_some_and(crate::mailbox::can_manage_folder))
            .collect();
    let selected_hit = selected.as_ref().is_some_and(|sel| {
        to_delete.iter().any(|id| id == sel)
            || crate::mailbox::mailbox_is_ancestor(&mailbox_id, sel, &nodes)
    });
    if selected_hit {
        if let Some(inbox) = crate::mailbox::find_mailbox_with_role(&nodes, MailboxRole::Inbox) {
            crate::ui_prefs::save_last_mailbox(&account_id, &inbox);
        }
    }

    for id in &to_delete {
        let folder_id = FolderId::new(id.to_string());
        if let Err(e) = connector.delete_folder(&folder_id).await {
            error!("Failed to delete folder {}: {e}", id.as_str());
            ctx.show_toast(ToastAction::error(format!("Could not delete folder: {e}")));
            list_folders_soft(manager, ctx, &account_id).await;
            return;
        }
        invalidate_mailbox_messages(manager.cache(), &account_id, id).await;
    }
    list_folders_soft(manager, ctx, &account_id).await;
    ctx.show_toast(ToastAction::info("Folder deleted"));
}

async fn handle_set_folder_subscribed(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    mailbox_id: MailboxId,
    subscribed: bool,
) {
    if !selected_account_is(ctx, &account_id) {
        return;
    }
    let allowed = ctx
        .mailbox_nodes
        .read()
        .get(&mailbox_id)
        .is_some_and(crate::mailbox::can_toggle_subscription);
    if !allowed {
        ctx.show_toast(ToastAction::error("Inbox cannot be unsubscribed"));
        return;
    }
    let Some(connector) = manager.get(&account_id) else {
        ctx.show_toast(ToastAction::error("Not connected"));
        return;
    };
    let folder_id = FolderId::new(mailbox_id.to_string());
    match connector
        .set_folder_subscribed(&folder_id, subscribed)
        .await
    {
        Ok(()) => {
            if let Some(node) = ctx.mailbox_nodes.write().get_mut(&mailbox_id) {
                node.subscribed = subscribed;
            }
            persist_folder_tree(manager.cache(), ctx, &account_id).await;
            let show_all = *ctx.show_all_folders.read();
            let selected_hidden = !subscribed
                && !show_all
                && ctx.selected_mailbox.read().as_ref() == Some(&mailbox_id);
            if selected_hidden {
                restore_mailbox(manager, ctx, &account_id).await;
            }
            let title = ctx
                .mailbox_nodes
                .read()
                .get(&mailbox_id)
                .map(|n| n.title().to_string())
                .unwrap_or_else(|| mailbox_id.to_string());
            let msg = if subscribed {
                format!("Subscribed to {title}")
            } else {
                format!("Unsubscribed from {title}")
            };
            ctx.show_toast(ToastAction::info(msg));
        }
        Err(e) => {
            error!("Failed to set folder subscription: {}", e);
            let action = if subscribed {
                "subscribe"
            } else {
                "unsubscribe"
            };
            ctx.show_toast(ToastAction::error(format!(
                "Could not {action} folder: {e}"
            )));
        }
    }
}

async fn schedule_permanent_delete(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    mailbox_id: MailboxId,
    message_ids: &[MessageId],
) {
    let (snapshots, removed_sel) = take_messages_from_ui(ctx, message_ids);
    ctx.show_toast(ToastAction::deleted(
        account_id.clone(),
        mailbox_id,
        snapshots,
    ));
    select_after_removed_row(manager, ctx, removed_sel).await;
    persist_selected_messages(manager.cache(), ctx, &account_id).await;
    persist_folder_tree(manager.cache(), ctx, &account_id).await;
}

async fn handle_undo(manager: &AccountConnectionManager, ctx: &mut AppContext, undo: UndoRequest) {
    match undo {
        UndoRequest::RestoreLocal {
            account_id,
            mailbox_id,
            snapshots,
        } => {
            if !selected_account_is(ctx, &account_id) {
                ctx.show_toast(ToastAction::error(
                    "Undo is not available after switching accounts",
                ));
                return;
            }
            restore_snapshots(ctx, &mailbox_id, snapshots, None);
            ctx.show_toast(ToastAction::info("Undone"));
            persist_selected_messages(manager.cache(), ctx, &account_id).await;
            persist_folder_tree(manager.cache(), ctx, &account_id).await;
        }
        UndoRequest::ReverseMove(undo) => {
            if !selected_account_is(ctx, &undo.account_id) {
                ctx.show_toast(ToastAction::error(
                    "Undo is not available after switching accounts",
                ));
                return;
            }
            let account_id = undo.account_id.clone();
            let Some(connector) = manager.get(&account_id) else {
                ctx.show_toast(ToastAction::error("Not connected"));
                return;
            };
            let from = FolderId::new(undo.from.to_string());
            let to = FolderId::new(undo.to.to_string());
            let core_ids = core_message_ids(&undo.dest_ids);
            match connector.move_messages(&from, &core_ids, &to).await {
                Ok(new_uids) => {
                    let unread_n = unread_in_removed(&undo.snapshots);
                    let viewing_from = ctx.selected_mailbox.read().as_ref() == Some(&undo.from);
                    if viewing_from {
                        let (_, removed_sel) = take_messages_from_ui(ctx, &undo.dest_ids);
                        select_after_removed_row(manager, ctx, removed_sel).await;
                    } else if unread_n != 0 {
                        bump_mailbox_unread(ctx, &undo.from, -unread_n, false);
                    }
                    let new_ids = new_uids;
                    restore_snapshots(ctx, &undo.to, undo.snapshots, Some(&new_ids));
                    ctx.show_toast(ToastAction::info("Undone"));
                    persist_selected_messages(manager.cache(), ctx, &account_id).await;
                    persist_folder_tree(manager.cache(), ctx, &account_id).await;
                    let selected = ctx.selected_mailbox.read().clone();
                    if selected.as_ref() != Some(&undo.from) {
                        invalidate_mailbox_messages(manager.cache(), &account_id, &undo.from).await;
                    }
                    if selected.as_ref() != Some(&undo.to) {
                        invalidate_mailbox_messages(manager.cache(), &account_id, &undo.to).await;
                    }
                }
                Err(e) => {
                    error!("Failed to undo move: {}", e);
                    manager.note_imap_error(&account_id, &e);
                    ctx.show_toast(ToastAction::error(format!("Could not undo: {e}")));
                }
            }
        }
        UndoRequest::Unsnooze(undo) => {
            if !selected_account_is(ctx, &undo.account_id) {
                ctx.show_toast(ToastAction::error(
                    "Undo is not available after switching accounts",
                ));
                return;
            }
            let next =
                crate::ui_prefs::unsnooze_messages(&undo.account_id, &undo.mailbox_id, &undo.uids);
            if ctx.selected_mailbox.read().as_ref() == Some(&undo.mailbox_id) {
                ctx.snoozed_messages.set(next);
                restore_snapshots(ctx, &undo.mailbox_id, undo.snapshots, None);
            }
            ctx.show_toast(ToastAction::info("Undone"));
            persist_selected_messages(manager.cache(), ctx, &undo.account_id).await;
            let ids: Vec<MessageId> = undo
                .uids
                .iter()
                .filter(|uid| !uid.is_empty())
                .map(|uid| MessageId::new(FolderId::new(undo.mailbox_id.to_string()), uid.clone()))
                .collect();
            set_later_keyword(
                manager,
                ctx,
                &undo.account_id,
                &undo.mailbox_id,
                &ids,
                false,
            )
            .await;
        }
        UndoRequest::OpenSnoozed {
            account_id,
            mailbox_id,
            uid,
        } => {
            handle_open_snoozed_message(manager, ctx, account_id, mailbox_id, uid).await;
        }
    }
}

async fn handle_commit_dismissed(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    commit: DismissCommit,
) {
    match commit {
        DismissCommit::Delete {
            account_id,
            mailbox_id,
            message_ids,
        } => {
            let Some(connector) = manager.get(&account_id) else {
                return;
            };
            let folder_id = FolderId::new(mailbox_id.to_string());
            let core_ids = core_message_ids(&message_ids);
            if let Err(e) = connector.delete_messages(&folder_id, &core_ids).await {
                error!("Failed to permanently delete: {}", e);
                manager.note_imap_error(&account_id, &e);
                ctx.show_toast(ToastAction::error(format!("Could not delete: {e}")));
            }
        }
    }
}

const HEADER_SECTION: &str = "HEADER";

fn headers_request_active(ctx: &AppContext, message_id: &MessageId) -> bool {
    matches!(
        &*ctx.message_headers.read(),
        MessageHeadersState::Loading { message_id: id } if id == message_id
    )
}

async fn handle_fetch_message_headers(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    mailbox_id: MailboxId,
    message_id: MessageId,
) {
    ctx.message_headers.set(MessageHeadersState::Loading {
        message_id: message_id.clone(),
    });

    let Some(account_id) = ctx.selected_account.read().clone() else {
        ctx.message_headers.set(MessageHeadersState::Error {
            message_id,
            message: "No account selected".into(),
        });
        return;
    };
    let Some(connector) = manager.get(&account_id) else {
        ctx.message_headers.set(MessageHeadersState::Error {
            message_id,
            message: "Not connected".into(),
        });
        return;
    };

    let folder_id = FolderId::new(mailbox_id.to_string());
    info!("Fetching headers for message {}", message_id);

    let sections = [HEADER_SECTION.to_string()];
    let result = connector
        .fetch_raw_parts(&folder_id, &message_id, &sections)
        .await;

    if !headers_request_active(ctx, &message_id) {
        return;
    }

    match result {
        Ok(map) => match map.get(HEADER_SECTION) {
            Some(bytes) => {
                ctx.message_headers.set(MessageHeadersState::Ready {
                    message_id,
                    text: crate::headers::headers_bytes_to_text(bytes),
                });
            }
            None => {
                ctx.message_headers.set(MessageHeadersState::Error {
                    message_id,
                    message: "Server did not return headers".into(),
                });
            }
        },
        Err(e) => {
            error!("Failed to fetch headers for {}: {}", message_id, e);
            ctx.message_headers.set(MessageHeadersState::Error {
                message_id,
                message: e.to_string(),
            });
        }
    }
}

fn source_request_active(
    ctx: &AppContext,
    account_id: &AccountId,
    message_id: &MessageId,
    request_id: u64,
) -> bool {
    if ctx.selected_account.read().as_ref() != Some(account_id) {
        return false;
    }
    matches!(
        &*ctx.message_source.read(),
        MessageSourceState::Loading {
            account_id: a,
            message_id: m,
            request_id: r,
        } if a == account_id && m == message_id && *r == request_id
    )
}

async fn handle_fetch_message_source(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    mailbox_id: MailboxId,
    message_id: MessageId,
    request_id: u64,
) {
    if !source_request_active(ctx, &account_id, &message_id, request_id) {
        return;
    }

    let Some(connector) = manager.get(&account_id) else {
        if source_request_active(ctx, &account_id, &message_id, request_id) {
            ctx.message_source.set(MessageSourceState::Error {
                account_id,
                message_id,
                request_id,
                message: "Not connected".into(),
            });
        }
        return;
    };

    let folder_id = FolderId::new(mailbox_id.to_string());
    info!("Fetching source for message {}", message_id);

    let result = connector.fetch_raw_message(&folder_id, &message_id).await;

    if !source_request_active(ctx, &account_id, &message_id, request_id) {
        return;
    }

    match result {
        Ok(bytes) => {
            ctx.message_source.set(MessageSourceState::Ready {
                account_id,
                message_id,
                request_id,
                text: crate::source::source_bytes_to_text(&bytes),
            });
        }
        Err(e) => {
            error!("Failed to fetch source for {}: {}", message_id, e);
            ctx.message_source.set(MessageSourceState::Error {
                account_id,
                message_id,
                request_id,
                message: e.to_string(),
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_download_attachment(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    mailbox_id: MailboxId,
    message_id: MessageId,
    section: String,
    filename: String,
    content_type: String,
    encoding: TransferEncoding,
    size_hint: Option<u64>,
) {
    if !attachment_request_still_current(ctx, &account_id, &mailbox_id, &message_id) {
        return;
    }
    if let Some(finished) = cached_attachment_blob(ctx, &section, &filename, &content_type) {
        match finished.trigger_save() {
            Ok(()) => {
                ctx.download_status
                    .write()
                    .insert(section, DownloadStatus::Finished);
            }
            Err(e) => {
                error!("save download failed: {}", e);
                ctx.download_status
                    .write()
                    .insert(section, DownloadStatus::Error(e));
            }
        }
        return;
    }

    let Some(download) = stream_attachment_blob(
        manager,
        ctx,
        account_id.clone(),
        mailbox_id.clone(),
        message_id.clone(),
        section.clone(),
        filename,
        content_type,
        encoding,
        size_hint,
    )
    .await
    else {
        return;
    };
    if !attachment_request_still_current(ctx, &account_id, &mailbox_id, &message_id) {
        return;
    }

    match download.finish() {
        Ok(finished) => {
            let save_result = finished.trigger_save();
            remember_or_revoke_blob(ctx, &section, finished);
            match save_result {
                Ok(()) => {
                    ctx.download_status
                        .write()
                        .insert(section, DownloadStatus::Finished);
                }
                Err(e) => {
                    error!("save download failed: {}", e);
                    ctx.download_status
                        .write()
                        .insert(section, DownloadStatus::Error(e));
                }
            }
        }
        Err(e) => {
            error!("save download failed: {}", e);
            ctx.download_status
                .write()
                .insert(section, DownloadStatus::Error(e));
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_preview_attachment(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    mailbox_id: MailboxId,
    message_id: MessageId,
    section: String,
    filename: String,
    content_type: String,
    encoding: TransferEncoding,
    size_hint: Option<u64>,
) {
    if !attachment_request_still_current(ctx, &account_id, &mailbox_id, &message_id) {
        return;
    }
    if !is_previewable_content_type(&content_type) {
        ctx.download_status.write().insert(
            section,
            DownloadStatus::Error("this attachment type cannot be previewed".into()),
        );
        return;
    }

    if let Some(url) = ctx
        .attachment_blobs
        .read()
        .get(&section)
        .cloned()
        .filter(|url| !url.is_empty())
    {
        ctx.open_attachment_preview(AttachmentPreview {
            section,
            filename,
            content_type,
            object_url: url,
        });
        return;
    }

    let Some(download) = stream_attachment_blob(
        manager,
        ctx,
        account_id.clone(),
        mailbox_id.clone(),
        message_id.clone(),
        section.clone(),
        filename.clone(),
        content_type.clone(),
        encoding,
        size_hint,
    )
    .await
    else {
        return;
    };
    if !attachment_request_still_current(ctx, &account_id, &mailbox_id, &message_id) {
        return;
    }

    match download.finish() {
        Ok(finished) => {
            let url = finished.object_url.clone();
            remember_or_revoke_blob(ctx, &section, finished);
            if url.is_empty() {
                ctx.download_status.write().insert(
                    section,
                    DownloadStatus::Error("preview is only available in the browser".into()),
                );
                return;
            }
            ctx.open_attachment_preview(AttachmentPreview {
                section: section.clone(),
                filename,
                content_type,
                object_url: url,
            });
            ctx.download_status
                .write()
                .insert(section, DownloadStatus::Finished);
        }
        Err(e) => {
            error!("preview assemble failed: {}", e);
            ctx.download_status
                .write()
                .insert(section, DownloadStatus::Error(e));
        }
    }
}

fn cached_attachment_blob(
    ctx: &AppContext,
    section: &str,
    filename: &str,
    content_type: &str,
) -> Option<FinishedAttachment> {
    let url = ctx.attachment_blobs.read().get(section).cloned()?;
    if url.is_empty() {
        return None;
    }
    Some(FinishedAttachment {
        object_url: url,
        filename: filename.to_string(),
        content_type: content_type.to_string(),
    })
}

fn remember_or_revoke_blob(ctx: &mut AppContext, section: &str, finished: FinishedAttachment) {
    if is_previewable_content_type(&finished.content_type) && !finished.object_url.is_empty() {
        ctx.attachment_blobs
            .write()
            .insert(section.to_string(), finished.object_url);
    } else {
        finished.revoke();
    }
}

#[allow(clippy::too_many_arguments)]
fn attachment_request_still_current(
    ctx: &AppContext,
    account_id: &AccountId,
    mailbox_id: &MailboxId,
    message_id: &MessageId,
) -> bool {
    selected_account_is(ctx, account_id)
        && ctx.selected_mailbox.read().as_ref() == Some(mailbox_id)
        && ctx.selection.read().focus() == Some(message_id)
        && matches!(
            &*ctx.message_view.read(),
            MessageViewState::Ready {
                account_id: view_account,
                message_id: view_message,
                ..
            } if view_account == account_id && view_message == message_id
        )
}

async fn stream_attachment_blob(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    mailbox_id: MailboxId,
    message_id: MessageId,
    section: String,
    filename: String,
    content_type: String,
    encoding: TransferEncoding,
    size_hint: Option<u64>,
) -> Option<StreamingBlobDownload> {
    // Ignore if user navigated away or switched account.
    if !attachment_request_still_current(ctx, &account_id, &mailbox_id, &message_id) {
        return None;
    }
    if size_hint.is_some_and(|s| s as usize > MAX_DOWNLOAD_BYTES) {
        ctx.download_status.write().insert(
            section.clone(),
            DownloadStatus::Error(format!(
                "attachment too large (max {} bytes)",
                MAX_DOWNLOAD_BYTES
            )),
        );
        return None;
    }

    let Some(connector) = manager.get(&account_id) else {
        ctx.download_status
            .write()
            .insert(section, DownloadStatus::Error("Not connected".into()));
        return None;
    };

    ctx.download_status.write().insert(
        section.clone(),
        DownloadStatus::InProgress {
            received: 0,
            total: size_hint,
        },
    );

    let folder_id = FolderId::new(mailbox_id.to_string());
    info!(
        "Downloading attachment section {} for message {}",
        section, message_id
    );

    let stream_result = connector
        .stream_raw_part(&folder_id, &message_id, &section)
        .await;

    let mut stream = match stream_result {
        Ok(s) => s,
        Err(e) => {
            error!("stream_raw_part failed: {}", e);
            note_selected_imap_error(manager, ctx, &e);
            ctx.download_status
                .write()
                .insert(section, DownloadStatus::Error(e.to_string()));
            return None;
        }
    };

    // Stream wire → TE decode → Blob parts (no full-file Vec in Rust).
    let mut download = StreamingBlobDownload::new(encoding, filename, content_type);
    let mut total_hint = size_hint;

    while let Some(item) = stream.next().await {
        match item {
            Ok(chunk) => {
                if let Some(h) = chunk.total_hint {
                    total_hint = Some(h);
                }
                if let Err(e) = download.push_wire_chunk(&chunk.data) {
                    error!("download stream error: {}", e);
                    ctx.download_status
                        .write()
                        .insert(section.clone(), DownloadStatus::Error(e));
                    return None;
                }
                // Drop wire chunk after decode; only Blob parts retain decoded data.
                drop(chunk);
                ctx.download_status.write().insert(
                    section.clone(),
                    DownloadStatus::InProgress {
                        received: download.wire_received,
                        total: total_hint,
                    },
                );
            }
            Err(e) => {
                error!("download chunk error: {}", e);
                note_selected_imap_error(manager, ctx, &e);
                ctx.download_status
                    .write()
                    .insert(section.clone(), DownloadStatus::Error(e.to_string()));
                return None;
            }
        }
    }

    Some(download)
}

async fn handle_save_message_eml(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    mailbox_id: MailboxId,
    message_id: MessageId,
    filename: String,
    size_hint: Option<u64>,
) {
    if !selected_account_is(ctx, &account_id) {
        return;
    }
    if ctx.selection.read().focus() != Some(&message_id) {
        return;
    }
    if matches!(
        ctx.download_status.read().get(EML_DOWNLOAD_KEY),
        Some(DownloadStatus::InProgress { .. })
    ) {
        return;
    }
    if size_hint.is_some_and(|s| s > MAX_DOWNLOAD_BYTES as u64) {
        ctx.show_toast(ToastAction::error(format!(
            "Message is too large to save (max {} bytes)",
            MAX_DOWNLOAD_BYTES
        )));
        return;
    }

    let Some(connector) = manager.get(&account_id) else {
        ctx.show_toast(ToastAction::error("Not connected"));
        return;
    };

    ctx.download_status.write().insert(
        EML_DOWNLOAD_KEY.into(),
        DownloadStatus::InProgress {
            received: 0,
            total: size_hint,
        },
    );
    ctx.show_toast(ToastAction::info("Saving message…"));

    let folder_id = FolderId::new(mailbox_id.to_string());
    info!("Saving message {} as {}", message_id, filename);

    let bytes = match connector.fetch_raw_message(&folder_id, &message_id).await {
        Ok(bytes) => bytes,
        Err(e) => {
            error!("fetch_raw_message failed: {}", e);
            if selected_account_is(ctx, &account_id)
                && ctx.selection.read().focus() == Some(&message_id)
            {
                ctx.download_status.write().insert(
                    EML_DOWNLOAD_KEY.into(),
                    DownloadStatus::Error(e.to_string()),
                );
                ctx.show_toast(ToastAction::error(format!("Could not save message: {e}")));
            } else {
                ctx.download_status.write().remove(EML_DOWNLOAD_KEY);
            }
            return;
        }
    };

    if !selected_account_is(ctx, &account_id) || ctx.selection.read().focus() != Some(&message_id) {
        ctx.download_status.write().remove(EML_DOWNLOAD_KEY);
        return;
    }

    let mut download = StreamingBlobDownload::new(
        TransferEncoding::SevenBit,
        filename,
        "message/rfc822".into(),
    );
    if let Err(e) = download.push_wire_chunk(&bytes) {
        error!("save .eml failed: {}", e);
        ctx.download_status
            .write()
            .insert(EML_DOWNLOAD_KEY.into(), DownloadStatus::Error(e.clone()));
        ctx.show_toast(ToastAction::error(format!("Could not save message: {e}")));
        return;
    }
    if let Err(e) = download.finish_and_save() {
        error!("save .eml failed: {}", e);
        ctx.download_status
            .write()
            .insert(EML_DOWNLOAD_KEY.into(), DownloadStatus::Error(e.clone()));
        ctx.show_toast(ToastAction::error(format!("Could not save message: {e}")));
        return;
    }

    ctx.download_status
        .write()
        .insert(EML_DOWNLOAD_KEY.into(), DownloadStatus::Finished);
}

fn mail_xfer_busy(ctx: &AppContext, key: &str) -> bool {
    matches!(
        ctx.download_status.read().get(key),
        Some(DownloadStatus::InProgress { .. } | DownloadStatus::Queued)
    )
}

async fn handle_export_messages(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    mailbox_id: MailboxId,
    items: Vec<ExportMessageItem>,
    format: MailExportFormat,
    folder_label: String,
) {
    if !selected_account_is(ctx, &account_id) {
        return;
    }
    if ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id) {
        return;
    }
    if mail_xfer_busy(ctx, MAIL_EXPORT_KEY) || mail_xfer_busy(ctx, EML_DOWNLOAD_KEY) {
        return;
    }
    if items.is_empty() {
        ctx.show_toast(ToastAction::error("Select messages to export"));
        return;
    }
    if items.len() > MAX_EXPORT_MESSAGES {
        ctx.show_toast(ToastAction::error(format!(
            "Export is limited to {MAX_EXPORT_MESSAGES} messages"
        )));
        return;
    }
    if items.iter().any(|item| {
        item.size_hint
            .is_some_and(|s| s > MAX_DOWNLOAD_BYTES as u64)
    }) {
        ctx.show_toast(ToastAction::error(format!(
            "A selected message is too large to export (max {} bytes)",
            MAX_DOWNLOAD_BYTES
        )));
        return;
    }

    let Some(connector) = manager.get(&account_id) else {
        ctx.show_toast(ToastAction::error("Not connected"));
        return;
    };

    let total = items.len() as u64;
    ctx.download_status.write().insert(
        MAIL_EXPORT_KEY.into(),
        DownloadStatus::InProgress {
            received: 0,
            total: Some(total),
        },
    );
    ctx.show_toast(ToastAction::info(if items.len() == 1 {
        "Exporting message…".into()
    } else {
        format!("Exporting {} messages…", items.len())
    }));

    let folder_id = FolderId::new(mailbox_id.to_string());
    let mut fetched = Vec::with_capacity(items.len());
    for (i, item) in items.into_iter().enumerate() {
        if !selected_account_is(ctx, &account_id)
            || ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id)
        {
            ctx.download_status.write().remove(MAIL_EXPORT_KEY);
            return;
        }
        match connector
            .fetch_raw_message(&folder_id, &item.message_id)
            .await
        {
            Ok(bytes) => {
                fetched.push(Rfc822Message {
                    filename: item.filename,
                    bytes,
                });
            }
            Err(e) => {
                error!("export fetch failed: {}", e);
                if selected_account_is(ctx, &account_id)
                    && ctx.selected_mailbox.read().as_ref() == Some(&mailbox_id)
                {
                    ctx.download_status
                        .write()
                        .insert(MAIL_EXPORT_KEY.into(), DownloadStatus::Error(e.to_string()));
                    ctx.show_toast(ToastAction::error(format!(
                        "Could not export messages: {e}"
                    )));
                } else {
                    ctx.download_status.write().remove(MAIL_EXPORT_KEY);
                }
                return;
            }
        }
        ctx.download_status.write().insert(
            MAIL_EXPORT_KEY.into(),
            DownloadStatus::InProgress {
                received: (i as u64) + 1,
                total: Some(total),
            },
        );
    }

    if !selected_account_is(ctx, &account_id)
        || ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id)
    {
        ctx.download_status.write().remove(MAIL_EXPORT_KEY);
        return;
    }

    match pack_export_named(&fetched, format, &folder_label) {
        Ok((filename, mime, bytes)) => {
            if let Err(e) = save_bytes_download(&filename, mime, &bytes) {
                error!("export save failed: {}", e);
                ctx.download_status
                    .write()
                    .insert(MAIL_EXPORT_KEY.into(), DownloadStatus::Error(e.clone()));
                ctx.show_toast(ToastAction::error(format!(
                    "Could not export messages: {e}"
                )));
                return;
            }
            ctx.download_status
                .write()
                .insert(MAIL_EXPORT_KEY.into(), DownloadStatus::Finished);
        }
        Err(e) => {
            error!("export pack failed: {}", e);
            ctx.download_status
                .write()
                .insert(MAIL_EXPORT_KEY.into(), DownloadStatus::Error(e.clone()));
            ctx.show_toast(ToastAction::error(format!(
                "Could not export messages: {e}"
            )));
        }
    }
}

async fn handle_import_messages(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    mailbox_id: MailboxId,
    messages: Vec<Rfc822Message>,
) {
    if !selected_account_is(ctx, &account_id) {
        return;
    }
    if ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id) {
        return;
    }
    if mail_xfer_busy(ctx, MAIL_IMPORT_KEY) {
        return;
    }
    if messages.is_empty() {
        ctx.show_toast(ToastAction::error("No messages to import"));
        return;
    }
    if messages.iter().any(|m| m.bytes.len() > MAX_DOWNLOAD_BYTES) {
        ctx.show_toast(ToastAction::error(format!(
            "A message is too large to import (max {} bytes)",
            MAX_DOWNLOAD_BYTES
        )));
        return;
    }

    let selectable = ctx
        .mailbox_nodes
        .read()
        .get(&mailbox_id)
        .is_some_and(|n| n.selectable);
    if !selectable {
        ctx.show_toast(ToastAction::error("This folder cannot receive messages"));
        return;
    }

    let Some(connector) = manager.get(&account_id) else {
        ctx.show_toast(ToastAction::error("Not connected"));
        return;
    };

    let total = messages.len() as u64;
    ctx.download_status.write().insert(
        MAIL_IMPORT_KEY.into(),
        DownloadStatus::InProgress {
            received: 0,
            total: Some(total),
        },
    );
    ctx.show_toast(ToastAction::info(if messages.len() == 1 {
        "Importing message…".into()
    } else {
        format!("Importing {} messages…", messages.len())
    }));

    let mailbox = mailbox_id.to_string();
    let mut imported = 0usize;
    for (i, msg) in messages.iter().enumerate() {
        if !selected_account_is(ctx, &account_id)
            || ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id)
        {
            ctx.download_status.write().remove(MAIL_IMPORT_KEY);
            return;
        }
        if let Err(e) = connector.append_rfc822(&mailbox, &msg.bytes).await {
            error!("import APPEND failed: {}", e);
            manager.note_imap_error_msg(&account_id, &e.to_string());
            if selected_account_is(ctx, &account_id)
                && ctx.selected_mailbox.read().as_ref() == Some(&mailbox_id)
            {
                ctx.download_status
                    .write()
                    .insert(MAIL_IMPORT_KEY.into(), DownloadStatus::Error(e.to_string()));
                let extra = if imported == 0 {
                    String::new()
                } else {
                    format!(" ({imported} imported before the error)")
                };
                ctx.show_toast(ToastAction::error(format!(
                    "Could not import {}: {e}{extra}",
                    msg.filename
                )));
            } else {
                ctx.download_status.write().remove(MAIL_IMPORT_KEY);
            }
            if imported > 0 {
                refresh_if_viewing_mailbox(manager, ctx, &account_id, &mailbox).await;
            }
            return;
        }
        imported += 1;
        ctx.download_status.write().insert(
            MAIL_IMPORT_KEY.into(),
            DownloadStatus::InProgress {
                received: (i as u64) + 1,
                total: Some(total),
            },
        );
    }

    ctx.download_status
        .write()
        .insert(MAIL_IMPORT_KEY.into(), DownloadStatus::Finished);
    refresh_if_viewing_mailbox(manager, ctx, &account_id, &mailbox).await;
    ctx.show_toast(ToastAction::info(if imported == 1 {
        "Imported 1 message".into()
    } else {
        format!("Imported {imported} messages")
    }));
}

struct PendingForwardFetch {
    attachment_id: String,
    message_id: MessageId,
    section: String,
    encoding: TransferEncoding,
}

fn compose_session_matches(ctx: &AppContext, draft_id: &str, account_id: &AccountId) -> bool {
    ctx.compose_draft
        .read()
        .as_ref()
        .is_some_and(|s| s.draft.id.as_str() == draft_id && s.account_id == *account_id)
}

fn collect_pending_forward_fetches(
    ctx: &AppContext,
    draft_id: &str,
) -> Option<Vec<PendingForwardFetch>> {
    let slot = ctx.compose_draft.read();
    let session = slot.as_ref()?;
    if session.draft.id.as_str() != draft_id {
        return None;
    }
    Some(
        session
            .draft
            .attachments
            .iter()
            .chain(session.stashed_originals.iter())
            .filter_map(|a| {
                if !matches!(a.data, AttachmentData::Pending) {
                    return None;
                }
                let src = a.source.as_ref()?;
                Some(PendingForwardFetch {
                    attachment_id: a.id.0.clone(),
                    message_id: src.message_id.clone(),
                    section: src.section.clone(),
                    encoding: src.encoding,
                })
            })
            .collect(),
    )
}

fn remove_forward_attachment(session: &mut ComposeSession, attachment_id: &str) {
    session
        .draft
        .attachments
        .retain(|a| a.id.0 != attachment_id);
    session
        .stashed_originals
        .retain(|a| a.id.0 != attachment_id);
}

fn fail_forward_fetches(ctx: &mut AppContext, draft_id: &str, message: &str) {
    let mut slot = ctx.compose_draft.write();
    let Some(session) = slot.as_mut() else {
        return;
    };
    if session.draft.id.as_str() != draft_id {
        return;
    }
    let ids: Vec<String> = session
        .draft
        .attachments
        .iter()
        .chain(session.stashed_originals.iter())
        .filter(|a| matches!(a.data, AttachmentData::Pending) && a.source.is_some())
        .map(|a| a.id.0.clone())
        .collect();
    for id in ids {
        remove_forward_attachment(session, &id);
    }
    session.draft.prefill_warnings.push(message.to_string());
    session.draft.touch();
}

fn apply_fetched_attachment(
    ctx: &mut AppContext,
    draft_id: &str,
    attachment_id: &str,
    bytes: Vec<u8>,
) {
    use mailiner_composer::shell::attachment_list::{draft_payload_bytes, would_exceed_draft_cap};

    let mut slot = ctx.compose_draft.write();
    let Some(session) = slot.as_mut() else {
        return;
    };
    if session.draft.id.as_str() != draft_id {
        return;
    }
    let sz = bytes.len() as u64;
    if sz > caps::MAX_FILE_BYTES {
        remove_forward_attachment(session, attachment_id);
        session.draft.prefill_warnings.push(format!(
            "Skipped an original attachment: larger than {} MiB.",
            caps::MAX_FILE_BYTES / (1024 * 1024)
        ));
        session.draft.touch();
        return;
    }
    let on_draft = session
        .draft
        .attachments
        .iter()
        .any(|a| a.id.0 == attachment_id);
    if on_draft && would_exceed_draft_cap(draft_payload_bytes(&session.draft), sz) {
        remove_forward_attachment(session, attachment_id);
        session
            .draft
            .prefill_warnings
            .push("Skipped an original attachment: draft size limit.".into());
        session.draft.touch();
        return;
    }
    let att = session
        .draft
        .attachments
        .iter_mut()
        .find(|a| a.id.0 == attachment_id)
        .or_else(|| {
            session
                .stashed_originals
                .iter_mut()
                .find(|a| a.id.0 == attachment_id)
        });
    if let Some(att) = att {
        att.size = sz;
        att.data = AttachmentData::Bytes(bytes);
        session.draft.touch();
    }
}

async fn handle_fetch_compose_attachments(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    draft_id: String,
    account_id: AccountId,
) {
    let Some(items) = collect_pending_forward_fetches(ctx, &draft_id) else {
        return;
    };
    if items.is_empty() {
        return;
    }
    if !compose_session_matches(ctx, &draft_id, &account_id) {
        return;
    }

    let Some(connector) = manager.get(&account_id) else {
        fail_forward_fetches(ctx, &draft_id, "Could not load original attachments.");
        return;
    };

    let mut by_msg: HashMap<MessageId, Vec<PendingForwardFetch>> = HashMap::new();
    for item in items {
        by_msg
            .entry(item.message_id.clone())
            .or_default()
            .push(item);
    }

    for (mid, group) in by_msg {
        let mut sections: Vec<String> = group.iter().map(|g| g.section.clone()).collect();
        sections.sort();
        sections.dedup();
        let raw = match connector
            .fetch_raw_parts(mid.folder_id(), &mid, &sections)
            .await
        {
            Ok(map) => map,
            Err(e) => {
                error!("forward attachment fetch failed: {e}");
                if !compose_session_matches(ctx, &draft_id, &account_id) {
                    return;
                }
                for item in &group {
                    apply_fetched_attachment_missing(ctx, &draft_id, &item.attachment_id);
                }
                continue;
            }
        };
        if !compose_session_matches(ctx, &draft_id, &account_id) {
            return;
        }
        for item in group {
            match raw.get(&item.section) {
                Some(wire) => match decode_transfer_encoding(wire, item.encoding.as_str()) {
                    Ok(bytes) => {
                        apply_fetched_attachment(ctx, &draft_id, &item.attachment_id, bytes)
                    }
                    Err(e) => {
                        error!("forward attachment decode failed: {e}");
                        apply_fetched_attachment_missing(ctx, &draft_id, &item.attachment_id);
                    }
                },
                None => apply_fetched_attachment_missing(ctx, &draft_id, &item.attachment_id),
            }
        }
    }
}

fn apply_fetched_attachment_missing(ctx: &mut AppContext, draft_id: &str, attachment_id: &str) {
    let mut slot = ctx.compose_draft.write();
    let Some(session) = slot.as_mut() else {
        return;
    };
    if session.draft.id.as_str() != draft_id {
        return;
    }
    remove_forward_attachment(session, attachment_id);
    if !session
        .draft
        .prefill_warnings
        .iter()
        .any(|w| w.contains("Could not load"))
    {
        session
            .draft
            .prefill_warnings
            .push("Could not load some original attachments.".into());
    }
    session.draft.touch();
}

async fn refresh_outbox_signal(outbox: &dyn OutboxStore, ctx: &mut AppContext) {
    match outbox.list().await {
        Ok(items) => {
            ctx.outbox
                .set(items.iter().map(OutboxListEntry::from).collect());
        }
        Err(e) => {
            warn!("outbox list failed: {e}");
        }
    }
}

async fn recover_outbox(outbox: &dyn OutboxStore, ctx: &mut AppContext) {
    if let Ok(items) = outbox.list().await {
        for mut item in items {
            if item.state == OutboxItemState::Sending {
                item.state = OutboxItemState::Queued;
                item.last_error = Some("Sending was interrupted. Will retry.".into());
                item.updated_at = chrono::Utc::now();
                let _ = outbox.upsert(&item).await;
            }
        }
    }
    refresh_outbox_signal(outbox, ctx).await;
}

async fn purge_missing_accounts(
    manager: &AccountConnectionManager,
    outbox: &dyn OutboxStore,
    ctx: &mut AppContext,
    inflight: &mut SmtpInflight,
) {
    let known: Vec<AccountId> = match manager.store().list().await {
        Ok(list) => list.into_iter().map(|c| c.id).collect(),
        Err(_) => return,
    };
    if let Ok(items) = outbox.list().await {
        for item in items {
            if !known.iter().any(|id| *id == item.account_id) {
                inflight.cancel_for_account(&item.account_id);
                let _ = outbox.delete_for_account(&item.account_id).await;
            }
        }
    }
    refresh_outbox_signal(outbox, ctx).await;
}

async fn handle_send_message(
    manager: &mut AccountConnectionManager,
    ctx: &mut AppContext,
    outbox: &dyn OutboxStore,
    smtp_tx: &UnboundedSender<CoreEvent>,
    inflight: &mut SmtpInflight,
    account_id: AccountId,
    request: SubmitRequest,
    display: OutboxDisplay,
    draft_id: String,
    bcc_header: Option<String>,
    reply_source: Option<MessageId>,
    imap_draft: Option<MessageId>,
) {
    let Some(config) = manager.resolve_config(&account_id).await else {
        ctx.set_send_status(
            account_id.clone(),
            SendState::Failed {
                account_id,
                kind: SendErrorKind::NotConfigured,
                message: "Account not found.".into(),
                retryable: false,
            },
        );
        return;
    };
    let config = match manager.ensure_oauth_fresh(config).await {
        Ok(c) => c,
        Err(e) => {
            ctx.set_send_status(
                account_id.clone(),
                SendState::Failed {
                    account_id,
                    kind: SendErrorKind::Auth,
                    message: e.message,
                    retryable: true,
                },
            );
            return;
        }
    };
    if let Err(err) = preflight(&config) {
        ctx.set_send_status(
            account_id.clone(),
            SendState::Failed {
                account_id,
                kind: err.kind,
                message: err.message,
                retryable: false,
            },
        );
        return;
    }
    let mut item = match OutboxItem::from_request(
        account_id.clone(),
        &request,
        display.subject,
        display.to_preview,
    ) {
        Ok(i) => i,
        Err(e) => {
            ctx.set_send_status(
                account_id.clone(),
                SendState::Failed {
                    account_id,
                    kind: SendErrorKind::MessageTooLarge,
                    message: e.to_string(),
                    retryable: false,
                },
            );
            return;
        }
    };
    if let Some(bcc) = bcc_header {
        item.set_bcc_header(bcc);
    }
    if let Some(source) = reply_source {
        item.set_reply_source(source);
    }
    if let Err(e) = outbox.upsert(&item).await {
        ctx.set_send_status(
            account_id.clone(),
            SendState::Failed {
                account_id,
                kind: SendErrorKind::Internal,
                message: e.to_string(),
                retryable: false,
            },
        );
        return;
    }
    refresh_outbox_signal(outbox, ctx).await;
    crate::draft_store::clear_draft_if(&account_id, &draft_id);
    if let Some(message_id) = imap_draft {
        handle_delete_imap_draft(manager, ctx, account_id.clone(), message_id).await;
    }
    if ctx
        .compose_draft
        .read()
        .as_ref()
        .is_some_and(|s| s.draft.id.as_str() == draft_id)
    {
        ctx.compose_draft.set(None);
    }
    if inflight.is_busy(&account_id) {
        return;
    }
    item.attempts = 1;
    persist_sending(outbox, &mut item).await;
    refresh_outbox_signal(outbox, ctx).await;
    start_send_item(ctx, smtp_tx, inflight, config, item, request);
}

fn start_send_item(
    ctx: &mut AppContext,
    smtp_tx: &UnboundedSender<CoreEvent>,
    inflight: &mut SmtpInflight,
    config: AccountConfig,
    item: OutboxItem,
    request: SubmitRequest,
) {
    let generation = inflight.alloc_generation();
    let (cancel_tx, cancel_rx) = futures_channel::oneshot::channel();
    let account_id = config.id.clone();
    inflight.insert(InFlightSmtp {
        account_id: account_id.clone(),
        generation,
        cancel_tx: Some(cancel_tx),
        outbox_id: Some(item.id.clone()),
        is_test: false,
        reply_source: item.reply_source.clone(),
    });
    ctx.set_send_status(
        account_id.clone(),
        SendState::Sending {
            account_id,
            phase: SendPhase::Connecting,
        },
    );
    spawn_submit(
        config,
        request,
        generation,
        cancel_rx,
        smtp_tx.clone(),
        SEND_TIMEOUT_MS,
        Some(item.id),
    );
}

async fn persist_sending(outbox: &dyn OutboxStore, item: &mut OutboxItem) {
    item.state = OutboxItemState::Sending;
    item.updated_at = chrono::Utc::now();
    let _ = outbox.upsert(item).await;
}

async fn drain_outbox(
    manager: &mut AccountConnectionManager,
    ctx: &mut AppContext,
    outbox: &dyn OutboxStore,
    smtp_tx: &UnboundedSender<CoreEvent>,
    inflight: &mut SmtpInflight,
) {
    // Fill one slot per idle account. Skip items already tried this pass so a
    // Failed upsert cannot spin; do not treat a failed item as occupying the
    // account slot (a later Queued row for the same account can still start).
    let mut skip_ids: Vec<OutboxId> = Vec::new();
    loop {
        let blocked: Vec<AccountId> = inflight.busy_account_ids().cloned().collect();
        let items = match outbox.list().await {
            Ok(v) => v,
            Err(e) => {
                warn!("outbox list failed: {e}");
                return;
            }
        };
        let Some(mut item) = pick_oldest_queued(&items, &blocked, &skip_ids) else {
            return;
        };
        skip_ids.push(item.id.clone());
        let Some(config) = manager.resolve_config(&item.account_id).await else {
            item.state = OutboxItemState::Failed;
            item.last_error = Some("Account is no longer available.".into());
            let _ = outbox.upsert(&item).await;
            refresh_outbox_signal(outbox, ctx).await;
            continue;
        };
        let config = match manager.ensure_oauth_fresh(config).await {
            Ok(c) => c,
            Err(e) => {
                item.state = OutboxItemState::Failed;
                item.last_error_kind = Some(SendErrorKind::Auth);
                item.last_error = Some(e.message);
                let _ = outbox.upsert(&item).await;
                refresh_outbox_signal(outbox, ctx).await;
                continue;
            }
        };
        if let Err(err) = preflight(&config) {
            item.state = OutboxItemState::Failed;
            item.last_error_kind = Some(err.kind);
            item.last_error = Some(err.message);
            let _ = outbox.upsert(&item).await;
            refresh_outbox_signal(outbox, ctx).await;
            continue;
        }
        let request = match item.to_request() {
            Ok(r) => r,
            Err(e) => {
                item.state = OutboxItemState::Failed;
                item.last_error = Some(e.to_string());
                let _ = outbox.upsert(&item).await;
                refresh_outbox_signal(outbox, ctx).await;
                continue;
            }
        };
        item.attempts = item.attempts.saturating_add(1);
        persist_sending(outbox, &mut item).await;
        refresh_outbox_signal(outbox, ctx).await;
        start_send_item(ctx, smtp_tx, inflight, config, item, request);
    }
}

async fn handle_test_smtp(
    ctx: &mut AppContext,
    smtp_tx: &UnboundedSender<CoreEvent>,
    inflight: &mut SmtpInflight,
    request_id: AccountId,
    config: AccountConfig,
) {
    if inflight.is_busy(&config.id) {
        ctx.set_smtp_test_status(
            request_id,
            SendState::Failed {
                account_id: config.id.clone(),
                kind: SendErrorKind::Internal,
                message: "A send is already in progress.".into(),
                retryable: true,
            },
        );
        return;
    }
    if let Err(err) = preflight(&config) {
        ctx.set_smtp_test_status(
            request_id,
            SendState::Failed {
                account_id: config.id.clone(),
                kind: err.kind,
                message: err.message,
                retryable: false,
            },
        );
        return;
    }
    if !ctx.set_smtp_test_status(
        request_id.clone(),
        SendState::Sending {
            account_id: config.id.clone(),
            phase: SendPhase::Connecting,
        },
    ) {
        return;
    }
    let generation = inflight.alloc_generation();
    let (cancel_tx, cancel_rx) = futures_channel::oneshot::channel();
    inflight.insert(InFlightSmtp {
        account_id: config.id.clone(),
        generation,
        cancel_tx: Some(cancel_tx),
        outbox_id: None,
        is_test: true,
        reply_source: None,
    });
    spawn_test(config, request_id, generation, cancel_rx, smtp_tx.clone());
}

async fn handle_lock_secrets(manager: &mut AccountConnectionManager, ctx: &mut AppContext) {
    manager.disconnect_all(ctx).await;
    manager.store().lock_session();
    ctx.reset_after_lock();
}

async fn handle_clear_local_data(
    manager: &mut AccountConnectionManager,
    ctx: &mut AppContext,
    outbox: &dyn OutboxStore,
) {
    manager.disconnect_all(ctx).await;

    match manager.store().list().await {
        Ok(list) => {
            for cfg in list {
                if let Err(e) = manager.store().delete(&cfg.id).await {
                    warn!("sign-out delete {} failed: {e}", cfg.id);
                }
            }
        }
        Err(e) => warn!("sign-out list accounts failed: {e}"),
    }
    if let Err(e) = manager.store().set_active_id(None).await {
        warn!("sign-out set_active_id(None) failed: {e}");
    }
    if let Ok(items) = outbox.list().await {
        for item in items {
            let _ = outbox.delete(&item.id).await;
        }
    }

    // After the current handler has finished: no in-flight persist can land
    // after this wipe.
    if let Err(e) = manager.cache().clear_all().await {
        warn!("mail cache clear_all failed: {e}");
    }
    let wipe_err = crate::local_data::clear_mailiner_local_storage().err();
    if let Some(e) = wipe_err {
        warn!("clear_mailiner_local_storage failed: {e}");
        ctx.show_toast(ToastAction::Error {
            message: format!("Some local Mailiner data could not be removed: {e}"),
        });
        ctx.sign_out_error.set(Some(e.to_string()));
        return;
    }
    ctx.reset_after_sign_out();
    let next_epoch = ctx.sign_out_epoch.peek().wrapping_add(1);
    ctx.sign_out_epoch.set(next_epoch);
}

async fn handle_smtp_finished(
    manager: &mut AccountConnectionManager,
    ctx: &mut AppContext,
    outbox: &dyn OutboxStore,
    smtp_tx: &UnboundedSender<CoreEvent>,
    inflight: &mut SmtpInflight,
    generation: u64,
    outcome: SmtpOutcome,
) {
    if inflight.is_empty() && !inflight.contains_generation(generation) {
        // Sign-out already dropped the slots; do not persist. Ordinary
        // cancel (Disconnect / DeleteOutbox / purge) keeps the slot so
        // this handler can settle it below.
        return;
    }
    let Some(flight) = inflight.take_by_generation(generation) else {
        // Slot already gone (item deleted) but DATA may still have succeeded.
        // Never leave that rfc822 queued for a second submit.
        if let SmtpOutcome::Send {
            outbox_id: Some(id),
            result: Ok(_),
        } = outcome
        {
            let mark = match outbox.get(&id).await {
                Ok(Some(item)) => item
                    .reply_source
                    .map(|message_id| (item.account_id, message_id)),
                _ => None,
            };
            let _ = outbox.delete(&id).await;
            refresh_outbox_signal(outbox, ctx).await;
            if let Some((account_id, message_id)) = mark {
                let _ = smtp_tx.unbounded_send(CoreEvent::MarkAnswered {
                    account_id,
                    message_id,
                });
            }
        }
        return;
    };
    match outcome {
        SmtpOutcome::Send {
            result: Ok(receipt),
            ..
        } => {
            let (rfc822, reply_source) = if let Some(id) = &flight.outbox_id {
                match outbox.get(id).await {
                    Ok(Some(item)) => (
                        item.rfc822_for_mailbox().ok(),
                        item.reply_source.or(flight.reply_source.clone()),
                    ),
                    _ => (None, flight.reply_source.clone()),
                }
            } else {
                (None, flight.reply_source.clone())
            };
            if let Some(id) = flight.outbox_id {
                let _ = outbox.delete(&id).await;
            }
            ctx.set_send_status(
                flight.account_id.clone(),
                SendState::Sent {
                    account_id: flight.account_id.clone(),
                },
            );
            ctx.show_toast(ToastAction::Sent);
            let _ = receipt;
            if let Some(rfc822) = rfc822 {
                let _ = smtp_tx.unbounded_send(CoreEvent::ArchiveSent {
                    account_id: flight.account_id.clone(),
                    rfc822,
                });
            }
            if let Some(message_id) = reply_source {
                let _ = smtp_tx.unbounded_send(CoreEvent::MarkAnswered {
                    account_id: flight.account_id,
                    message_id,
                });
            }
        }
        SmtpOutcome::Send {
            result: Err(err), ..
        } => {
            if let Some(id) = &flight.outbox_id {
                if let Ok(Some(mut item)) = outbox.get(id).await {
                    item.last_error_kind = Some(err.kind);
                    item.last_error = Some(err.message.clone());
                    item.updated_at = chrono::Utc::now();
                    if err.kind == SendErrorKind::Cancelled
                        || (err.kind.is_retryable() && item.attempts < MAX_OUTBOX_AUTO_ATTEMPTS)
                    {
                        item.state = OutboxItemState::Queued;
                    } else {
                        item.state = OutboxItemState::Failed;
                    }
                    let _ = outbox.upsert(&item).await;
                }
            }
            ctx.set_send_status(
                flight.account_id.clone(),
                SendState::Failed {
                    account_id: flight.account_id,
                    kind: err.kind,
                    message: err.message,
                    retryable: err.kind.is_retryable(),
                },
            );
        }
        SmtpOutcome::Test { request_id, result } => {
            let state = match result {
                Ok(()) => SendState::Sent {
                    account_id: flight.account_id,
                },
                Err(err) => SendState::Failed {
                    account_id: flight.account_id,
                    kind: err.kind,
                    message: err.message,
                    retryable: err.kind.is_retryable(),
                },
            };
            ctx.set_smtp_test_status(request_id, state);
        }
    }
    refresh_outbox_signal(outbox, ctx).await;
    drain_outbox(manager, ctx, outbox, smtp_tx, inflight).await;
}

const ARCHIVE_SENT_WARN: &str = "Could not save a copy in Sent.";
const ARCHIVE_DRAFT_WARN: &str = "Could not save this draft on the server.";
const DELETE_DRAFT_WARN: &str = "Could not delete the server draft.";

async fn handle_archive_sent(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    rfc822: Vec<u8>,
) {
    let Some(connector) = manager.get(&account_id) else {
        warn!("ArchiveSent: no IMAP connector for {account_id}");
        ctx.show_toast(ToastAction::error(ARCHIVE_SENT_WARN));
        return;
    };
    let sent = match connector.find_sent_folder().await {
        Ok(Some(name)) => name,
        Ok(None) => {
            warn!("ArchiveSent: no Sent mailbox for {account_id}");
            ctx.show_toast(ToastAction::error(ARCHIVE_SENT_WARN));
            return;
        }
        Err(e) => {
            warn!("ArchiveSent: LIST failed for {account_id}: {e}");
            manager.note_imap_error_msg(&account_id, &e.to_string());
            ctx.show_toast(ToastAction::error(ARCHIVE_SENT_WARN));
            return;
        }
    };
    if let Err(e) = connector.append_rfc822_seen(&sent, &rfc822).await {
        warn!("ArchiveSent: APPEND failed for {account_id} → {sent}: {e}");
        manager.note_imap_error_msg(&account_id, &e.to_string());
        ctx.show_toast(ToastAction::error(ARCHIVE_SENT_WARN));
        return;
    }
    info!("ArchiveSent: appended to {sent} for {account_id}");
    let viewing_account = ctx.selected_account.read().as_ref() == Some(&account_id);
    let viewing_sent = ctx
        .selected_mailbox
        .read()
        .as_ref()
        .is_some_and(|id| id.to_string() == sent);
    if viewing_account && viewing_sent {
        handle_select_mailbox(manager, ctx, MailboxId::from(sent), false).await;
    }
}

async fn handle_save_imap_draft(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    draft_id: String,
    rfc822: Vec<u8>,
    replace: Option<MessageId>,
) {
    let Some(connector) = manager.get(&account_id) else {
        warn!("SaveImapDraft: no IMAP connector for {account_id}");
        ctx.show_toast(ToastAction::error(ARCHIVE_DRAFT_WARN));
        return;
    };
    let drafts = match connector.find_drafts_folder().await {
        Ok(Some(name)) => name,
        Ok(None) => {
            warn!("SaveImapDraft: no Drafts mailbox for {account_id}");
            ctx.show_toast(ToastAction::error(ARCHIVE_DRAFT_WARN));
            return;
        }
        Err(e) => {
            warn!("SaveImapDraft: LIST failed for {account_id}: {e}");
            manager.note_imap_error_msg(&account_id, &e.to_string());
            ctx.show_toast(ToastAction::error(ARCHIVE_DRAFT_WARN));
            return;
        }
    };
    let new_id = match connector.append_rfc822_draft(&drafts, &rfc822).await {
        Ok(id) => id,
        Err(e) => {
            warn!("SaveImapDraft: APPEND failed for {account_id} → {drafts}: {e}");
            manager.note_imap_error_msg(&account_id, &e.to_string());
            ctx.show_toast(ToastAction::error(ARCHIVE_DRAFT_WARN));
            return;
        }
    };
    if let Some(old) = replace.as_ref() {
        if new_id.as_ref() != Some(old) {
            if let Err(e) = connector
                .delete_messages(old.folder_id(), std::slice::from_ref(old))
                .await
            {
                warn!("SaveImapDraft: delete old draft failed for {account_id}: {e}");
            }
        }
    }
    if let Some(id) = new_id.clone() {
        crate::draft_store::set_imap_draft(&account_id, &draft_id, Some(id.clone()));
        if let Some(session) = ctx.compose_draft.write().as_mut() {
            if session.draft.id.as_str() == draft_id {
                session.imap_draft = Some(id);
            }
        }
    }
    info!("SaveImapDraft: appended to {drafts} for {account_id}");
    refresh_if_viewing_mailbox(manager, ctx, &account_id, &drafts).await;
}

async fn handle_delete_imap_draft(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    message_id: MessageId,
) {
    let Some(connector) = manager.get(&account_id) else {
        warn!("DeleteImapDraft: no IMAP connector for {account_id}");
        return;
    };
    let folder = message_id.folder_id().as_str().to_string();
    if let Err(e) = connector
        .delete_messages(message_id.folder_id(), std::slice::from_ref(&message_id))
        .await
    {
        warn!("DeleteImapDraft: failed for {account_id}: {e}");
        manager.note_imap_error_msg(&account_id, &e.to_string());
        ctx.show_toast(ToastAction::error(DELETE_DRAFT_WARN));
        return;
    }
    info!("DeleteImapDraft: removed {message_id} for {account_id}");
    refresh_if_viewing_mailbox(manager, ctx, &account_id, &folder).await;
}

async fn refresh_if_viewing_mailbox(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: &AccountId,
    mailbox: &str,
) {
    let viewing = ctx.selected_account.read().as_ref() == Some(account_id)
        && ctx
            .selected_mailbox
            .read()
            .as_ref()
            .is_some_and(|id| id.to_string() == mailbox);
    if viewing {
        handle_select_mailbox(manager, ctx, MailboxId::from(mailbox.to_string()), false).await;
    } else {
        invalidate_mailbox_messages(
            manager.cache(),
            account_id,
            &MailboxId::from(mailbox.to_string()),
        )
        .await;
    }
}

async fn handle_mark_answered(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
    message_id: MessageId,
) {
    let Some(connector) = manager.get(&account_id) else {
        warn!("MarkAnswered: no IMAP connector for {account_id}");
        return;
    };
    let folder_id = message_id.folder_id().clone();
    if let Err(e) = connector
        .update_envelope_flags(
            &folder_id,
            std::slice::from_ref(&message_id),
            &[(EnvelopeFlag::Answered, true)],
        )
        .await
    {
        warn!("MarkAnswered: STORE \\Answered failed for {message_id}: {e}");
        return;
    }
    info!("MarkAnswered: set \\Answered on {message_id}");
    let viewing_source = ctx.selected_account.read().as_ref() == Some(&account_id)
        && ctx
            .selected_mailbox
            .read()
            .as_ref()
            .is_some_and(|id| id.to_string() == folder_id.as_str());
    if viewing_source {
        apply_answered_flag(ctx, &message_id);
        persist_selected_messages(manager.cache(), ctx, &account_id).await;
    }
}

fn apply_answered_flag(ctx: &mut AppContext, id: &MessageId) {
    for msg in ctx.messages.write().iter_mut() {
        if msg.id == *id && !msg.is_answered {
            let mut next = (**msg).clone();
            next.is_answered = true;
            next.envelope.is_answered = true;
            *msg = Arc::new(next);
            break;
        }
    }
}

async fn handle_retry_outbox(
    manager: &mut AccountConnectionManager,
    ctx: &mut AppContext,
    outbox: &dyn OutboxStore,
    smtp_tx: &UnboundedSender<CoreEvent>,
    inflight: &mut SmtpInflight,
    id: crate::outbox_store::OutboxId,
) {
    let Ok(Some(mut item)) = outbox.get(&id).await else {
        return;
    };
    item.state = OutboxItemState::Queued;
    item.attempts = 0;
    item.last_error = None;
    item.last_error_kind = None;
    item.updated_at = chrono::Utc::now();
    let _ = outbox.upsert(&item).await;
    refresh_outbox_signal(outbox, ctx).await;
    drain_outbox(manager, ctx, outbox, smtp_tx, inflight).await;
}

enum RecvOutcome {
    Event {
        event: CoreEvent,
        follow_up: Option<CoreEvent>,
    },
    Continue,
    Closed,
}

fn watch_target(
    manager: &AccountConnectionManager,
    ctx: &AppContext,
) -> Option<(AccountId, MailboxId)> {
    let account_id = ctx.selected_account.read().clone()?;
    let mailbox_id = ctx.selected_mailbox.read().clone()?;
    let ready = ctx
        .connection_states
        .read()
        .get(&account_id)
        .is_some_and(|s| matches!(s, ConnectionState::Ready));
    if !ready || manager.get(&account_id).is_none() {
        return None;
    }
    let selectable = ctx
        .mailbox_nodes
        .read()
        .get(&mailbox_id)
        .is_some_and(|n| n.selectable);
    selectable.then_some((account_id, mailbox_id))
}

async fn recv_next_or_watch(
    core_rx: &mut UnboundedReceiver<CoreEvent>,
    smtp_rx: &mut SmtpUnboundedReceiver<CoreEvent>,
    watches: Vec<(AccountId, crate::websocket_stream::WsDeathWatch)>,
    manager: &AccountConnectionManager,
    ctx: &AppContext,
) -> RecvOutcome {
    let recv = recv_next_event(core_rx, smtp_rx, watches);
    let Some((account_id, mailbox_id)) = watch_target(manager, ctx) else {
        return match recv.await {
            Some(event) => RecvOutcome::Event {
                event,
                follow_up: None,
            },
            None => RecvOutcome::Closed,
        };
    };
    let Some(connector) = manager.get(&account_id) else {
        return match recv.await {
            Some(event) => RecvOutcome::Event {
                event,
                follow_up: None,
            },
            None => RecvOutcome::Closed,
        };
    };

    let folder_id = FolderId::new(mailbox_id.to_string());
    let (cancel_tx, cancel_rx) = futures_channel::oneshot::channel::<()>();
    let interval_ms = if connector.supports_idle() {
        IDLE_REISSUE_MS
    } else {
        NOOP_INTERVAL_MS
    };
    let tick = TimeoutFuture::new(interval_ms);
    let watch = connector.watch_mailbox(
        &folder_id,
        async {
            let _ = cancel_rx.await;
        },
        tick,
    );

    futures_util::pin_mut!(recv);
    futures_util::pin_mut!(watch);
    match select(recv, watch).await {
        Either::Left((ev, watch)) => {
            let _ = cancel_tx.send(());
            let follow_up = match watch.await {
                Ok(outcome) if outcome.needs_refresh() => Some(CoreEvent::MailboxActivity {
                    account_id,
                    mailbox_id,
                }),
                Ok(_) => None,
                Err(e) => {
                    warn!("mailbox watch failed for {account_id}: {e}");
                    manager.note_imap_error(&account_id, &e);
                    Some(CoreEvent::SessionDropped {
                        account_id: account_id.clone(),
                    })
                }
            };
            match ev {
                Some(event) => RecvOutcome::Event { event, follow_up },
                None => match follow_up {
                    Some(event) => RecvOutcome::Event {
                        event,
                        follow_up: None,
                    },
                    None => RecvOutcome::Closed,
                },
            }
        }
        Either::Right((Ok(outcome), _)) => {
            if outcome.needs_refresh() {
                RecvOutcome::Event {
                    event: CoreEvent::MailboxActivity {
                        account_id,
                        mailbox_id,
                    },
                    follow_up: None,
                }
            } else {
                RecvOutcome::Continue
            }
        }
        Either::Right((Err(e), _)) => {
            warn!("mailbox watch failed for {account_id}: {e}");
            manager.note_imap_error(&account_id, &e);
            RecvOutcome::Event {
                event: CoreEvent::SessionDropped { account_id },
                follow_up: None,
            }
        }
    }
}

async fn recv_next_event(
    core_rx: &mut UnboundedReceiver<CoreEvent>,
    smtp_rx: &mut SmtpUnboundedReceiver<CoreEvent>,
    watches: Vec<(AccountId, crate::websocket_stream::WsDeathWatch)>,
) -> Option<CoreEvent> {
    if watches.is_empty() {
        return recv_ui_or_smtp(core_rx, smtp_rx).await;
    }

    let ids: Vec<AccountId> = watches.iter().map(|(id, _)| id.clone()).collect();
    let futs: Vec<_> = watches.into_iter().map(|(_, w)| w).collect();
    let deaths = select_all(futs);
    let ui_or_smtp = recv_ui_or_smtp(core_rx, smtp_rx);
    futures_util::pin_mut!(deaths);
    futures_util::pin_mut!(ui_or_smtp);
    match select(ui_or_smtp, deaths).await {
        Either::Left((ev, _)) => ev,
        Either::Right(((_, idx, _), _)) => Some(CoreEvent::SessionDropped {
            account_id: ids[idx].clone(),
        }),
    }
}

async fn recv_ui_or_smtp(
    core_rx: &mut UnboundedReceiver<CoreEvent>,
    smtp_rx: &mut SmtpUnboundedReceiver<CoreEvent>,
) -> Option<CoreEvent> {
    let ui = core_rx.next();
    let smtp = smtp_rx.next();
    futures_util::pin_mut!(ui);
    futures_util::pin_mut!(smtp);
    match select(ui, smtp).await {
        Either::Left((Some(ev), _)) | Either::Right((Some(ev), _)) => Some(ev),
        Either::Left((None, _)) => None,
        Either::Right((None, _)) => core_rx.next().await,
    }
}
