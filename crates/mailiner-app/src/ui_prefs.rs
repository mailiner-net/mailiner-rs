//! UI preferences that are not account secrets (`mailiner.ui.lastMailbox.v1`).
//!
//! Separate from [`crate::account_store`] so a folder click does not rewrite
//! passwords or bump account `updated_at`.

use std::collections::{HashMap, HashSet};

use mailiner_core::ids::AccountId;
use serde::{Deserialize, Serialize};

#[cfg(target_arch = "wasm32")]
use crate::account_store::WebLocalStorage;
use crate::account_store::{AccountStoreError, StringKvStore};
use crate::mailbox::MailboxId;
use mailiner_core::MessageSort;

/// `localStorage` key for the message-list sort.
pub const MESSAGE_SORT_KEY: &str = "mailiner.ui.messageSort";

/// `localStorage` key for last-opened mailbox per account.
pub const LAST_MAILBOX_KEY: &str = "mailiner.ui.lastMailbox.v1";
/// Schema version for [`LastMailboxBlob`] (independent of the account store).
pub const LAST_MAILBOX_SCHEMA_VERSION: u32 = 1;

/// `localStorage` key for last-acknowledged unread counts (opened-folder watermark).
pub const ACK_UNREAD_KEY: &str = "mailiner.ui.ackUnread.v1";
/// Schema version for [`AckUnreadBlob`].
pub const ACK_UNREAD_SCHEMA_VERSION: u32 = 1;

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
        host_kv::reset();
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
}
