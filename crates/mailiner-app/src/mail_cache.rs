//! Local mailbox-tree and message-list cache.
//!
//! Persistence is abstract via [`MailCache`] so the browser can use
//! IndexedDB ([`crate::object_cache::ObjectStoreMailCache`]) or `localStorage`,
//! while host unit tests use an in-memory backend.
//!
//! Message lists are stored as a **contiguous prefix** (indices `0..n`) so
//! virtual-scroll incremental loading is unchanged: holes beyond the prefix
//! still surface as [`crate::components::virtual_scroll::SparseList::missing_ranges`].

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dioxus::logger::tracing::warn;
use mailiner_core::ids::{AccountId, MessageId};
use mailiner_core::models::{
    Envelope, Folder, FolderCounts, LoadedMessage, MessagePart, MessageSort,
};
use serde::{Deserialize, Serialize};

use crate::account_store::{AccountStoreError, MemoryKvStore, StringKvStore, WebLocalStorage};
use crate::mailbox::{
    MailboxId, MailboxNode, apply_folder_counts, apply_unread_new_state, build_mailbox_tree,
    resolve_startup_mailbox,
};
use crate::message::Message;

/// `localStorage` key for the v1 mail cache blob.
pub const MAIL_CACHE_LOCAL_STORAGE_KEY: &str = "mailiner.cache.v1";
/// Schema version for [`MailCacheBlob`].
pub const MAIL_CACHE_SCHEMA_VERSION: u32 = 1;
/// Max message-list snapshots retained per account (LRU by last access).
pub const MAX_CACHED_FOLDERS: usize = 8;
/// Max contiguous prefix envelopes stored per folder.
pub const MAX_MESSAGES_PER_FOLDER: usize = 50;
/// Soft cap on the encoded JSON blob (leave room for accounts + outbox).
pub const MAX_CACHE_BLOB_BYTES: usize = 1_500_000;

/// Persistence for mailbox trees and message-list prefixes.
///
/// `?Send` because the browser/WASM target is single-threaded.
#[async_trait(?Send)]
pub trait MailCache {
    /// Last cached folder list + STATUS totals for `account_id`.
    async fn load_folders(
        &self,
        account_id: &AccountId,
    ) -> Result<Option<CachedFolderTree>, AccountStoreError>;

    async fn save_folders(
        &self,
        account_id: &AccountId,
        tree: &CachedFolderTree,
    ) -> Result<(), AccountStoreError>;

    /// Prefix snapshot for `mailbox_id` when it was saved under `sort`.
    ///
    /// Returns `None` on a cache miss **or** when the stored sort does not
    /// match (stale order must not be shown). A hit updates LRU recency.
    async fn load_messages(
        &self,
        account_id: &AccountId,
        mailbox_id: &MailboxId,
        sort: MessageSort,
    ) -> Result<Option<CachedMessageList>, AccountStoreError>;

    /// Store a prefix snapshot. Truncates to [`MAX_MESSAGES_PER_FOLDER`] and
    /// evicts the least-recently-used folder when over [`MAX_CACHED_FOLDERS`].
    async fn save_messages(
        &self,
        account_id: &AccountId,
        list: &CachedMessageList,
    ) -> Result<(), AccountStoreError>;

    /// Drop a folder's message snapshot (e.g. after mail was moved into it).
    async fn invalidate_messages(
        &self,
        account_id: &AccountId,
        mailbox_id: &MailboxId,
    ) -> Result<(), AccountStoreError>;

    async fn delete_account(&self, account_id: &AccountId) -> Result<(), AccountStoreError>;

    /// Drop cache rows for accounts that are no longer known.
    async fn retain_accounts(&self, known: &HashSet<AccountId>) -> Result<(), AccountStoreError>;

    /// Persist decoded parts for a recently opened message.
    ///
    /// Default is a no-op so localStorage backends (tight quota) can skip bodies.
    async fn save_parts(
        &self,
        account_id: &AccountId,
        message_id: &MessageId,
        parts: &[MessagePart],
    ) -> Result<(), AccountStoreError> {
        let _ = (account_id, message_id, parts);
        Ok(())
    }

    /// Load previously opened parts. `None` on a miss.
    async fn load_parts(
        &self,
        account_id: &AccountId,
        message_id: &MessageId,
    ) -> Result<Option<Vec<MessagePart>>, AccountStoreError> {
        let _ = (account_id, message_id);
        Ok(None)
    }

    /// Drop every persisted row (sign-out / clear local data).
    async fn clear_all(&self) -> Result<(), AccountStoreError> {
        Ok(())
    }
}

/// Rebuild a [`LoadedMessage`] from cached parts when they include a body.
pub fn loaded_message_from_parts(
    message_id: &MessageId,
    parts: Vec<MessagePart>,
) -> Option<LoadedMessage> {
    let has_body = parts.iter().any(|p| {
        p.is_display_part() && !matches!(p.content, mailiner_core::models::MessageContent::Empty)
    });
    if !has_body {
        return None;
    }
    Some(LoadedMessage {
        envelope_id: message_id.clone(),
        folder_id: message_id.folder_id().clone(),
        parts,
    })
}

/// Load a cached body for offline / instant reopen.
pub async fn load_cached_loaded_message(
    cache: &dyn MailCache,
    account_id: &AccountId,
    message_id: &MessageId,
) -> Option<LoadedMessage> {
    let parts = cache
        .load_parts(account_id, message_id)
        .await
        .ok()
        .flatten()?;
    loaded_message_from_parts(message_id, parts)
}

/// Persist parts after a successful FETCH. Failures are logged, not fatal.
pub async fn persist_loaded_parts(
    cache: &dyn MailCache,
    account_id: &AccountId,
    loaded: &LoadedMessage,
) {
    if let Err(e) = cache
        .save_parts(account_id, &loaded.envelope_id, &loaded.parts)
        .await
    {
        warn!("mail cache save parts failed: {e}");
    }
}

