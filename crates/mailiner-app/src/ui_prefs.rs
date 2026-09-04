//! UI preferences that are not account secrets (`mailiner.ui.lastMailbox.v1`).
//!
//! Separate from [`crate::account_store`] so a folder click does not rewrite
//! passwords or bump account `updated_at`.

use std::collections::{HashMap, HashSet};

use mailiner_core::ids::AccountId;
use mailiner_core::mailbox_search_is_active;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(target_arch = "wasm32")]
use crate::account_store::WebLocalStorage;
use crate::account_store::{AccountStoreError, StringKvStore};
use crate::mailbox::MailboxId;
use mailiner_core::{MessageListFilter, MessageSort};

/// `localStorage` key for the message-list sort.
pub const MESSAGE_SORT_KEY: &str = "mailiner.ui.messageSort";

/// `localStorage` key for the message-list row density.
pub const MESSAGE_LIST_DENSITY_KEY: &str = "mailiner.ui.messageListDensity";

/// `localStorage` key: show unsubscribed folders in the tree / pickers.
pub const SHOW_ALL_FOLDERS_KEY: &str = "mailiner.ui.showAllFolders";

/// Virtualized message-list row density.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MessageListDensity {
    Compact,
    Cozy,
    /// Matches the original 52px rows.
    #[default]
    Comfortable,
}

impl MessageListDensity {
    pub const ALL: [Self; 3] = [Self::Compact, Self::Cozy, Self::Comfortable];

    pub fn as_key(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Cozy => "cozy",
            Self::Comfortable => "comfortable",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "compact" => Some(Self::Compact),
            "cozy" => Some(Self::Cozy),
            "comfortable" => Some(Self::Comfortable),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Compact => "Compact",
            Self::Cozy => "Cozy",
            Self::Comfortable => "Comfortable",
        }
    }

    /// Virtualized row height; must match `#messagelist` density CSS.
    pub fn item_height(self) -> f64 {
        match self {
            Self::Compact => 40.0,
            Self::Cozy => 46.0,
            Self::Comfortable => 52.0,
        }
    }

    pub fn css_class(self) -> &'static str {
        match self {
            Self::Compact => "density-compact",
            Self::Cozy => "density-cozy",
            Self::Comfortable => "density-comfortable",
        }
    }
}

/// `localStorage` key for the color-theme override (`system` | `light` | `dark`).
pub const THEME_KEY: &str = "mailiner.ui.theme";

/// `localStorage` key for the default composer body mode.
pub const COMPOSE_BODY_MODE_KEY: &str = "mailiner.ui.composeBodyMode";

/// `localStorage` key for composer placement (`modal` | `docked`).
pub const COMPOSE_PLACEMENT_KEY: &str = "mailiner.ui.composePlacement";

/// `localStorage` key for the preferred compose From account.
pub const DEFAULT_FROM_ACCOUNT_KEY: &str = "mailiner.ui.defaultFromAccount";

/// `localStorage` key for the global remote-image default (`true` | `false`).
pub const ALLOW_REMOTE_IMAGES_KEY: &str = "mailiner.ui.allowRemoteImages";

/// `localStorage` key for per-sender / per-domain remote-image overrides.
pub const REMOTE_IMAGE_SENDERS_KEY: &str = "mailiner.ui.remoteImageSenders.v1";
/// Schema version for [`RemoteImageSendersBlob`].
pub const REMOTE_IMAGE_SENDERS_SCHEMA_VERSION: u32 = 1;
/// Cap on remembered From addresses (evict an arbitrary extra on insert).
pub const MAX_REMOTE_IMAGE_ADDRESSES: usize = 200;
/// Cap on remembered From domains.
pub const MAX_REMOTE_IMAGE_DOMAINS: usize = 100;

/// Color theme preference. `System` follows `prefers-color-scheme`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePref {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn as_key(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    /// `data-theme` on `<html>`. `None` leaves the attribute unset (OS default).
    pub fn data_theme(self) -> Option<&'static str> {
        match self {
            Self::System => None,
            Self::Light => Some("light"),
            Self::Dark => Some("dark"),
        }
    }
}

/// Default editor format when opening a new message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ComposeBodyMode {
    /// Matches the current plain-text compose overlay.
    #[default]
    Plain,
    Rich,
}

impl ComposeBodyMode {
    pub const ALL: [Self; 2] = [Self::Plain, Self::Rich];

    pub fn as_key(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Rich => "rich",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "plain" => Some(Self::Plain),
            "rich" => Some(Self::Rich),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Plain => "Plain text",
            Self::Rich => "Rich text",
        }
    }
}

/// Where the composer is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ComposePlacement {
    /// Centered modal dialog (covers the mailbox).
    #[default]
    Modal,
    /// In-flow pane at the bottom of the mail chrome.
    Docked,
}

impl ComposePlacement {
    pub const ALL: [Self; 2] = [Self::Modal, Self::Docked];

    pub fn as_key(self) -> &'static str {
        match self {
            Self::Modal => "modal",
            Self::Docked => "docked",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "modal" => Some(Self::Modal),
            "docked" => Some(Self::Docked),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Modal => "Dialog",
            Self::Docked => "Docked to bottom",
        }
    }

    /// Modal compose covers the mailbox; docked leaves mail shortcuts usable.
    pub fn blocks_mail_shortcuts(self, compose_open: bool) -> bool {
        compose_open && matches!(self, Self::Modal)
    }
}

/// `localStorage` key for mail chrome arrangement (`stacked` | `classic`).
pub const MAIL_LAYOUT_KEY: &str = "mailiner.ui.mailLayout";

/// Mail chrome pane arrangement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MailLayout {
    /// Folders | (list stacked above viewer).
    #[default]
    Stacked,
    /// Folders | list | viewer side-by-side.
    Classic,
}

impl MailLayout {
    pub const ALL: [Self; 2] = [Self::Stacked, Self::Classic];

    pub fn as_key(self) -> &'static str {
        match self {
            Self::Stacked => "stacked",
            Self::Classic => "classic",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "stacked" => Some(Self::Stacked),
            "classic" => Some(Self::Classic),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Stacked => "List above message",
            Self::Classic => "Three columns",
        }
    }

    /// Class on `#app` so chrome CSS can switch pane axes.
    pub fn css_class(self) -> &'static str {
        match self {
            Self::Stacked => "layout-stacked",
            Self::Classic => "layout-classic",
        }
    }
}

/// Remembered allow/block for remote images.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteImagePref {
    Allow,
    Block,
}

impl RemoteImagePref {
    pub fn label(self) -> &'static str {
        match self {
            Self::Allow => "Allow",
            Self::Block => "Block",
        }
    }
}

/// Which rule produced a [`RemoteImageDecision`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RemoteImageSource {
    Address,
    Domain,
    Global,
}

/// Resolved remote-image policy for one From address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteImageDecision {
    pub pref: RemoteImagePref,
    pub source: RemoteImageSource,
}

impl RemoteImageDecision {
    pub fn allowed(self) -> bool {
        self.pref == RemoteImagePref::Allow
    }
}

/// Address vs domain row in the remembered-sender list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RemoteImageSenderKind {
    Address,
    Domain,
}

/// One remembered From address or domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteImageSenderEntry {
    pub key: String,
    pub kind: RemoteImageSenderKind,
    pub pref: RemoteImagePref,
}

impl RemoteImageSenderEntry {
    pub fn display_key(&self) -> String {
        match self.kind {
            RemoteImageSenderKind::Address => self.key.clone(),
            RemoteImageSenderKind::Domain => format!("@{}", self.key),
        }
    }
}

/// Per-From-address and per-domain remote-image overrides.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RemoteImageSendersBlob {
    pub schema_version: u32,
    /// Lowercased `local@domain` → pref.
    #[serde(default)]
    pub addresses: HashMap<String, RemoteImagePref>,
    /// Lowercased domain (no `@`) → pref.
    #[serde(default)]
    pub domains: HashMap<String, RemoteImagePref>,
}

impl RemoteImageSendersBlob {
    pub fn empty() -> Self {
        Self {
            schema_version: REMOTE_IMAGE_SENDERS_SCHEMA_VERSION,
            addresses: HashMap::new(),
            domains: HashMap::new(),
        }
    }

    pub fn encode(&self) -> Result<String, AccountStoreError> {
        serde_json::to_string(self).map_err(|e| AccountStoreError::Serialization(e.to_string()))
    }

    pub fn decode(json: &str) -> Result<Self, AccountStoreError> {
        let blob: Self = serde_json::from_str(json)
            .map_err(|e| AccountStoreError::Serialization(e.to_string()))?;
        if blob.schema_version > REMOTE_IMAGE_SENDERS_SCHEMA_VERSION {
            return Err(AccountStoreError::Serialization(format!(
                "unsupported remote-image-senders schema_version {} (max supported {})",
                blob.schema_version, REMOTE_IMAGE_SENDERS_SCHEMA_VERSION
            )));
        }
        Ok(blob)
    }

    /// Address match wins over domain. `email` is normalized first.
    pub fn pref_for(&self, email: &str) -> Option<(RemoteImagePref, RemoteImageSource)> {
        let norm = normalize_email(email)?;
        if let Some(pref) = self.addresses.get(&norm) {
            return Some((*pref, RemoteImageSource::Address));
        }
        let domain = domain_of_normalized(&norm)?;
        self.domains
            .get(&domain)
            .copied()
            .map(|pref| (pref, RemoteImageSource::Domain))
    }

    pub fn set_address(&mut self, email: String, pref: RemoteImagePref) {
        evict_if_new(&mut self.addresses, &email, MAX_REMOTE_IMAGE_ADDRESSES);
        self.addresses.insert(email, pref);
        self.schema_version = REMOTE_IMAGE_SENDERS_SCHEMA_VERSION;
    }

