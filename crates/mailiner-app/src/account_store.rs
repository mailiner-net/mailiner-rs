//! Account configuration store (secrets + connection settings).
//!
//! Browser persistence uses a single `localStorage` JSON blob
//! ([`ACCOUNTS_LOCAL_STORAGE_KEY`]); IndexedDB is deferred.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;

use async_trait::async_trait;
use mailiner_core::ids::AccountId;
use serde::{Deserialize, Serialize};

use crate::account_config::{ACCOUNT_STORE_SCHEMA_VERSION, AccountConfig};

/// `localStorage` key for the v1 account-configs blob.
pub const ACCOUNTS_LOCAL_STORAGE_KEY: &str = "mailiner.accounts.v1";

/// Errors from account config persistence backends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountStoreError {
    /// Storage is blocked or unavailable (e.g. private mode / SecurityError).
    Unavailable,
    /// JSON or schema serialization failure.
    Serialization(String),
    /// Other backend-specific failure.
    Other(String),
}

impl fmt::Display for AccountStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => write!(f, "account storage is unavailable"),
            Self::Serialization(msg) => write!(f, "account store serialization error: {msg}"),
            Self::Other(msg) => write!(f, "account store error: {msg}"),
        }
    }
}

impl std::error::Error for AccountStoreError {}

/// Persistence for account configs and the active-account pointer.
///
/// `?Send` because the browser/WASM target is single-threaded.
#[async_trait(?Send)]
pub trait AccountStore {
    /// List all stored account configs.
    ///
    /// Order is implementation-defined for external implementors. Both in-tree
    /// stores (`InMemoryAccountStore` and `BrowserAccountStore`) return a stable
    /// order by `display_name` then `id`. Callers that need a specific order
    /// against an unknown backend should sort themselves.
    async fn list(&self) -> Result<Vec<AccountConfig>, AccountStoreError>;
    async fn get(&self, id: &AccountId) -> Result<Option<AccountConfig>, AccountStoreError>;
    async fn upsert(&self, config: &AccountConfig) -> Result<(), AccountStoreError>;
    async fn delete(&self, id: &AccountId) -> Result<(), AccountStoreError>;
    async fn get_active_id(&self) -> Result<Option<AccountId>, AccountStoreError>;
    async fn set_active_id(&self, id: Option<&AccountId>) -> Result<(), AccountStoreError>;
}

/// In-memory store for unit tests and session-only fallback.
///
/// `Debug` is derived; nested [`AccountConfig`] redacts passwords/tokens.
#[derive(Debug, Default)]
pub struct InMemoryAccountStore {
    accounts: RefCell<HashMap<AccountId, AccountConfig>>,
    active_id: RefCell<Option<AccountId>>,
}

impl InMemoryAccountStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait(?Send)]
impl AccountStore for InMemoryAccountStore {
    async fn list(&self) -> Result<Vec<AccountConfig>, AccountStoreError> {
        let mut configs: Vec<AccountConfig> = self.accounts.borrow().values().cloned().collect();
        // Stable order for UI / tests: display_name, then id.
        configs.sort_by(|a, b| {
            a.display_name
                .cmp(&b.display_name)
                .then_with(|| a.id.as_str().cmp(b.id.as_str()))
        });
        Ok(configs)
    }

    async fn get(&self, id: &AccountId) -> Result<Option<AccountConfig>, AccountStoreError> {
        Ok(self.accounts.borrow().get(id).cloned())
    }

    async fn upsert(&self, config: &AccountConfig) -> Result<(), AccountStoreError> {
        self.accounts
            .borrow_mut()
            .insert(config.id.clone(), config.clone());
        Ok(())
    }

    async fn delete(&self, id: &AccountId) -> Result<(), AccountStoreError> {
        self.accounts.borrow_mut().remove(id);
        let mut active = self.active_id.borrow_mut();
        if active.as_ref() == Some(id) {
            *active = None;
        }
        Ok(())
    }

