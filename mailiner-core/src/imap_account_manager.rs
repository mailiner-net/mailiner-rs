use dioxus::prelude::*;
use dioxus_logger::tracing::error;
use gloo_storage::{errors::StorageError, LocalStorage, Storage};
use serde::{de::Deserializer, Deserialize, Serialize, Serializer};
use std::collections::HashMap;
use uuid::Uuid;

use super::security::{Authentication, Security};

fn serialize_uuid<S>(uuid: &Uuid, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&uuid.to_string())
}

fn deserialize_uuid<'de, D>(deserializer: D) -> Result<Uuid, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Uuid::try_parse(&s).map_err(serde::de::Error::custom)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImapAccount {
    #[serde(
        serialize_with = "serialize_uuid",
        deserialize_with = "deserialize_uuid"
    )]
    pub id: Uuid,
    pub name: String,

    pub hostname: String,
    pub port: u16,
    pub authentication: Authentication,
    pub security: Security,
}

pub struct ImapAccountManager {
    accounts: HashMap<Uuid, Signal<ImapAccount>>,
}

impl ImapAccountManager {
    pub fn new() -> Result<Self, StorageError> {
        Ok(Self {
            accounts: load_accounts().map_err(|err| {
                error!("Failed to load accounts from Local Storage: {:?}", err);
                err
            })?,
        })
    }

    pub fn accounts(&self) -> Vec<&Signal<ImapAccount>> {
        self.accounts.values().collect()
    }

    pub fn add_account(&mut self, account: ImapAccount) {
        self.accounts.insert(account.id, Signal::new(account));
        self.save();
    }

    pub fn update_account(&mut self, account: ImapAccount) {
        if let Some(signal) = self.accounts.get_mut(&account.id) {
            signal.set(account);
        }
    }

    pub fn get_account(&self, id: Uuid) -> Option<Signal<ImapAccount>> {
        self.accounts.get(&id).cloned()
    }

    pub fn save(&self) {
        if let Err(err) = save_accounts(&self.accounts) {
            error!("Failed to save accounts to Local Storage: {:?}", err);
        }
    }
}

fn load_accounts() -> Result<HashMap<Uuid, Signal<ImapAccount>>, StorageError> {
    let accounts = match LocalStorage::get::<Vec<ImapAccount>>("accounts") {
        Ok(accounts) => accounts,
        Err(StorageError::KeyNotFound(_)) => return Ok(HashMap::new()),
        Err(err) => return Err(err),
    };

    Ok(HashMap::from_iter(
        accounts
            .iter()
            .map(|account| (account.id, Signal::new(account.clone()))),
    ))
}

fn save_accounts(accounts: &HashMap<Uuid, Signal<ImapAccount>>) -> Result<(), StorageError> {
    let accounts = accounts
        .values()
        .map(|signal| signal.read().clone())
        .collect::<Vec<_>>();

    LocalStorage::set("accounts", accounts)
}

pub fn use_imap_account_manager() -> Signal<ImapAccountManager> {
    use_context::<Signal<ImapAccountManager>>()
}