/// Folder LIST + STATUS snapshot for one account.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CachedFolderTree {
    pub folders: Vec<Folder>,
    /// Folder id string → `STATUS` totals.
    #[serde(default)]
    pub counts: HashMap<String, FolderCounts>,
}

impl CachedFolderTree {
    pub fn new(folders: Vec<Folder>, counts: HashMap<String, FolderCounts>) -> Self {
        Self { folders, counts }
    }

    /// Convert UI tree nodes back into a persistable snapshot.
    pub fn from_nodes(account_id: &AccountId, nodes: &HashMap<MailboxId, MailboxNode>) -> Self {
        let mut folders = Vec::with_capacity(nodes.len());
        let mut counts = HashMap::with_capacity(nodes.len());
        for node in nodes.values() {
            folders.push(Folder {
                id: mailiner_core::FolderId::new(node.id.as_str()),
                account_id: account_id.clone(),
                name: node.name.clone(),
                parent_id: node
                    .parent
                    .as_ref()
                    .map(|p| mailiner_core::FolderId::new(p.as_str())),
                role: node.role,
                selectable: node.selectable,
                subscribed: node.subscribed,
            });
            counts.insert(
                node.id.as_str().to_string(),
                FolderCounts {
                    total_messages: node.total_count as u64,
                    unread_messages: node.unread_count as u64,
                },
            );
        }
        folders.sort_by(|a, b| {
            folder_depth(&MailboxId::from(a.id.clone()), nodes)
                .cmp(&folder_depth(&MailboxId::from(b.id.clone()), nodes))
                .then_with(|| a.id.to_string().cmp(&b.id.to_string()))
        });
        Self { folders, counts }
    }

    pub fn counts_by_folder_id(&self) -> HashMap<mailiner_core::FolderId, FolderCounts> {
        self.counts
            .iter()
            .map(|(id, c)| (mailiner_core::FolderId::new(id.clone()), *c))
            .collect()
    }
}

fn folder_depth(id: &MailboxId, nodes: &HashMap<MailboxId, MailboxNode>) -> usize {
    let mut depth = 0;
    let mut current = nodes.get(id).and_then(|n| n.parent.clone());
    while let Some(parent) = current {
        depth += 1;
        current = nodes.get(&parent).and_then(|n| n.parent.clone());
        if depth > 64 {
            break;
        }
    }
    depth
}

/// Contiguous message-list prefix for one folder (indices `0..envelopes.len()`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CachedMessageList {
    pub mailbox_id: String,
    pub sort: MessageSort,
    pub total: usize,
    #[serde(default)]
    pub unread: Option<usize>,
    /// Prefix only — never a sparse window from the middle of the list.
    pub envelopes: Vec<Envelope>,
    pub accessed_at: DateTime<Utc>,
}

impl CachedMessageList {
    pub fn new(
        mailbox_id: &MailboxId,
        sort: MessageSort,
        total: usize,
        unread: Option<usize>,
        envelopes: Vec<Envelope>,
    ) -> Self {
        let mut envelopes = envelopes;
        if envelopes.len() > MAX_MESSAGES_PER_FOLDER {
            envelopes.truncate(MAX_MESSAGES_PER_FOLDER);
        }
        Self {
            mailbox_id: mailbox_id.as_str().to_string(),
            sort,
            total,
            unread,
            envelopes,
            accessed_at: Utc::now(),
        }
    }

    /// Build a snapshot from a live sparse list: only the contiguous head.
    pub fn from_prefix(
        mailbox_id: &MailboxId,
        sort: MessageSort,
        total: usize,
        unread: Option<usize>,
        prefix: Vec<Envelope>,
    ) -> Self {
        Self::new(mailbox_id, sort, total, unread, prefix)
    }

    pub fn mailbox_id(&self) -> MailboxId {
        MailboxId::from(self.mailbox_id.clone())
    }

    /// UI messages for indices `0..prefix.len()`, plus the cached `total`.
    pub fn to_ui_prefix(&self) -> HydratedMessageList {
        HydratedMessageList {
            total: self.total,
            prefix: self
                .envelopes
                .iter()
                .cloned()
                .map(|e| Arc::new(Message::from(e)))
                .collect(),
        }
    }
}

/// Folder tree (and optional message prefix) ready to apply to UI signals.
#[derive(Debug, Clone)]
pub struct HydratedAccount {
    pub roots: Vec<MailboxId>,
    pub nodes: HashMap<MailboxId, MailboxNode>,
    pub selected_mailbox: Option<MailboxId>,
    pub messages: Option<HydratedMessageList>,
}

/// Cached list prefix. Caller inserts into a [`crate::components::virtual_scroll::SparseList`] at `0`.
#[derive(Debug, Clone)]
pub struct HydratedMessageList {
    pub total: usize,
    pub prefix: Vec<Arc<Message>>,
}

/// Load a cached tree (and last-mailbox prefix) for instant first paint.
///
/// Returns `Ok(None)` when this account has no folder snapshot.
pub async fn hydrate_account(
    cache: &dyn MailCache,
    account_id: &AccountId,
    sort: MessageSort,
    saved_mailbox: Option<&MailboxId>,
    acknowledged: &HashMap<MailboxId, usize>,
) -> Result<Option<HydratedAccount>, AccountStoreError> {
    let Some(tree) = cache.load_folders(account_id).await? else {
        return Ok(None);
    };
    let (roots, mut nodes) = build_mailbox_tree(tree.folders.clone());
    let counts = tree.counts_by_folder_id();
    apply_folder_counts(&mut nodes, &counts);
    apply_unread_new_state(&mut nodes, &counts, acknowledged);
    let show_all = crate::ui_prefs::load_show_all_folders();
    let selected = resolve_startup_mailbox(saved_mailbox, &nodes, &roots, show_all);
    let messages = match selected.as_ref() {
        Some(mailbox_id) => match cache.load_messages(account_id, mailbox_id, sort).await {
            Ok(list) => list.map(|list| list.to_ui_prefix()),
            Err(e) => {
                // Keep the folder tree; a list read / LRU-touch write must
                // not look like a cache miss to the caller.
                warn!("mail cache load messages failed for {account_id}: {e}");
                None
            }
        },
        None => None,
    };
    Ok(Some(HydratedAccount {
        roots,
        nodes,
        selected_mailbox: selected,
        messages,
    }))
}