    async fn get_active_id(&self) -> Result<Option<AccountId>, AccountStoreError> {
        Ok(self.active_id.borrow().clone())
    }

    async fn set_active_id(&self, id: Option<&AccountId>) -> Result<(), AccountStoreError> {
        *self.active_id.borrow_mut() = id.cloned();
        Ok(())
    }
}

// ── Persisted blob schema (localStorage JSON v1) ────────────────────────────

/// Single JSON document stored under [`ACCOUNTS_LOCAL_STORAGE_KEY`].
///
/// Pure encode/decode helpers are unit-tested on the host without a browser.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountsStoreBlob {
    pub schema_version: u32,
    pub active_account_id: Option<AccountId>,
    pub accounts: Vec<AccountConfig>,
}

impl AccountsStoreBlob {
    /// Empty blob at the current schema version.
    pub fn empty() -> Self {
        Self {
            schema_version: ACCOUNT_STORE_SCHEMA_VERSION,
            active_account_id: None,
            accounts: Vec::new(),
        }
    }

    /// Serialize to a JSON string for `localStorage`.
    pub fn encode(&self) -> Result<String, AccountStoreError> {
        serde_json::to_string(self).map_err(|e| AccountStoreError::Serialization(e.to_string()))
    }

    /// Deserialize from a JSON string.
    ///
    /// Rejects blobs whose `schema_version` is **greater** than
    /// [`ACCOUNT_STORE_SCHEMA_VERSION`] so a future format is not silently
    /// loaded and then rewritten as v1 (data loss). Older or equal versions
    /// are accepted; upgrades happen by stamping the current version on write.
    pub fn decode(json: &str) -> Result<Self, AccountStoreError> {
        let blob: Self = serde_json::from_str(json)
            .map_err(|e| AccountStoreError::Serialization(e.to_string()))?;
        if blob.schema_version > ACCOUNT_STORE_SCHEMA_VERSION {
            return Err(AccountStoreError::Serialization(format!(
                "unsupported account store schema_version {} (max supported {})",
                blob.schema_version, ACCOUNT_STORE_SCHEMA_VERSION
            )));
        }
        Ok(blob)
    }

    fn sort_accounts(accounts: &mut [AccountConfig]) {
        accounts.sort_by(|a, b| {
            a.display_name
                .cmp(&b.display_name)
                .then_with(|| a.id.as_str().cmp(b.id.as_str()))
        });
    }
}

// ── String key-value backend (localStorage abstraction) ─────────────────────

/// Minimal string key-value API matching browser `Storage` (`getItem` / `setItem`).
///
/// Implemented for `web_sys::Storage` and an in-process map for host unit tests.
pub trait StringKvStore {
    fn get_item(&self, key: &str) -> Result<Option<String>, AccountStoreError>;
    fn set_item(&self, key: &str, value: &str) -> Result<(), AccountStoreError>;
}

/// In-process `StringKvStore` for unit tests (same blob format as the browser).
#[derive(Debug, Default)]
pub struct MemoryKvStore {
    map: RefCell<HashMap<String, String>>,
}

impl MemoryKvStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl StringKvStore for MemoryKvStore {
    fn get_item(&self, key: &str) -> Result<Option<String>, AccountStoreError> {
        Ok(self.map.borrow().get(key).cloned())
    }

    fn set_item(&self, key: &str, value: &str) -> Result<(), AccountStoreError> {
        self.map
            .borrow_mut()
            .insert(key.to_string(), value.to_string());
        Ok(())
    }
}

/// Sentinel key used only to probe write access during [`WebLocalStorage::try_open`].
#[cfg(target_arch = "wasm32")]
const LOCAL_STORAGE_PROBE_KEY: &str = "mailiner.accounts.__probe";

