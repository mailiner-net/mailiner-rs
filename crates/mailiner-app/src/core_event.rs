use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use dioxus::logger::tracing::{error, info, warn};
use dioxus::prelude::*;
use futures_util::StreamExt;
use mailiner_core::connector::EmailConnector;
use mailiner_core::models::TransferEncoding;
use mailiner_core::{Folder, FolderId, MessageId as CoreMessageId};

use crate::account::AccountId;
use crate::account_config::AccountConfig;
use crate::account_store::AccountStore;
use crate::components::virtual_scroll::SparseList;
use crate::connection::{
    AccountConnectionManager, ConnectErrorKind, ConnectionState, EnsureConnectedMode,
    set_connection_state,
};
use crate::context::{AppContext, MessageViewState};
use crate::download::{DownloadStatus, MAX_DOWNLOAD_BYTES, StreamingBlobDownload};
use crate::mailbox::{MailboxId, MailboxNode};
use crate::message::MessageId;
use crate::message_loader::load_message;

pub enum CoreEvent {
    // —— mail ops ——
    SelectMailbox(MailboxId),
    /// Load envelopes for UI indices `[range.start, range.end)` into the sparse cache.
    FetchMessageRange {
        mailbox_id: MailboxId,
        range: Range<usize>,
    },
    SelectMessage(MessageId),
    /// Stream a single attachment part and save to disk (browser download).
    DownloadAttachment {
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

    DisconnectAccount(AccountId),

    /// UI mutated store (edit/delete). Manager drops deleted connectors; does not auto-connect.
    AccountsChanged,
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
    mut ctx: AppContext,
    store: Rc<dyn AccountStore>,
    initial_bootstrap: InitialBootstrap,
) {
    let mut manager = AccountConnectionManager::new(store);

    if let InitialBootstrap::Run { active } = initial_bootstrap {
        handle_bootstrap(&mut manager, &mut ctx, active).await;
    }

    while let Some(event) = core_rx.next().await {
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
            }
            CoreEvent::SelectMailbox(mailbox_id) => {
                handle_select_mailbox(&manager, &mut ctx, mailbox_id).await;
            }
            CoreEvent::FetchMessageRange { mailbox_id, range } => {
                handle_fetch_message_range(&manager, &mut ctx, mailbox_id, range).await;
            }
            CoreEvent::SelectMessage(message_id) => {
                handle_select_message(&manager, &mut ctx, message_id).await;
            }
            CoreEvent::DownloadAttachment {
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
            clear_mailbox_ui(ctx);
        }
    }
}

