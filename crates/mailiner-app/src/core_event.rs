use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use dioxus::logger::tracing::{error, info, warn};
use dioxus::prelude::*;
use futures_channel::mpsc::{UnboundedReceiver as SmtpUnboundedReceiver, UnboundedSender};
use futures_util::StreamExt;
use futures_util::future::{Either, select};
use mailiner_core::connector::EmailConnector;
use mailiner_core::models::TransferEncoding;
use mailiner_core::submit::{SendErrorKind, SubmitRequest};
use mailiner_core::{EnvelopeFlag, FolderId, MailboxRole, MessageSort};

use crate::account::AccountId;
use crate::account_config::AccountConfig;
use crate::account_store::AccountStore;
use crate::components::virtual_scroll::{SparseList, adjacent_index, index_after_removal};
use crate::connection::{
    AccountConnectionManager, ConnectErrorKind, ConnectionState, EnsureConnectedMode,
    set_connection_state,
};
use crate::context::{AppContext, MessageViewState};
use crate::download::{
    DownloadStatus, EML_DOWNLOAD_KEY, MAX_DOWNLOAD_BYTES, StreamingBlobDownload,
};
use crate::mail_cache::{
    CachedFolderTree, CachedMessageList, HydratedAccount, MailCache, contiguous_envelope_prefix,
    hydrate_account,
};
use crate::mailbox::MailboxId;
use crate::message::MessageId;
use crate::message_loader::load_message;
use crate::outbox_store::{
    MAX_OUTBOX_AUTO_ATTEMPTS, OutboxItem, OutboxItemState, OutboxListEntry, OutboxStore,
};
use crate::send::{OutboxDisplay, SendPhase, SendState};
use crate::smtp_session::{
    InFlightSmtp, SEND_TIMEOUT_MS, SmtpOutcome, preflight, spawn_submit, spawn_test,
};
use crate::toast::{DismissCommit, MoveUndo, RemovedMessage, ToastAction, UndoRequest};