/// Contiguous envelope prefix from a sparse list (stops at the first hole).
pub fn contiguous_envelope_prefix(
    get: impl Fn(usize) -> Option<Envelope>,
    total: usize,
) -> Vec<Envelope> {
    let cap = total.min(MAX_MESSAGES_PER_FOLDER);
    let mut out = Vec::with_capacity(cap);
    for i in 0..cap {
        match get(i) {
            Some(env) => out.push(env),
            None => break,
        }
    }
    out
}

// ── Blob + LRU ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
struct AccountCacheBlob {
    #[serde(default)]
    folders: Option<CachedFolderTree>,
    /// Mailbox id → prefix snapshot.
    #[serde(default)]
    messages: HashMap<String, CachedMessageList>,
}

/// Single JSON document stored under [`MAIL_CACHE_LOCAL_STORAGE_KEY`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MailCacheBlob {
    pub schema_version: u32,
    #[serde(default)]
    accounts: HashMap<AccountId, AccountCacheBlob>,
}

impl Default for MailCacheBlob {
    fn default() -> Self {
        Self::empty()
    }
}

impl MailCacheBlob {
    pub fn empty() -> Self {
        Self {
            schema_version: MAIL_CACHE_SCHEMA_VERSION,
            accounts: HashMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }

    /// Copy folder trees and message prefixes into `dest` (IndexedDB import).
    pub async fn replay_into(&self, dest: &dyn MailCache) -> Result<(), AccountStoreError> {
        for (account_id, acc) in &self.accounts {
            if let Some(tree) = &acc.folders {
                dest.save_folders(account_id, tree).await?;
            }
            for list in acc.messages.values() {
                dest.save_messages(account_id, list).await?;
            }
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<String, AccountStoreError> {
        serde_json::to_string(self).map_err(|e| AccountStoreError::Serialization(e.to_string()))
    }

    /// Deserialize. Rejects a **future** `schema_version` so a newer format is
    /// not silently rewritten as v1.
    pub fn decode(json: &str) -> Result<Self, AccountStoreError> {
        let blob: Self = serde_json::from_str(json)
            .map_err(|e| AccountStoreError::Serialization(e.to_string()))?;
        if blob.schema_version > MAIL_CACHE_SCHEMA_VERSION {
            return Err(AccountStoreError::Serialization(format!(
                "unsupported mail cache schema_version {} (max supported {})",
                blob.schema_version, MAIL_CACHE_SCHEMA_VERSION
            )));
        }
        Ok(blob)
    }

    fn account_mut(&mut self, account_id: &AccountId) -> &mut AccountCacheBlob {
        self.accounts.entry(account_id.clone()).or_default()
    }

    pub fn get_folders(&self, account_id: &AccountId) -> Option<&CachedFolderTree> {
        self.accounts.get(account_id)?.folders.as_ref()
    }

    pub fn set_folders(&mut self, account_id: &AccountId, tree: CachedFolderTree) {
        self.account_mut(account_id).folders = Some(tree);
        self.schema_version = MAIL_CACHE_SCHEMA_VERSION;
    }

    /// Hit only when `sort` matches. Updates `accessed_at`.
    pub fn take_messages(
        &mut self,
        account_id: &AccountId,
        mailbox_id: &MailboxId,
        sort: MessageSort,
    ) -> Option<CachedMessageList> {
        let acc = self.accounts.get_mut(account_id)?;
        let list = acc.messages.get_mut(mailbox_id.as_str())?;
        if list.sort != sort {
            return None;
        }
        list.accessed_at = Utc::now();
        Some(list.clone())
    }

    pub fn set_messages(&mut self, account_id: &AccountId, mut list: CachedMessageList) {
        if list.envelopes.len() > MAX_MESSAGES_PER_FOLDER {
            list.envelopes.truncate(MAX_MESSAGES_PER_FOLDER);
        }
        list.accessed_at = Utc::now();
        let key = list.mailbox_id.clone();
        let acc = self.account_mut(account_id);
        acc.messages.insert(key, list);
        evict_lru_folders(acc);
        self.schema_version = MAIL_CACHE_SCHEMA_VERSION;
    }

    pub fn invalidate_messages(&mut self, account_id: &AccountId, mailbox_id: &MailboxId) {
        if let Some(acc) = self.accounts.get_mut(account_id) {
            acc.messages.remove(mailbox_id.as_str());
        }
        self.schema_version = MAIL_CACHE_SCHEMA_VERSION;
    }

    pub fn delete_account(&mut self, account_id: &AccountId) {
        self.accounts.remove(account_id);
        self.schema_version = MAIL_CACHE_SCHEMA_VERSION;
    }

    pub fn retain_accounts(&mut self, known: &HashSet<AccountId>) {
        self.accounts.retain(|id, _| known.contains(id));
        self.schema_version = MAIL_CACHE_SCHEMA_VERSION;
    }

    /// Drop the globally oldest message-list snapshot. `true` if something was removed.
    pub fn evict_oldest_message_list(&mut self) -> bool {
        let mut oldest: Option<(AccountId, String, DateTime<Utc>)> = None;
        for (acc_id, acc) in &self.accounts {
            for (mb, list) in &acc.messages {
                let worse = oldest
                    .as_ref()
                    .is_none_or(|(_, _, t)| list.accessed_at < *t);
                if worse {
                    oldest = Some((acc_id.clone(), mb.clone(), list.accessed_at));
                }
            }
        }
        let Some((acc_id, mb, _)) = oldest else {
            return false;
        };
        if let Some(acc) = self.accounts.get_mut(&acc_id) {
            acc.messages.remove(&mb);
        }
        true
    }

    /// Encode, evicting LRU message lists until the JSON fits `limit`.
    pub fn encode_within(&mut self, limit: usize) -> Result<String, AccountStoreError> {
        loop {
            let json = self.encode()?;
            if json.len() <= limit {
                return Ok(json);
            }
            if !self.evict_oldest_message_list() {
                return Err(AccountStoreError::Other(
                    "mail cache exceeds storage budget and has nothing left to evict".into(),
                ));
            }
        }
    }
}

fn evict_lru_folders(acc: &mut AccountCacheBlob) {
    while acc.messages.len() > MAX_CACHED_FOLDERS {
        let oldest = acc
            .messages
            .iter()
            .min_by(|a, b| {
                a.1.accessed_at
                    .cmp(&b.1.accessed_at)
                    .then_with(|| a.0.cmp(b.0))
            })
            .map(|(k, _)| k.clone());
        match oldest {
            Some(k) => {
                acc.messages.remove(&k);
            }
            None => break,
        }
    }
}

// ── In-memory backend ───────────────────────────────────────────────────────

/// Process-memory [`MailCache`] for unit tests and session-only fallback.
#[derive(Debug, Default)]
pub struct InMemoryMailCache {
    blob: RefCell<MailCacheBlob>,
}

impl InMemoryMailCache {
    pub fn new() -> Self {
        Self {
            blob: RefCell::new(MailCacheBlob::empty()),
        }
    }

    #[cfg(test)]
    fn blob(&self) -> MailCacheBlob {
        self.blob.borrow().clone()
    }
}

#[async_trait(?Send)]
impl MailCache for InMemoryMailCache {
    async fn load_folders(
        &self,
        account_id: &AccountId,
    ) -> Result<Option<CachedFolderTree>, AccountStoreError> {
        Ok(self.blob.borrow().get_folders(account_id).cloned())
    }

    async fn save_folders(
        &self,
        account_id: &AccountId,
        tree: &CachedFolderTree,
    ) -> Result<(), AccountStoreError> {
        self.blob.borrow_mut().set_folders(account_id, tree.clone());
        Ok(())
    }

    async fn load_messages(
        &self,
        account_id: &AccountId,
        mailbox_id: &MailboxId,
        sort: MessageSort,
    ) -> Result<Option<CachedMessageList>, AccountStoreError> {
        Ok(self
            .blob
            .borrow_mut()
            .take_messages(account_id, mailbox_id, sort))
    }

    async fn save_messages(
        &self,
        account_id: &AccountId,
        list: &CachedMessageList,
    ) -> Result<(), AccountStoreError> {
        self.blob
            .borrow_mut()
            .set_messages(account_id, list.clone());
        Ok(())
    }

    async fn invalidate_messages(
        &self,
        account_id: &AccountId,
        mailbox_id: &MailboxId,
    ) -> Result<(), AccountStoreError> {
        self.blob
            .borrow_mut()
            .invalidate_messages(account_id, mailbox_id);
        Ok(())
    }

    async fn delete_account(&self, account_id: &AccountId) -> Result<(), AccountStoreError> {
        self.blob.borrow_mut().delete_account(account_id);
        Ok(())
    }

    async fn retain_accounts(&self, known: &HashSet<AccountId>) -> Result<(), AccountStoreError> {
        self.blob.borrow_mut().retain_accounts(known);
        Ok(())
    }

    async fn clear_all(&self) -> Result<(), AccountStoreError> {
        *self.blob.borrow_mut() = MailCacheBlob::empty();
        Ok(())
    }
}

// ── Browser / StringKvStore backend ─────────────────────────────────────────

/// [`MailCache`] over a string key-value store (`localStorage` in production).
pub struct BrowserMailCache<K: StringKvStore = WebLocalStorage> {
    kv: K,
}

impl BrowserMailCache<WebLocalStorage> {
    /// Open `window.localStorage`, or [`AccountStoreError::Unavailable`].
    pub async fn open() -> Result<Self, AccountStoreError> {
        Ok(Self {
            kv: WebLocalStorage::try_open()?,
        })
    }
}

impl BrowserMailCache<MemoryKvStore> {
    pub fn open_memory() -> Self {
        Self {
            kv: MemoryKvStore::new(),
        }
    }
}

impl<K: StringKvStore> BrowserMailCache<K> {
    pub fn with_kv(kv: K) -> Self {
        Self { kv }
    }

    fn load_blob(&self) -> Result<MailCacheBlob, AccountStoreError> {
        match self.kv.get_item(MAIL_CACHE_LOCAL_STORAGE_KEY)? {
            None => Ok(MailCacheBlob::empty()),
            Some(s) if s.trim().is_empty() => Ok(MailCacheBlob::empty()),
            Some(s) => MailCacheBlob::decode(&s),
        }
    }

    /// Snapshot used to import a leftover localStorage blob into IndexedDB.
    pub fn snapshot_blob(&self) -> Result<MailCacheBlob, AccountStoreError> {
        self.load_blob()
    }

    fn save_blob(&self, blob: &mut MailCacheBlob) -> Result<(), AccountStoreError> {
        let mut json = blob.encode_within(MAX_CACHE_BLOB_BYTES)?;
        loop {
            match self.kv.set_item(MAIL_CACHE_LOCAL_STORAGE_KEY, &json) {
                Ok(()) => return Ok(()),
                Err(err) => {
                    if !blob.evict_oldest_message_list() {
                        return Err(err);
                    }
                    json = blob.encode_within(MAX_CACHE_BLOB_BYTES)?;
                }
            }
        }
    }

    fn mutate<T>(
        &self,
        f: impl FnOnce(&mut MailCacheBlob) -> Result<T, AccountStoreError>,
    ) -> Result<T, AccountStoreError> {
        let mut blob = self.load_blob()?;
        let out = f(&mut blob)?;
        self.save_blob(&mut blob)?;
        Ok(out)
    }
}

impl<K: StringKvStore> fmt::Debug for BrowserMailCache<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrowserMailCache").finish_non_exhaustive()
    }
}

#[async_trait(?Send)]
impl<K: StringKvStore> MailCache for BrowserMailCache<K> {
    async fn load_folders(
        &self,
        account_id: &AccountId,
    ) -> Result<Option<CachedFolderTree>, AccountStoreError> {
        Ok(self.load_blob()?.get_folders(account_id).cloned())
    }

