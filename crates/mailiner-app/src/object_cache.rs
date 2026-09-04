//! [`MailCache`] over a [`JsonObjectStore`] (memory or IndexedDB).

use std::collections::HashSet;
use std::fmt;

use async_trait::async_trait;
use chrono::Utc;
use mailiner_core::ids::{AccountId, MessageId};
use mailiner_core::models::{MessagePart, MessageSort};

use crate::account_store::AccountStoreError;
use crate::mail_cache::{
    CachedFolderTree, CachedMessageList, MAX_CACHED_FOLDERS, MAX_MESSAGES_PER_FOLDER, MailCache,
};
use crate::mailbox::MailboxId;
use crate::offline_cache::{
    CachedPartRecord, EnvelopeRecord, FolderListRecord, JsonObjectStore, MAX_CACHED_PART_BYTES,
    MAX_CACHED_PART_MESSAGES, STORE_ENVELOPES, STORE_FOLDER_LISTS, STORE_FOLDERS, STORE_PARTS,
    decode_json, delete_keys_with_prefix, encode_json, envelope_account_prefix,
    envelope_folder_prefix, envelope_key, folder_list_account_prefix, folder_list_key,
    part_account_prefix, part_folder_prefix, part_key, part_message_prefix,
    part_stats_from_records, pick_lru_keys, pick_part_evictions,
};

/// [`MailCache`] that stores folder trees, per-envelope rows, folder-list
/// indexes, and opened parts in a [`JsonObjectStore`].
pub struct ObjectStoreMailCache<S> {
    store: S,
}

impl<S> ObjectStoreMailCache<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &S {
        &self.store
    }
}

impl<S: JsonObjectStore> ObjectStoreMailCache<S> {
    pub async fn is_empty(&self) -> Result<bool, AccountStoreError> {
        if !self.store.keys(STORE_FOLDERS).await?.is_empty() {
            return Ok(false);
        }
        if !self.store.keys(STORE_FOLDER_LISTS).await?.is_empty() {
            return Ok(false);
        }
        if !self.store.keys(STORE_ENVELOPES).await?.is_empty() {
            return Ok(false);
        }
        if !self.store.keys(STORE_PARTS).await?.is_empty() {
            return Ok(false);
        }
        Ok(true)
    }

    async fn evict_lru_folders(&self, account_id: &AccountId) -> Result<(), AccountStoreError> {
        let prefix = folder_list_account_prefix(account_id.as_str())?;
        let keys = self.store.keys(STORE_FOLDER_LISTS).await?;
        let mut items = Vec::new();
        for key in keys {
            if !key.starts_with(&prefix) {
                continue;
            }
            let Some(json) = self.store.get(STORE_FOLDER_LISTS, &key).await? else {
                continue;
            };
            let rec: FolderListRecord = decode_json(&json)?;
            items.push((key, rec.accessed_at));
        }
        for key in pick_lru_keys(items, MAX_CACHED_FOLDERS) {
            let Some(json) = self.store.get(STORE_FOLDER_LISTS, &key).await? else {
                continue;
            };
            let rec: FolderListRecord = decode_json(&json)?;
            self.store.delete(STORE_FOLDER_LISTS, &key).await?;
            let env_prefix = envelope_folder_prefix(account_id.as_str(), &rec.mailbox_id)?;
            delete_keys_with_prefix(&self.store, STORE_ENVELOPES, &env_prefix).await?;
        }
        Ok(())
    }

    async fn evict_lru_parts(&self) -> Result<(), AccountStoreError> {
        let records = self.load_all_part_records().await?;
        let evict = pick_part_evictions(
            part_stats_from_records(&records),
            MAX_CACHED_PART_MESSAGES,
            MAX_CACHED_PART_BYTES,
        );
        for message_key in evict {
            self.delete_parts_for_message_key(&message_key).await?;
        }
        Ok(())
    }

    async fn load_all_part_records(&self) -> Result<Vec<CachedPartRecord>, AccountStoreError> {
        let mut out = Vec::new();
        for json in self.store.values(STORE_PARTS).await? {
            match decode_json::<CachedPartRecord>(&json) {
                Ok(rec) => out.push(rec),
                Err(e) => {
                    dioxus::logger::tracing::warn!("skipping corrupt part cache row: {e}");
                }
            }
        }
        Ok(out)
    }

