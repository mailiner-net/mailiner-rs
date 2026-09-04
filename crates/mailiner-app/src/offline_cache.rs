//! Envelope and body-part records, collision-free keys, and LRU helpers.
//!
//! Persistence is abstract via [`JsonObjectStore`] so host tests use
//! [`MemoryObjectStore`] and the browser uses IndexedDB.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use mailiner_core::models::{Envelope, MessageContent, MessagePart, MessageSort};
use serde::{Deserialize, Serialize};

use crate::account_store::AccountStoreError;

/// IndexedDB database name (`indexedDB.open`).
pub const IDB_DB_NAME: &str = "mailiner";
/// Schema version for [`IDB_DB_NAME`].
pub const IDB_VERSION: u32 = 1;

/// Folder LIST + STATUS snapshots, keyed by account id.
pub const STORE_FOLDERS: &str = "folders";
/// Individual envelopes, keyed by account + folder + uid.
pub const STORE_ENVELOPES: &str = "envelopes";
/// Per-folder prefix metadata (sort, totals, uid order).
pub const STORE_FOLDER_LISTS: &str = "folder_lists";
/// Decoded body parts, keyed by account + folder + uid + section.
pub const STORE_PARTS: &str = "parts";

/// Object stores created in [`IDB_DB_NAME`].
pub const IDB_STORES: &[&str] = &[
    STORE_FOLDERS,
    STORE_ENVELOPES,
    STORE_FOLDER_LISTS,
    STORE_PARTS,
];

/// Max opened messages whose parts are retained (LRU by last access).
pub const MAX_CACHED_PART_MESSAGES: usize = 32;
/// Soft cap on decoded part payloads kept on disk.
pub const MAX_CACHED_PART_BYTES: usize = 8 * 1024 * 1024;

/// Length-prefixed key encoding failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKeyError(pub &'static str);

impl fmt::Display for CacheKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cache key error: {}", self.0)
    }
}

impl std::error::Error for CacheKeyError {}

impl From<CacheKeyError> for AccountStoreError {
    fn from(err: CacheKeyError) -> Self {
        AccountStoreError::Serialization(err.to_string())
    }
}

/// Encode `segments` as `n:len:seg:len:seg…` so `:` / separators inside a
/// segment cannot collide with another tuple.
pub fn encode_key(segments: &[&str]) -> String {
    let mut out = segments.len().to_string();
    for seg in segments {
        out.push(':');
        out.push_str(&seg.len().to_string());
        out.push(':');
        out.push_str(seg);
    }
    out
}

/// Inverse of [`encode_key`]. Rejects truncated or trailing input.
pub fn decode_key(key: &str) -> Result<Vec<String>, CacheKeyError> {
    let bytes = key.as_bytes();
    let mut i = 0;
    let n = parse_digits(bytes, &mut i)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        expect_colon(bytes, &mut i)?;
        let len = parse_digits(bytes, &mut i)?;
        expect_colon(bytes, &mut i)?;
        let end = i.checked_add(len).ok_or(CacheKeyError("overflow"))?;
        if end > bytes.len() {
            return Err(CacheKeyError("truncated segment"));
        }
        let s = std::str::from_utf8(&bytes[i..end]).map_err(|_| CacheKeyError("utf8"))?;
        out.push(s.to_string());
        i = end;
    }
    if i != bytes.len() {
        return Err(CacheKeyError("trailing bytes"));
    }
    Ok(out)
}

/// Prefix of every [`encode_key`] whose first `segments` match and whose
/// total segment count is `total_segments`.
pub fn encode_key_prefix(
    segments: &[&str],
    total_segments: usize,
) -> Result<String, CacheKeyError> {
    if total_segments < segments.len() {
        return Err(CacheKeyError("total_segments shorter than prefix"));
    }
    let mut out = total_segments.to_string();
    for seg in segments {
        out.push(':');
        out.push_str(&seg.len().to_string());
        out.push(':');
        out.push_str(seg);
    }
    out.push(':');
    Ok(out)
}

pub fn envelope_key(account_id: &str, folder_id: &str, uid: &str) -> String {
    encode_key(&[account_id, folder_id, uid])
}

pub fn folder_list_key(account_id: &str, folder_id: &str) -> String {
    encode_key(&[account_id, folder_id])
}

pub fn part_key(account_id: &str, folder_id: &str, uid: &str, section: &str) -> String {
    encode_key(&[account_id, folder_id, uid, section])
}