pub enum CoreEvent {
    // —— mail ops ——
    SelectMailbox(MailboxId),
    /// Open a mailbox and select the newest message (keyboard jump).
    JumpToMailbox(MailboxId),
    /// Rebuild the current folder's list in a new order.
    SetMessageSort(MessageSort),
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
    MarkRead {
        mailbox_id: MailboxId,
        message_ids: Vec<MessageId>,
        is_read: bool,
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
    /// Move to the Trash special-use folder, or permanently delete when already there.
    MoveToTrash {
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
    /// Inverse of a toasted action (central undo).
    Undo(UndoRequest),
    /// Work held until a toast dismissed without Undo (permanent delete).
    CommitDismissed(DismissCommit),
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

    DisconnectAccount(AccountId),

    /// UI mutated store (edit/delete). Manager drops deleted connectors; does not auto-connect.
    AccountsChanged,

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
    let mut smtp_generation: u64 = 0;
    let mut inflight: Option<InFlightSmtp> = None;

    if let InitialBootstrap::Run { active } = initial_bootstrap {
        handle_bootstrap(&mut manager, &mut ctx, active).await;
        recover_outbox(outbox.as_ref(), &mut ctx).await;
        drain_outbox(
            &mut manager,
            &mut ctx,
            outbox.as_ref(),
            &smtp_tx,
            &mut inflight,
            &mut smtp_generation,
        )
        .await;
    }

    loop {
        let event = {
            let ui = core_rx.next();
            let smtp = smtp_rx.next();
            futures_util::pin_mut!(ui);
            futures_util::pin_mut!(smtp);
            match select(ui, smtp).await {
                Either::Left((Some(ev), _)) | Either::Right((Some(ev), _)) => ev,
                Either::Left((None, _)) => break,
                Either::Right((None, _)) => {
                    // smtp_tx dropped — keep draining UI events
                    match core_rx.next().await {
                        Some(ev) => ev,
                        None => break,
                    }
                }
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
                handle_reconnect(&mut manager, &mut ctx, account_id).await;
            }
            CoreEvent::DisconnectAccount(account_id) => {
                cancel_inflight_for(
                    &mut inflight,
                    &mut smtp_generation,
                    &account_id,
                    outbox.as_ref(),
                )
                .await;
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
                purge_missing_accounts(
                    &manager,
                    outbox.as_ref(),
                    &mut ctx,
                    &mut inflight,
                    &mut smtp_generation,
                )
                .await;
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
            CoreEvent::FetchMessageRange { mailbox_id, range } => {
                handle_fetch_message_range(&manager, &mut ctx, mailbox_id, range).await;
            }
            CoreEvent::SelectMessage(message_id) => {
                handle_select_message(&manager, &mut ctx, message_id, true, true).await;
            }
            CoreEvent::SelectListClick {
                message_id,
                index,
                extend,
                toggle,
            } => {
                handle_select_list_click(&manager, &mut ctx, message_id, index, extend, toggle)
                    .await;
            }
            CoreEvent::SelectAdjacent { delta, extend } => {
                handle_select_adjacent(&manager, &mut ctx, delta, extend).await;
            }
            CoreEvent::MarkRead {
                mailbox_id,
                message_ids,
                is_read,
            } => {
                handle_mark_read(&manager, &mut ctx, mailbox_id, message_ids, is_read).await;
            }
            CoreEvent::MoveMessages {
                mailbox_id,
                message_ids,
                dest_mailbox_id,
            } => {
                handle_move_messages(&manager, &mut ctx, mailbox_id, message_ids, dest_mailbox_id)
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
            CoreEvent::MoveToTrash {
                mailbox_id,
                message_ids,
            } => {
                handle_move_to_trash(&manager, &mut ctx, mailbox_id, message_ids).await;
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
            CoreEvent::Undo(undo) => {
                handle_undo(&manager, &mut ctx, undo).await;
            }
            CoreEvent::CommitDismissed(commit) => {
                handle_commit_dismissed(&manager, &mut ctx, commit).await;
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
            CoreEvent::SendMessage {
                account_id,
                request,
                display,
                draft_id,
                bcc_header,
                reply_source,
            } => {
                handle_send_message(
                    &mut manager,
                    &mut ctx,
                    outbox.as_ref(),
                    &smtp_tx,
                    &mut inflight,
                    &mut smtp_generation,
                    account_id,
                    request,
                    display,
                    draft_id,
                    bcc_header,
                    reply_source,
                )
                .await;
            }
            CoreEvent::TestSmtpConnection { request_id, config } => {
                handle_test_smtp(
                    &mut ctx,
                    &smtp_tx,
                    &mut inflight,
                    &mut smtp_generation,
                    request_id,
                    config,
                )
                .await;
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
                    &mut smtp_generation,
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
                    &mut smtp_generation,
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
                    &mut smtp_generation,
                    id,
                )
                .await;
            }
            CoreEvent::DeleteOutboxItem { id } => {
                if let Some(flight) = inflight.as_mut() {
                    if flight.outbox_id.as_ref() == Some(&id) {
                        if let Some(tx) = flight.cancel_tx.take() {
                            let _ = tx.send(());
                        }
                        smtp_generation = smtp_generation.wrapping_add(1);
                    }
                }
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
        }
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
    account_id: AccountId,
) {
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

    match manager
        .ensure_connected(&config, ctx, EnsureConnectedMode::Switch)
        .await
    {
        Ok(()) => {
            if ctx.selected_account.read().as_ref() == Some(&account_id) {
                list_folders_soft(manager, ctx, &account_id).await;
            }
        }
        Err(e) => {
            error!(
                "Reconnect failed for {}: {} ({:?})",
                account_id, e.message, e.kind
            );
        }
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
    // store writes. disconnect_others only after full commit success when we activate.
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

                manager.disconnect_others(Some(&account_id), ctx).await;
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
    crate::ui_prefs::retain_last_mailboxes(&known);
    crate::ui_prefs::retain_ack_unread(&known);
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
                .filter(|f| f.selectable)
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
                crate::mailbox::resolve_startup_mailbox(saved.as_ref(), &nodes, &roots)
            };
            if let Some(startup_id) = startup.as_ref() {
                let one = [FolderId::new(startup_id.to_string())];
                if let Ok(counts) = connector.folder_counts(&one).await {
                    let ack = crate::ui_prefs::load_ack_unread(account_id);
                    let mut nodes = ctx.mailbox_nodes.write();
                    crate::mailbox::apply_folder_counts(&mut nodes, &counts);
                    crate::mailbox::apply_unread_new_state(&mut nodes, &counts, &ack);
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
                        let mut nodes = ctx.mailbox_nodes.write();
                        crate::mailbox::apply_folder_counts(&mut nodes, &counts);
                        crate::mailbox::apply_unread_new_state(&mut nodes, &counts, &ack);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        warn!("folder_counts {} failed: {}", id, e);
                    }
                }
            }
            persist_folder_tree(manager.cache(), ctx, account_id).await;
        }
        Err(e) => {
            error!("Failed to list folders for {}: {}", account_id, e);
            // Keep a cached tree if we already painted one.
            if ctx.mailbox_roots.read().is_empty() {
                ctx.mailbox_nodes.set(HashMap::new());
                ctx.mailbox_roots.set(Vec::new());
            }
        }
    }
}

/// Open the last folder for this account, or Inbox / first root when none is saved.
async fn restore_mailbox(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: &AccountId,
) {
    let to_open = {
        let nodes = ctx.mailbox_nodes.read();
        let roots = ctx.mailbox_roots.read();
        let saved = crate::ui_prefs::load_last_mailbox(account_id);
        crate::mailbox::resolve_startup_mailbox(saved.as_ref(), &nodes, &roots)
    };
    let Some(mailbox_id) = to_open else {
        return;
    };
    handle_select_mailbox(manager, ctx, mailbox_id, true).await;
}

fn clear_mailbox_ui(ctx: &mut AppContext) {
    ctx.selected_mailbox.set(None);
    ctx.messages.set(SparseList::new(0));
    ctx.messages_loading.set(false);
    ctx.selection.write().clear();
    ctx.message_view.set(MessageViewState::Empty);
    ctx.download_status.set(HashMap::new());
    ctx.mailbox_nodes.set(HashMap::new());
    ctx.mailbox_roots.set(Vec::new());
}

/// Paint a [`HydratedAccount`] onto UI signals (cache hit, no IMAP).
pub(crate) fn apply_hydrated(ctx: &mut AppContext, hydrated: HydratedAccount) {
    ctx.mailbox_nodes.set(hydrated.nodes);
    ctx.mailbox_roots.set(hydrated.roots);
    if let Some(mailbox_id) = hydrated.selected_mailbox {
        ctx.selected_mailbox.set(Some(mailbox_id));
    }
    match hydrated.messages {
        Some(msgs) => {
            let mut list = SparseList::new(msgs.total);
            list.insert_batch(0, msgs.prefix);
            ctx.messages.set(list);
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

async fn handle_select_mailbox(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    mailbox_id: MailboxId,
    select_first: bool,
) {
    if ctx
        .mailbox_nodes
        .read()
        .get(&mailbox_id)
        .is_some_and(|n| !n.selectable)
    {
        return;
    }

    let already_showing = ctx.selected_mailbox.read().as_ref() == Some(&mailbox_id)
        && ctx.messages.read().cached_count() > 0;
    if !already_showing {
        ctx.selection.write().clear();
        ctx.message_view.set(MessageViewState::Empty);
        ctx.download_status.set(HashMap::new());
        ctx.selected_mailbox.set(Some(mailbox_id.clone()));
        let sort = *ctx.message_sort.peek();
        let account = ctx.selected_account.read().clone();
        let hydrated = match account {
            Some(account_id) => manager
                .cache()
                .load_messages(&account_id, &mailbox_id, sort)
                .await
                .ok()
                .flatten(),
            None => None,
        };
        if let Some(cached) = hydrated {
            apply_cached_message_list(ctx, &cached);
        } else {
            ctx.messages.set(SparseList::new(0));
            ctx.messages_loading.set(true);
        }
    } else {
        ctx.selected_mailbox.set(Some(mailbox_id.clone()));
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
    match connector.prepare_folder_list(&folder_id, requested).await {
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
            acknowledge_mailbox_open(ctx, &account_id, &mailbox_id, state.total, state.unread);

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
                    let batch: Vec<_> = envelopes.into_iter().map(|e| Arc::new(e.into())).collect();
                    list.insert_batch(0, batch);
                    ctx.messages.set(list);
                    ctx.messages_loading.set(false);
                    persist_selected_messages(manager.cache(), ctx, &account_id).await;
                    persist_folder_tree(manager.cache(), ctx, &account_id).await;
                }
                Err(e) => {
                    error!(
                        "Failed to fetch first page of {}: {}",
                        mailbox_id.as_str(),
                        e
                    );
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
                }
            }

            if select_first && state.total > 0 {
                let first_id = ctx.messages.read().get(0).map(|m| m.id.clone());
                if let Some(id) = first_id {
                    // Unread-first: selecting the top row would immediately
                    // consume the message the user just asked to see as unread.
                    let auto_mark = state.sort != MessageSort::Unread;
                    handle_select_message(manager, ctx, id, auto_mark, true).await;
                }
            }
        }
        Err(e) => {
            error!("Failed to open mailbox: {}", e);
            ctx.messages_loading.set(false);
        }
    }
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

async fn handle_fetch_message_range(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    mailbox_id: MailboxId,
    range: Range<usize>,
) {
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
            let batch: Vec<_> = envelopes.into_iter().map(|e| Arc::new(e.into())).collect();
            ctx.messages.write().insert_batch(range.start, batch);
            // Only rewrite localStorage when the contiguous cached prefix grew.
            if contiguous_loaded_prefix_len(&ctx.messages.read()) > prefix_before {
                persist_selected_messages(manager.cache(), ctx, &account_id).await;
            }
        }
        Err(e) => {
            error!(
                "Failed to fetch message range {}..{}: {}",
                range.start, range.end, e
            );
        }
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
    let total = ctx.messages.read().total_count();
    let current = ctx.selection.read().focus_at_index().or_else(|| {
        ctx.selection
            .read()
            .focus()
            .cloned()
            .and_then(|id| ctx.messages.read().position(|m| m.id == id))
    });
    let Some(index) = adjacent_index(total, current, delta) else {
        return;
    };
    if extend {
        apply_index_range_selection(manager, ctx, index).await;
    }
    select_list_index(manager, ctx, index, !extend).await;
}

async fn handle_select_list_click(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    message_id: MessageId,
    index: usize,
    extend: bool,
    toggle: bool,
) {
    if extend {
        apply_index_range_selection(manager, ctx, index).await;
        handle_select_message(manager, ctx, message_id, false, false).await;
        return;
    }
    if toggle {
        ctx.selection
            .write()
            .toggle(message_id.clone(), Some(index));
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
    }
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
) {
    let total = ctx.messages.read().total_count();
    if ctx.messages.read().get(index).is_none() {
        let Some(mailbox_id) = ctx.selected_mailbox.read().clone() else {
            return;
        };
        let start = index.saturating_sub(5);
        let end = (index + 15).min(total);
        handle_fetch_message_range(manager, ctx, mailbox_id, start..end).await;
    }
    let Some(message_id) = ctx.messages.read().get(index).map(|m| m.id.clone()) else {
        return;
    };
    handle_select_message(
        manager,
        ctx,
        message_id,
        replace_selection,
        replace_selection,
    )
    .await;
}

async fn select_after_removed_row(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    removed_index: Option<usize>,
) {
    let Some(removed_index) = removed_index else {
        return;
    };
    let total = ctx.messages.read().total_count();
    let Some(index) = index_after_removal(total, removed_index) else {
        return;
    };
    select_list_index(manager, ctx, index, true).await;
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
    ctx.download_status.set(HashMap::new());
    ctx.message_view.set(MessageViewState::Loading {
        message_id: message_id.clone(),
    });

    let Some(mailbox_id) = ctx.selected_mailbox.read().clone() else {
        ctx.message_view.set(MessageViewState::Error {
            message_id: message_id.clone(),
            message: "No mailbox selected".into(),
        });
        return;
    };

    let Some(account_id) = ctx.selected_account.read().clone() else {
        ctx.message_view.set(MessageViewState::Error {
            message_id: message_id.clone(),
            message: "No account selected".into(),
        });
        return;
    };
    let Some(connector) = manager.get(&account_id) else {
        ctx.message_view.set(MessageViewState::Error {
            message_id: message_id.clone(),
            message: "Not connected".into(),
        });
        return;
    };

    let folder_id = FolderId::new(mailbox_id.to_string());
    info!(
        "Loading message {} in {}",
        message_id,
        mailbox_id.to_string()
    );

    match load_message(connector, &folder_id, &message_id).await {
        Ok(loaded) => {
            if ctx.selection.read().focus() != Some(&message_id) {
                return;
            }
            ctx.message_view.set(MessageViewState::Ready {
                message_id: message_id.clone(),
                loaded: Arc::new(loaded),
            });
            let was_unread = ctx
                .messages
                .read()
                .find(|m| m.id == message_id)
                .is_some_and(|m| !m.is_read);
            let is_multi = ctx.selection.read().is_multi();
            if was_unread && crate::selection::should_auto_mark_read(auto_mark_read, is_multi) {
                apply_read_flag(ctx, std::slice::from_ref(&message_id), true);
                if let Err(e) = connector
                    .update_envelope_flags(
                        &folder_id,
                        std::slice::from_ref(&message_id),
                        &[(EnvelopeFlag::Read, true)],
                    )
                    .await
                {
                    warn!("Auto-mark as read failed for {}: {}", message_id, e);
                    apply_read_flag(ctx, std::slice::from_ref(&message_id), false);
                } else {
                    relocate_unread_sort_rows(
                        connector,
                        ctx,
                        std::slice::from_ref(&message_id),
                        true,
                    )
                    .await;
                    let account_id = ctx.selected_account.read().clone();
                    if let Some(account_id) = account_id {
                        persist_selected_messages(manager.cache(), ctx, &account_id).await;
                        persist_folder_tree(manager.cache(), ctx, &account_id).await;
                    }
                }
            }
        }
        Err(e) => {
            if ctx.selection.read().focus() != Some(&message_id) {
                return;
            }
            error!("Failed to load message {}: {}", message_id, e);
            ctx.message_view.set(MessageViewState::Error {
                message_id,
                message: e.to_string(),
            });
        }
    }
}

fn apply_read_flag(ctx: &mut AppContext, ids: &[MessageId], is_read: bool) {
    let idset: std::collections::HashSet<&MessageId> = ids.iter().collect();
    let mut unread_delta: i32 = 0;
    for msg in ctx.messages.write().iter_mut() {
        if idset.contains(&msg.id) && msg.is_read != is_read {
            let mut next = (**msg).clone();
            next.is_read = is_read;
            next.envelope.is_read = is_read;
            *msg = Arc::new(next);
            if is_read {
                unread_delta -= 1;
            } else {
                unread_delta += 1;
            }
        }
    }
    if unread_delta != 0 {
        let mailbox_id = ctx.selected_mailbox.read().clone();
        if let Some(mailbox_id) = mailbox_id {
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
    let n = taken.len();
    let unread_n = taken.iter().filter(|(_, m)| !m.is_read).count() as i32;
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
    if selected_removed_index.is_some() {
        ctx.selection.write().clear();
        ctx.message_view.set(MessageViewState::Empty);
        ctx.download_status.set(HashMap::new());
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
    if ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id) {
        return;
    }
    let Some(account_id) = ctx.selected_account.read().clone() else {
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
        apply_read_flag(ctx, &message_ids, !is_read);
        ctx.show_toast(ToastAction::error(format!(
            "Could not update read state: {e}"
        )));
        return;
    }
    relocate_unread_sort_rows(connector, ctx, &message_ids, is_read).await;
    persist_selected_messages(manager.cache(), ctx, &account_id).await;
    persist_folder_tree(manager.cache(), ctx, &account_id).await;
}

/// Slide rows in the unread-first index without SELECT/SEARCH or a list rebuild.
async fn relocate_unread_sort_rows(
    connector: &mailiner_imap_connector::ImapConnector<crate::websocket_stream::WebSocketStream>,
    ctx: &mut AppContext,
    message_ids: &[MessageId],
    now_read: bool,
) {
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
        }
        Err(e) => {
            warn!("unread-sort relocate failed: {e}");
        }
    }
}

async fn handle_move_messages(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    mailbox_id: MailboxId,
    message_ids: Vec<MessageId>,
    dest_mailbox_id: MailboxId,
) {
    if message_ids.is_empty() || mailbox_id == dest_mailbox_id {
        return;
    }
    if ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id) {
        return;
    }
    let Some(account_id) = ctx.selected_account.read().clone() else {
        ctx.show_toast(ToastAction::error("No account selected"));
        return;
    };
    let Some(connector) = manager.get(&account_id) else {
        ctx.show_toast(ToastAction::error("Not connected"));
        return;
    };

    let folder_id = FolderId::new(mailbox_id.to_string());
    let dest_id = FolderId::new(dest_mailbox_id.to_string());
    let core_ids = core_message_ids(&message_ids);
    match connector
        .move_messages(&folder_id, &core_ids, &dest_id)
        .await
    {
        Ok(dest_uids) => {
            let (snapshots, removed_sel) = take_messages_from_ui(ctx, &message_ids);
            let unread_n = unread_in_removed(&snapshots);
            if unread_n != 0 {
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
    if !selected_account_is(ctx, &account_id)
        || ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id)
    {
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

async fn handle_move_to_trash(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    mailbox_id: MailboxId,
    message_ids: Vec<MessageId>,
) {
    if message_ids.is_empty() {
        return;
    }
    if ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id) {
        return;
    }
    let Some(account_id) = ctx.selected_account.read().clone() else {
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
    if ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id) {
        return;
    }
    let Some(account_id) = ctx.selected_account.read().clone() else {
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
            ctx.download_status.set(HashMap::new());
            if let Some(node) = ctx.mailbox_nodes.write().get_mut(&mailbox_id) {
                node.total_count = 0;
                node.unread_count = 0;
                node.has_new = false;
            }
            crate::ui_prefs::save_ack_unread(&account_id, &mailbox_id, 0);
            persist_selected_messages(manager.cache(), ctx, &account_id).await;
            persist_folder_tree(manager.cache(), ctx, &account_id).await;
            ctx.show_toast(ToastAction::info("Trash emptied"));
        }
        Err(e) => {
            error!("Failed to empty trash: {}", e);
            ctx.show_toast(ToastAction::error(format!("Could not empty Trash: {e}")));
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
                    ctx.show_toast(ToastAction::error(format!("Could not undo: {e}")));
                }
            }
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
                ctx.show_toast(ToastAction::error(format!("Could not delete: {e}")));
            }
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
    // Ignore if user navigated away or switched accounts (queued Save all).
    if ctx.selection.read().focus() != Some(&message_id) || !selected_account_is(ctx, &account_id) {
        return;
    }
    if size_hint.is_some_and(|s| s as usize > MAX_DOWNLOAD_BYTES) {
        ctx.download_status.write().insert(
            section.clone(),
            DownloadStatus::Error(format!(
                "attachment too large (max {} bytes)",
                MAX_DOWNLOAD_BYTES
            )),
        );
        return;
    }

    let Some(connector) = manager.get(&account_id) else {
        ctx.download_status
            .write()
            .insert(section, DownloadStatus::Error("Not connected".into()));
        return;
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
            ctx.download_status
                .write()
                .insert(section, DownloadStatus::Error(e.to_string()));
            return;
        }
    };

    // Stream wire → TE decode → Blob parts (no full-file Vec in Rust).
    let mut download = StreamingBlobDownload::new(encoding, filename, content_type);
    let mut total_hint = size_hint;
    let mut failed = false;

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
                    failed = true;
                    break;
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
                ctx.download_status
                    .write()
                    .insert(section.clone(), DownloadStatus::Error(e.to_string()));
                failed = true;
                break;
            }
        }
    }

    if failed {
        return;
    }
    if ctx.selection.read().focus() != Some(&message_id) {
        return;
    }

    match download.finish_and_save() {
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

async fn cancel_inflight_for(
    inflight: &mut Option<InFlightSmtp>,
    smtp_generation: &mut u64,
    account_id: &AccountId,
    _outbox: &dyn OutboxStore,
) {
    let Some(flight) = inflight.as_mut() else {
        return;
    };
    if flight.account_id != *account_id {
        return;
    }
    // Signal the task but keep the slot. SmtpFinished settles the row
    // (delete on success, requeue on Cancelled) so DrainOutbox cannot
    // start a second DATA for the same rfc822.
    if let Some(tx) = flight.cancel_tx.take() {
        let _ = tx.send(());
    }
    *smtp_generation = smtp_generation.wrapping_add(1);
}

async fn purge_missing_accounts(
    manager: &AccountConnectionManager,
    outbox: &dyn OutboxStore,
    ctx: &mut AppContext,
    inflight: &mut Option<InFlightSmtp>,
    smtp_generation: &mut u64,
) {
    let known: Vec<AccountId> = match manager.store().list().await {
        Ok(list) => list.into_iter().map(|c| c.id).collect(),
        Err(_) => return,
    };
    if let Ok(items) = outbox.list().await {
        for item in items {
            if !known.iter().any(|id| *id == item.account_id) {
                if inflight
                    .as_ref()
                    .map(|f| f.account_id == item.account_id)
                    .unwrap_or(false)
                {
                    cancel_inflight_for(inflight, smtp_generation, &item.account_id, outbox).await;
                }
                let _ = outbox.delete_for_account(&item.account_id).await;
            }
        }
    }
    refresh_outbox_signal(outbox, ctx).await;
}

fn bump_smtp_gen(smtp_generation: &mut u64) -> u64 {
    *smtp_generation = smtp_generation.wrapping_add(1);
    *smtp_generation
}

async fn handle_send_message(
    manager: &mut AccountConnectionManager,
    ctx: &mut AppContext,
    outbox: &dyn OutboxStore,
    smtp_tx: &UnboundedSender<CoreEvent>,
    inflight: &mut Option<InFlightSmtp>,
    smtp_generation: &mut u64,
    account_id: AccountId,
    request: SubmitRequest,
    display: OutboxDisplay,
    draft_id: String,
    bcc_header: Option<String>,
    reply_source: Option<MessageId>,
) {
    let Some(config) = manager.resolve_config(&account_id).await else {
        ctx.send_status.set(Some(SendState::Failed {
            account_id,
            kind: SendErrorKind::NotConfigured,
            message: "Account not found.".into(),
            retryable: false,
        }));
        return;
    };
    if let Err(err) = preflight(&config) {
        ctx.send_status.set(Some(SendState::Failed {
            account_id,
            kind: err.kind,
            message: err.message,
            retryable: false,
        }));
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
            ctx.send_status.set(Some(SendState::Failed {
                account_id,
                kind: SendErrorKind::MessageTooLarge,
                message: e.to_string(),
                retryable: false,
            }));
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
        ctx.send_status.set(Some(SendState::Failed {
            account_id,
            kind: SendErrorKind::Internal,
            message: e.to_string(),
            retryable: false,
        }));
        return;
    }
    refresh_outbox_signal(outbox, ctx).await;
    if ctx
        .compose_draft
        .read()
        .as_ref()
        .is_some_and(|s| s.draft.id.as_str() == draft_id)
    {
        ctx.compose_draft.set(None);
    }
    if inflight.is_some() {
        ctx.send_status.set(Some(SendState::Idle));
        return;
    }
    item.attempts = 1;
    persist_sending(outbox, &mut item).await;
    refresh_outbox_signal(outbox, ctx).await;
    start_send_item(
        ctx,
        smtp_tx,
        inflight,
        smtp_generation,
        config,
        item,
        request,
    );
}

fn start_send_item(
    ctx: &mut AppContext,
    smtp_tx: &UnboundedSender<CoreEvent>,
    inflight: &mut Option<InFlightSmtp>,
    smtp_generation: &mut u64,
    config: AccountConfig,
    item: OutboxItem,
    request: SubmitRequest,
) {
    let generation = bump_smtp_gen(smtp_generation);
    let (cancel_tx, cancel_rx) = futures_channel::oneshot::channel();
    let account_id = config.id.clone();
    *inflight = Some(InFlightSmtp {
        account_id: account_id.clone(),
        generation,
        cancel_tx: Some(cancel_tx),
        outbox_id: Some(item.id.clone()),
        is_test: false,
        reply_source: item.reply_source.clone(),
    });
    ctx.send_status.set(Some(SendState::Sending {
        account_id,
        phase: SendPhase::Connecting,
    }));
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
    inflight: &mut Option<InFlightSmtp>,
    smtp_generation: &mut u64,
) {
    if inflight.is_some() {
        return;
    }
    let Some(mut item) = (match outbox.oldest_queued().await {
        Ok(v) => v,
        Err(e) => {
            warn!("oldest_queued failed: {e}");
            return;
        }
    }) else {
        return;
    };
    let Some(config) = manager.resolve_config(&item.account_id).await else {
        item.state = OutboxItemState::Failed;
        item.last_error = Some("Account is no longer available.".into());
        let _ = outbox.upsert(&item).await;
        refresh_outbox_signal(outbox, ctx).await;
        return;
    };
    if let Err(err) = preflight(&config) {
        item.state = OutboxItemState::Failed;
        item.last_error_kind = Some(err.kind);
        item.last_error = Some(err.message);
        let _ = outbox.upsert(&item).await;
        refresh_outbox_signal(outbox, ctx).await;
        return;
    }
    let request = match item.to_request() {
        Ok(r) => r,
        Err(e) => {
            item.state = OutboxItemState::Failed;
            item.last_error = Some(e.to_string());
            let _ = outbox.upsert(&item).await;
            refresh_outbox_signal(outbox, ctx).await;
            return;
        }
    };
    item.attempts = item.attempts.saturating_add(1);
    persist_sending(outbox, &mut item).await;
    refresh_outbox_signal(outbox, ctx).await;
    start_send_item(
        ctx,
        smtp_tx,
        inflight,
        smtp_generation,
        config,
        item,
        request,
    );
}

async fn handle_test_smtp(
    ctx: &mut AppContext,
    smtp_tx: &UnboundedSender<CoreEvent>,
    inflight: &mut Option<InFlightSmtp>,
    smtp_generation: &mut u64,
    request_id: AccountId,
    config: AccountConfig,
) {
    if inflight.is_some() {
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
    let generation = bump_smtp_gen(smtp_generation);
    let (cancel_tx, cancel_rx) = futures_channel::oneshot::channel();
    *inflight = Some(InFlightSmtp {
        account_id: config.id.clone(),
        generation,
        cancel_tx: Some(cancel_tx),
        outbox_id: None,
        is_test: true,
        reply_source: None,
    });
    if !ctx.set_smtp_test_status(
        request_id.clone(),
        SendState::Sending {
            account_id: config.id.clone(),
            phase: SendPhase::Connecting,
        },
    ) {
        *inflight = None;
        return;
    }
    spawn_test(config, request_id, generation, cancel_rx, smtp_tx.clone());
}

async fn handle_smtp_finished(
    manager: &mut AccountConnectionManager,
    ctx: &mut AppContext,
    outbox: &dyn OutboxStore,
    smtp_tx: &UnboundedSender<CoreEvent>,
    inflight: &mut Option<InFlightSmtp>,
    smtp_generation: &mut u64,
    generation: u64,
    outcome: SmtpOutcome,
) {
    let Some(flight) = inflight.take() else {
        // Inflight was dropped (item deleted) but DATA may still have succeeded.
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
    if flight.generation != generation {
        *inflight = Some(flight);
        return;
    }
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
            ctx.send_status.set(Some(SendState::Sent {
                account_id: flight.account_id.clone(),
            }));
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
            ctx.send_status.set(Some(SendState::Failed {
                account_id: flight.account_id,
                kind: err.kind,
                message: err.message,
                retryable: err.kind.is_retryable(),
            }));
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
    drain_outbox(manager, ctx, outbox, smtp_tx, inflight, smtp_generation).await;
}

const ARCHIVE_SENT_WARN: &str = "Could not save a copy in Sent.";

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
            ctx.show_toast(ToastAction::error(ARCHIVE_SENT_WARN));
            return;
        }
    };
    if let Err(e) = connector.append_rfc822_seen(&sent, &rfc822).await {
        warn!("ArchiveSent: APPEND failed for {account_id} → {sent}: {e}");
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
    inflight: &mut Option<InFlightSmtp>,
    smtp_generation: &mut u64,
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
    drain_outbox(manager, ctx, outbox, smtp_tx, inflight, smtp_generation).await;
}