    pub fn set_domain(&mut self, domain: String, pref: RemoteImagePref) {
        evict_if_new(&mut self.domains, &domain, MAX_REMOTE_IMAGE_DOMAINS);
        self.domains.insert(domain, pref);
        self.schema_version = REMOTE_IMAGE_SENDERS_SCHEMA_VERSION;
    }

    pub fn clear_address(&mut self, email: &str) {
        self.addresses.remove(email);
        self.schema_version = REMOTE_IMAGE_SENDERS_SCHEMA_VERSION;
    }

    pub fn clear_domain(&mut self, domain: &str) {
        self.domains.remove(domain);
        self.schema_version = REMOTE_IMAGE_SENDERS_SCHEMA_VERSION;
    }

    pub fn entries(&self) -> Vec<RemoteImageSenderEntry> {
        let mut out: Vec<RemoteImageSenderEntry> = self
            .addresses
            .iter()
            .map(|(key, pref)| RemoteImageSenderEntry {
                key: key.clone(),
                kind: RemoteImageSenderKind::Address,
                pref: *pref,
            })
            .chain(
                self.domains
                    .iter()
                    .map(|(key, pref)| RemoteImageSenderEntry {
                        key: key.clone(),
                        kind: RemoteImageSenderKind::Domain,
                        pref: *pref,
                    }),
            )
            .collect();
        out.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.key.cmp(&b.key)));
        out
    }
}

fn evict_if_new(map: &mut HashMap<String, RemoteImagePref>, key: &str, max: usize) {
    if map.contains_key(key) || map.len() < max {
        return;
    }
    if let Some(victim) = map.keys().next().cloned() {
        map.remove(&victim);
    }
}

/// Trim + ASCII-lowercase `local@domain`. Rejects missing parts.
pub fn normalize_email(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (local, domain) = trimmed.rsplit_once('@')?;
    if local.is_empty() || domain.is_empty() {
        return None;
    }
    if domain.starts_with('.') || domain.ends_with('.') || domain.contains('@') {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

/// Domain of a raw or already-normalized email (`example.com`, no `@`).
pub fn domain_of_email(raw: &str) -> Option<String> {
    domain_of_normalized(&normalize_email(raw)?)
}

/// Trim `@` / dots and lowercase a domain. Rejects empty or address-like input.
pub fn normalize_domain(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches('@').trim_matches('.');
    if trimmed.is_empty() || trimmed.contains('@') || trimmed.contains('/') || trimmed.contains(' ')
    {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

fn domain_of_normalized(email: &str) -> Option<String> {
    let (_, domain) = email.rsplit_once('@')?;
    normalize_domain(domain)
}

/// `localStorage` key for message-list quick filters.
pub const MESSAGE_LIST_FILTER_KEY: &str = "mailiner.ui.messageListFilter";

/// `localStorage` key for last-opened mailbox per account.
pub const LAST_MAILBOX_KEY: &str = "mailiner.ui.lastMailbox.v1";
/// Schema version for [`LastMailboxBlob`] (independent of the account store).
pub const LAST_MAILBOX_SCHEMA_VERSION: u32 = 1;

/// `localStorage` key for last-acknowledged unread counts (opened-folder watermark).
pub const ACK_UNREAD_KEY: &str = "mailiner.ui.ackUnread.v1";
/// Schema version for [`AckUnreadBlob`].
pub const ACK_UNREAD_SCHEMA_VERSION: u32 = 1;

/// `localStorage` key for the Inbox desktop-notification toggle (default off).
pub const NOTIFY_INBOX_KEY: &str = "mailiner.ui.notifyInbox";

/// `localStorage` key for user shortcut remaps (missing ids keep catalog defaults).
pub const SHORTCUT_MAP_KEY: &str = "mailiner.ui.shortcuts.v1";
/// Schema version for [`ShortcutMapBlob`].
pub const SHORTCUT_MAP_SCHEMA_VERSION: u32 = 1;

/// `localStorage` key for saved IMAP searches (virtual folders).
pub const SAVED_SEARCHES_KEY: &str = "mailiner.ui.savedSearches.v1";
/// Schema version for [`SavedSearchesBlob`].
pub const SAVED_SEARCHES_SCHEMA_VERSION: u32 = 1;
/// Cap on remembered searches (evict the oldest on insert).
pub const MAX_SAVED_SEARCHES: usize = 50;

/// `localStorage` key for per-folder pinned message UIDs.
pub const PINNED_MESSAGES_KEY: &str = "mailiner.ui.pinnedMessages.v1";
/// Schema version for [`PinnedMessagesBlob`].
pub const PINNED_MESSAGES_SCHEMA_VERSION: u32 = 1;
/// Cap on pinned UIDs remembered for one account+mailbox.
pub const MAX_PINNED_PER_MAILBOX: usize = 50;

/// One remapped binding. `key` is a `KeyboardEvent.key` value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShortcutBinding {
    pub key: String,
    #[serde(default)]
    pub shift: bool,
}

/// Persisted remaps. Absent ids use [`crate::shortcuts::GLOBAL_SHORTCUTS`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ShortcutMapBlob {
    pub schema_version: u32,
    /// [`crate::shortcuts::ShortcutId::as_key`] → binding.
    #[serde(default)]
    pub remaps: HashMap<String, ShortcutBinding>,
}

impl ShortcutMapBlob {
    pub fn empty() -> Self {
        Self {
            schema_version: SHORTCUT_MAP_SCHEMA_VERSION,
            remaps: HashMap::new(),
        }
    }

    pub fn encode(&self) -> Result<String, AccountStoreError> {
        serde_json::to_string(self).map_err(|e| AccountStoreError::Serialization(e.to_string()))
    }

    pub fn decode(json: &str) -> Result<Self, AccountStoreError> {
        let blob: Self = serde_json::from_str(json)
            .map_err(|e| AccountStoreError::Serialization(e.to_string()))?;
        if blob.schema_version > SHORTCUT_MAP_SCHEMA_VERSION {
            return Err(AccountStoreError::Serialization(format!(
                "unsupported shortcut-map schema_version {} (max supported {})",
                blob.schema_version, SHORTCUT_MAP_SCHEMA_VERSION
            )));
        }
        Ok(blob)
    }
}

/// One user-saved IMAP search, shown as a virtual folder in the tree.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SavedSearch {
    pub id: String,
    pub name: String,
    pub query: String,
    pub account_id: AccountId,
    /// IMAP folder id the search was saved against.
    pub mailbox_id: String,
}

impl SavedSearch {
    pub fn mailbox(&self) -> MailboxId {
        MailboxId::from(self.mailbox_id.clone())
    }

    /// True when `query` matches the name or IMAP query (folder-filter box).
    pub fn matches_filter(&self, query: &str) -> bool {
        let words: Vec<String> = query
            .split_whitespace()
            .map(|w| w.to_ascii_lowercase())
            .collect();
        if words.is_empty() {
            return true;
        }
        let name = self.name.to_ascii_lowercase();
        let q = self.query.to_ascii_lowercase();
        words
            .iter()
            .all(|w| name.contains(w.as_str()) || q.contains(w.as_str()))
    }
}

/// Why [`SavedSearchesBlob::add`] refused a search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveSearchError {
    EmptyQuery,
}

/// Persisted virtual folders. Order is insertion order (oldest first).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SavedSearchesBlob {
    pub schema_version: u32,
    #[serde(default)]
    pub searches: Vec<SavedSearch>,
}

impl SavedSearchesBlob {
    pub fn empty() -> Self {
        Self {
            schema_version: SAVED_SEARCHES_SCHEMA_VERSION,
            searches: Vec::new(),
        }
    }

    pub fn encode(&self) -> Result<String, AccountStoreError> {
        serde_json::to_string(self).map_err(|e| AccountStoreError::Serialization(e.to_string()))
    }

    pub fn decode(json: &str) -> Result<Self, AccountStoreError> {
        let blob: Self = serde_json::from_str(json)
            .map_err(|e| AccountStoreError::Serialization(e.to_string()))?;
        if blob.schema_version > SAVED_SEARCHES_SCHEMA_VERSION {
            return Err(AccountStoreError::Serialization(format!(
                "unsupported saved-searches schema_version {} (max supported {})",
                blob.schema_version, SAVED_SEARCHES_SCHEMA_VERSION
            )));
        }
        Ok(blob)
    }

    pub fn get(&self, id: &str) -> Option<&SavedSearch> {
        self.searches.iter().find(|s| s.id == id)
    }

    pub fn for_account(&self, account_id: &AccountId) -> Vec<SavedSearch> {
        self.searches
            .iter()
            .filter(|s| &s.account_id == account_id)
            .cloned()
            .collect()
    }