pub fn envelope_folder_prefix(account_id: &str, folder_id: &str) -> Result<String, CacheKeyError> {
    encode_key_prefix(&[account_id, folder_id], 3)
}

pub fn envelope_account_prefix(account_id: &str) -> Result<String, CacheKeyError> {
    encode_key_prefix(&[account_id], 3)
}

pub fn folder_list_account_prefix(account_id: &str) -> Result<String, CacheKeyError> {
    encode_key_prefix(&[account_id], 2)
}

pub fn part_message_prefix(
    account_id: &str,
    folder_id: &str,
    uid: &str,
) -> Result<String, CacheKeyError> {
    encode_key_prefix(&[account_id, folder_id, uid], 4)
}

pub fn part_folder_prefix(account_id: &str, folder_id: &str) -> Result<String, CacheKeyError> {
    encode_key_prefix(&[account_id, folder_id], 4)
}

pub fn part_account_prefix(account_id: &str) -> Result<String, CacheKeyError> {
    encode_key_prefix(&[account_id], 4)
}

fn parse_digits(bytes: &[u8], i: &mut usize) -> Result<usize, CacheKeyError> {
    let start = *i;
    while *i < bytes.len() && bytes[*i].is_ascii_digit() {
        *i += 1;
    }
    if start == *i {
        return Err(CacheKeyError("expected digits"));
    }
    std::str::from_utf8(&bytes[start..*i])
        .map_err(|_| CacheKeyError("utf8"))?
        .parse()
        .map_err(|_| CacheKeyError("number"))
}

fn expect_colon(bytes: &[u8], i: &mut usize) -> Result<(), CacheKeyError> {
    if *i >= bytes.len() || bytes[*i] != b':' {
        return Err(CacheKeyError("expected ':'"));
    }
    *i += 1;
    Ok(())
}

// ── Records ─────────────────────────────────────────────────────────────────

/// One envelope row (`envelopes` store).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvelopeRecord {
    pub account_id: String,
    pub folder_id: String,
    pub uid: String,
    pub envelope: Envelope,
    pub accessed_at: DateTime<Utc>,
}

/// Prefix index for one folder (`folder_lists` store).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FolderListRecord {
    pub account_id: String,
    pub mailbox_id: String,
    pub sort: MessageSort,
    pub total: usize,
    pub unread: Option<usize>,
    pub uids: Vec<String>,
    pub accessed_at: DateTime<Utc>,
}

/// Decoded part payload stored beside a content-stripped [`MessagePart`] shell.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CachedPartContent {
    Empty,
    Text(String),
    /// Standard base64 — `Vec<u8>` must not go through JSON number arrays.
    BinaryBase64(String),
}

impl CachedPartContent {
    pub fn from_content(content: &MessageContent) -> Self {
        match content {
            MessageContent::Empty => Self::Empty,
            MessageContent::Text(text) => Self::Text(text.clone()),
            MessageContent::Binary(bytes) => Self::BinaryBase64(base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                bytes,
            )),
        }
    }

    pub fn into_content(self) -> MessageContent {
        match self {
            Self::Empty => MessageContent::Empty,
            Self::Text(text) => MessageContent::Text(text),
            Self::BinaryBase64(b64) => match base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                b64.as_bytes(),
            ) {
                Ok(bytes) => MessageContent::Binary(bytes),
                Err(_) => MessageContent::Empty,
            },
        }
    }

    pub fn byte_len(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Text(text) => text.len(),
            Self::BinaryBase64(b64) => base64_decoded_len(b64),
        }
    }
}

fn base64_decoded_len(b64: &str) -> usize {
    let padded = b64.bytes().filter(|b| !b.is_ascii_whitespace()).count();
    let pad = b64.bytes().rev().take_while(|b| *b == b'=').count();
    padded.saturating_mul(3) / 4 - pad
}

/// One part row (`parts` store).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CachedPartRecord {
    pub account_id: String,
    pub folder_id: String,
    pub uid: String,
    pub section: String,
    pub order: u32,
    /// `content` is always [`MessageContent::Empty`]; payload is [`Self::content`].
    pub part: MessagePart,
    pub content: CachedPartContent,
    pub byte_len: usize,
    pub accessed_at: DateTime<Utc>,
}

