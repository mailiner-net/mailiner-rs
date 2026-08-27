//! Write-ahead SMTP outbox (`mailiner.outbox.v1`).

use std::cell::RefCell;

use async_trait::async_trait;
use base64::Engine;
use chrono::{DateTime, Utc};
use mailiner_core::ids::AccountId;
use mailiner_core::submit::{SendErrorKind, SubmitRequest};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::account_store::{AccountStoreError, MemoryKvStore, StringKvStore, WebLocalStorage};

/// `localStorage` key for the outbox blob (account schema is untouched).
pub const OUTBOX_LOCAL_STORAGE_KEY: &str = "mailiner.outbox.v1";
/// Outbox blob schema (independent of [`crate::account_config::ACCOUNT_STORE_SCHEMA_VERSION`]).
pub const OUTBOX_STORE_SCHEMA_VERSION: u32 = 1;
/// Max raw RFC 822 bytes per item.
pub const MAX_OUTBOX_ITEM_BYTES: usize = 1_500_000;
/// Max items in the blob.
pub const MAX_OUTBOX_ITEMS: usize = 20;
/// Max encoded JSON blob size.
pub const MAX_OUTBOX_BLOB_BYTES: usize = 4_000_000;
/// Consecutive retryable failures before auto-drain marks `Failed`.
pub const MAX_OUTBOX_AUTO_ATTEMPTS: u32 = 5;

/// Stable outbox row id (uuid v4).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OutboxId(pub String);

impl OutboxId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for OutboxId {
    fn default() -> Self {
        Self::new()
    }
}

/// Persisted send-queue state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxItemState {
    Queued,
    Sending,
    Failed,
}

/// One durable outbound message. No passwords.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutboxItem {
    pub id: OutboxId,
    pub account_id: AccountId,
    pub mail_from: String,
    pub rcpt_to: Vec<String>,
    /// Standard base64 of RFC 5322 bytes.
    pub rfc822_b64: String,
    /// Sent-folder copy (Bcc restored). Absent on older blobs and when identical to [`Self::rfc822_b64`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rfc822_sent_b64: Option<String>,
    pub message_id: String,
    pub subject: String,
    pub to_preview: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub attempts: u32,
    pub last_error_kind: Option<SendErrorKind>,
    pub last_error: Option<String>,
    pub state: OutboxItemState,
}

impl OutboxItem {
    pub fn rfc822(&self) -> Result<Vec<u8>, AccountStoreError> {
        base64::engine::general_purpose::STANDARD
            .decode(&self.rfc822_b64)
            .map_err(|e| AccountStoreError::Serialization(e.to_string()))
    }

    pub fn from_request(
        account_id: AccountId,
        request: &SubmitRequest,
        subject: String,
        to_preview: String,
    ) -> Result<Self, AccountStoreError> {
        if request.rfc822.len() > MAX_OUTBOX_ITEM_BYTES {
            return Err(AccountStoreError::Other(format!(
                "Message is too large to keep in this browser ({} bytes).",
                request.rfc822.len()
            )));
        }
        let now = Utc::now();
        Ok(Self {
            id: OutboxId::new(),
            account_id,
            mail_from: request.mail_from.clone(),
            rcpt_to: request.rcpt_to.clone(),
            rfc822_b64: base64::engine::general_purpose::STANDARD.encode(&request.rfc822),
            rfc822_sent_b64: None,
            message_id: request.message_id.clone(),
            subject,
            to_preview,
            created_at: now,
            updated_at: now,
            attempts: 0,
            last_error_kind: None,
            last_error: None,
            state: OutboxItemState::Queued,
        })
    }

    pub fn to_request(&self) -> Result<SubmitRequest, AccountStoreError> {
        Ok(SubmitRequest {
            mail_from: self.mail_from.clone(),
            rcpt_to: self.rcpt_to.clone(),
            rfc822: self.rfc822()?,
            message_id: self.message_id.clone(),
        })
    }

    /// Store a Sent-folder copy that includes `Bcc:` (SMTP DATA must not).
    pub fn set_sent_copy(&mut self, rfc822_sent: &[u8]) -> Result<(), AccountStoreError> {
        if rfc822_sent.len() > MAX_OUTBOX_ITEM_BYTES {
            return Err(AccountStoreError::Other(format!(
                "Message is too large to keep in this browser ({} bytes).",
                rfc822_sent.len()
            )));
        }
        self.rfc822_sent_b64 = Some(base64::engine::general_purpose::STANDARD.encode(rfc822_sent));
        Ok(())
    }