/// Browser `window.localStorage` backend.
///
/// Construction probes read **and** write access so private-mode / SecurityError /
/// quota-on-write surfaces as [`AccountStoreError::Unavailable`] at open time
/// rather than on the first account save.
pub struct WebLocalStorage {
    storage: web_sys::Storage,
}

impl WebLocalStorage {
    /// Open `window.localStorage`, or [`AccountStoreError::Unavailable`].
    ///
    /// On host (non-WASM) targets this always returns
    /// [`AccountStoreError::Unavailable`] without touching `web_sys` (which
    /// panics on imported statics outside wasm).
    pub fn try_open() -> Result<Self, AccountStoreError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Err(AccountStoreError::Unavailable)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let window = web_sys::window().ok_or(AccountStoreError::Unavailable)?;
            let storage = window
                .local_storage()
                .map_err(|_| AccountStoreError::Unavailable)?
                .ok_or(AccountStoreError::Unavailable)?;
            // Probe: some environments throw only on first property access.
            let _ = storage
                .length()
                .map_err(|_| AccountStoreError::Unavailable)?;
            // Reversible write probe: private/strict modes may allow `length()`
            // but throw SecurityError / QuotaExceededError only on `setItem`.
            storage
                .set_item(LOCAL_STORAGE_PROBE_KEY, "1")
                .map_err(|_| AccountStoreError::Unavailable)?;
            let _ = storage.remove_item(LOCAL_STORAGE_PROBE_KEY);
            Ok(Self { storage })
        }
    }
}

impl StringKvStore for WebLocalStorage {
    fn get_item(&self, key: &str) -> Result<Option<String>, AccountStoreError> {
        // Any failure means we cannot read persisted accounts → Unavailable
        // (bootstrap can fall back to session-only memory).
        self.storage
            .get_item(key)
            .map_err(|_| AccountStoreError::Unavailable)
    }

    fn set_item(&self, key: &str, value: &str) -> Result<(), AccountStoreError> {
        // Account configs are tiny; any `setItem` failure (SecurityError,
        // QuotaExceededError, private-mode, etc.) means we cannot persist.
        // Map all failures to Unavailable so UI can offer session-only fallback.
        // Note: `JsValue::as_string()` is None for DOMException objects, so
        // classifying by message string is unreliable — do not use it here.
        self.storage
            .set_item(key, value)
            .map_err(|_| AccountStoreError::Unavailable)
    }
}

impl fmt::Debug for WebLocalStorage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebLocalStorage").finish_non_exhaustive()
    }
}

// ── BrowserAccountStore (blob over StringKvStore) ───────────────────────────

/// Account store backed by a single JSON blob in string key-value storage.
///
/// Production path: [`BrowserAccountStore::open`] → browser `localStorage`.
/// Tests: [`BrowserAccountStore::with_kv`] + [`MemoryKvStore`].
pub struct BrowserAccountStore<K: StringKvStore = WebLocalStorage> {
    kv: K,
}

impl BrowserAccountStore<WebLocalStorage> {
    /// Open the browser `localStorage` backend.
    ///
    /// Returns [`AccountStoreError::Unavailable`] when there is no window or
    /// storage is blocked. Async for symmetry with a future IndexedDB backend.
    pub async fn open() -> Result<Self, AccountStoreError> {
        Ok(Self {
            kv: WebLocalStorage::try_open()?,
        })
    }
}

impl BrowserAccountStore<MemoryKvStore> {
    /// Host-test / session helper using an in-memory string map.
    pub fn open_memory() -> Self {
        Self {
            kv: MemoryKvStore::new(),
        }
    }
}

impl<K: StringKvStore> BrowserAccountStore<K> {
    /// Wrap an arbitrary [`StringKvStore`] (tests, alternate backends).
    pub fn with_kv(kv: K) -> Self {
        Self { kv }
    }