    async fn delete_parts_for_message_key(
        &self,
        message_key: &str,
    ) -> Result<(), AccountStoreError> {
        let segs = crate::offline_cache::decode_key(message_key)?;
        if segs.len() != 3 {
            return Err(AccountStoreError::Serialization(
                "part message key must be account/folder/uid".into(),
            ));
        }
        let prefix = part_message_prefix(&segs[0], &segs[1], &segs[2])?;
        delete_keys_with_prefix(&self.store, STORE_PARTS, &prefix).await?;
        Ok(())
    }

    async fn delete_account_rows(&self, account_id: &AccountId) -> Result<(), AccountStoreError> {
        self.store
            .delete(STORE_FOLDERS, account_id.as_str())
            .await?;
        delete_keys_with_prefix(
            &self.store,
            STORE_FOLDER_LISTS,
            &folder_list_account_prefix(account_id.as_str())?,
        )
        .await?;
        delete_keys_with_prefix(
            &self.store,
            STORE_ENVELOPES,
            &envelope_account_prefix(account_id.as_str())?,
        )
        .await?;
        delete_keys_with_prefix(
            &self.store,
            STORE_PARTS,
            &part_account_prefix(account_id.as_str())?,
        )
        .await?;
        Ok(())
    }
}

impl<S> fmt::Debug for ObjectStoreMailCache<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ObjectStoreMailCache")
            .finish_non_exhaustive()
    }
}

#[async_trait(?Send)]
impl<S: JsonObjectStore> MailCache for ObjectStoreMailCache<S> {
    async fn load_folders(
        &self,
        account_id: &AccountId,
    ) -> Result<Option<CachedFolderTree>, AccountStoreError> {
        let Some(json) = self.store.get(STORE_FOLDERS, account_id.as_str()).await? else {
            return Ok(None);
        };
        Ok(Some(decode_json(&json)?))
    }

    async fn save_folders(
        &self,
        account_id: &AccountId,
        tree: &CachedFolderTree,
    ) -> Result<(), AccountStoreError> {
        self.store
            .put(STORE_FOLDERS, account_id.as_str(), &encode_json(tree)?)
            .await
    }

    async fn load_messages(
        &self,
        account_id: &AccountId,
        mailbox_id: &MailboxId,
        sort: MessageSort,
    ) -> Result<Option<CachedMessageList>, AccountStoreError> {
        let list_key = folder_list_key(account_id.as_str(), mailbox_id.as_str());
        let Some(json) = self.store.get(STORE_FOLDER_LISTS, &list_key).await? else {
            return Ok(None);
        };
        let mut meta: FolderListRecord = decode_json(&json)?;
        if meta.sort != sort {
            return Ok(None);
        }
        let now = Utc::now();
        meta.accessed_at = now;
        let mut envelopes = Vec::with_capacity(meta.uids.len());
        for uid in &meta.uids {
            let key = envelope_key(account_id.as_str(), mailbox_id.as_str(), uid);
            let Some(env_json) = self.store.get(STORE_ENVELOPES, &key).await? else {
                continue;
            };
            let mut rec: EnvelopeRecord = decode_json(&env_json)?;
            rec.accessed_at = now;
            envelopes.push(rec.envelope.clone());
            if let Err(e) = self
                .store
                .put(STORE_ENVELOPES, &key, &encode_json(&rec)?)
                .await
            {
                dioxus::logger::tracing::warn!("mail cache envelope LRU touch failed: {e}");
            }
        }
        if envelopes.is_empty() && !meta.uids.is_empty() {
            return Ok(None);
        }
        if let Err(e) = self
            .store
            .put(STORE_FOLDER_LISTS, &list_key, &encode_json(&meta)?)
            .await
        {
            dioxus::logger::tracing::warn!("mail cache folder-list LRU touch failed: {e}");
        }
        Ok(Some(CachedMessageList {
            mailbox_id: meta.mailbox_id,
            sort: meta.sort,
            total: meta.total,
            unread: meta.unread,
            envelopes,
            accessed_at: meta.accessed_at,
        }))
    }