impl CachedPartRecord {
    pub fn from_part(
        account_id: &str,
        folder_id: &str,
        uid: &str,
        order: u32,
        part: &MessagePart,
        accessed_at: DateTime<Utc>,
    ) -> Self {
        let content = CachedPartContent::from_content(&part.content);
        let byte_len = content.byte_len();
        let mut shell = part.clone();
        shell.content = MessageContent::Empty;
        Self {
            account_id: account_id.to_string(),
            folder_id: folder_id.to_string(),
            uid: uid.to_string(),
            section: part.section(),
            order,
            part: shell,
            content,
            byte_len,
            accessed_at,
        }
    }

    pub fn into_part(self) -> MessagePart {
        let mut part = self.part;
        part.content = self.content.into_content();
        part
    }

    pub fn message_key(&self) -> String {
        envelope_key(&self.account_id, &self.folder_id, &self.uid)
    }
}

// ── LRU ─────────────────────────────────────────────────────────────────────

/// Keys of the oldest items that must go so at most `keep` remain.
pub fn pick_lru_keys(items: Vec<(String, DateTime<Utc>)>, keep: usize) -> Vec<String> {
    if items.len() <= keep {
        return Vec::new();
    }
    let mut items = items;
    items.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let drop_n = items.len() - keep;
    items.into_iter().take(drop_n).map(|(k, _)| k).collect()
}

/// Per-message rollup used to evict opened bodies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartMessageStat {
    pub message_key: String,
    pub accessed_at: DateTime<Utc>,
    pub byte_len: usize,
}

/// Oldest messages first until `max_messages` and `max_bytes` both hold.
pub fn pick_part_evictions(
    mut stats: Vec<PartMessageStat>,
    max_messages: usize,
    max_bytes: usize,
) -> Vec<String> {
    stats.sort_by(|a, b| {
        a.accessed_at
            .cmp(&b.accessed_at)
            .then_with(|| a.message_key.cmp(&b.message_key))
    });
    let mut evict = Vec::new();
    let mut count = stats.len();
    let mut bytes: usize = stats.iter().map(|s| s.byte_len).sum();
    for s in stats {
        if count <= max_messages && bytes <= max_bytes {
            break;
        }
        evict.push(s.message_key);
        count = count.saturating_sub(1);
        bytes = bytes.saturating_sub(s.byte_len);
    }
    evict
}

pub fn part_stats_from_records(records: &[CachedPartRecord]) -> Vec<PartMessageStat> {
    let mut map: HashMap<String, PartMessageStat> = HashMap::new();
    for rec in records {
        let key = rec.message_key();
        let entry = map.entry(key.clone()).or_insert(PartMessageStat {
            message_key: key,
            accessed_at: rec.accessed_at,
            byte_len: 0,
        });
        entry.byte_len = entry.byte_len.saturating_add(rec.byte_len);
        if rec.accessed_at > entry.accessed_at {
            entry.accessed_at = rec.accessed_at;
        }
    }
    map.into_values().collect()
}

// ── Object store ────────────────────────────────────────────────────────────

/// JSON document store (`?Send` — WASM is single-threaded).
#[async_trait(?Send)]
pub trait JsonObjectStore {
    async fn get(&self, store: &str, key: &str) -> Result<Option<String>, AccountStoreError>;
    async fn put(&self, store: &str, key: &str, value: &str) -> Result<(), AccountStoreError>;
    async fn delete(&self, store: &str, key: &str) -> Result<(), AccountStoreError>;
    async fn keys(&self, store: &str) -> Result<Vec<String>, AccountStoreError>;
    async fn values(&self, store: &str) -> Result<Vec<String>, AccountStoreError>;
    async fn clear(&self, store: &str) -> Result<(), AccountStoreError>;
}

/// Process-memory [`JsonObjectStore`] (unit tests / session fallback).
#[derive(Debug, Default)]
pub struct MemoryObjectStore {
    stores: RefCell<HashMap<String, HashMap<String, String>>>,
}

impl MemoryObjectStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait(?Send)]
impl JsonObjectStore for MemoryObjectStore {
    async fn get(&self, store: &str, key: &str) -> Result<Option<String>, AccountStoreError> {
        Ok(self
            .stores
            .borrow()
            .get(store)
            .and_then(|m| m.get(key))
            .cloned())
    }

    async fn put(&self, store: &str, key: &str, value: &str) -> Result<(), AccountStoreError> {
        self.stores
            .borrow_mut()
            .entry(store.to_string())
            .or_default()
            .insert(key.to_string(), value.to_string());
        Ok(())
    }