    async fn save_folders(
        &self,
        account_id: &AccountId,
        tree: &CachedFolderTree,
    ) -> Result<(), AccountStoreError> {
        self.mutate(|blob| {
            blob.set_folders(account_id, tree.clone());
            Ok(())
        })
    }

    async fn load_messages(
        &self,
        account_id: &AccountId,
        mailbox_id: &MailboxId,
        sort: MessageSort,
    ) -> Result<Option<CachedMessageList>, AccountStoreError> {
        let mut blob = self.load_blob()?;
        let got = blob.take_messages(account_id, mailbox_id, sort);
        // Only persist the LRU touch on a hit. A failed recency write must
        // still return the already-loaded prefix.
        if got.is_some()
            && let Err(e) = self.save_blob(&mut blob)
        {
            warn!("mail cache LRU touch failed: {e}");
        }
        Ok(got)
    }

    async fn save_messages(
        &self,
        account_id: &AccountId,
        list: &CachedMessageList,
    ) -> Result<(), AccountStoreError> {
        self.mutate(|blob| {
            blob.set_messages(account_id, list.clone());
            Ok(())
        })
    }

    async fn invalidate_messages(
        &self,
        account_id: &AccountId,
        mailbox_id: &MailboxId,
    ) -> Result<(), AccountStoreError> {
        self.mutate(|blob| {
            blob.invalidate_messages(account_id, mailbox_id);
            Ok(())
        })
    }

