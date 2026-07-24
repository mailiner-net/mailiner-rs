//! Account configuration store (secrets + connection settings).
//!
//! Separate from `mailiner_core::Storage` (mail cache, no secrets).

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;

use async_trait::async_trait;
use mailiner_core::ids::AccountId;

use crate::account_config::AccountConfig;

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
    async fn list(&self) -> Result<Vec<AccountConfig>, AccountStoreError>;
    async fn get(&self, id: &AccountId) -> Result<Option<AccountConfig>, AccountStoreError>;
    async fn upsert(&self, config: &AccountConfig) -> Result<(), AccountStoreError>;
    async fn delete(&self, id: &AccountId) -> Result<(), AccountStoreError>;
    async fn get_active_id(&self) -> Result<Option<AccountId>, AccountStoreError>;
    async fn set_active_id(&self, id: Option<&AccountId>) -> Result<(), AccountStoreError>;
}

/// In-memory store for unit tests and session-only fallback.
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
        Ok(self.accounts.borrow().values().cloned().collect())
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
            imap: ImapSettings {
                host: "imap.example.com".into(),
                port: 993,
                username: format!("{name}@example.com"),
                password: "pw".into(),
                use_tls: true,
            },
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

    #[test]
    fn upsert_list_get_delete_roundtrip() {
        let store = InMemoryAccountStore::new();
        let a = sample_config("a1", "alice");
        let b = sample_config("b2", "bob");

        block_on(async {
            store.upsert(&a).await.unwrap();
            store.upsert(&b).await.unwrap();

            let mut list = store.list().await.unwrap();
            list.sort_by(|x, y| x.id.as_str().cmp(y.id.as_str()));
            assert_eq!(list.len(), 2);
            assert_eq!(list[0].id.as_str(), "a1");
            assert_eq!(list[1].display_name, "bob");

            let got = store.get(&AccountId::new("a1")).await.unwrap().unwrap();
            assert_eq!(got.imap.password, "pw");

            store.delete(&AccountId::new("a1")).await.unwrap();
            assert!(store.get(&AccountId::new("a1")).await.unwrap().is_none());
            assert_eq!(store.list().await.unwrap().len(), 1);
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
}