    fn load_blob(&self) -> Result<AccountsStoreBlob, AccountStoreError> {
        match self.kv.get_item(ACCOUNTS_LOCAL_STORAGE_KEY)? {
            None => Ok(AccountsStoreBlob::empty()),
            Some(s) if s.trim().is_empty() => Ok(AccountsStoreBlob::empty()),
            Some(s) => AccountsStoreBlob::decode(&s),
        }
    }

    fn save_blob(&self, blob: &AccountsStoreBlob) -> Result<(), AccountStoreError> {
        let json = blob.encode()?;
        self.kv.set_item(ACCOUNTS_LOCAL_STORAGE_KEY, &json)
    }
}

impl<K: StringKvStore> fmt::Debug for BrowserAccountStore<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Do not load the blob (would surface secrets in Debug).
        f.debug_struct("BrowserAccountStore")
            .finish_non_exhaustive()
    }
}

#[async_trait(?Send)]
impl<K: StringKvStore> AccountStore for BrowserAccountStore<K> {
    async fn list(&self) -> Result<Vec<AccountConfig>, AccountStoreError> {
        let mut blob = self.load_blob()?;
        AccountsStoreBlob::sort_accounts(&mut blob.accounts);
        Ok(blob.accounts)
    }

    async fn get(&self, id: &AccountId) -> Result<Option<AccountConfig>, AccountStoreError> {
        let blob = self.load_blob()?;
        Ok(blob.accounts.into_iter().find(|a| &a.id == id))
    }

    async fn upsert(&self, config: &AccountConfig) -> Result<(), AccountStoreError> {
        let mut blob = self.load_blob()?;
        if let Some(slot) = blob.accounts.iter_mut().find(|a| a.id == config.id) {
            *slot = config.clone();
        } else {
            blob.accounts.push(config.clone());
        }
        blob.schema_version = ACCOUNT_STORE_SCHEMA_VERSION;
        self.save_blob(&blob)
    }

    async fn delete(&self, id: &AccountId) -> Result<(), AccountStoreError> {
        let mut blob = self.load_blob()?;
        blob.accounts.retain(|a| &a.id != id);
        if blob.active_account_id.as_ref() == Some(id) {
            blob.active_account_id = None;
        }
        blob.schema_version = ACCOUNT_STORE_SCHEMA_VERSION;
        self.save_blob(&blob)
    }

    async fn get_active_id(&self) -> Result<Option<AccountId>, AccountStoreError> {
        Ok(self.load_blob()?.active_account_id)
    }