    async fn save_messages(
        &self,
        account_id: &AccountId,
        list: &CachedMessageList,
    ) -> Result<(), AccountStoreError> {
        let mut envelopes = list.envelopes.clone();
        if envelopes.len() > MAX_MESSAGES_PER_FOLDER {
            envelopes.truncate(MAX_MESSAGES_PER_FOLDER);
        }
        let now = Utc::now();
        let folder = list.mailbox_id.as_str();
        let keep: HashSet<&str> = envelopes.iter().map(|e| e.id.as_uid()).collect();
        let env_prefix = envelope_folder_prefix(account_id.as_str(), folder)?;
        let existing = self.store.keys(STORE_ENVELOPES).await?;
        for key in existing {
            if !key.starts_with(&env_prefix) {
                continue;
            }
            let segs = match crate::offline_cache::decode_key(&key) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if segs.len() == 3 && !keep.contains(segs[2].as_str()) {
                self.store.delete(STORE_ENVELOPES, &key).await?;
            }
        }
        let mut uids = Vec::with_capacity(envelopes.len());
        for env in &envelopes {
            let uid = env.id.as_uid().to_string();
            let key = envelope_key(account_id.as_str(), folder, &uid);
            let rec = EnvelopeRecord {
                account_id: account_id.as_str().to_string(),
                folder_id: folder.to_string(),
                uid: uid.clone(),
                envelope: env.clone(),
                accessed_at: now,
            };
            self.store
                .put(STORE_ENVELOPES, &key, &encode_json(&rec)?)
                .await?;
            uids.push(uid);
        }
        let meta = FolderListRecord {
            account_id: account_id.as_str().to_string(),
            mailbox_id: list.mailbox_id.clone(),
            sort: list.sort,
            total: list.total,
            unread: list.unread,
            uids,
            accessed_at: now,
        };
        self.store
            .put(
                STORE_FOLDER_LISTS,
                &folder_list_key(account_id.as_str(), folder),
                &encode_json(&meta)?,
            )
            .await?;
        self.evict_lru_folders(account_id).await
    }

    async fn invalidate_messages(
        &self,
        account_id: &AccountId,
        mailbox_id: &MailboxId,
    ) -> Result<(), AccountStoreError> {
        self.store
            .delete(
                STORE_FOLDER_LISTS,
                &folder_list_key(account_id.as_str(), mailbox_id.as_str()),
            )
            .await?;
        delete_keys_with_prefix(
            &self.store,
            STORE_ENVELOPES,
            &envelope_folder_prefix(account_id.as_str(), mailbox_id.as_str())?,
        )
        .await?;
        delete_keys_with_prefix(
            &self.store,
            STORE_PARTS,
            &part_folder_prefix(account_id.as_str(), mailbox_id.as_str())?,
        )
        .await?;
        Ok(())
    }

    async fn delete_account(&self, account_id: &AccountId) -> Result<(), AccountStoreError> {
        self.delete_account_rows(account_id).await
    }

    async fn retain_accounts(&self, known: &HashSet<AccountId>) -> Result<(), AccountStoreError> {
        let mut seen = HashSet::new();
        for key in self.store.keys(STORE_FOLDERS).await? {
            seen.insert(AccountId::new(key));
        }
        for key in self.store.keys(STORE_FOLDER_LISTS).await? {
            if let Ok(segs) = crate::offline_cache::decode_key(&key)
                && let Some(acc) = segs.first()
            {
                seen.insert(AccountId::new(acc.clone()));
            }
        }
        for key in self.store.keys(STORE_ENVELOPES).await? {
            if let Ok(segs) = crate::offline_cache::decode_key(&key)
                && let Some(acc) = segs.first()
            {
                seen.insert(AccountId::new(acc.clone()));
            }
        }
        for key in self.store.keys(STORE_PARTS).await? {
            if let Ok(segs) = crate::offline_cache::decode_key(&key)
                && let Some(acc) = segs.first()
            {
                seen.insert(AccountId::new(acc.clone()));
            }
        }
        for acc in seen {
            if !known.contains(&acc) {
                self.delete_account_rows(&acc).await?;
            }
        }
        Ok(())
    }