    async fn delete_account(&self, account_id: &AccountId) -> Result<(), AccountStoreError> {
        self.mutate(|blob| {
            blob.delete_account(account_id);
            Ok(())
        })
    }

    async fn retain_accounts(&self, known: &HashSet<AccountId>) -> Result<(), AccountStoreError> {
        self.mutate(|blob| {
            blob.retain_accounts(known);
            Ok(())
        })
    }

    async fn clear_all(&self) -> Result<(), AccountStoreError> {
        let mut empty = MailCacheBlob::empty();
        self.save_blob(&mut empty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use mailiner_core::{AccountId, EmailAddr, EmailAddress, FolderId, MailboxRole, MessageId};

    fn ts() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 6, 1, 12, 0, 0).unwrap()
    }

    fn folder(account: &str, id: &str, name: &str, role: MailboxRole) -> Folder {
        folder_parent(account, id, name, None, role)
    }

    fn folder_parent(
        account: &str,
        id: &str,
        name: &str,
        parent: Option<&str>,
        role: MailboxRole,
    ) -> Folder {
        Folder {
            id: FolderId::new(id),
            account_id: AccountId::new(account),
            name: name.into(),
            parent_id: parent.map(FolderId::new),
            role,
            selectable: true,
            subscribed: true,
        }
    }

    fn envelope(folder: &str, uid: &str, subject: &str) -> Envelope {
        let folder_id = FolderId::new(folder);
        Envelope {
            id: MessageId::new(folder_id.clone(), uid),
            account_id: AccountId::new("acc"),
            folder_id,
            subject: Some(subject.into()),
            from: Some(EmailAddress::List(vec![EmailAddr {
                name: Some("Ada".into()),
                email: Some("ada@example.com".into()),
            }])),
            to: None,
            cc: None,
            bcc: None,
            reply_to: None,
            rfc_message_id: None,
            in_reply_to: None,
            references: vec![],
            date: ts(),
            is_read: false,
            is_answered: false,
            is_starred: false,
            is_flagged: false,
            is_draft: false,
            is_deleted: false,
            keywords: Vec::new(),
            has_attachments: false,
            size: None,
            snippet: None,
            auth_results: Default::default(),
        }
    }

    fn list_for(mailbox: &str, n: usize, total: usize, sort: MessageSort) -> CachedMessageList {
        let envs: Vec<_> = (1..=n)
            .map(|i| envelope(mailbox, &i.to_string(), &format!("s{i}")))
            .collect();
        let mut list = CachedMessageList::new(
            &MailboxId::from(mailbox.to_string()),
            sort,
            total,
            Some(n),
            envs,
        );
        // Stable recency for LRU tests (constructor stamps Utc::now()).
        list.accessed_at = ts();
        list
    }

    // ── Blob helpers ────────────────────────────────────────────────────────

    #[test]
    fn empty_blob_and_memory_kv_snapshot() {
        assert!(MailCacheBlob::empty().is_empty());
        let cache = BrowserMailCache::<MemoryKvStore>::open_memory();
        assert!(cache.snapshot_blob().unwrap().is_empty());
    }

    #[test]
    fn storage_key_is_versioned() {
        assert_eq!(MAIL_CACHE_LOCAL_STORAGE_KEY, "mailiner.cache.v1");
        assert_eq!(MAIL_CACHE_SCHEMA_VERSION, 1);
    }

    #[test]
    fn blob_encode_decode_roundtrip() {
        let acc = AccountId::new("a1");
        let mut blob = MailCacheBlob::empty();
        blob.set_folders(
            &acc,
            CachedFolderTree::new(
                vec![folder("a1", "INBOX", "INBOX", MailboxRole::Inbox)],
                HashMap::from([(
                    "INBOX".into(),
                    FolderCounts {
                        total_messages: 3,
                        unread_messages: 1,
                    },
                )]),
            ),
        );
        let json = blob.encode().unwrap();
        assert!(json.contains("\"schema_version\":1"), "json={json}");
        assert!(json.contains("INBOX"), "json={json}");
        let back = MailCacheBlob::decode(&json).unwrap();
        assert_eq!(back, blob);
    }