    /// Bytes to APPEND to Sent: Bcc-restored copy when present.
    pub fn rfc822_for_mailbox(&self) -> Result<Vec<u8>, AccountStoreError> {
        let Some(b64) = self.rfc822_sent_b64.as_deref() else {
            return self.rfc822();
        };
        base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| AccountStoreError::Serialization(e.to_string()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OutboxBlob {
    schema_version: u32,
    items: Vec<OutboxItem>,
}

impl OutboxBlob {
    fn empty() -> Self {
        Self {
            schema_version: OUTBOX_STORE_SCHEMA_VERSION,
            items: Vec::new(),
        }
    }

    fn encode(&self) -> Result<String, AccountStoreError> {
        let json = serde_json::to_string(self)
            .map_err(|e| AccountStoreError::Serialization(e.to_string()))?;
        if json.len() > MAX_OUTBOX_BLOB_BYTES {
            return Err(AccountStoreError::Other(
                "Outbox storage is full. Delete a queued message and try again.".into(),
            ));
        }
        Ok(json)
    }

    fn decode(json: &str) -> Result<Self, AccountStoreError> {
        let blob: Self = serde_json::from_str(json)
            .map_err(|e| AccountStoreError::Serialization(e.to_string()))?;
        if blob.schema_version > OUTBOX_STORE_SCHEMA_VERSION {
            return Err(AccountStoreError::Serialization(format!(
                "unsupported outbox schema_version {} (max {})",
                blob.schema_version, OUTBOX_STORE_SCHEMA_VERSION
            )));
        }
        Ok(blob)
    }
}

/// Durable outbox (write-ahead before SMTP).
#[async_trait(?Send)]
pub trait OutboxStore {
    async fn list(&self) -> Result<Vec<OutboxItem>, AccountStoreError>;
    async fn get(&self, id: &OutboxId) -> Result<Option<OutboxItem>, AccountStoreError>;
    async fn upsert(&self, item: &OutboxItem) -> Result<(), AccountStoreError>;
    async fn delete(&self, id: &OutboxId) -> Result<(), AccountStoreError>;
    async fn delete_for_account(&self, account_id: &AccountId) -> Result<(), AccountStoreError>;
    async fn oldest_queued(&self) -> Result<Option<OutboxItem>, AccountStoreError>;
}

/// In-memory store for host tests.
#[derive(Debug, Default)]
pub struct InMemoryOutboxStore {
    items: RefCell<Vec<OutboxItem>>,
}

impl InMemoryOutboxStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait(?Send)]
impl OutboxStore for InMemoryOutboxStore {
    async fn list(&self) -> Result<Vec<OutboxItem>, AccountStoreError> {
        Ok(self.items.borrow().clone())
    }

    async fn get(&self, id: &OutboxId) -> Result<Option<OutboxItem>, AccountStoreError> {
        Ok(self.items.borrow().iter().find(|i| i.id == *id).cloned())
    }

    async fn upsert(&self, item: &OutboxItem) -> Result<(), AccountStoreError> {
        if item.rfc822_b64.len() > MAX_OUTBOX_ITEM_BYTES * 2 {
            return Err(AccountStoreError::Other("message too large".into()));
        }
        let raw_len = item.rfc822()?.len();
        if raw_len > MAX_OUTBOX_ITEM_BYTES {
            return Err(AccountStoreError::Other(format!(
                "Message is too large to keep in this browser ({raw_len} bytes)."
            )));
        }
        let mut items = self.items.borrow_mut();
        if let Some(existing) = items.iter_mut().find(|i| i.id == item.id) {
            *existing = item.clone();
            return Ok(());
        }
        if items.len() >= MAX_OUTBOX_ITEMS {
            return Err(AccountStoreError::Other(
                "Outbox is full (20 messages). Delete one and try again.".into(),
            ));
        }
        items.push(item.clone());
        Ok(())
    }

    async fn delete(&self, id: &OutboxId) -> Result<(), AccountStoreError> {
        self.items.borrow_mut().retain(|i| i.id != *id);
        Ok(())
    }

    async fn delete_for_account(&self, account_id: &AccountId) -> Result<(), AccountStoreError> {
        self.items
            .borrow_mut()
            .retain(|i| i.account_id != *account_id);
        Ok(())
    }

    async fn oldest_queued(&self) -> Result<Option<OutboxItem>, AccountStoreError> {
        let items = self.items.borrow();
        Ok(items
            .iter()
            .filter(|i| i.state == OutboxItemState::Queued)
            .min_by(|a, b| {
                a.created_at
                    .cmp(&b.created_at)
                    .then_with(|| a.id.as_str().cmp(b.id.as_str()))
            })
            .cloned())
    }
}

/// Browser / test `StringKvStore` outbox.
pub struct BrowserOutboxStore<K: StringKvStore = WebLocalStorage> {
    kv: K,
}

impl BrowserOutboxStore<WebLocalStorage> {
    pub async fn open() -> Result<Self, AccountStoreError> {
        Ok(Self {
            kv: WebLocalStorage::try_open()?,
        })
    }
}

impl BrowserOutboxStore<MemoryKvStore> {
    pub fn open_memory() -> Self {
        Self {
            kv: MemoryKvStore::new(),
        }
    }

    pub fn with_kv(kv: MemoryKvStore) -> Self {
        Self { kv }
    }
}

impl<K: StringKvStore> BrowserOutboxStore<K> {
    fn load(&self) -> Result<OutboxBlob, AccountStoreError> {
        match self.kv.get_item(OUTBOX_LOCAL_STORAGE_KEY)? {
            None => Ok(OutboxBlob::empty()),
            Some(json) if json.is_empty() => Ok(OutboxBlob::empty()),
            Some(json) => OutboxBlob::decode(&json),
        }
    }

    fn save(&self, blob: &OutboxBlob) -> Result<(), AccountStoreError> {
        self.kv.set_item(OUTBOX_LOCAL_STORAGE_KEY, &blob.encode()?)
    }
}

#[async_trait(?Send)]
impl<K: StringKvStore> OutboxStore for BrowserOutboxStore<K> {
    async fn list(&self) -> Result<Vec<OutboxItem>, AccountStoreError> {
        Ok(self.load()?.items)
    }

    async fn get(&self, id: &OutboxId) -> Result<Option<OutboxItem>, AccountStoreError> {
        Ok(self.load()?.items.into_iter().find(|i| i.id == *id))
    }

    async fn upsert(&self, item: &OutboxItem) -> Result<(), AccountStoreError> {
        let raw_len = item.rfc822()?.len();
        if raw_len > MAX_OUTBOX_ITEM_BYTES {
            return Err(AccountStoreError::Other(format!(
                "Message is too large to keep in this browser ({raw_len} bytes)."
            )));
        }
        let mut blob = self.load()?;
        if let Some(existing) = blob.items.iter_mut().find(|i| i.id == item.id) {
            *existing = item.clone();
        } else {
            if blob.items.len() >= MAX_OUTBOX_ITEMS {
                return Err(AccountStoreError::Other(
                    "Outbox is full (20 messages). Delete one and try again.".into(),
                ));
            }
            blob.items.push(item.clone());
        }
        self.save(&blob)
    }

    async fn delete(&self, id: &OutboxId) -> Result<(), AccountStoreError> {
        let mut blob = self.load()?;
        blob.items.retain(|i| i.id != *id);
        self.save(&blob)
    }

    async fn delete_for_account(&self, account_id: &AccountId) -> Result<(), AccountStoreError> {
        let mut blob = self.load()?;
        blob.items.retain(|i| i.account_id != *account_id);
        self.save(&blob)
    }

    async fn oldest_queued(&self) -> Result<Option<OutboxItem>, AccountStoreError> {
        let blob = self.load()?;
        Ok(blob
            .items
            .into_iter()
            .filter(|i| i.state == OutboxItemState::Queued)
            .min_by(|a, b| {
                a.created_at
                    .cmp(&b.created_at)
                    .then_with(|| a.id.as_str().cmp(b.id.as_str()))
            }))
    }
}

/// UI row (no rfc822).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxListEntry {
    pub id: OutboxId,
    pub account_id: AccountId,
    pub subject: String,
    pub to_preview: String,
    pub state: OutboxItemState,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl From<&OutboxItem> for OutboxListEntry {
    fn from(item: &OutboxItem) -> Self {
        Self {
            id: item.id.clone(),
            account_id: item.account_id.clone(),
            subject: item.subject.clone(),
            to_preview: item.to_preview.clone(),
            state: item.state,
            attempts: item.attempts,
            last_error: item.last_error.clone(),
            created_at: item.created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailiner_core::SubmitRequest;

    fn req(n: usize) -> SubmitRequest {
        SubmitRequest {
            mail_from: "me@example.com".into(),
            rcpt_to: vec!["you@example.com".into()],
            rfc822: vec![b'x'; n],
            message_id: "<id@example.com>".into(),
        }
    }

    #[tokio::test]
    async fn round_trip_and_oldest() {
        let store = BrowserOutboxStore::<MemoryKvStore>::open_memory();
        let item = OutboxItem::from_request(
            AccountId::new("a"),
            &req(16),
            "Hi".into(),
            "you@example.com".into(),
        )
        .unwrap();
        store.upsert(&item).await.unwrap();
        let back = store.get(&item.id).await.unwrap().unwrap();
        assert_eq!(back.rfc822().unwrap(), vec![b'x'; 16]);
        assert_eq!(back.rfc822_for_mailbox().unwrap(), vec![b'x'; 16]);
        assert_eq!(store.oldest_queued().await.unwrap().unwrap().id, item.id);
    }

    #[tokio::test]
    async fn sent_copy_round_trip() {
        let store = BrowserOutboxStore::<MemoryKvStore>::open_memory();
        let mut item = OutboxItem::from_request(
            AccountId::new("a"),
            &req(8),
            "Hi".into(),
            "you@example.com".into(),
        )
        .unwrap();
        item.set_sent_copy(b"Bcc: secret@example.com\r\n").unwrap();
        store.upsert(&item).await.unwrap();
        let back = store.get(&item.id).await.unwrap().unwrap();
        assert_eq!(back.rfc822().unwrap(), vec![b'x'; 8]);
        assert_eq!(
            back.rfc822_for_mailbox().unwrap(),
            b"Bcc: secret@example.com\r\n"
        );
    }

    #[test]
    fn old_blob_without_sent_copy_decodes() {
        let json = format!(
            r#"{{"schema_version":1,"items":[{{"id":"i1","account_id":"a","mail_from":"me@x.com","rcpt_to":["you@x.com"],"rfc822_b64":"eHg=","message_id":"<id@x.com>","subject":"Hi","to_preview":"you","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","attempts":0,"last_error_kind":null,"last_error":null,"state":"queued"}}]}}"#
        );
        let blob = OutboxBlob::decode(&json).expect("legacy blob");
        assert_eq!(blob.items.len(), 1);
        assert!(blob.items[0].rfc822_sent_b64.is_none());
        assert_eq!(blob.items[0].rfc822().unwrap(), b"xx");
    }

    #[tokio::test]
    async fn reject_oversize() {
        let err = OutboxItem::from_request(
            AccountId::new("a"),
            &req(MAX_OUTBOX_ITEM_BYTES + 1),
            "Hi".into(),
            "you".into(),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("too large"));
    }

    #[tokio::test]
    async fn reject_future_schema() {
        let kv = MemoryKvStore::new();
        kv.set_item(
            OUTBOX_LOCAL_STORAGE_KEY,
            r#"{"schema_version":99,"items":[]}"#,
        )
        .unwrap();
        let store = BrowserOutboxStore::with_kv(kv);
        let err = store.list().await.unwrap_err();
        assert!(format!("{err}").contains("schema_version"));
    }

    #[tokio::test]
    async fn delete_for_account() {
        let store = InMemoryOutboxStore::new();
        let a =
            OutboxItem::from_request(AccountId::new("a"), &req(4), "A".into(), "t".into()).unwrap();
        let b =
            OutboxItem::from_request(AccountId::new("b"), &req(4), "B".into(), "t".into()).unwrap();
        store.upsert(&a).await.unwrap();
        store.upsert(&b).await.unwrap();
        store
            .delete_for_account(&AccountId::new("a"))
            .await
            .unwrap();
        assert_eq!(store.list().await.unwrap().len(), 1);
        assert_eq!(store.list().await.unwrap()[0].account_id.as_str(), "b");
    }
}