    /// Insert or refresh a search for this account + folder + query.
    ///
    /// An existing row is renamed when `name` is non-empty. The oldest row is
    /// dropped when the cap is reached.
    pub fn add(
        &mut self,
        name: &str,
        query: &str,
        account_id: AccountId,
        mailbox_id: &MailboxId,
    ) -> Result<SavedSearch, SaveSearchError> {
        let query = query.trim().to_string();
        if !mailbox_search_is_active(&query) {
            return Err(SaveSearchError::EmptyQuery);
        }
        let name = {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                query.clone()
            } else {
                trimmed.to_string()
            }
        };
        let mailbox = mailbox_id.as_str();
        if let Some(existing) = self
            .searches
            .iter_mut()
            .find(|s| s.account_id == account_id && s.mailbox_id == mailbox && s.query == query)
        {
            existing.name = name;
            self.schema_version = SAVED_SEARCHES_SCHEMA_VERSION;
            return Ok(existing.clone());
        }
        if self.searches.len() >= MAX_SAVED_SEARCHES {
            self.searches.remove(0);
        }
        let search = SavedSearch {
            id: Uuid::new_v4().to_string(),
            name,
            query,
            account_id,
            mailbox_id: mailbox.to_string(),
        };
        self.searches.push(search.clone());
        self.schema_version = SAVED_SEARCHES_SCHEMA_VERSION;
        Ok(search)
    }

    pub fn rename(&mut self, id: &str, name: &str) -> Option<SavedSearch> {
        let search = self.searches.iter_mut().find(|s| s.id == id)?;
        let trimmed = name.trim();
        search.name = if trimmed.is_empty() {
            search.query.clone()
        } else {
            trimmed.to_string()
        };
        self.schema_version = SAVED_SEARCHES_SCHEMA_VERSION;
        Some(search.clone())
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.searches.len();
        self.searches.retain(|s| s.id != id);
        if self.searches.len() != before {
            self.schema_version = SAVED_SEARCHES_SCHEMA_VERSION;
            true
        } else {
            false
        }
    }

    pub fn retain_accounts(&mut self, known: &HashSet<AccountId>) {
        self.searches.retain(|s| known.contains(&s.account_id));
        self.schema_version = SAVED_SEARCHES_SCHEMA_VERSION;
    }
}

/// Single JSON document stored under [`LAST_MAILBOX_KEY`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LastMailboxBlob {
    pub schema_version: u32,
    /// Account id → IMAP folder id (mailbox id string).
    pub last_mailbox: HashMap<AccountId, String>,
}

impl LastMailboxBlob {
    pub fn empty() -> Self {
        Self {
            schema_version: LAST_MAILBOX_SCHEMA_VERSION,
            last_mailbox: HashMap::new(),
        }
    }

    pub fn encode(&self) -> Result<String, AccountStoreError> {
        serde_json::to_string(self).map_err(|e| AccountStoreError::Serialization(e.to_string()))
    }

    /// Rejects blobs whose `schema_version` is greater than
    /// [`LAST_MAILBOX_SCHEMA_VERSION`].
    pub fn decode(json: &str) -> Result<Self, AccountStoreError> {
        let blob: Self = serde_json::from_str(json)
            .map_err(|e| AccountStoreError::Serialization(e.to_string()))?;
        if blob.schema_version > LAST_MAILBOX_SCHEMA_VERSION {
            return Err(AccountStoreError::Serialization(format!(
                "unsupported last-mailbox schema_version {} (max supported {})",
                blob.schema_version, LAST_MAILBOX_SCHEMA_VERSION
            )));
        }
        Ok(blob)
    }

    pub fn get(&self, account_id: &AccountId) -> Option<MailboxId> {
        self.last_mailbox
            .get(account_id)
            .filter(|s| !s.is_empty())
            .map(|s| MailboxId::from(s.clone()))
    }

    pub fn set(&mut self, account_id: AccountId, mailbox_id: &MailboxId) {
        self.last_mailbox
            .insert(account_id, mailbox_id.as_str().to_string());
        self.schema_version = LAST_MAILBOX_SCHEMA_VERSION;
    }

    pub fn retain_accounts(&mut self, known: &HashSet<AccountId>) {
        self.last_mailbox.retain(|id, _| known.contains(id));
        self.schema_version = LAST_MAILBOX_SCHEMA_VERSION;
    }
}

/// Per-account unread watermarks: opening a folder acknowledges its current unread count.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AckUnreadBlob {
    pub schema_version: u32,
    /// Account id → mailbox id → unread count last seen while that folder was open.
    pub acknowledged: HashMap<AccountId, HashMap<String, usize>>,
}

impl AckUnreadBlob {
    pub fn empty() -> Self {
        Self {
            schema_version: ACK_UNREAD_SCHEMA_VERSION,
            acknowledged: HashMap::new(),
        }
    }

    pub fn encode(&self) -> Result<String, AccountStoreError> {
        serde_json::to_string(self).map_err(|e| AccountStoreError::Serialization(e.to_string()))
    }

    pub fn decode(json: &str) -> Result<Self, AccountStoreError> {
        let blob: Self = serde_json::from_str(json)
            .map_err(|e| AccountStoreError::Serialization(e.to_string()))?;
        if blob.schema_version > ACK_UNREAD_SCHEMA_VERSION {
            return Err(AccountStoreError::Serialization(format!(
                "unsupported ack-unread schema_version {} (max supported {})",
                blob.schema_version, ACK_UNREAD_SCHEMA_VERSION
            )));
        }
        Ok(blob)
    }