    #[test]
    fn blob_decode_rejects_future_schema() {
        let json = r#"{"schema_version":99,"accounts":{}}"#;
        let err = MailCacheBlob::decode(json).unwrap_err();
        match err {
            AccountStoreError::Serialization(msg) => {
                assert!(
                    msg.contains("unsupported") && msg.contains("99"),
                    "msg={msg}"
                );
            }
            other => panic!("expected Serialization, got {other:?}"),
        }
    }

    #[test]
    fn set_messages_truncates_prefix() {
        let mut blob = MailCacheBlob::empty();
        let acc = AccountId::new("a");
        let oversized: Vec<_> = (1..=MAX_MESSAGES_PER_FOLDER + 10)
            .map(|i| envelope("INBOX", &i.to_string(), "x"))
            .collect();
        blob.set_messages(
            &acc,
            CachedMessageList::new(
                &MailboxId::from("INBOX".to_string()),
                MessageSort::Arrival,
                200,
                None,
                oversized,
            ),
        );
        let got = blob
            .take_messages(
                &acc,
                &MailboxId::from("INBOX".to_string()),
                MessageSort::Arrival,
            )
            .unwrap();
        assert_eq!(got.envelopes.len(), MAX_MESSAGES_PER_FOLDER);
        assert_eq!(got.total, 200);
    }

    #[test]
    fn lru_evicts_oldest_folder_over_limit() {
        let mut blob = MailCacheBlob::empty();
        let acc = AccountId::new("a");
        for i in 0..=MAX_CACHED_FOLDERS {
            let name = format!("F{i}");
            let mut list = list_for(&name, 1, 1, MessageSort::Arrival);
            list.accessed_at = ts() + chrono::Duration::seconds(i as i64);
            blob.set_messages(&acc, list);
        }
        let acc_blob = blob.accounts.get(&acc).unwrap();
        assert_eq!(acc_blob.messages.len(), MAX_CACHED_FOLDERS);
        assert!(
            !acc_blob.messages.contains_key("F0"),
            "oldest folder should be evicted: {:?}",
            acc_blob.messages.keys().collect::<Vec<_>>()
        );
        assert!(
            acc_blob
                .messages
                .contains_key(&format!("F{MAX_CACHED_FOLDERS}"))
        );
    }

    #[test]
    fn take_messages_misses_on_sort_mismatch() {
        let mut blob = MailCacheBlob::empty();
        let acc = AccountId::new("a");
        blob.set_messages(&acc, list_for("INBOX", 2, 2, MessageSort::Arrival));
        assert!(
            blob.take_messages(
                &acc,
                &MailboxId::from("INBOX".to_string()),
                MessageSort::Unread
            )
            .is_none()
        );
        assert!(
            blob.take_messages(
                &acc,
                &MailboxId::from("INBOX".to_string()),
                MessageSort::Arrival
            )
            .is_some()
        );
    }

    #[test]
    fn encode_within_evicts_until_fit() {
        let mut blob = MailCacheBlob::empty();
        let acc = AccountId::new("a");
        for i in 0..4 {
            let mut list = list_for(&format!("F{i}"), 8, 8, MessageSort::Arrival);
            list.accessed_at = ts() + chrono::Duration::seconds(i as i64);
            blob.set_messages(&acc, list);
        }
        let full = blob.encode().unwrap().len();
        assert!(full > 200, "fixture too small to exercise budget: {full}");
        let json = blob.encode_within(full / 2).unwrap();
        assert!(json.len() <= full / 2);
        let back = MailCacheBlob::decode(&json).unwrap();
        let left = back.accounts.get(&acc).unwrap().messages.len();
        assert!(left < 4, "expected eviction, still {left} lists");
        assert!(
            !back.accounts.get(&acc).unwrap().messages.contains_key("F0"),
            "oldest list should go first"
        );
    }

    #[test]
    fn retain_accounts_drops_unknown() {
        let mut blob = MailCacheBlob::empty();
        blob.set_folders(
            &AccountId::new("keep"),
            CachedFolderTree::new(vec![], HashMap::new()),
        );
        blob.set_folders(
            &AccountId::new("gone"),
            CachedFolderTree::new(vec![], HashMap::new()),
        );
        blob.retain_accounts(&HashSet::from([AccountId::new("keep")]));
        assert!(blob.get_folders(&AccountId::new("keep")).is_some());
        assert!(blob.get_folders(&AccountId::new("gone")).is_none());
    }

    // ── Trait backends ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn in_memory_folders_and_messages_roundtrip() {
        let cache = InMemoryMailCache::new();
        let acc = AccountId::new("acc");
        let inbox = MailboxId::from("INBOX".to_string());
        let tree = CachedFolderTree::new(
            vec![folder("acc", "INBOX", "INBOX", MailboxRole::Inbox)],
            HashMap::from([(
                "INBOX".into(),
                FolderCounts {
                    total_messages: 2,
                    unread_messages: 1,
                },
            )]),
        );
        cache.save_folders(&acc, &tree).await.unwrap();
        cache
            .save_messages(&acc, &list_for("INBOX", 2, 10, MessageSort::Arrival))
            .await
            .unwrap();

        let loaded = cache.load_folders(&acc).await.unwrap().unwrap();
        assert_eq!(loaded.folders.len(), 1);
        assert_eq!(loaded.counts["INBOX"].total_messages, 2);

        let msgs = cache
            .load_messages(&acc, &inbox, MessageSort::Arrival)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(msgs.mailbox_id().as_str(), "INBOX");
        assert_eq!(msgs.envelopes.len(), 2);
        assert_eq!(msgs.total, 10);
        assert_eq!(msgs.envelopes[0].subject.as_deref(), Some("s1"));
    }