    async fn set_active_id(&self, id: Option<&AccountId>) -> Result<(), AccountStoreError> {
        let mut blob = self.load_blob()?;
        blob.active_account_id = id.cloned();
        blob.schema_version = ACCOUNT_STORE_SCHEMA_VERSION;
        self.save_blob(&blob)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    use crate::account_config::{ImapSettings, ProxySettings};

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    fn sample_config(id: &str, name: &str) -> AccountConfig {
        let ts = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        AccountConfig {
            id: AccountId::new(id),
            display_name: name.into(),
            email: format!("{name}@example.com"),
            signature: None,
            imap: ImapSettings::new(
                "imap.example.com".into(),
                993,
                format!("{name}@example.com"),
                "pw".into(),
                crate::account_config::ImapTlsMode::Implicit,
            ),
            smtp: None,
            proxy: ProxySettings {
                base_url: "ws://localhost:9400/proxy".into(),
                token: "t".into(),
                remote_host: None,
                remote_port: None,
            },
            created_at: ts,
            updated_at: ts,
        }
    }

    // ── InMemoryAccountStore ────────────────────────────────────────────────

    #[test]
    fn upsert_list_get_delete_roundtrip() {
        let store = InMemoryAccountStore::new();
        let a = sample_config("a1", "alice");
        let b = sample_config("b2", "bob");

        block_on(async {
            store.upsert(&a).await.unwrap();
            store.upsert(&b).await.unwrap();

            // InMemoryAccountStore returns stable order by display_name then id.
            let list = store.list().await.unwrap();
            assert_eq!(list.len(), 2);
            assert_eq!(list[0].id.as_str(), "a1");
            assert_eq!(list[0].display_name, "alice");
            assert_eq!(list[1].display_name, "bob");

            let got = store.get(&AccountId::new("a1")).await.unwrap().unwrap();
            assert_eq!(got.imap.password, "pw");

            store.delete(&AccountId::new("a1")).await.unwrap();
            assert!(store.get(&AccountId::new("a1")).await.unwrap().is_none());
            assert_eq!(store.list().await.unwrap().len(), 1);
        });
    }

    #[test]
    fn list_stable_order_by_display_name() {
        let store = InMemoryAccountStore::new();
        block_on(async {
            store.upsert(&sample_config("z", "zeta")).await.unwrap();
            store.upsert(&sample_config("a", "alpha")).await.unwrap();
            store.upsert(&sample_config("m", "mu")).await.unwrap();
            let names: Vec<_> = store
                .list()
                .await
                .unwrap()
                .into_iter()
                .map(|c| c.display_name)
                .collect();
            assert_eq!(names, vec!["alpha", "mu", "zeta"]);
        });
    }

    #[test]
    fn active_id_set_get_and_cleared_on_delete() {
        let store = InMemoryAccountStore::new();
        let a = sample_config("a1", "alice");

        block_on(async {
            assert!(store.get_active_id().await.unwrap().is_none());

            store.upsert(&a).await.unwrap();
            store
                .set_active_id(Some(&AccountId::new("a1")))
                .await
                .unwrap();
            assert_eq!(store.get_active_id().await.unwrap().unwrap().as_str(), "a1");

            store.delete(&AccountId::new("a1")).await.unwrap();
            assert!(store.get_active_id().await.unwrap().is_none());

            store
                .set_active_id(Some(&AccountId::new("ghost")))
                .await
                .unwrap();
            store.set_active_id(None).await.unwrap();
            assert!(store.get_active_id().await.unwrap().is_none());
        });
    }

    #[test]
    fn upsert_overwrites_existing() {
        let store = InMemoryAccountStore::new();
        let mut a = sample_config("a1", "alice");

        block_on(async {
            store.upsert(&a).await.unwrap();
            a.display_name = "Alice Updated".into();
            a.imap.password = "new-pw".into();
            store.upsert(&a).await.unwrap();

            let got = store.get(&AccountId::new("a1")).await.unwrap().unwrap();
            assert_eq!(got.display_name, "Alice Updated");
            assert_eq!(got.imap.password, "new-pw");
            assert_eq!(store.list().await.unwrap().len(), 1);
        });
    }

    #[test]
    fn debug_redacts_secrets_in_store() {
        let store = InMemoryAccountStore::new();
        block_on(async {
            store.upsert(&sample_config("a1", "alice")).await.unwrap();
        });
        let dbg = format!("{store:?}");
        assert!(
            !dbg.contains("\"pw\""),
            "password leaked via store Debug: {dbg}"
        );
        assert!(
            !dbg.contains("token: \"t\""),
            "token leaked via store Debug: {dbg}"
        );
    }

    // ── AccountsStoreBlob pure helpers ──────────────────────────────────────

    #[test]
    fn blob_encode_decode_roundtrip_preserves_password_and_schema() {
        let mut blob = AccountsStoreBlob::empty();
        assert_eq!(blob.schema_version, ACCOUNT_STORE_SCHEMA_VERSION);
        assert!(blob.active_account_id.is_none());
        assert!(blob.accounts.is_empty());

        let a = sample_config("550e8400-e29b-41d4-a716-446655440000", "alice");
        blob.accounts.push(a.clone());
        blob.active_account_id = Some(a.id.clone());

        let json = blob.encode().expect("encode");
        // Schema meta present in JSON
        assert!(
            json.contains("\"schema_version\":1"),
            "schema_version missing: {json}"
        );
        assert!(
            json.contains("\"active_account_id\":\"550e8400-e29b-41d4-a716-446655440000\""),
            "active id missing: {json}"
        );
        // Password survives serialization (never log in production)
        assert!(
            json.contains("\"password\":\"pw\""),
            "password lost: {json}"
        );

        let back = AccountsStoreBlob::decode(&json).expect("decode");
        assert_eq!(back, blob);
        assert_eq!(back.accounts[0].imap.password, "pw");
        assert_eq!(back.accounts[0].proxy.token, "t");
        assert_eq!(back.schema_version, 1);
    }

    #[test]
    fn blob_decode_rejects_invalid_json() {
        let err = AccountsStoreBlob::decode("not-json").unwrap_err();
        match err {
            AccountStoreError::Serialization(_) => {}
            other => panic!("expected Serialization, got {other:?}"),
        }
    }

    #[test]
    fn blob_empty_json_object_is_not_valid_v1_blob() {
        // Missing required fields → serialization error (no silent empty).
        let err = AccountsStoreBlob::decode("{}").unwrap_err();
        assert!(matches!(err, AccountStoreError::Serialization(_)));
    }

    #[test]
    fn blob_decode_rejects_future_schema_version() {
        let json = r#"{
            "schema_version": 99,
            "active_account_id": null,
            "accounts": []
        }"#;
        let err = AccountsStoreBlob::decode(json).unwrap_err();
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

    // ── BrowserAccountStore via MemoryKvStore ───────────────────────────────

    #[test]
    fn browser_store_upsert_list_get_delete_roundtrip() {
        let store = BrowserAccountStore::open_memory();
        let a = sample_config("a1", "alice");
        let b = sample_config("b2", "bob");

        block_on(async {
            store.upsert(&a).await.unwrap();
            store.upsert(&b).await.unwrap();

            let list = store.list().await.unwrap();
            assert_eq!(list.len(), 2);
            assert_eq!(list[0].display_name, "alice");
            assert_eq!(list[1].display_name, "bob");

            let got = store.get(&AccountId::new("a1")).await.unwrap().unwrap();
            assert_eq!(got.imap.password, "pw");

            store.delete(&AccountId::new("a1")).await.unwrap();
            assert!(store.get(&AccountId::new("a1")).await.unwrap().is_none());
            assert_eq!(store.list().await.unwrap().len(), 1);
        });
    }

    #[test]
    fn browser_store_active_id_and_schema_version_after_write() {
        let store = BrowserAccountStore::open_memory();
        let a = sample_config("a1", "alice");

        block_on(async {
            assert!(store.get_active_id().await.unwrap().is_none());

            store.upsert(&a).await.unwrap();
            store
                .set_active_id(Some(&AccountId::new("a1")))
                .await
                .unwrap();
            assert_eq!(store.get_active_id().await.unwrap().unwrap().as_str(), "a1");

            // Raw blob under the canonical key has schema_version + active id.
            let raw = store
                .kv
                .get_item(ACCOUNTS_LOCAL_STORAGE_KEY)
                .unwrap()
                .expect("blob written");
            let blob = AccountsStoreBlob::decode(&raw).unwrap();
            assert_eq!(blob.schema_version, ACCOUNT_STORE_SCHEMA_VERSION);
            assert_eq!(
                blob.active_account_id.as_ref().map(|i| i.as_str()),
                Some("a1")
            );
            assert_eq!(blob.accounts.len(), 1);
            assert_eq!(blob.accounts[0].imap.password, "pw");

            store.delete(&AccountId::new("a1")).await.unwrap();
            assert!(store.get_active_id().await.unwrap().is_none());
        });
    }

    #[test]
    fn browser_store_upsert_overwrites_and_password_roundtrip() {
        let store = BrowserAccountStore::open_memory();
        let mut a = sample_config("a1", "alice");

        block_on(async {
            store.upsert(&a).await.unwrap();
            a.display_name = "Alice Updated".into();
            a.imap.password = "new-secret-password".into();
            store.upsert(&a).await.unwrap();

            let got = store.get(&AccountId::new("a1")).await.unwrap().unwrap();
            assert_eq!(got.display_name, "Alice Updated");
            assert_eq!(got.imap.password, "new-secret-password");
            assert_eq!(store.list().await.unwrap().len(), 1);
        });
    }

    #[test]
    fn browser_store_list_stable_order() {
        let store = BrowserAccountStore::open_memory();
        block_on(async {
            store.upsert(&sample_config("z", "zeta")).await.unwrap();
            store.upsert(&sample_config("a", "alpha")).await.unwrap();
            store.upsert(&sample_config("m", "mu")).await.unwrap();
            let names: Vec<_> = store
                .list()
                .await
                .unwrap()
                .into_iter()
                .map(|c| c.display_name)
                .collect();
            assert_eq!(names, vec!["alpha", "mu", "zeta"]);
        });
    }

    #[test]
    fn browser_store_empty_key_yields_empty_list() {
        let store = BrowserAccountStore::open_memory();
        block_on(async {
            assert!(store.list().await.unwrap().is_empty());
            assert!(store.get_active_id().await.unwrap().is_none());
            assert!(
                store
                    .get(&AccountId::new("missing"))
                    .await
                    .unwrap()
                    .is_none()
            );
        });
    }

    #[test]
    fn browser_store_debug_does_not_load_secrets() {
        let store = BrowserAccountStore::open_memory();
        block_on(async {
            store.upsert(&sample_config("a1", "alice")).await.unwrap();
        });
        let dbg = format!("{store:?}");
        assert!(
            !dbg.contains("pw"),
            "password leaked via BrowserAccountStore Debug: {dbg}"
        );
        assert!(
            dbg.contains("BrowserAccountStore"),
            "unexpected Debug: {dbg}"
        );
    }

    #[test]
    fn storage_key_is_versioned() {
        assert_eq!(ACCOUNTS_LOCAL_STORAGE_KEY, "mailiner.accounts.v1");
    }

    /// KV backend that always fails with [`AccountStoreError::Unavailable`].
    struct UnavailableKv;

    impl StringKvStore for UnavailableKv {
        fn get_item(&self, _key: &str) -> Result<Option<String>, AccountStoreError> {
            Err(AccountStoreError::Unavailable)
        }

        fn set_item(&self, _key: &str, _value: &str) -> Result<(), AccountStoreError> {
            Err(AccountStoreError::Unavailable)
        }
    }

    #[test]
    fn browser_store_propagates_unavailable_from_kv() {
        let store = BrowserAccountStore::with_kv(UnavailableKv);
        let a = sample_config("a1", "alice");

        block_on(async {
            assert_eq!(
                store.list().await.unwrap_err(),
                AccountStoreError::Unavailable
            );
            assert_eq!(
                store.get(&AccountId::new("a1")).await.unwrap_err(),
                AccountStoreError::Unavailable
            );
            assert_eq!(
                store.upsert(&a).await.unwrap_err(),
                AccountStoreError::Unavailable
            );
            assert_eq!(
                store.delete(&AccountId::new("a1")).await.unwrap_err(),
                AccountStoreError::Unavailable
            );
            assert_eq!(
                store.get_active_id().await.unwrap_err(),
                AccountStoreError::Unavailable
            );
            assert_eq!(
                store
                    .set_active_id(Some(&AccountId::new("a1")))
                    .await
                    .unwrap_err(),
                AccountStoreError::Unavailable
            );
        });
    }

    #[test]
    fn browser_store_open_on_host_is_unavailable() {
        // Host unit tests have no DOM `window`; open must not panic.
        block_on(async {
            let err = BrowserAccountStore::open().await.unwrap_err();
            assert_eq!(err, AccountStoreError::Unavailable);
        });
    }
}