async fn handle_select_account(
    manager: &mut AccountConnectionManager,
    ctx: &mut AppContext,
    account_id: AccountId,
) {
    ctx.selected_account.set(Some(account_id.clone()));
    clear_mailbox_ui(ctx);

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
    manager.disconnect_account(&account_id, ctx).await;

    // disconnect_account clears cached config — re-resolve from store / cache.
    let Some(config) = manager.resolve_config(&account_id).await else {
        error!("Reconnect: unknown account {}", account_id);
        return;
    };
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

    // Force a fresh connect so credential edits re-verify (ensure_connected would
    // otherwise short-circuit when a Ready connector already exists for this id).
    // Drop connector only; store entry is left intact until Ready + upsert.
    if manager.get(&account_id).is_some() {
        manager.disconnect_account(&account_id, ctx).await;
    }

    // KeepActiveUntilReady: prior *other* active session stays up through connect **and**
    // store writes. disconnect_others only after full commit success (below).
    match manager
        .ensure_connected(&config, ctx, EnsureConnectedMode::KeepActiveUntilReady)
        .await
    {
        Ok(()) => {
            // Connect-before-persist: only write store on Ready.
            if let Err(e) = manager.store().upsert(&config).await {
                error!("CommitNewAccount: store upsert failed: {}", e);
                // Drop only the new trial session; prior active session is still live.
                manager.disconnect_account(&account_id, ctx).await;
                set_connection_state(
                    ctx,
                    &account_id,
                    ConnectionState::Error {
                        message: format!("Connected, but failed to save account: {e}"),
                        kind: ConnectErrorKind::Internal,
                        retryable: true,
                    },
                );
                return;
            }
            if let Err(e) = manager.store().set_active_id(Some(&account_id)).await {
                error!("CommitNewAccount: set_active_id failed: {}", e);
                // Drop only the new trial session; prior remains. Account stays in store
                // for retry via SelectAccount / set_active without re-entering credentials.
                manager.disconnect_account(&account_id, ctx).await;
                set_connection_state(
                    ctx,
                    &account_id,
                    ConnectionState::Error {
                        message: format!(
                            "Connected and saved, but failed to set active account: {e}. \
                             The account may already be saved — reload the page or try again."
                        ),
                        kind: ConnectErrorKind::Internal,
                        retryable: true,
                    },
                );
                refresh_ui_accounts(manager, ctx).await;
                return;
            }

            // Full commit success: switch active-only (tear down prior sessions).
            manager.disconnect_others(Some(&account_id), ctx).await;
            manager.cache_config(config.clone());
            refresh_ui_accounts(manager, ctx).await;
            ctx.selected_account.set(Some(account_id.clone()));
            list_folders_soft(manager, ctx, &account_id).await;
        }
        Err(e) => {
            // No store write on failure; prior active session left intact.
            error!(
                "CommitNewAccount connect failed: {} ({:?})",
                e.message, e.kind
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
            let (root_ids, mboxes) = build_mailbox_tree(mboxes);
            ctx.mailbox_nodes.set(mboxes);
            ctx.mailbox_roots.set(root_ids);
        }
        Err(e) => {
            error!("Failed to list folders for {}: {}", account_id, e);
            ctx.mailbox_nodes.set(HashMap::new());
            ctx.mailbox_roots.set(Vec::new());
            // Soft-fail: keep Ready connection but surface list failure on state if desired.
            // Leave connection Ready; empty tree is the UI signal.
        }
    }
}

fn clear_mailbox_ui(ctx: &mut AppContext) {
    ctx.selected_mailbox.set(None);
    ctx.messages.set(SparseList::new(0));
    ctx.messages_loading.set(false);
    ctx.selected_message.set(None);
    ctx.message_view.set(MessageViewState::Empty);
    ctx.download_status.set(HashMap::new());
    ctx.mailbox_nodes.set(HashMap::new());
    ctx.mailbox_roots.set(Vec::new());
}

async fn handle_select_mailbox(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    mailbox_id: MailboxId,
) {
    ctx.selected_message.set(None);
    ctx.message_view.set(MessageViewState::Empty);
    ctx.download_status.set(HashMap::new());
    ctx.messages.set(SparseList::new(0));
    ctx.messages_loading.set(true);
    ctx.selected_mailbox.set(Some(mailbox_id.clone()));

    let Some(account_id) = ctx.selected_account.read().clone() else {
        error!("SelectMailbox: no account selected");
        ctx.messages_loading.set(false);
        return;
    };
    let Some(connector) = manager.get(&account_id) else {
        error!("SelectMailbox: no connector for {}", account_id);
        ctx.messages_loading.set(false);
        return;
    };

    let folder_id = FolderId::new(mailbox_id.to_string());
    match connector.open_folder(&folder_id).await {
        Ok(total) => {
            info!(
                "Opened mailbox {} with {} messages",
                mailbox_id.to_string(),
                total
            );
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

async fn handle_fetch_message_range(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    mailbox_id: MailboxId,
    range: Range<usize>,
) {
    if ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id) {
        return;
    }
    if range.start >= range.end {
        return;
    }

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
            if ctx.selected_mailbox.read().as_ref() != Some(&mailbox_id) {
                return;
            }
            let batch: Vec<_> = envelopes.into_iter().map(|e| Arc::new(e.into())).collect();
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

async fn handle_select_message(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    message_id: MessageId,
) {
    ctx.selected_message.set(Some(message_id.clone()));
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
    let core_id = CoreMessageId::new(message_id.to_string());
    info!(
        "Loading message {} in {}",
        message_id,
        mailbox_id.to_string()
    );

    match load_message(connector, &folder_id, &core_id).await {
        Ok(loaded) => {
            if ctx.selected_message.read().as_ref() != Some(&message_id) {
                return;
            }
            ctx.message_view.set(MessageViewState::Ready {
                message_id,
                loaded: Arc::new(loaded),
            });
        }
        Err(e) => {
            if ctx.selected_message.read().as_ref() != Some(&message_id) {
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

#[allow(clippy::too_many_arguments)]
async fn handle_download_attachment(
    manager: &AccountConnectionManager,
    ctx: &mut AppContext,
    mailbox_id: MailboxId,
    message_id: MessageId,
    section: String,
    filename: String,
    content_type: String,
    encoding: TransferEncoding,
    size_hint: Option<u64>,
) {
    // Ignore if user navigated away.
    if ctx.selected_message.read().as_ref() != Some(&message_id) {
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

    let Some(account_id) = ctx.selected_account.read().clone() else {
        ctx.download_status
            .write()
            .insert(section, DownloadStatus::Error("No account selected".into()));
        return;
    };
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
    let core_id = CoreMessageId::new(message_id.to_string());
    info!(
        "Downloading attachment section {} for message {}",
        section, message_id
    );

    let stream_result = connector
        .stream_raw_part(&folder_id, &core_id, &section)
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
    if ctx.selected_message.read().as_ref() != Some(&message_id) {
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