    async fn save_parts(
        &self,
        account_id: &AccountId,
        message_id: &MessageId,
        parts: &[MessagePart],
    ) -> Result<(), AccountStoreError> {
        if parts.is_empty() {
            return Ok(());
        }
        let folder = message_id.folder_id().as_str();
        let uid = message_id.as_uid();
        let now = Utc::now();
        let prefix = part_message_prefix(account_id.as_str(), folder, uid)?;
        delete_keys_with_prefix(&self.store, STORE_PARTS, &prefix).await?;
        for (order, part) in parts.iter().enumerate() {
            let rec = CachedPartRecord::from_part(
                account_id.as_str(),
                folder,
                uid,
                order as u32,
                part,
                now,
            );
            let key = part_key(account_id.as_str(), folder, uid, &rec.section);
            self.store
                .put(STORE_PARTS, &key, &encode_json(&rec)?)
                .await?;
        }
        self.evict_lru_parts().await
    }

    async fn load_parts(
        &self,
        account_id: &AccountId,
        message_id: &MessageId,
    ) -> Result<Option<Vec<MessagePart>>, AccountStoreError> {
        let folder = message_id.folder_id().as_str();
        let uid = message_id.as_uid();
        let prefix = part_message_prefix(account_id.as_str(), folder, uid)?;
        let keys = self.store.keys(STORE_PARTS).await?;
        let mut recs = Vec::new();
        let now = Utc::now();
        for key in keys {
            if !key.starts_with(&prefix) {
                continue;
            }
            let Some(json) = self.store.get(STORE_PARTS, &key).await? else {
                continue;
            };
            let mut rec: CachedPartRecord = decode_json(&json)?;
            rec.accessed_at = now;
            recs.push((key, rec));
        }
        if recs.is_empty() {
            return Ok(None);
        }
        recs.sort_by_key(|(_, r)| r.order);
        for (key, rec) in &recs {
            if let Err(e) = self.store.put(STORE_PARTS, key, &encode_json(rec)?).await {
                dioxus::logger::tracing::warn!("mail cache part LRU touch failed: {e}");
            }
        }
        Ok(Some(recs.into_iter().map(|(_, r)| r.into_part()).collect()))
    }