    async fn delete(&self, store: &str, key: &str) -> Result<(), AccountStoreError> {
        if let Some(m) = self.stores.borrow_mut().get_mut(store) {
            m.remove(key);
        }
        Ok(())
    }

    async fn keys(&self, store: &str) -> Result<Vec<String>, AccountStoreError> {
        Ok(self
            .stores
            .borrow()
            .get(store)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default())
    }

    async fn values(&self, store: &str) -> Result<Vec<String>, AccountStoreError> {
        Ok(self
            .stores
            .borrow()
            .get(store)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default())
    }

    async fn clear(&self, store: &str) -> Result<(), AccountStoreError> {
        if let Some(m) = self.stores.borrow_mut().get_mut(store) {
            m.clear();
        }
        Ok(())
    }
}

pub async fn delete_keys_with_prefix(
    store: &impl JsonObjectStore,
    store_name: &str,
    prefix: &str,
) -> Result<usize, AccountStoreError> {
    let keys = store.keys(store_name).await?;
    let mut n = 0;
    for key in keys {
        if key.starts_with(prefix) {
            store.delete(store_name, &key).await?;
            n += 1;
        }
    }
    Ok(n)
}

pub fn decode_json<T: for<'de> Deserialize<'de>>(json: &str) -> Result<T, AccountStoreError> {
    serde_json::from_str(json).map_err(|e| AccountStoreError::Serialization(e.to_string()))
}