    #[tokio::test]
    async fn browser_memory_kv_same_as_in_memory() {
        let cache = BrowserMailCache::<MemoryKvStore>::open_memory();
        let acc = AccountId::new("acc");
        cache
            .save_folders(
                &acc,
                &CachedFolderTree::new(
                    vec![folder("acc", "Sent", "Sent", MailboxRole::Sent)],
                    HashMap::new(),
                ),
            )
            .await
            .unwrap();
        let got = cache.load_folders(&acc).await.unwrap().unwrap();
        assert_eq!(got.folders[0].name, "Sent");
        let raw = cache
            .kv
            .get_item(MAIL_CACHE_LOCAL_STORAGE_KEY)
            .unwrap()
            .expect("blob written");
        assert!(raw.contains("Sent"), "json={raw}");
        assert!(
            raw.contains("\"schema_version\":1"),
            "schema missing: {raw}"
        );
    }

    #[tokio::test]
    async fn invalidate_and_delete_account() {
        let cache = InMemoryMailCache::new();
        let acc = AccountId::new("acc");
        let inbox = MailboxId::from("INBOX".to_string());
        cache
            .save_messages(&acc, &list_for("INBOX", 1, 1, MessageSort::Arrival))
            .await
            .unwrap();
        cache.invalidate_messages(&acc, &inbox).await.unwrap();
        assert!(
            cache
                .load_messages(&acc, &inbox, MessageSort::Arrival)
                .await
                .unwrap()
                .is_none()
        );
        cache
            .save_folders(
                &acc,
                &CachedFolderTree::new(
                    vec![folder("acc", "INBOX", "INBOX", MailboxRole::Inbox)],
                    HashMap::new(),
                ),
            )
            .await
            .unwrap();
        cache.delete_account(&acc).await.unwrap();
        assert!(cache.load_folders(&acc).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn hydrate_builds_tree_and_prefix() {
        let cache = InMemoryMailCache::new();
        let acc = AccountId::new("acc");
        let inbox = MailboxId::from("INBOX".to_string());
        cache
            .save_folders(
                &acc,
                &CachedFolderTree::new(
                    vec![
                        folder("acc", "INBOX", "INBOX", MailboxRole::Inbox),
                        folder("acc", "Sent", "Sent", MailboxRole::Sent),
                    ],
                    HashMap::from([(
                        "INBOX".into(),
                        FolderCounts {
                            total_messages: 100,
                            unread_messages: 4,
                        },
                    )]),
                ),
            )
            .await
            .unwrap();
        cache
            .save_messages(&acc, &list_for("INBOX", 5, 100, MessageSort::Arrival))
            .await
            .unwrap();

        let mut ack = HashMap::new();
        ack.insert(inbox.clone(), 4);
        let hydrated = hydrate_account(&cache, &acc, MessageSort::Arrival, Some(&inbox), &ack)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(hydrated.roots.len(), 2);
        assert_eq!(
            hydrated.selected_mailbox.as_ref().unwrap().as_str(),
            "INBOX"
        );
        let node = hydrated.nodes.get(&inbox).unwrap();
        assert_eq!(node.unread_count, 4);
        assert_eq!(node.total_count, 100);
        assert!(!node.has_new);
        let msgs = hydrated.messages.unwrap();
        assert_eq!(msgs.total, 100);
        assert_eq!(msgs.prefix.len(), 5);
        assert_eq!(msgs.prefix[0].subject, "s1");
    }

    #[tokio::test]
    async fn hydrate_skips_messages_when_sort_differs() {
        let cache = InMemoryMailCache::new();
        let acc = AccountId::new("acc");
        cache
            .save_folders(
                &acc,
                &CachedFolderTree::new(
                    vec![folder("acc", "INBOX", "INBOX", MailboxRole::Inbox)],
                    HashMap::new(),
                ),
            )
            .await
            .unwrap();
        cache
            .save_messages(&acc, &list_for("INBOX", 3, 3, MessageSort::Arrival))
            .await
            .unwrap();
        let hydrated = hydrate_account(&cache, &acc, MessageSort::Unread, None, &HashMap::new())
            .await
            .unwrap()
            .unwrap();
        assert!(hydrated.messages.is_none());
        assert_eq!(
            hydrated.selected_mailbox.as_ref().map(|id| id.as_str()),
            Some("INBOX")
        );
    }

    #[test]
    fn from_nodes_roundtrip_keeps_nested_children() {
        let acc = AccountId::new("acc");
        let (roots, nodes) = build_mailbox_tree(vec![
            folder_parent("acc", "KDE.pim", "pim", Some("KDE"), MailboxRole::Other),
            folder_parent("acc", "KDE", "KDE", None, MailboxRole::Other),
        ]);
        assert!(
            nodes
                .get(&MailboxId::from("KDE".to_string()))
                .is_some_and(|n| n.children.iter().any(|c| c.as_str() == "KDE.pim")),
            "precondition: child-first LIST must keep the child"
        );
        let tree = CachedFolderTree::from_nodes(&acc, &nodes);
        assert!(
            tree.folders
                .iter()
                .position(|f| f.id.to_string() == "KDE")
                .zip(
                    tree.folders
                        .iter()
                        .position(|f| f.id.to_string() == "KDE.pim")
                )
                .is_some_and(|(parent, child)| parent < child),
            "parent should be persisted before child: {:?}",
            tree.folders
                .iter()
                .map(|f| f.id.to_string())
                .collect::<Vec<_>>()
        );
        let (roots2, nodes2) = build_mailbox_tree(tree.folders);
        assert_eq!(roots.len(), roots2.len());
        let kde = nodes2
            .get(&MailboxId::from("KDE".to_string()))
            .expect("parent after hydrate");
        assert!(
            kde.children.iter().any(|id| id.as_str() == "KDE.pim"),
            "cached nested child disappeared: {:?}",
            kde.children
        );
    }

    #[test]
    fn cached_envelope_snippet_roundtrips() {
        let mut env = envelope("INBOX", "1", "s1");
        env.snippet = Some("Hello preview line".into());
        let list = CachedMessageList::new(
            &MailboxId::from("INBOX".to_string()),
            MessageSort::Arrival,
            1,
            None,
            vec![env],
        );
        let ui = list.to_ui_prefix();
        assert_eq!(ui.prefix[0].snippet.as_deref(), Some("Hello preview line"));

        let mut blob = MailCacheBlob::empty();
        blob.set_messages(&AccountId::new("a"), list);
        let json = blob.encode().unwrap();
        let mut back = MailCacheBlob::decode(&json).unwrap();
        let got = back
            .take_messages(
                &AccountId::new("a"),
                &MailboxId::from("INBOX".to_string()),
                MessageSort::Arrival,
            )
            .unwrap();
        assert_eq!(
            got.envelopes[0].snippet.as_deref(),
            Some("Hello preview line")
        );
    }

    #[test]
    fn cached_prefix_is_contiguous_head_for_virtual_scroll() {
        let list = list_for("INBOX", 5, 40, MessageSort::Arrival);
        let ui = list.to_ui_prefix();
        // SparseList is filled only at 0..prefix.len(); the rest stay holes
        // so virtual scroll still requests FetchMessageRange for 5..40.
        assert_eq!(ui.total, 40);
        assert_eq!(ui.prefix.len(), 5);
        assert_eq!(ui.prefix[0].id.as_uid(), "1");
        assert_eq!(ui.prefix[4].id.as_uid(), "5");
    }

    #[test]
    fn contiguous_prefix_stops_at_first_hole() {
        let items = [
            Some(envelope("INBOX", "1", "a")),
            Some(envelope("INBOX", "2", "b")),
            None,
            Some(envelope("INBOX", "4", "d")),
        ];
        let prefix = contiguous_envelope_prefix(|i| items.get(i).and_then(|e| e.clone()), 10);
        assert_eq!(prefix.len(), 2);
        assert_eq!(prefix[1].subject.as_deref(), Some("b"));
    }

    #[test]
    fn hydrate_none_without_folder_cache() {
        let cache = InMemoryMailCache::new();
        let acc = AccountId::new("missing");
        let out = futures_executor::block_on(hydrate_account(
            &cache,
            &acc,
            MessageSort::Arrival,
            None,
            &HashMap::new(),
        ))
        .unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn in_memory_lru_matches_blob() {
        let cache = InMemoryMailCache::new();
        let acc = AccountId::new("a");
        futures_executor::block_on(async {
            for i in 0..=MAX_CACHED_FOLDERS {
                let mut list = list_for(&format!("F{i}"), 1, 1, MessageSort::Arrival);
                list.accessed_at = ts() + chrono::Duration::seconds(i as i64);
                cache.save_messages(&acc, &list).await.unwrap();
            }
        });
        let blob = cache.blob();
        assert_eq!(
            blob.accounts.get(&acc).unwrap().messages.len(),
            MAX_CACHED_FOLDERS
        );
        assert!(!blob.accounts.get(&acc).unwrap().messages.contains_key("F0"));
    }

    #[test]
    fn loaded_message_from_parts_requires_display_body() {
        let id = MessageId::new(FolderId::new("INBOX"), "1");
        assert!(loaded_message_from_parts(&id, vec![]).is_none());

        let now = ts();
        let empty = mailiner_core::models::MessagePart {
            id: mailiner_core::ids::MessagePartId::new("TEXT"),
            envelope_id: id.clone(),
            path: vec![],
            kind: mailiner_core::models::PartKind::TextPlain,
            content_type: "text/plain".into(),
            charset: None,
            content_id: None,
            description: None,
            filename: None,
            encoding: mailiner_core::models::TransferEncoding::SevenBit,
            original_size: None,
            size: 0,
            is_attachment: false,
            is_hidden: false,
            nested_in: None,
            nested_headers: None,
            content: mailiner_core::models::MessageContent::Empty,
            created_at: now,
            updated_at: now,
        };
        assert!(loaded_message_from_parts(&id, vec![empty.clone()]).is_none());

        let mut body = empty;
        body.content = mailiner_core::models::MessageContent::Text("hi".into());
        let loaded = loaded_message_from_parts(&id, vec![body]).unwrap();
        assert_eq!(loaded.envelope_id, id);
        assert_eq!(loaded.folder_id.as_str(), "INBOX");
    }

    #[tokio::test]
    async fn blob_replay_into_memory_cache() {
        let acc = AccountId::new("acc");
        let mut blob = MailCacheBlob::empty();
        blob.set_folders(
            &acc,
            CachedFolderTree::new(
                vec![folder("acc", "INBOX", "INBOX", MailboxRole::Inbox)],
                HashMap::new(),
            ),
        );
        blob.set_messages(&acc, list_for("INBOX", 2, 2, MessageSort::Arrival));

        let dest = InMemoryMailCache::new();
        blob.replay_into(&dest).await.unwrap();
        assert_eq!(
            dest.load_folders(&acc)
                .await
                .unwrap()
                .unwrap()
                .folders
                .len(),
            1
        );
        assert_eq!(
            dest.load_messages(
                &acc,
                &MailboxId::from("INBOX".to_string()),
                MessageSort::Arrival
            )
            .await
            .unwrap()
            .unwrap()
            .envelopes
            .len(),
            2
        );
    }

    #[tokio::test]
    async fn in_memory_parts_default_to_miss() {
        let cache = InMemoryMailCache::new();
        let acc = AccountId::new("acc");
        let id = MessageId::new(FolderId::new("INBOX"), "1");
        cache.save_parts(&acc, &id, &[]).await.unwrap();
        assert!(cache.load_parts(&acc, &id).await.unwrap().is_none());
        assert!(
            load_cached_loaded_message(&cache, &acc, &id)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn in_memory_clear_all() {
        let cache = InMemoryMailCache::new();
        let acc = AccountId::new("acc");
        cache
            .save_folders(
                &acc,
                &CachedFolderTree::new(
                    vec![folder("acc", "INBOX", "INBOX", MailboxRole::Inbox)],
                    HashMap::new(),
                ),
            )
            .await
            .unwrap();
        cache.clear_all().await.unwrap();
        assert!(cache.load_folders(&acc).await.unwrap().is_none());
    }
}