    pub fn get(&self, account_id: &AccountId) -> HashMap<MailboxId, usize> {
        self.acknowledged
            .get(account_id)
            .map(|m| {
                m.iter()
                    .map(|(id, n)| (MailboxId::from(id.clone()), *n))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn set(&mut self, account_id: AccountId, mailbox_id: &MailboxId, unread: usize) {
        self.acknowledged
            .entry(account_id)
            .or_default()
            .insert(mailbox_id.as_str().to_string(), unread);
        self.schema_version = ACK_UNREAD_SCHEMA_VERSION;
    }

    pub fn retain_accounts(&mut self, known: &HashSet<AccountId>) {
        self.acknowledged.retain(|id, _| known.contains(id));
        self.schema_version = ACK_UNREAD_SCHEMA_VERSION;
    }
}

/// Per-account, per-mailbox pinned IMAP UIDs (local order overlay).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PinnedMessagesBlob {
    pub schema_version: u32,
    /// Account id → mailbox id → UIDs (pin order, first is top of the list).
    #[serde(default)]
    pub pinned: HashMap<AccountId, HashMap<String, Vec<String>>>,
}

impl PinnedMessagesBlob {
    pub fn empty() -> Self {
        Self {
            schema_version: PINNED_MESSAGES_SCHEMA_VERSION,
            pinned: HashMap::new(),
        }
    }

    pub fn encode(&self) -> Result<String, AccountStoreError> {
        serde_json::to_string(self).map_err(|e| AccountStoreError::Serialization(e.to_string()))
    }

    pub fn decode(json: &str) -> Result<Self, AccountStoreError> {
        let blob: Self = serde_json::from_str(json)
            .map_err(|e| AccountStoreError::Serialization(e.to_string()))?;
        if blob.schema_version > PINNED_MESSAGES_SCHEMA_VERSION {
            return Err(AccountStoreError::Serialization(format!(
                "unsupported pinned-messages schema_version {} (max supported {})",
                blob.schema_version, PINNED_MESSAGES_SCHEMA_VERSION
            )));
        }
        Ok(blob)
    }

    pub fn uids(&self, account_id: &AccountId, mailbox_id: &MailboxId) -> Vec<String> {
        self.pinned
            .get(account_id)
            .and_then(|m| m.get(mailbox_id.as_str()))
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_uids(
        &mut self,
        account_id: AccountId,
        mailbox_id: &MailboxId,
        mut uids: Vec<String>,
    ) {
        uids.retain(|uid| !uid.is_empty());
        uids.truncate(MAX_PINNED_PER_MAILBOX);
        let mailbox = mailbox_id.as_str().to_string();
        if uids.is_empty() {
            if let Some(mailboxes) = self.pinned.get_mut(&account_id) {
                mailboxes.remove(&mailbox);
                if mailboxes.is_empty() {
                    self.pinned.remove(&account_id);
                }
            }
        } else {
            self.pinned
                .entry(account_id)
                .or_default()
                .insert(mailbox, uids);
        }
        self.schema_version = PINNED_MESSAGES_SCHEMA_VERSION;
    }

    /// Pin (`pin`) or unpin `uids` for one folder. Pinned UIDs are moved to the
    /// front in the given order. Returns the resulting UID list.
    pub fn apply_toggle(
        &mut self,
        account_id: AccountId,
        mailbox_id: &MailboxId,
        uids: &[String],
        pin: bool,
    ) -> Vec<String> {
        let mut current = self.uids(&account_id, mailbox_id);
        if pin {
            let selected: Vec<String> = uids
                .iter()
                .filter(|uid| !uid.is_empty())
                .cloned()
                .collect::<Vec<_>>();
            current.retain(|uid| !selected.iter().any(|s| s == uid));
            let mut next = selected;
            next.extend(current);
            self.set_uids(account_id.clone(), mailbox_id, next);
        } else {
            current.retain(|uid| !uids.iter().any(|s| s == uid));
            self.set_uids(account_id.clone(), mailbox_id, current);
        }
        self.uids(&account_id, mailbox_id)
    }

    pub fn retain_accounts(&mut self, known: &HashSet<AccountId>) {
        self.pinned.retain(|id, _| known.contains(id));
        self.schema_version = PINNED_MESSAGES_SCHEMA_VERSION;
    }
}

fn load_blob(kv: &dyn StringKvStore) -> Result<LastMailboxBlob, AccountStoreError> {
    match kv.get_item(LAST_MAILBOX_KEY)? {
        None => Ok(LastMailboxBlob::empty()),
        Some(s) if s.trim().is_empty() => Ok(LastMailboxBlob::empty()),
        Some(s) => LastMailboxBlob::decode(&s),
    }
}

fn save_blob(kv: &dyn StringKvStore, blob: &LastMailboxBlob) -> Result<(), AccountStoreError> {
    kv.set_item(LAST_MAILBOX_KEY, &blob.encode()?)
}

fn with_kv<T>(f: impl FnOnce(&dyn StringKvStore) -> Result<T, AccountStoreError>) -> Option<T> {
    #[cfg(target_arch = "wasm32")]
    {
        let storage = WebLocalStorage::try_open().ok()?;
        f(&storage).ok()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        host_kv::with(|kv| f(kv).ok())
    }
}

/// Last successfully opened mailbox for `account_id`, if any.
pub fn load_last_mailbox(account_id: &AccountId) -> Option<MailboxId> {
    with_kv(|kv| Ok(load_blob(kv)?.get(account_id)))?
}

/// Persist a successful folder open. Failures are ignored (preference only).
pub fn save_last_mailbox(account_id: &AccountId, mailbox_id: &MailboxId) {
    let _ = with_kv(|kv| {
        let mut blob = load_blob(kv)?;
        blob.set(account_id.clone(), mailbox_id);
        save_blob(kv, &blob)
    });
}

pub fn load_message_sort() -> MessageSort {
    with_kv(|kv| {
        Ok(kv
            .get_item(MESSAGE_SORT_KEY)?
            .as_deref()
            .and_then(MessageSort::from_key)
            .unwrap_or_default())
    })
    .unwrap_or_default()
}

pub fn save_message_sort(sort: MessageSort) {
    let _ = with_kv(|kv| kv.set_item(MESSAGE_SORT_KEY, sort.as_key()));
}

pub fn load_message_list_density() -> MessageListDensity {
    with_kv(|kv| {
        Ok(kv
            .get_item(MESSAGE_LIST_DENSITY_KEY)?
            .as_deref()
            .and_then(MessageListDensity::from_key)
            .unwrap_or_default())
    })
    .unwrap_or_default()
}

pub fn load_message_list_filter() -> MessageListFilter {
    with_kv(|kv| {
        Ok(kv
            .get_item(MESSAGE_LIST_FILTER_KEY)?
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default())
    })
    .unwrap_or_default()
}

pub fn save_message_list_density(density: MessageListDensity) {
    let _ = with_kv(|kv| kv.set_item(MESSAGE_LIST_DENSITY_KEY, density.as_key()));
}

pub fn load_theme() -> ThemePref {
    with_kv(|kv| {
        Ok(kv
            .get_item(THEME_KEY)?
            .as_deref()
            .and_then(ThemePref::from_key)
            .unwrap_or_default())
    })
    .unwrap_or_default()
}

pub fn save_theme(theme: ThemePref) {
    let _ = with_kv(|kv| kv.set_item(THEME_KEY, theme.as_key()));
}

pub fn load_compose_body_mode() -> ComposeBodyMode {
    with_kv(|kv| {
        Ok(kv
            .get_item(COMPOSE_BODY_MODE_KEY)?
            .as_deref()
            .and_then(ComposeBodyMode::from_key)
            .unwrap_or_default())
    })
    .unwrap_or_default()
}

pub fn save_compose_body_mode(mode: ComposeBodyMode) {
    let _ = with_kv(|kv| kv.set_item(COMPOSE_BODY_MODE_KEY, mode.as_key()));
}

pub fn load_compose_placement() -> ComposePlacement {
    with_kv(|kv| {
        Ok(kv
            .get_item(COMPOSE_PLACEMENT_KEY)?
            .as_deref()
            .and_then(ComposePlacement::from_key)
            .unwrap_or_default())
    })
    .unwrap_or_default()
}

pub fn save_compose_placement(placement: ComposePlacement) {
    let _ = with_kv(|kv| kv.set_item(COMPOSE_PLACEMENT_KEY, placement.as_key()));
}

pub fn load_mail_layout() -> MailLayout {
    with_kv(|kv| {
        Ok(kv
            .get_item(MAIL_LAYOUT_KEY)?
            .as_deref()
            .and_then(MailLayout::from_key)
            .unwrap_or_default())
    })
    .unwrap_or_default()
}

pub fn save_mail_layout(layout: MailLayout) {
    let _ = with_kv(|kv| kv.set_item(MAIL_LAYOUT_KEY, layout.as_key()));
}

/// Preferred compose From account, if one was saved.
pub fn load_default_from_account() -> Option<AccountId> {
    with_kv(|kv| {
        Ok(kv
            .get_item(DEFAULT_FROM_ACCOUNT_KEY)?
            .filter(|s| !s.is_empty())
            .map(AccountId::new))
    })
    .flatten()
}

pub fn save_default_from_account(account_id: Option<&AccountId>) {
    let _ = with_kv(|kv| match account_id {
        Some(id) if !id.as_str().is_empty() => kv.set_item(DEFAULT_FROM_ACCOUNT_KEY, id.as_str()),
        _ => kv.set_item(DEFAULT_FROM_ACCOUNT_KEY, ""),
    });
}

/// Use the saved From account when it still exists; otherwise the active account.
pub fn resolve_compose_account_id(
    preferred: Option<&AccountId>,
    selected: Option<&AccountId>,
    known: impl Fn(&AccountId) -> bool,
) -> Option<AccountId> {
    if let Some(id) = preferred.filter(|id| known(id)) {
        return Some(id.clone());
    }
    selected.filter(|id| known(id)).cloned()
}

pub fn load_allow_remote_images() -> bool {
    with_kv(|kv| {
        Ok(kv
            .get_item(ALLOW_REMOTE_IMAGES_KEY)?
            .as_deref()
            .map(|s| s == "true")
            .unwrap_or(false))
    })
    .unwrap_or(false)
}

pub fn save_allow_remote_images(allow: bool) {
    let value = if allow { "true" } else { "false" };
    let _ = with_kv(|kv| kv.set_item(ALLOW_REMOTE_IMAGES_KEY, value));
}

fn load_remote_senders_blob(
    kv: &dyn StringKvStore,
) -> Result<RemoteImageSendersBlob, AccountStoreError> {
    match kv.get_item(REMOTE_IMAGE_SENDERS_KEY)? {
        None => Ok(RemoteImageSendersBlob::empty()),
        Some(s) if s.trim().is_empty() => Ok(RemoteImageSendersBlob::empty()),
        Some(s) => RemoteImageSendersBlob::decode(&s),
    }
}

fn save_remote_senders_blob(
    kv: &dyn StringKvStore,
    blob: &RemoteImageSendersBlob,
) -> Result<(), AccountStoreError> {
    kv.set_item(REMOTE_IMAGE_SENDERS_KEY, &blob.encode()?)
}

pub fn load_remote_image_senders() -> RemoteImageSendersBlob {
    with_kv(load_remote_senders_blob).unwrap_or_else(RemoteImageSendersBlob::empty)
}

/// Address override, then domain, then the global default.
pub fn remote_image_decision(from_email: Option<&str>) -> RemoteImageDecision {
    if let Some(email) = from_email
        && let Some((pref, source)) = load_remote_image_senders().pref_for(email)
    {
        return RemoteImageDecision { pref, source };
    }
    RemoteImageDecision {
        pref: if load_allow_remote_images() {
            RemoteImagePref::Allow
        } else {
            RemoteImagePref::Block
        },
        source: RemoteImageSource::Global,
    }
}

pub fn save_remote_image_address(email: &str, pref: RemoteImagePref) -> bool {
    let Some(norm) = normalize_email(email) else {
        return false;
    };
    with_kv(|kv| {
        let mut blob = load_remote_senders_blob(kv)?;
        blob.set_address(norm, pref);
        save_remote_senders_blob(kv, &blob)
    })
    .is_some()
}

pub fn clear_remote_image_address(email: &str) {
    let Some(norm) = normalize_email(email) else {
        return;
    };
    let _ = with_kv(|kv| {
        let mut blob = load_remote_senders_blob(kv)?;
        blob.clear_address(&norm);
        save_remote_senders_blob(kv, &blob)
    });
}

pub fn save_remote_image_domain(domain: &str, pref: RemoteImagePref) -> bool {
    let Some(norm) = normalize_domain(domain) else {
        return false;
    };
    with_kv(|kv| {
        let mut blob = load_remote_senders_blob(kv)?;
        blob.set_domain(norm, pref);
        save_remote_senders_blob(kv, &blob)
    })
    .is_some()
}

pub fn clear_remote_image_domain(domain: &str) {
    let Some(norm) = normalize_domain(domain) else {
        return;
    };
    let _ = with_kv(|kv| {
        let mut blob = load_remote_senders_blob(kv)?;
        blob.clear_domain(&norm);
        save_remote_senders_blob(kv, &blob)
    });
}

pub fn clear_remote_image_entry(kind: RemoteImageSenderKind, key: &str) {
    match kind {
        RemoteImageSenderKind::Address => clear_remote_image_address(key),
        RemoteImageSenderKind::Domain => clear_remote_image_domain(key),
    }
}

/// Set or clear `data-theme` on `<html>` so CSS tokens follow the pref.
pub fn apply_theme(pref: ThemePref) {
    let value = pref.data_theme();
    #[cfg(target_arch = "wasm32")]
    {
        let Some(el) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.document_element())
        else {
            return;
        };
        match value {
            Some(theme) => {
                let _ = el.set_attribute("data-theme", theme);
            }
            None => {
                let _ = el.remove_attribute("data-theme");
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = value;
    }
}

pub fn load_show_all_folders() -> bool {
    with_kv(|kv| {
        Ok(kv
            .get_item(SHOW_ALL_FOLDERS_KEY)?
            .as_deref()
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true")))
    })
    .unwrap_or(false)
}

pub fn save_show_all_folders(show_all: bool) {
    let _ = with_kv(|kv| kv.set_item(SHOW_ALL_FOLDERS_KEY, if show_all { "1" } else { "0" }));
}

pub fn save_message_list_filter(filter: MessageListFilter) {
    let _ = with_kv(|kv| {
        let json = serde_json::to_string(&filter)
            .map_err(|e| AccountStoreError::Serialization(e.to_string()))?;
        kv.set_item(MESSAGE_LIST_FILTER_KEY, &json)
    });
}

/// Drop last-mailbox rows for accounts that are no longer known.
pub fn retain_last_mailboxes(known: &HashSet<AccountId>) {
    let _ = with_kv(|kv| {
        let mut blob = load_blob(kv)?;
        blob.retain_accounts(known);
        save_blob(kv, &blob)
    });
}

fn load_ack_blob(kv: &dyn StringKvStore) -> Result<AckUnreadBlob, AccountStoreError> {
    match kv.get_item(ACK_UNREAD_KEY)? {
        None => Ok(AckUnreadBlob::empty()),
        Some(s) if s.trim().is_empty() => Ok(AckUnreadBlob::empty()),
        Some(s) => AckUnreadBlob::decode(&s),
    }
}

fn save_ack_blob(kv: &dyn StringKvStore, blob: &AckUnreadBlob) -> Result<(), AccountStoreError> {
    kv.set_item(ACK_UNREAD_KEY, &blob.encode()?)
}

/// Last unread counts the user opened each folder with, for `account_id`.
pub fn load_ack_unread(account_id: &AccountId) -> HashMap<MailboxId, usize> {
    with_kv(|kv| Ok(load_ack_blob(kv)?.get(account_id))).unwrap_or_default()
}

/// Persist the unread count seen while `mailbox_id` was open.
pub fn save_ack_unread(account_id: &AccountId, mailbox_id: &MailboxId, unread: usize) {
    let _ = with_kv(|kv| {
        let mut blob = load_ack_blob(kv)?;
        blob.set(account_id.clone(), mailbox_id, unread);
        save_ack_blob(kv, &blob)
    });
}

/// Drop acknowledged-unread rows for accounts that are no longer known.
pub fn retain_ack_unread(known: &HashSet<AccountId>) {
    let _ = with_kv(|kv| {
        let mut blob = load_ack_blob(kv)?;
        blob.retain_accounts(known);
        save_ack_blob(kv, &blob)
    });
}

fn load_pinned_blob(kv: &dyn StringKvStore) -> Result<PinnedMessagesBlob, AccountStoreError> {
    match kv.get_item(PINNED_MESSAGES_KEY)? {
        None => Ok(PinnedMessagesBlob::empty()),
        Some(s) if s.trim().is_empty() => Ok(PinnedMessagesBlob::empty()),
        Some(s) => PinnedMessagesBlob::decode(&s),
    }
}

fn save_pinned_blob(
    kv: &dyn StringKvStore,
    blob: &PinnedMessagesBlob,
) -> Result<(), AccountStoreError> {
    kv.set_item(PINNED_MESSAGES_KEY, &blob.encode()?)
}

/// Pinned IMAP UIDs for `account_id` + `mailbox_id` (pin order).
pub fn load_pinned_uids(account_id: &AccountId, mailbox_id: &MailboxId) -> Vec<String> {
    with_kv(|kv| Ok(load_pinned_blob(kv)?.uids(account_id, mailbox_id))).unwrap_or_default()
}

/// Replace the pinned UID list for one folder.
pub fn save_pinned_uids(account_id: &AccountId, mailbox_id: &MailboxId, uids: Vec<String>) {
    let _ = with_kv(|kv| {
        let mut blob = load_pinned_blob(kv)?;
        blob.set_uids(account_id.clone(), mailbox_id, uids);
        save_pinned_blob(kv, &blob)
    });
}

/// Pin or unpin `uids` in one folder. Returns the resulting UID list.
pub fn toggle_pinned_uids(
    account_id: &AccountId,
    mailbox_id: &MailboxId,
    uids: &[String],
    pin: bool,
) -> Vec<String> {
    with_kv(|kv| {
        let mut blob = load_pinned_blob(kv)?;
        let next = blob.apply_toggle(account_id.clone(), mailbox_id, uids, pin);
        save_pinned_blob(kv, &blob)?;
        Ok(next)
    })
    .unwrap_or_default()
}

/// Drop pinned rows for accounts that are no longer known.
pub fn retain_pinned_messages(known: &HashSet<AccountId>) {
    let _ = with_kv(|kv| {
        let mut blob = load_pinned_blob(kv)?;
        blob.retain_accounts(known);
        save_pinned_blob(kv, &blob)
    });
}

/// Whether the user opted in to Inbox desktop notifications.
///
/// Default is off. The toggle is only persisted as on after Notification
/// permission is granted.
pub fn load_notify_inbox() -> bool {
    with_kv(|kv| Ok(kv.get_item(NOTIFY_INBOX_KEY)?.as_deref() == Some("1"))).unwrap_or(false)
}

pub fn save_notify_inbox(enabled: bool) {
    let _ = with_kv(|kv| kv.set_item(NOTIFY_INBOX_KEY, if enabled { "1" } else { "0" }));
}

fn load_shortcut_map_blob(kv: &dyn StringKvStore) -> Result<ShortcutMapBlob, AccountStoreError> {
    match kv.get_item(SHORTCUT_MAP_KEY)? {
        None => Ok(ShortcutMapBlob::empty()),
        Some(s) if s.trim().is_empty() => Ok(ShortcutMapBlob::empty()),
        Some(s) => ShortcutMapBlob::decode(&s),
    }
}

/// User remaps, or an empty map (catalog defaults).
pub fn load_shortcut_map() -> ShortcutMapBlob {
    with_kv(load_shortcut_map_blob).unwrap_or_else(ShortcutMapBlob::empty)
}

pub fn save_shortcut_map(blob: &ShortcutMapBlob) {
    let mut stored = blob.clone();
    stored.schema_version = SHORTCUT_MAP_SCHEMA_VERSION;
    let _ = with_kv(|kv| kv.set_item(SHORTCUT_MAP_KEY, &stored.encode()?));
}

fn load_saved_searches_blob(
    kv: &dyn StringKvStore,
) -> Result<SavedSearchesBlob, AccountStoreError> {
    match kv.get_item(SAVED_SEARCHES_KEY)? {
        None => Ok(SavedSearchesBlob::empty()),
        Some(s) if s.trim().is_empty() => Ok(SavedSearchesBlob::empty()),
        Some(s) => SavedSearchesBlob::decode(&s),
    }
}

fn save_saved_searches_blob(
    kv: &dyn StringKvStore,
    blob: &SavedSearchesBlob,
) -> Result<(), AccountStoreError> {
    kv.set_item(SAVED_SEARCHES_KEY, &blob.encode()?)
}

/// All saved searches, or an empty list if storage is missing / unreadable.
pub fn load_saved_searches() -> Vec<SavedSearch> {
    with_kv(load_saved_searches_blob)
        .unwrap_or_else(SavedSearchesBlob::empty)
        .searches
}

fn mutate_saved_searches<T>(f: impl FnOnce(&mut SavedSearchesBlob) -> T) -> Option<T> {
    with_kv(|kv| {
        let mut blob = load_saved_searches_blob(kv)?;
        let out = f(&mut blob);
        save_saved_searches_blob(kv, &blob)?;
        Ok(out)
    })
}

/// Persist a search for the open folder. Same account + folder + query updates
/// the name instead of inserting a duplicate.
pub fn add_saved_search(
    name: &str,
    query: &str,
    account_id: AccountId,
    mailbox_id: &MailboxId,
) -> Result<SavedSearch, SaveSearchError> {
    let result = mutate_saved_searches(|blob| blob.add(name, query, account_id, mailbox_id));
    match result {
        Some(inner) => inner,
        None => Err(SaveSearchError::EmptyQuery),
    }
}

pub fn rename_saved_search(id: &str, name: &str) -> Option<SavedSearch> {
    mutate_saved_searches(|blob| blob.rename(id, name)).flatten()
}

pub fn remove_saved_search(id: &str) -> bool {
    mutate_saved_searches(|blob| blob.remove(id)).unwrap_or(false)
}

/// Drop saved searches whose account is no longer known.
pub fn retain_saved_searches(known: &HashSet<AccountId>) {
    let _ = mutate_saved_searches(|blob| blob.retain_accounts(known));
}

#[cfg(not(target_arch = "wasm32"))]
mod host_kv {
    use crate::account_store::MemoryKvStore;
    use std::cell::RefCell;

    thread_local! {
        static KV: RefCell<MemoryKvStore> = RefCell::new(MemoryKvStore::new());
    }

    pub fn with<T>(f: impl FnOnce(&MemoryKvStore) -> T) -> T {
        KV.with(|cell| f(&cell.borrow()))
    }

    #[cfg(test)]
    pub fn reset() {
        KV.with(|cell| *cell.borrow_mut() = MemoryKvStore::new());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_key_is_versioned() {
        assert_eq!(LAST_MAILBOX_KEY, "mailiner.ui.lastMailbox.v1");
        assert_eq!(LAST_MAILBOX_SCHEMA_VERSION, 1);
    }

    #[test]
    fn blob_encode_decode_roundtrip() {
        let mut blob = LastMailboxBlob::empty();
        let acc = AccountId::new("acc-1");
        blob.set(acc.clone(), &MailboxId::from("INBOX.Work".to_string()));

        let json = blob.encode().expect("encode");
        assert!(json.contains("\"schema_version\":1"), "json={json}");
        assert!(json.contains("INBOX.Work"), "json={json}");

        let back = LastMailboxBlob::decode(&json).expect("decode");
        assert_eq!(back, blob);
        assert_eq!(
            back.get(&acc).as_ref().map(|id| id.as_str()),
            Some("INBOX.Work")
        );
    }

    #[test]
    fn blob_decode_rejects_future_schema() {
        let json = r#"{"schema_version":99,"last_mailbox":{}}"#;
        let err = LastMailboxBlob::decode(json).unwrap_err();
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
    fn blob_get_skips_empty_folder_id() {
        let mut blob = LastMailboxBlob::empty();
        blob.last_mailbox
            .insert(AccountId::new("acc"), String::new());
        assert!(blob.get(&AccountId::new("acc")).is_none());
    }

    #[test]
    fn retain_drops_unknown_accounts() {
        let mut blob = LastMailboxBlob::empty();
        blob.set(
            AccountId::new("keep"),
            &MailboxId::from("INBOX".to_string()),
        );
        blob.set(AccountId::new("gone"), &MailboxId::from("Sent".to_string()));
        let known = HashSet::from([AccountId::new("keep")]);
        blob.retain_accounts(&known);
        assert!(blob.get(&AccountId::new("keep")).is_some());
        assert!(blob.get(&AccountId::new("gone")).is_none());
    }

    #[test]
    fn host_load_save_roundtrip() {
        host_kv::reset();
        let acc = AccountId::new("host-acc");
        assert!(load_last_mailbox(&acc).is_none());

        save_last_mailbox(&acc, &MailboxId::from("Archive".to_string()));
        assert_eq!(
            load_last_mailbox(&acc).as_ref().map(|id| id.as_str()),
            Some("Archive")
        );

        save_last_mailbox(&acc, &MailboxId::from("INBOX".to_string()));
        assert_eq!(
            load_last_mailbox(&acc).as_ref().map(|id| id.as_str()),
            Some("INBOX")
        );

        retain_last_mailboxes(&HashSet::new());
        assert!(load_last_mailbox(&acc).is_none());
        host_kv::reset();
    }

    #[test]
    fn message_sort_roundtrip() {
        host_kv::reset();
        assert_eq!(load_message_sort(), MessageSort::Arrival);
        save_message_sort(MessageSort::Unread);
        assert_eq!(load_message_sort(), MessageSort::Unread);
        save_message_sort(MessageSort::Sender);
        assert_eq!(load_message_sort(), MessageSort::Sender);
        save_message_sort(MessageSort::Date);
        assert_eq!(load_message_sort(), MessageSort::Date);
        host_kv::reset();
    }

    #[test]
    fn message_list_density_encode_decode_roundtrip() {
        for density in MessageListDensity::ALL {
            let key = density.as_key();
            assert_eq!(MessageListDensity::from_key(key), Some(density));
        }
        assert_eq!(MessageListDensity::from_key("nope"), None);
        assert_eq!(
            MessageListDensity::default(),
            MessageListDensity::Comfortable
        );
        assert_eq!(MessageListDensity::Compact.item_height(), 40.0);
        assert_eq!(MessageListDensity::Cozy.item_height(), 46.0);
        assert_eq!(MessageListDensity::Comfortable.item_height(), 52.0);
        assert_eq!(
            MessageListDensity::Comfortable.css_class(),
            "density-comfortable"
        );
    }

    #[test]
    fn message_list_density_roundtrip() {
        host_kv::reset();
        assert_eq!(load_message_list_density(), MessageListDensity::Comfortable);
        save_message_list_density(MessageListDensity::Compact);
        assert_eq!(load_message_list_density(), MessageListDensity::Compact);
        save_message_list_density(MessageListDensity::Cozy);
        assert_eq!(load_message_list_density(), MessageListDensity::Cozy);
        save_message_list_density(MessageListDensity::Comfortable);
        assert_eq!(load_message_list_density(), MessageListDensity::Comfortable);

        host_kv::with(|kv| {
            kv.set_item(MESSAGE_LIST_DENSITY_KEY, "nope")
                .expect("set unknown density");
        });
        assert_eq!(load_message_list_density(), MessageListDensity::Comfortable);
        host_kv::reset();
    }

    #[test]
    fn message_list_filter_roundtrip() {
        host_kv::reset();
        assert!(load_message_list_filter().is_empty());
        let filter = MessageListFilter {
            unread: true,
            flagged: false,
            has_attachment: true,
        };
        save_message_list_filter(filter);
        assert_eq!(load_message_list_filter(), filter);
        save_message_list_filter(MessageListFilter::default());
        assert!(load_message_list_filter().is_empty());
        host_kv::reset();
    }

    #[test]
    fn theme_pref_keys_roundtrip() {
        for pref in ThemePref::ALL {
            assert_eq!(ThemePref::from_key(pref.as_key()), Some(pref));
        }
        assert_eq!(ThemePref::from_key("nope"), None);
        assert_eq!(ThemePref::from_key(""), None);
        assert_eq!(ThemePref::default(), ThemePref::System);
        assert_eq!(ThemePref::System.data_theme(), None);
        assert_eq!(ThemePref::Light.data_theme(), Some("light"));
        assert_eq!(ThemePref::Dark.data_theme(), Some("dark"));
    }

    #[test]
    fn theme_pref_load_save_roundtrip() {
        host_kv::reset();
        assert_eq!(load_theme(), ThemePref::System);
        save_theme(ThemePref::Dark);
        assert_eq!(load_theme(), ThemePref::Dark);
        save_theme(ThemePref::Light);
        assert_eq!(load_theme(), ThemePref::Light);
        save_theme(ThemePref::System);
        assert_eq!(load_theme(), ThemePref::System);
        host_kv::reset();
    }

    #[test]
    fn theme_pref_unknown_falls_back_to_system() {
        host_kv::reset();
        host_kv::with(|kv| {
            kv.set_item(THEME_KEY, "rainbow").expect("set");
        });
        assert_eq!(load_theme(), ThemePref::System);
    }

    #[test]
    fn show_all_folders_roundtrip() {
        host_kv::reset();
        assert!(!load_show_all_folders());
        save_show_all_folders(true);
        assert!(load_show_all_folders());
        save_show_all_folders(false);
        assert!(!load_show_all_folders());
        host_kv::reset();
    }

    #[test]
    fn compose_body_mode_encode_decode_roundtrip() {
        for mode in ComposeBodyMode::ALL {
            assert_eq!(ComposeBodyMode::from_key(mode.as_key()), Some(mode));
        }
        assert_eq!(ComposeBodyMode::from_key("html"), None);
        assert_eq!(ComposeBodyMode::default(), ComposeBodyMode::Plain);
    }

    #[test]
    fn compose_body_mode_roundtrip() {
        host_kv::reset();
        assert_eq!(load_compose_body_mode(), ComposeBodyMode::Plain);
        save_compose_body_mode(ComposeBodyMode::Rich);
        assert_eq!(load_compose_body_mode(), ComposeBodyMode::Rich);
        save_compose_body_mode(ComposeBodyMode::Plain);
        assert_eq!(load_compose_body_mode(), ComposeBodyMode::Plain);
        host_kv::with(|kv| {
            kv.set_item(COMPOSE_BODY_MODE_KEY, "nope")
                .expect("set unknown compose mode");
        });
        assert_eq!(load_compose_body_mode(), ComposeBodyMode::Plain);
        host_kv::reset();
    }

    #[test]
    fn compose_placement_encode_decode_roundtrip() {
        for placement in ComposePlacement::ALL {
            assert_eq!(
                ComposePlacement::from_key(placement.as_key()),
                Some(placement)
            );
        }
        assert_eq!(ComposePlacement::from_key("side"), None);
        assert_eq!(ComposePlacement::default(), ComposePlacement::Modal);
        assert_eq!(ComposePlacement::Modal.label(), "Dialog");
        assert_eq!(ComposePlacement::Docked.label(), "Docked to bottom");
        assert!(ComposePlacement::Modal.blocks_mail_shortcuts(true));
        assert!(!ComposePlacement::Docked.blocks_mail_shortcuts(true));
        assert!(!ComposePlacement::Modal.blocks_mail_shortcuts(false));
    }

    #[test]
    fn compose_placement_roundtrip() {
        host_kv::reset();
        assert_eq!(load_compose_placement(), ComposePlacement::Modal);
        save_compose_placement(ComposePlacement::Docked);
        assert_eq!(load_compose_placement(), ComposePlacement::Docked);
        save_compose_placement(ComposePlacement::Modal);
        assert_eq!(load_compose_placement(), ComposePlacement::Modal);
        host_kv::with(|kv| {
            kv.set_item(COMPOSE_PLACEMENT_KEY, "nope")
                .expect("set unknown compose placement");
        });
        assert_eq!(load_compose_placement(), ComposePlacement::Modal);
        host_kv::reset();
    }

    #[test]
    fn mail_layout_encode_decode_roundtrip() {
        for layout in MailLayout::ALL {
            assert_eq!(MailLayout::from_key(layout.as_key()), Some(layout));
        }
        assert_eq!(MailLayout::from_key("wide"), None);
        assert_eq!(MailLayout::default(), MailLayout::Stacked);
        assert_eq!(MailLayout::Stacked.label(), "List above message");
        assert_eq!(MailLayout::Classic.label(), "Three columns");
        assert_eq!(MailLayout::Stacked.css_class(), "layout-stacked");
        assert_eq!(MailLayout::Classic.css_class(), "layout-classic");
    }

    #[test]
    fn mail_layout_roundtrip() {
        host_kv::reset();
        assert_eq!(load_mail_layout(), MailLayout::Stacked);
        save_mail_layout(MailLayout::Classic);
        assert_eq!(load_mail_layout(), MailLayout::Classic);
        save_mail_layout(MailLayout::Stacked);
        assert_eq!(load_mail_layout(), MailLayout::Stacked);
        host_kv::with(|kv| {
            kv.set_item(MAIL_LAYOUT_KEY, "nope")
                .expect("set unknown mail layout");
        });
        assert_eq!(load_mail_layout(), MailLayout::Stacked);
        host_kv::reset();
    }

    #[test]
    fn default_from_account_roundtrip() {
        host_kv::reset();
        assert!(load_default_from_account().is_none());
        let acc = AccountId::new("from-acc");
        save_default_from_account(Some(&acc));
        assert_eq!(
            load_default_from_account().as_ref().map(|id| id.as_str()),
            Some("from-acc")
        );
        save_default_from_account(None);
        assert!(load_default_from_account().is_none());
        host_kv::reset();
    }

    #[test]
    fn resolve_compose_account_prefers_saved_when_known() {
        let preferred = AccountId::new("pref");
        let selected = AccountId::new("sel");
        let known = |id: &AccountId| id.as_str() == "pref" || id.as_str() == "sel";
        assert_eq!(
            resolve_compose_account_id(Some(&preferred), Some(&selected), known)
                .as_ref()
                .map(|id| id.as_str()),
            Some("pref")
        );
        let unknown = AccountId::new("gone");
        assert_eq!(
            resolve_compose_account_id(Some(&unknown), Some(&selected), known)
                .as_ref()
                .map(|id| id.as_str()),
            Some("sel")
        );
        assert!(resolve_compose_account_id(Some(&unknown), None, known).is_none());
    }

    #[test]
    fn ack_unread_roundtrip() {
        host_kv::reset();
        let acc = AccountId::new("ack-acc");
        let inbox = MailboxId::from("INBOX".to_string());
        assert!(load_ack_unread(&acc).is_empty());

        save_ack_unread(&acc, &inbox, 4);
        assert_eq!(load_ack_unread(&acc).get(&inbox).copied(), Some(4));

        save_ack_unread(&acc, &inbox, 1);
        assert_eq!(load_ack_unread(&acc).get(&inbox).copied(), Some(1));

        retain_ack_unread(&HashSet::new());
        assert!(load_ack_unread(&acc).is_empty());
        host_kv::reset();
    }

    #[test]
    fn ack_unread_decode_rejects_future_schema() {
        let json = r#"{"schema_version":99,"acknowledged":{}}"#;
        let err = AckUnreadBlob::decode(json).unwrap_err();
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
    fn allow_remote_images_roundtrip() {
        host_kv::reset();
        assert!(!load_allow_remote_images());
        save_allow_remote_images(true);
        assert!(load_allow_remote_images());
        save_allow_remote_images(false);
        assert!(!load_allow_remote_images());
        host_kv::with(|kv| {
            kv.set_item(ALLOW_REMOTE_IMAGES_KEY, "yes")
                .expect("set unknown remote-images value");
        });
        assert!(!load_allow_remote_images());
        host_kv::reset();
    }

    #[test]
    fn normalize_email_and_domain() {
        assert_eq!(
            normalize_email("  Alice@Example.COM "),
            Some("alice@example.com".into())
        );
        assert_eq!(normalize_email("nodomain"), None);
        assert_eq!(normalize_email("@example.com"), None);
        assert_eq!(normalize_email("alice@"), None);
        assert_eq!(normalize_email(""), None);
        assert_eq!(normalize_email("alice@.example.com"), None);
        assert_eq!(
            domain_of_email("Alice@News.Example.COM"),
            Some("news.example.com".into())
        );
        assert_eq!(
            normalize_domain("@Example.COM."),
            Some("example.com".into())
        );
        assert_eq!(normalize_domain("alice@example.com"), None);
        assert_eq!(normalize_domain("example.com/path"), None);
        assert_eq!(normalize_domain(""), None);
    }

    #[test]
    fn sender_pref_address_beats_domain_beats_global() {
        host_kv::reset();
        save_allow_remote_images(false);
        assert_eq!(
            remote_image_decision(Some("alice@example.com")),
            RemoteImageDecision {
                pref: RemoteImagePref::Block,
                source: RemoteImageSource::Global,
            }
        );

        assert!(save_remote_image_domain(
            "example.com",
            RemoteImagePref::Allow
        ));
        assert_eq!(
            remote_image_decision(Some("ALICE@Example.COM")),
            RemoteImageDecision {
                pref: RemoteImagePref::Allow,
                source: RemoteImageSource::Domain,
            }
        );

        assert!(save_remote_image_address(
            "alice@example.com",
            RemoteImagePref::Block
        ));
        assert_eq!(
            remote_image_decision(Some("Alice@example.com")),
            RemoteImageDecision {
                pref: RemoteImagePref::Block,
                source: RemoteImageSource::Address,
            }
        );

        assert_eq!(
            remote_image_decision(Some("bob@example.com")),
            RemoteImageDecision {
                pref: RemoteImagePref::Allow,
                source: RemoteImageSource::Domain,
            }
        );
        assert_eq!(
            remote_image_decision(Some("carol@other.test")),
            RemoteImageDecision {
                pref: RemoteImagePref::Block,
                source: RemoteImageSource::Global,
            }
        );
        assert_eq!(
            remote_image_decision(None),
            RemoteImageDecision {
                pref: RemoteImagePref::Block,
                source: RemoteImageSource::Global,
            }
        );
        host_kv::reset();
    }

    #[test]
    fn sender_pref_clear_falls_back() {
        host_kv::reset();
        save_allow_remote_images(true);
        assert!(save_remote_image_address(
            "mallory@phish.test",
            RemoteImagePref::Block
        ));
        assert!(!remote_image_decision(Some("mallory@phish.test")).allowed());
        clear_remote_image_address("Mallory@phish.test");
        assert!(remote_image_decision(Some("mallory@phish.test")).allowed());
        host_kv::reset();
    }

    #[test]
    fn remote_image_senders_blob_roundtrip_and_entries() {
        let mut blob = RemoteImageSendersBlob::empty();
        blob.set_address("bob@example.com".into(), RemoteImagePref::Allow);
        blob.set_address("alice@example.com".into(), RemoteImagePref::Block);
        blob.set_domain("news.example.com".into(), RemoteImagePref::Allow);

        let json = blob.encode().expect("encode");
        assert!(json.contains("\"schema_version\":1"), "json={json}");
        let back = RemoteImageSendersBlob::decode(&json).expect("decode");
        assert_eq!(back, blob);

        let entries = back.entries();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].key, "alice@example.com");
        assert_eq!(entries[0].kind, RemoteImageSenderKind::Address);
        assert_eq!(entries[1].key, "bob@example.com");
        assert_eq!(entries[2].kind, RemoteImageSenderKind::Domain);
        assert_eq!(entries[2].display_key(), "@news.example.com");
    }

    #[test]
    fn remote_image_senders_decode_rejects_future_schema() {
        let json = r#"{"schema_version":99,"addresses":{},"domains":{}}"#;
        let err = RemoteImageSendersBlob::decode(json).unwrap_err();
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
    fn remote_image_senders_unknown_pref_is_empty() {
        host_kv::reset();
        host_kv::with(|kv| {
            kv.set_item(
                REMOTE_IMAGE_SENDERS_KEY,
                r#"{"schema_version":1,"addresses":{"a@b.c":"maybe"},"domains":{}}"#,
            )
            .expect("set");
        });
        assert!(load_remote_image_senders().addresses.is_empty());
        host_kv::reset();
    }

    #[test]
    fn remote_image_address_cap_evicts_another_key() {
        let mut blob = RemoteImageSendersBlob::empty();
        for i in 0..MAX_REMOTE_IMAGE_ADDRESSES {
            blob.set_address(format!("u{i}@example.com"), RemoteImagePref::Allow);
        }
        assert_eq!(blob.addresses.len(), MAX_REMOTE_IMAGE_ADDRESSES);
        blob.set_address("new@example.com".into(), RemoteImagePref::Allow);
        assert_eq!(blob.addresses.len(), MAX_REMOTE_IMAGE_ADDRESSES);
        assert!(blob.addresses.contains_key("new@example.com"));
        blob.set_address("new@example.com".into(), RemoteImagePref::Block);
        assert_eq!(blob.addresses.len(), MAX_REMOTE_IMAGE_ADDRESSES);
        assert_eq!(
            blob.addresses.get("new@example.com").copied(),
            Some(RemoteImagePref::Block)
        );
    }

    #[test]
    fn invalid_email_is_not_saved() {
        host_kv::reset();
        assert!(!save_remote_image_address(
            "not-an-email",
            RemoteImagePref::Allow
        ));
        assert!(!save_remote_image_domain(
            "alice@example.com",
            RemoteImagePref::Allow
        ));
        assert!(load_remote_image_senders().addresses.is_empty());
        assert!(load_remote_image_senders().domains.is_empty());
        host_kv::reset();
    }

    #[test]
    fn notify_inbox_defaults_off_and_roundtrips() {
        host_kv::reset();
        assert!(!load_notify_inbox());
        save_notify_inbox(true);
        assert!(load_notify_inbox());
        save_notify_inbox(false);
        assert!(!load_notify_inbox());
        host_kv::reset();
    }

    #[test]
    fn shortcut_map_key_is_versioned() {
        assert_eq!(SHORTCUT_MAP_KEY, "mailiner.ui.shortcuts.v1");
        assert_eq!(SHORTCUT_MAP_SCHEMA_VERSION, 1);
    }

    #[test]
    fn shortcut_map_encode_decode_roundtrip() {
        let mut blob = ShortcutMapBlob::empty();
        blob.remaps.insert(
            "compose".into(),
            ShortcutBinding {
                key: "x".into(),
                shift: false,
            },
        );
        blob.remaps.insert(
            "copy_to_folder".into(),
            ShortcutBinding {
                key: "y".into(),
                shift: true,
            },
        );
        let json = blob.encode().expect("encode");
        assert!(json.contains("\"schema_version\":1"), "json={json}");
        assert!(json.contains("compose"), "json={json}");
        let back = ShortcutMapBlob::decode(&json).expect("decode");
        assert_eq!(back, blob);
    }

    #[test]
    fn shortcut_map_decode_rejects_future_schema() {
        let json = r#"{"schema_version":99,"remaps":{}}"#;
        let err = ShortcutMapBlob::decode(json).unwrap_err();
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
    fn shortcut_map_decode_defaults_missing_shift() {
        let json = r#"{"schema_version":1,"remaps":{"compose":{"key":"n"}}}"#;
        let blob = ShortcutMapBlob::decode(json).expect("decode");
        let binding = blob.remaps.get("compose").expect("compose");
        assert_eq!(binding.key, "n");
        assert!(!binding.shift);
    }

    #[test]
    fn shortcut_map_load_save_roundtrip() {
        host_kv::reset();
        assert!(load_shortcut_map().remaps.is_empty());
        let mut blob = ShortcutMapBlob::empty();
        blob.remaps.insert(
            "archive".into(),
            ShortcutBinding {
                key: "e".into(),
                shift: true,
            },
        );
        save_shortcut_map(&blob);
        assert_eq!(load_shortcut_map(), blob);

        host_kv::with(|kv| {
            kv.set_item(SHORTCUT_MAP_KEY, "not-json")
                .expect("set garbage");
        });
        assert!(load_shortcut_map().remaps.is_empty());

        host_kv::with(|kv| {
            kv.set_item(SHORTCUT_MAP_KEY, r#"{"schema_version":99,"remaps":{}}"#)
                .expect("set future");
        });
        assert!(load_shortcut_map().remaps.is_empty());
        host_kv::reset();
    }

    #[test]
    fn saved_searches_key_is_versioned() {
        assert_eq!(SAVED_SEARCHES_KEY, "mailiner.ui.savedSearches.v1");
        assert_eq!(SAVED_SEARCHES_SCHEMA_VERSION, 1);
    }

    #[test]
    fn saved_searches_encode_decode_roundtrip() {
        let mut blob = SavedSearchesBlob::empty();
        let acc = AccountId::new("acc-1");
        let inbox = MailboxId::from("INBOX".to_string());
        let saved = blob
            .add("Boss unread", "from:boss is:unread", acc, &inbox)
            .expect("add");
        assert!(!saved.id.is_empty());
        assert_eq!(saved.mailbox().as_str(), "INBOX");

        let json = blob.encode().expect("encode");
        assert!(json.contains("\"schema_version\":1"), "json={json}");
        assert!(json.contains("from:boss is:unread"), "json={json}");
        let back = SavedSearchesBlob::decode(&json).expect("decode");
        assert_eq!(back, blob);
        assert_eq!(
            back.get(&saved.id).map(|s| s.name.as_str()),
            Some("Boss unread")
        );
    }

    #[test]
    fn saved_searches_decode_rejects_future_schema() {
        let json = r#"{"schema_version":99,"searches":[]}"#;
        let err = SavedSearchesBlob::decode(json).unwrap_err();
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
    fn saved_search_add_rejects_empty_query() {
        let mut blob = SavedSearchesBlob::empty();
        let acc = AccountId::new("acc");
        let inbox = MailboxId::from("INBOX".to_string());
        assert_eq!(
            blob.add("x", "   ", acc.clone(), &inbox),
            Err(SaveSearchError::EmptyQuery)
        );
        assert_eq!(
            blob.add("x", "after:nope", acc, &inbox),
            Err(SaveSearchError::EmptyQuery)
        );
        assert!(blob.searches.is_empty());
    }

    #[test]
    fn saved_search_add_defaults_name_and_dedupes() {
        let mut blob = SavedSearchesBlob::empty();
        let acc = AccountId::new("acc");
        let inbox = MailboxId::from("INBOX".to_string());
        let first = blob.add("", "from:ada", acc.clone(), &inbox).expect("add");
        assert_eq!(first.name, "from:ada");
        let again = blob
            .add("Ada", "from:ada", acc.clone(), &inbox)
            .expect("dedupe");
        assert_eq!(again.id, first.id);
        assert_eq!(again.name, "Ada");
        assert_eq!(blob.searches.len(), 1);

        let other_folder = blob
            .add("Ada", "from:ada", acc, &MailboxId::from("Sent".to_string()))
            .expect("other folder");
        assert_ne!(other_folder.id, first.id);
        assert_eq!(blob.searches.len(), 2);
    }

    #[test]
    fn saved_search_rename_remove_and_retain() {
        let mut blob = SavedSearchesBlob::empty();
        let keep = AccountId::new("keep");
        let gone = AccountId::new("gone");
        let inbox = MailboxId::from("INBOX".to_string());
        let a = blob.add("A", "from:a", keep.clone(), &inbox).expect("a");
        let b = blob.add("B", "from:b", gone, &inbox).expect("b");
        assert_eq!(
            blob.rename(&a.id, "  Ada  ").map(|s| s.name),
            Some("Ada".into())
        );
        assert_eq!(
            blob.rename(&a.id, "  ").map(|s| s.name),
            Some("from:a".into())
        );
        assert!(blob.remove(&b.id));
        assert!(!blob.remove("missing"));
        blob.retain_accounts(&HashSet::from([keep.clone()]));
        assert_eq!(blob.for_account(&keep).len(), 1);
        assert!(blob.get(&b.id).is_none());
    }

    #[test]
    fn saved_search_cap_evicts_oldest() {
        let mut blob = SavedSearchesBlob::empty();
        let acc = AccountId::new("acc");
        let inbox = MailboxId::from("INBOX".to_string());
        let first = blob
            .add("first", "from:first", acc.clone(), &inbox)
            .expect("first");
        for i in 0..MAX_SAVED_SEARCHES - 1 {
            blob.add(&format!("n{i}"), &format!("from:u{i}"), acc.clone(), &inbox)
                .expect("fill");
        }
        assert_eq!(blob.searches.len(), MAX_SAVED_SEARCHES);
        blob.add("new", "from:new", acc, &inbox).expect("overflow");
        assert_eq!(blob.searches.len(), MAX_SAVED_SEARCHES);
        assert!(blob.get(&first.id).is_none());
        assert!(blob.searches.iter().any(|s| s.query == "from:new"));
    }

    #[test]
    fn saved_search_matches_filter() {
        let search = SavedSearch {
            id: "1".into(),
            name: "Boss unread".into(),
            query: "from:boss is:unread".into(),
            account_id: AccountId::new("acc"),
            mailbox_id: "INBOX".into(),
        };
        assert!(search.matches_filter(""));
        assert!(search.matches_filter("  boss  "));
        assert!(search.matches_filter("unread boss"));
        assert!(search.matches_filter("from:boss"));
        assert!(!search.matches_filter("invoice"));
    }

    #[test]
    fn saved_searches_load_save_roundtrip() {
        host_kv::reset();
        assert!(load_saved_searches().is_empty());
        let acc = AccountId::new("host-acc");
        let inbox = MailboxId::from("INBOX".to_string());
        let saved =
            add_saved_search("Work", "from:work is:unread", acc.clone(), &inbox).expect("save");
        assert_eq!(load_saved_searches(), vec![saved.clone()]);
        assert_eq!(
            rename_saved_search(&saved.id, "Office").map(|s| s.name),
            Some("Office".into())
        );
        assert!(remove_saved_search(&saved.id));
        assert!(load_saved_searches().is_empty());

        add_saved_search("Gone", "from:x", acc.clone(), &inbox).expect("save");
        retain_saved_searches(&HashSet::new());
        assert!(load_saved_searches().is_empty());

        host_kv::with(|kv| {
            kv.set_item(SAVED_SEARCHES_KEY, "not-json")
                .expect("set garbage");
        });
        assert!(load_saved_searches().is_empty());
        host_kv::reset();
    }

    #[test]
    fn pinned_messages_roundtrip_and_toggle() {
        host_kv::reset();
        let acc = AccountId::new("acc-pin");
        let inbox = MailboxId::from("INBOX".to_string());
        assert!(load_pinned_uids(&acc, &inbox).is_empty());

        let next = toggle_pinned_uids(&acc, &inbox, &["10".into(), "11".into()], true);
        assert_eq!(next, vec!["10", "11"]);
        assert_eq!(load_pinned_uids(&acc, &inbox), vec!["10", "11"]);

        let next = toggle_pinned_uids(&acc, &inbox, &["12".into()], true);
        assert_eq!(next, vec!["12", "10", "11"]);

        let next = toggle_pinned_uids(&acc, &inbox, &["10".into()], false);
        assert_eq!(next, vec!["12", "11"]);

        let sent = MailboxId::from("Sent".to_string());
        toggle_pinned_uids(&acc, &sent, &["99".into()], true);
        assert_eq!(load_pinned_uids(&acc, &inbox), vec!["12", "11"]);
        assert_eq!(load_pinned_uids(&acc, &sent), vec!["99"]);

        retain_pinned_messages(&HashSet::new());
        assert!(load_pinned_uids(&acc, &inbox).is_empty());
        host_kv::reset();
    }

    #[test]
    fn pinned_messages_cap_and_empty_uid() {
        let mut blob = PinnedMessagesBlob::empty();
        let acc = AccountId::new("acc");
        let inbox = MailboxId::from("INBOX".to_string());
        let uids: Vec<String> = (0..MAX_PINNED_PER_MAILBOX + 5)
            .map(|n| n.to_string())
            .collect();
        blob.set_uids(acc.clone(), &inbox, uids);
        assert_eq!(blob.uids(&acc, &inbox).len(), MAX_PINNED_PER_MAILBOX);
        blob.set_uids(acc.clone(), &inbox, vec![String::new(), "7".into()]);
        assert_eq!(blob.uids(&acc, &inbox), vec!["7"]);
        blob.set_uids(acc.clone(), &inbox, Vec::new());
        assert!(blob.uids(&acc, &inbox).is_empty());
        assert!(blob.pinned.is_empty());
    }

    #[test]
    fn pinned_messages_decode_rejects_future_schema() {
        let err = PinnedMessagesBlob::decode(r#"{"schema_version":99,"pinned":{}}"#).unwrap_err();
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
}