pub fn encode_json<T: Serialize>(value: &T) -> Result<String, AccountStoreError> {
    serde_json::to_string(value).map_err(|e| AccountStoreError::Serialization(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use mailiner_core::ids::{FolderId, MessageId, MessagePartId};
    use mailiner_core::models::{PartKind, TransferEncoding};

    fn ts() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap()
    }

    #[test]
    fn idb_schema_names() {
        assert_eq!(IDB_DB_NAME, "mailiner");
        assert_eq!(IDB_VERSION, 1);
        assert_eq!(
            IDB_STORES,
            &[
                STORE_FOLDERS,
                STORE_ENVELOPES,
                STORE_FOLDER_LISTS,
                STORE_PARTS
            ]
        );
    }

    #[test]
    fn encode_decode_roundtrip_and_separator_safety() {
        let key = encode_key(&["acc:1", "IN:BOX", "12:3", "1.2"]);
        assert_eq!(key, "4:5:acc:1:6:IN:BOX:4:12:3:3:1.2");
        assert_eq!(
            decode_key(&key).unwrap(),
            vec!["acc:1", "IN:BOX", "12:3", "1.2"]
        );
    }

    #[test]
    fn encode_does_not_collide_when_folder_contains_uid() {
        let a = envelope_key("acc", "IN\u{1f}BOX", "12");
        let b = envelope_key("acc", "IN", "BOX\u{1f}12");
        assert_ne!(a, b);
        assert_eq!(decode_key(&a).unwrap()[1], "IN\u{1f}BOX");
        assert_eq!(decode_key(&b).unwrap()[2], "BOX\u{1f}12");
    }

    #[test]
    fn decode_rejects_truncated_and_trailing() {
        assert!(decode_key("3:3:acc:5:INBO").is_err());
        assert!(decode_key("1:3:acc:extra").is_err());
        assert!(decode_key("not-a-key").is_err());
        assert!(decode_key("").is_err());
    }

    #[test]
    fn empty_segment_roundtrips() {
        let key = encode_key(&["acc", "", "1"]);
        assert_eq!(decode_key(&key).unwrap(), vec!["acc", "", "1"]);
    }

    #[test]
    fn folder_prefix_matches_only_that_folder() {
        let prefix = envelope_folder_prefix("acc", "INBOX").unwrap();
        let hit = envelope_key("acc", "INBOX", "1");
        let other_folder = envelope_key("acc", "Sent", "1");
        let other_account = envelope_key("acc2", "INBOX", "1");
        let similar = envelope_key("acc", "IN", "BOX");
        assert!(hit.starts_with(&prefix), "hit={hit} prefix={prefix}");
        assert!(!other_folder.starts_with(&prefix));
        assert!(!other_account.starts_with(&prefix));
        assert!(!similar.starts_with(&prefix));
        assert!(envelope_key("acc", "INBOX", "12").starts_with(&prefix));
    }

    #[test]
    fn part_message_prefix_does_not_match_sibling_uid() {
        let prefix = part_message_prefix("acc", "INBOX", "1").unwrap();
        let hit = part_key("acc", "INBOX", "1", "TEXT");
        let sibling = part_key("acc", "INBOX", "12", "TEXT");
        assert!(hit.starts_with(&prefix), "hit={hit} prefix={prefix}");
        assert!(!sibling.starts_with(&prefix), "sibling={sibling}");
    }

    #[test]
    fn pick_lru_keys_drops_oldest() {
        let t0 = ts();
        let items = vec![
            ("c".into(), t0 + chrono::Duration::seconds(2)),
            ("a".into(), t0),
            ("b".into(), t0 + chrono::Duration::seconds(1)),
        ];
        assert_eq!(pick_lru_keys(items.clone(), 3), Vec::<String>::new());
        assert_eq!(pick_lru_keys(items, 2), vec!["a".to_string()]);
    }

    #[test]
    fn pick_part_evictions_by_count_and_bytes() {
        let t0 = ts();
        let stats = vec![
            PartMessageStat {
                message_key: "old".into(),
                accessed_at: t0,
                byte_len: 100,
            },
            PartMessageStat {
                message_key: "mid".into(),
                accessed_at: t0 + chrono::Duration::seconds(1),
                byte_len: 100,
            },
            PartMessageStat {
                message_key: "new".into(),
                accessed_at: t0 + chrono::Duration::seconds(2),
                byte_len: 100,
            },
        ];
        assert_eq!(
            pick_part_evictions(stats.clone(), 2, 10_000),
            vec!["old".to_string()]
        );
        assert_eq!(
            pick_part_evictions(stats, 10, 150),
            vec!["old".to_string(), "mid".to_string()]
        );
    }

    fn sample_part(text: &str) -> MessagePart {
        let now = ts();
        MessagePart {
            id: MessagePartId::new("TEXT"),
            envelope_id: MessageId::new(FolderId::new("INBOX"), "1"),
            path: vec!["TEXT".into()],
            kind: PartKind::TextPlain,
            content_type: "text/plain".into(),
            charset: Some("UTF-8".into()),
            content_id: None,
            description: None,
            filename: None,
            encoding: TransferEncoding::SevenBit,
            original_size: Some(text.len() as u64),
            size: text.len() as u64,
            is_attachment: false,
            is_hidden: false,
            nested_in: None,
            nested_headers: None,
            content: MessageContent::Text(text.into()),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn cached_part_text_and_binary_roundtrip() {
        let text = CachedPartRecord::from_part("acc", "INBOX", "1", 0, &sample_part("hello"), ts());
        assert_eq!(text.byte_len, 5);
        assert!(matches!(text.part.content, MessageContent::Empty));
        assert_eq!(
            text.into_part().content,
            MessageContent::Text("hello".into())
        );

        let mut bin = sample_part("");
        bin.content = MessageContent::Binary(vec![0, 1, 2, 255]);
        let rec = CachedPartRecord::from_part("acc", "INBOX", "1", 1, &bin, ts());
        assert_eq!(rec.byte_len, 4);
        match rec.clone().into_part().content {
            MessageContent::Binary(b) => assert_eq!(b, vec![0, 1, 2, 255]),
            other => panic!("expected binary, got {other:?}"),
        }
        let json = encode_json(&rec).unwrap();
        assert!(
            !json.contains("[0,1,2,255]"),
            "binary must not be a JSON array: {json}"
        );
        assert!(json.contains("BinaryBase64"), "json={json}");
        let back: CachedPartRecord = decode_json(&json).unwrap();
        assert_eq!(
            back.into_part().content,
            MessageContent::Binary(vec![0, 1, 2, 255])
        );
    }

    #[tokio::test]
    async fn memory_object_store_roundtrip() {
        let store = MemoryObjectStore::new();
        store.put("envelopes", "k", "{\"n\":1}").await.unwrap();
        assert_eq!(
            store.get("envelopes", "k").await.unwrap().as_deref(),
            Some("{\"n\":1}")
        );
        assert_eq!(store.keys("envelopes").await.unwrap(), vec!["k"]);
        store.delete("envelopes", "k").await.unwrap();
        assert!(store.get("envelopes", "k").await.unwrap().is_none());
    }
}