    async fn clear_all(&self) -> Result<(), AccountStoreError> {
        for store in [
            STORE_FOLDERS,
            STORE_ENVELOPES,
            STORE_FOLDER_LISTS,
            STORE_PARTS,
        ] {
            self.store.clear(store).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone};
    use mailiner_core::ids::FolderId;
    use mailiner_core::models::{
        EmailAddr, EmailAddress, Envelope, Folder, FolderCounts, MailboxRole, MessageContent,
        PartKind, TransferEncoding,
    };
    use mailiner_core::{AccountId, MessageId, MessagePartId};

    use crate::offline_cache::{MemoryObjectStore, envelope_key, part_key};

    fn ts() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 6, 1, 12, 0, 0).unwrap()
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
        CachedMessageList::new(
            &MailboxId::from(mailbox.to_string()),
            sort,
            total,
            Some(n),
            envs,
        )
    }

    fn sample_part(folder: &str, uid: &str, section: &str, text: &str) -> MessagePart {
        let now = ts();
        MessagePart {
            id: MessagePartId::new(section),
            envelope_id: MessageId::new(FolderId::new(folder), uid),
            path: if section == "TEXT" {
                Vec::new()
            } else {
                section.split('.').map(str::to_string).collect()
            },
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

    fn cache() -> ObjectStoreMailCache<MemoryObjectStore> {
        ObjectStoreMailCache::new(MemoryObjectStore::new())
    }

    #[tokio::test]
    async fn folders_and_envelope_prefix_roundtrip() {
        let cache = cache();
        let acc = AccountId::new("acc");
        let inbox = MailboxId::from("INBOX".to_string());
        let tree = CachedFolderTree::new(
            vec![Folder {
                id: FolderId::new("INBOX"),
                account_id: acc.clone(),
                name: "INBOX".into(),
                parent_id: None,
                role: MailboxRole::Inbox,
                selectable: true,
                subscribed: true,
            }],
            inbox_counts(),
        );
        cache.save_folders(&acc, &tree).await.unwrap();
        cache
            .save_messages(&acc, &list_for("INBOX", 2, 10, MessageSort::Arrival))
            .await
            .unwrap();

        let loaded = cache.load_folders(&acc).await.unwrap().unwrap();
        assert_eq!(loaded.folders.len(), 1);
        let msgs = cache
            .load_messages(&acc, &inbox, MessageSort::Arrival)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(msgs.envelopes.len(), 2);
        assert_eq!(msgs.total, 10);
        assert_eq!(msgs.envelopes[0].subject.as_deref(), Some("s1"));

        let env_json = cache
            .store()
            .get(STORE_ENVELOPES, &envelope_key("acc", "INBOX", "1"))
            .await
            .unwrap();
        assert!(env_json.is_some(), "individual envelope row missing");
    }

    fn inbox_counts() -> std::collections::HashMap<String, FolderCounts> {
        std::collections::HashMap::from([(
            "INBOX".into(),
            FolderCounts {
                total_messages: 10,
                unread_messages: 2,
            },
        )])
    }

    #[tokio::test]
    async fn load_messages_misses_on_sort_mismatch() {
        let cache = cache();
        let acc = AccountId::new("acc");
        let inbox = MailboxId::from("INBOX".to_string());
        cache
            .save_messages(&acc, &list_for("INBOX", 2, 2, MessageSort::Arrival))
            .await
            .unwrap();
        assert!(
            cache
                .load_messages(&acc, &inbox, MessageSort::Unread)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn save_messages_truncates_and_evicts_oldest_folder() {
        let cache = cache();
        let acc = AccountId::new("a");
        let oversized: Vec<_> = (1..=MAX_MESSAGES_PER_FOLDER + 10)
            .map(|i| envelope("INBOX", &i.to_string(), "x"))
            .collect();
        cache
            .save_messages(
                &acc,
                &CachedMessageList::new(
                    &MailboxId::from("INBOX".to_string()),
                    MessageSort::Arrival,
                    200,
                    None,
                    oversized,
                ),
            )
            .await
            .unwrap();
        let got = cache
            .load_messages(
                &acc,
                &MailboxId::from("INBOX".to_string()),
                MessageSort::Arrival,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.envelopes.len(), MAX_MESSAGES_PER_FOLDER);
        assert_eq!(got.total, 200);

        for i in 0..=MAX_CACHED_FOLDERS {
            cache
                .save_messages(
                    &acc,
                    &list_for(&format!("F{i}"), 1, 1, MessageSort::Arrival),
                )
                .await
                .unwrap();
        }
        let lists = cache.store().keys(STORE_FOLDER_LISTS).await.unwrap();
        assert_eq!(lists.len(), MAX_CACHED_FOLDERS);
        let f0 = folder_list_key("a", "F0");
        assert!(
            !lists.contains(&f0),
            "oldest folder list should be evicted: {lists:?}"
        );
        assert!(
            cache
                .store()
                .get(STORE_ENVELOPES, &envelope_key("a", "F0", "1"))
                .await
                .unwrap()
                .is_none(),
            "evicted folder envelopes should be gone"
        );
    }

    #[tokio::test]
    async fn parts_roundtrip_and_lru() {
        let cache = cache();
        let acc = AccountId::new("acc");
        let mid = |uid: &str| MessageId::new(FolderId::new("INBOX"), uid);

        cache
            .save_parts(
                &acc,
                &mid("1"),
                &[sample_part("INBOX", "1", "TEXT", "hello")],
            )
            .await
            .unwrap();
        let loaded = cache.load_parts(&acc, &mid("1")).await.unwrap().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].content, MessageContent::Text("hello".into()));
        assert!(
            cache
                .store()
                .get(STORE_PARTS, &part_key("acc", "INBOX", "1", "TEXT"))
                .await
                .unwrap()
                .is_some()
        );

        for i in 0..=MAX_CACHED_PART_MESSAGES {
            let uid = format!("u{i}");
            cache
                .save_parts(&acc, &mid(&uid), &[sample_part("INBOX", &uid, "TEXT", "x")])
                .await
                .unwrap();
        }
        assert!(
            cache.load_parts(&acc, &mid("1")).await.unwrap().is_none(),
            "oldest opened message parts should evict"
        );
        let last = format!("u{MAX_CACHED_PART_MESSAGES}");
        assert!(cache.load_parts(&acc, &mid(&last)).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn parts_evict_by_byte_budget() {
        let cache = cache();
        let acc = AccountId::new("acc");
        let big = "x".repeat(MAX_CACHED_PART_BYTES / 2 + 16);
        cache
            .save_parts(
                &acc,
                &MessageId::new(FolderId::new("INBOX"), "old"),
                &[sample_part("INBOX", "old", "TEXT", &big)],
            )
            .await
            .unwrap();
        cache
            .save_parts(
                &acc,
                &MessageId::new(FolderId::new("INBOX"), "new"),
                &[sample_part("INBOX", "new", "TEXT", &big)],
            )
            .await
            .unwrap();
        assert!(
            cache
                .load_parts(&acc, &MessageId::new(FolderId::new("INBOX"), "old"))
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            cache
                .load_parts(&acc, &MessageId::new(FolderId::new("INBOX"), "new"))
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn invalidate_delete_and_retain() {
        let cache = cache();
        let acc = AccountId::new("acc");
        let other = AccountId::new("other");
        let inbox = MailboxId::from("INBOX".to_string());
        cache
            .save_messages(&acc, &list_for("INBOX", 1, 1, MessageSort::Arrival))
            .await
            .unwrap();
        cache
            .save_parts(
                &acc,
                &MessageId::new(FolderId::new("INBOX"), "1"),
                &[sample_part("INBOX", "1", "TEXT", "body")],
            )
            .await
            .unwrap();
        cache
            .save_folders(
                &other,
                &CachedFolderTree::new(vec![], std::collections::HashMap::new()),
            )
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
        assert!(
            cache
                .load_parts(&acc, &MessageId::new(FolderId::new("INBOX"), "1"))
                .await
                .unwrap()
                .is_none()
        );

        cache
            .save_parts(
                &acc,
                &MessageId::new(FolderId::new("INBOX"), "2"),
                &[sample_part("INBOX", "2", "TEXT", "keep")],
            )
            .await
            .unwrap();
        cache
            .retain_accounts(&HashSet::from([other.clone()]))
            .await
            .unwrap();
        assert!(
            cache
                .load_parts(&acc, &MessageId::new(FolderId::new("INBOX"), "2"))
                .await
                .unwrap()
                .is_none()
        );
        assert!(cache.load_folders(&other).await.unwrap().is_some());

        cache.delete_account(&other).await.unwrap();
        assert!(cache.load_folders(&other).await.unwrap().is_none());
        assert!(cache.is_empty().await.unwrap());
    }

    #[tokio::test]
    async fn cached_loaded_message_helper_roundtrip() {
        let cache = cache();
        let acc = AccountId::new("acc");
        let id = MessageId::new(FolderId::new("INBOX"), "9");
        cache
            .save_parts(
                &acc,
                &id,
                &[sample_part("INBOX", "9", "TEXT", "offline body")],
            )
            .await
            .unwrap();
        let loaded = crate::mail_cache::load_cached_loaded_message(&cache, &acc, &id)
            .await
            .unwrap();
        assert_eq!(loaded.envelope_id, id);
        assert_eq!(
            loaded.parts[0].content,
            MessageContent::Text("offline body".into())
        );
    }

    #[tokio::test]
    async fn clear_all_empties_stores() {
        let cache = cache();
        let acc = AccountId::new("acc");
        cache
            .save_messages(&acc, &list_for("INBOX", 1, 1, MessageSort::Arrival))
            .await
            .unwrap();
        cache.clear_all().await.unwrap();
        assert!(cache.is_empty().await.unwrap());
    }
}
