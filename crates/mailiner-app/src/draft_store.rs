//! Local compose drafts (`mailiner.drafts.v1`).
//!
//! One draft per account. Close keeps it; Send and Discard clear it.
//! Not the IMAP Drafts folder.

use std::collections::HashSet;

use base64::Engine;
use chrono::{DateTime, Utc};
use mailiner_composer::model::attachment::{AttachmentData, AttachmentId, FileAttachment};
use mailiner_composer::model::draft::{BodyMode, ComposerAddress, DraftDocument, DraftId};
use mailiner_core::ids::{AccountId, MessageId};
use serde::{Deserialize, Serialize};

#[cfg(target_arch = "wasm32")]
use crate::account_store::WebLocalStorage;
use crate::account_store::{AccountStoreError, StringKvStore};
use crate::send::ComposeSession;

/// `localStorage` key for the drafts blob (account / outbox schemas are untouched).
pub const DRAFTS_LOCAL_STORAGE_KEY: &str = "mailiner.drafts.v1";
/// Drafts blob schema (independent of the account store).
pub const DRAFT_STORE_SCHEMA_VERSION: u32 = 1;
/// Max encoded JSON blob size.
pub const MAX_DRAFT_BLOB_BYTES: usize = 1_500_000;
/// Skip persisting a single attachment larger than this (raw bytes).
const MAX_ATTACHMENT_PERSIST_BYTES: usize = 256 * 1024;
/// Cap on raw attachment bytes stored in the blob.
const MAX_ATTACHMENT_TOTAL_BYTES: usize = 750_000;

/// Single JSON document stored under [`DRAFTS_LOCAL_STORAGE_KEY`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DraftsBlob {
    pub schema_version: u32,
    pub drafts: Vec<PersistedComposeDraft>,
}

impl DraftsBlob {
    pub fn empty() -> Self {
        Self {
            schema_version: DRAFT_STORE_SCHEMA_VERSION,
            drafts: Vec::new(),
        }
    }

    pub fn encode(&self) -> Result<String, AccountStoreError> {
        let json = serde_json::to_string(self)
            .map_err(|e| AccountStoreError::Serialization(e.to_string()))?;
        if json.len() > MAX_DRAFT_BLOB_BYTES {
            return Err(AccountStoreError::Other(
                "Draft is too large to keep in this browser.".into(),
            ));
        }
        Ok(json)
    }

    /// Rejects blobs whose `schema_version` is greater than
    /// [`DRAFT_STORE_SCHEMA_VERSION`] so a future format is not rewritten as v1.
    pub fn decode(json: &str) -> Result<Self, AccountStoreError> {
        #[derive(Deserialize)]
        struct SchemaProbe {
            schema_version: u32,
        }
        let probe: SchemaProbe = serde_json::from_str(json)
            .map_err(|e| AccountStoreError::Serialization(e.to_string()))?;
        if probe.schema_version > DRAFT_STORE_SCHEMA_VERSION {
            return Err(AccountStoreError::Serialization(format!(
                "unsupported draft schema_version {} (max supported {})",
                probe.schema_version, DRAFT_STORE_SCHEMA_VERSION
            )));
        }
        serde_json::from_str(json).map_err(|e| AccountStoreError::Serialization(e.to_string()))
    }

    fn get(&self, account_id: &AccountId) -> Option<&PersistedComposeDraft> {
        self.drafts.iter().find(|d| d.account_id == *account_id)
    }

    fn upsert(&mut self, draft: PersistedComposeDraft) {
        if let Some(existing) = self
            .drafts
            .iter_mut()
            .find(|d| d.account_id == draft.account_id)
        {
            *existing = draft;
        } else {
            self.drafts.push(draft);
        }
        self.schema_version = DRAFT_STORE_SCHEMA_VERSION;
    }

    fn remove(&mut self, account_id: &AccountId) -> bool {
        let before = self.drafts.len();
        self.drafts.retain(|d| d.account_id != *account_id);
        self.schema_version = DRAFT_STORE_SCHEMA_VERSION;
        self.drafts.len() != before
    }

    fn retain_accounts(&mut self, known: &HashSet<AccountId>) {
        self.drafts.retain(|d| known.contains(&d.account_id));
        self.schema_version = DRAFT_STORE_SCHEMA_VERSION;
    }
}

/// One account's local compose draft. No passwords.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedComposeDraft {
    pub account_id: AccountId,
    pub title: String,
    pub draft_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<PersistedAddress>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub to: Vec<PersistedAddress>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cc: Vec<PersistedAddress>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bcc: Vec<PersistedAddress>,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub mode: PersistedBodyMode,
    #[serde(default)]
    pub plain_body: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub html_body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_reply_to: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<PersistedAttachment>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Source message to mark `\Answered` after a successful Reply / Reply All.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_source: Option<MessageId>,
    /// IMAP Drafts message this local draft last saved as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imap_draft: Option<MessageId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedAddress {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub email: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PersistedBodyMode {
    #[default]
    Plain,
    Rich,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedAttachment {
    pub id: String,
    pub filename: String,
    pub content_type: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_b64: Option<String>,
}

impl From<BodyMode> for PersistedBodyMode {
    fn from(mode: BodyMode) -> Self {
        match mode {
            BodyMode::Plain => Self::Plain,
            BodyMode::Rich => Self::Rich,
        }
    }
}

impl From<PersistedBodyMode> for BodyMode {
    fn from(mode: PersistedBodyMode) -> Self {
        match mode {
            PersistedBodyMode::Plain => Self::Plain,
            PersistedBodyMode::Rich => Self::Rich,
        }
    }
}

impl From<&ComposerAddress> for PersistedAddress {
    fn from(addr: &ComposerAddress) -> Self {
        Self {
            name: addr.name.clone(),
            email: addr.email.clone(),
        }
    }
}

impl From<&PersistedAddress> for ComposerAddress {
    fn from(addr: &PersistedAddress) -> Self {
        Self {
            name: addr.name.clone(),
            email: addr.email.clone(),
        }
    }
}

/// Recipients, subject, body, attachments, or reply threading.
pub fn session_has_content(session: &ComposeSession) -> bool {
    let d = &session.draft;
    !d.to.is_empty()
        || !d.cc.is_empty()
        || !d.bcc.is_empty()
        || !d.subject.trim().is_empty()
        || !d.plain_body.trim().is_empty()
        || !d.html_body.trim().is_empty()
        || !d.attachments.is_empty()
        || !d.inline_images.is_empty()
        || d.in_reply_to.is_some()
        || !d.references.is_empty()
}

fn persistable_attachments(attachments: &[FileAttachment]) -> Vec<PersistedAttachment> {
    let mut out = Vec::with_capacity(attachments.len());
    let mut used = 0usize;
    for a in attachments {
        let AttachmentData::Bytes(bytes) = &a.data else {
            continue;
        };
        if bytes.len() > MAX_ATTACHMENT_PERSIST_BYTES {
            continue;
        }
        if used.saturating_add(bytes.len()) > MAX_ATTACHMENT_TOTAL_BYTES {
            continue;
        }
        used = used.saturating_add(bytes.len());
        out.push(PersistedAttachment {
            id: a.id.0.clone(),
            filename: a.filename.clone(),
            content_type: a.content_type.clone(),
            size: a.size,
            data_b64: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
        });
    }
    out
}

fn restore_attachments(persisted: &[PersistedAttachment]) -> Vec<FileAttachment> {
    let mut out = Vec::with_capacity(persisted.len());
    for a in persisted {
        let Some(b64) = a.data_b64.as_deref() else {
            continue;
        };
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) else {
            continue;
        };
        out.push(FileAttachment {
            id: AttachmentId(a.id.clone()),
            filename: a.filename.clone(),
            content_type: a.content_type.clone(),
            size: if a.size == 0 {
                bytes.len() as u64
            } else {
                a.size
            },
            data: AttachmentData::Bytes(bytes),
            source: None,
        });
    }
    out
}

impl PersistedComposeDraft {
    pub fn from_session(account_id: AccountId, session: &ComposeSession) -> Self {
        let d = &session.draft;
        Self {
            account_id,
            title: session.title.clone(),
            draft_id: d.id.as_str().to_string(),
            from: d.from.as_ref().map(PersistedAddress::from),
            to: d.to.iter().map(PersistedAddress::from).collect(),
            cc: d.cc.iter().map(PersistedAddress::from).collect(),
            bcc: d.bcc.iter().map(PersistedAddress::from).collect(),
            subject: d.subject.clone(),
            mode: PersistedBodyMode::from(d.mode),
            plain_body: d.plain_body.clone(),
            html_body: d.html_body.clone(),
            in_reply_to: d.in_reply_to.clone(),
            references: d.references.clone(),
            attachments: persistable_attachments(&d.attachments),
            created_at: d.created_at,
            updated_at: d.updated_at,
            reply_source: session.reply_source.clone(),
            imap_draft: session.imap_draft.clone(),
        }
    }

    pub fn into_session(self) -> ComposeSession {
        ComposeSession {
            account_id: self.account_id,
            title: self.title,
            reply_source: self.reply_source,
            imap_draft: self.imap_draft,
            draft: DraftDocument {
                id: DraftId(self.draft_id),
                from: self.from.as_ref().map(ComposerAddress::from),
                to: self.to.iter().map(ComposerAddress::from).collect(),
                cc: self.cc.iter().map(ComposerAddress::from).collect(),
                bcc: self.bcc.iter().map(ComposerAddress::from).collect(),
                subject: self.subject,
                mode: BodyMode::from(self.mode),
                plain_body: self.plain_body,
                html_body: self.html_body,
                plain_cache_dirty: false,
                attachments: restore_attachments(&self.attachments),
                inline_images: Vec::new(),
                in_reply_to: self.in_reply_to,
                references: self.references,
                prefill_warnings: Vec::new(),
                created_at: self.created_at,
                updated_at: self.updated_at,
            },
            stashed_originals: Vec::new(),
        }
    }
}

fn is_future_schema_error(err: &AccountStoreError) -> bool {
    matches!(
        err,
        AccountStoreError::Serialization(msg) if msg.contains("unsupported") && msg.contains("schema_version")
    )
}

fn load_blob(kv: &dyn StringKvStore) -> Result<Option<DraftsBlob>, AccountStoreError> {
    match kv.get_item(DRAFTS_LOCAL_STORAGE_KEY)? {
        None => Ok(Some(DraftsBlob::empty())),
        Some(s) if s.trim().is_empty() => Ok(Some(DraftsBlob::empty())),
        Some(s) => match DraftsBlob::decode(&s) {
            Ok(blob) => Ok(Some(blob)),
            Err(err) if is_future_schema_error(&err) => Ok(None),
            Err(_) => Ok(Some(DraftsBlob::empty())),
        },
    }
}

fn save_blob(kv: &dyn StringKvStore, blob: &DraftsBlob) -> Result<(), AccountStoreError> {
    kv.set_item(DRAFTS_LOCAL_STORAGE_KEY, &blob.encode()?)
}

fn load_draft_in(
    kv: &dyn StringKvStore,
    account_id: &AccountId,
) -> Result<Option<ComposeSession>, AccountStoreError> {
    let Some(blob) = load_blob(kv)? else {
        return Ok(None);
    };
    Ok(blob
        .get(account_id)
        .cloned()
        .map(PersistedComposeDraft::into_session))
}

fn save_draft_in(
    kv: &dyn StringKvStore,
    account_id: &AccountId,
    session: &ComposeSession,
) -> Result<(), AccountStoreError> {
    let Some(mut blob) = load_blob(kv)? else {
        return Ok(());
    };
    if !session_has_content(session) {
        blob.remove(account_id);
        return save_blob(kv, &blob);
    }
    blob.upsert(PersistedComposeDraft::from_session(
        account_id.clone(),
        session,
    ));
    match save_blob(kv, &blob) {
        Ok(()) => Ok(()),
        Err(AccountStoreError::Other(_)) => {
            let Some(draft) = blob.drafts.iter_mut().find(|d| d.account_id == *account_id) else {
                return Err(AccountStoreError::Other(
                    "Draft is too large to keep in this browser.".into(),
                ));
            };
            if draft.attachments.is_empty() {
                return Err(AccountStoreError::Other(
                    "Draft is too large to keep in this browser.".into(),
                ));
            }
            draft.attachments.clear();
            save_blob(kv, &blob)
        }
        Err(e) => Err(e),
    }
}

fn clear_draft_in(kv: &dyn StringKvStore, account_id: &AccountId) -> Result<(), AccountStoreError> {
    let Some(mut blob) = load_blob(kv)? else {
        return Ok(());
    };
    if blob.remove(account_id) {
        save_blob(kv, &blob)?;
    }
    Ok(())
}

fn retain_drafts_in(
    kv: &dyn StringKvStore,
    known: &HashSet<AccountId>,
) -> Result<(), AccountStoreError> {
    let Some(mut blob) = load_blob(kv)? else {
        return Ok(());
    };
    blob.retain_accounts(known);
    save_blob(kv, &blob)
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

/// Saved compose session for `account_id`, if any.
pub fn load_draft(account_id: &AccountId) -> Option<ComposeSession> {
    with_kv(|kv| load_draft_in(kv, account_id))?
}

/// Persist the open draft. Empty drafts are removed. Failures are ignored.
pub fn save_draft(account_id: &AccountId, session: &ComposeSession) {
    let _ = with_kv(|kv| save_draft_in(kv, account_id, session));
}

/// Drop the local draft for `account_id`.
pub fn clear_draft(account_id: &AccountId) {
    let _ = with_kv(|kv| clear_draft_in(kv, account_id));
}

/// Drop the local draft only if it is still `draft_id` (do not clobber a newer one).
pub fn clear_draft_if(account_id: &AccountId, draft_id: &str) {
    let _ = with_kv(|kv| {
        let Some(mut blob) = load_blob(kv)? else {
            return Ok(());
        };
        let same = blob.get(account_id).is_some_and(|d| d.draft_id == draft_id);
        if same {
            blob.remove(account_id);
            save_blob(kv, &blob)?;
        }
        Ok(())
    });
}

/// Drop drafts for accounts that are no longer known.
pub fn retain_drafts(known: &HashSet<AccountId>) {
    let _ = with_kv(|kv| retain_drafts_in(kv, known));
}

/// Record the IMAP Drafts UID on the saved local draft, if it is still `draft_id`.
pub fn set_imap_draft(account_id: &AccountId, draft_id: &str, imap_draft: Option<MessageId>) {
    let _ = with_kv(|kv| {
        let Some(mut blob) = load_blob(kv)? else {
            return Ok(());
        };
        let Some(draft) = blob.drafts.iter_mut().find(|d| d.account_id == *account_id) else {
            return Ok(());
        };
        if draft.draft_id != draft_id {
            return Ok(());
        }
        draft.imap_draft = imap_draft;
        save_blob(kv, &blob)
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
    use crate::account_store::MemoryKvStore;
    use mailiner_composer::identity::FromIdentity;

    fn session(subject: &str, body: &str) -> ComposeSession {
        session_for(AccountId::new("acc"), subject, body)
    }

    fn session_for(account_id: AccountId, subject: &str, body: &str) -> ComposeSession {
        let id = FromIdentity::new("Me", "me@example.com");
        let mut draft = DraftDocument::new_empty(&id);
        draft.mode = BodyMode::Plain;
        draft.subject = subject.into();
        draft.plain_body = body.into();
        draft
            .to
            .push(ComposerAddress::email_only("you@example.com"));
        ComposeSession {
            account_id,
            title: "New message".into(),
            draft,
            reply_source: None,
            imap_draft: None,
            stashed_originals: Vec::new(),
        }
    }

    #[test]
    fn storage_key_is_versioned() {
        assert_eq!(DRAFTS_LOCAL_STORAGE_KEY, "mailiner.drafts.v1");
        assert_eq!(DRAFT_STORE_SCHEMA_VERSION, 1);
    }

    #[test]
    fn blob_encode_decode_roundtrip() {
        let account = AccountId::new("acc-1");
        let session = session("Hello", "Body text");
        let mut blob = DraftsBlob::empty();
        blob.upsert(PersistedComposeDraft::from_session(
            account.clone(),
            &session,
        ));

        let json = blob.encode().expect("encode");
        assert!(json.contains("\"schema_version\":1"), "json={json}");
        assert!(json.contains("Hello"), "json={json}");
        assert!(json.contains("Body text"), "json={json}");
        assert!(!json.to_lowercase().contains("password"), "json={json}");

        let back = DraftsBlob::decode(&json).expect("decode");
        assert_eq!(back, blob);
        let restored = back.get(&account).cloned().unwrap().into_session();
        assert_eq!(restored.title, session.title);
        assert_eq!(restored.draft.id.as_str(), session.draft.id.as_str());
        assert_eq!(restored.draft.subject, "Hello");
        assert_eq!(restored.draft.plain_body, "Body text");
        assert_eq!(restored.draft.to[0].email, "you@example.com");
        assert_eq!(restored.draft.mode, BodyMode::Plain);
        assert_eq!(
            restored.draft.from.as_ref().map(|f| f.email.as_str()),
            Some("me@example.com")
        );
        assert_eq!(
            restored.draft.from.as_ref().and_then(|f| f.name.as_deref()),
            Some("Me")
        );
    }

    #[test]
    fn persisted_from_roundtrip_keeps_alias() {
        let account = AccountId::new("acc");
        let mut session = session("Hi", "body");
        session.draft.from = Some(ComposerAddress {
            name: Some("Support".into()),
            email: "support@example.com".into(),
        });
        let persisted = PersistedComposeDraft::from_session(account, &session);
        assert_eq!(
            persisted.from.as_ref().map(|f| f.email.as_str()),
            Some("support@example.com")
        );
        let restored = persisted.into_session();
        let from = restored.draft.from.expect("from");
        assert_eq!(from.email, "support@example.com");
        assert_eq!(from.name.as_deref(), Some("Support"));
    }

    #[test]
    fn blob_decode_rejects_future_schema() {
        let json = r#"{"schema_version":99,"drafts":[]}"#;
        let err = DraftsBlob::decode(json).unwrap_err();
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
    fn future_schema_is_not_overwritten() {
        let kv = MemoryKvStore::new();
        let future = r#"{"schema_version":99,"drafts":[{"keep":true}]}"#;
        kv.set_item(DRAFTS_LOCAL_STORAGE_KEY, future).unwrap();
        let account = AccountId::new("acc");
        save_draft_in(&kv, &account, &session("Hi", "x")).unwrap();
        assert_eq!(
            kv.get_item(DRAFTS_LOCAL_STORAGE_KEY).unwrap().as_deref(),
            Some(future)
        );
        assert!(load_draft_in(&kv, &account).unwrap().is_none());
    }

    #[test]
    fn empty_session_is_not_kept() {
        let kv = MemoryKvStore::new();
        let account = AccountId::new("acc");
        let id = FromIdentity::new("Me", "me@example.com");
        let mut draft = DraftDocument::new_empty(&id);
        draft.mode = BodyMode::Plain;
        let empty = ComposeSession {
            account_id: account.clone(),
            title: "New message".into(),
            draft,
            reply_source: None,
            imap_draft: None,
            stashed_originals: Vec::new(),
        };
        assert!(!session_has_content(&empty));
        save_draft_in(&kv, &account, &session("Hi", "x")).unwrap();
        save_draft_in(&kv, &account, &empty).unwrap();
        assert!(load_draft_in(&kv, &account).unwrap().is_none());
    }

    #[test]
    fn attachments_round_trip() {
        let account = AccountId::new("acc");
        let mut s = session("File", "see attached");
        s.draft.attachments.push(FileAttachment {
            id: AttachmentId("att-1".into()),
            filename: "note.txt".into(),
            content_type: "text/plain".into(),
            size: 5,
            data: AttachmentData::Bytes(b"hello".to_vec()),
            source: None,
        });
        let persisted = PersistedComposeDraft::from_session(account, &s);
        assert_eq!(persisted.attachments.len(), 1);
        assert!(persisted.attachments[0].data_b64.is_some());
        let restored = persisted.into_session();
        assert_eq!(restored.draft.attachments.len(), 1);
        assert_eq!(restored.draft.attachments[0].filename, "note.txt");
        match &restored.draft.attachments[0].data {
            AttachmentData::Bytes(b) => assert_eq!(b, b"hello"),
            AttachmentData::Pending => panic!("expected bytes"),
        }
    }

    #[test]
    fn oversized_attachment_is_dropped() {
        let account = AccountId::new("acc");
        let mut s = session("File", "see attached");
        s.draft.attachments.push(FileAttachment {
            id: AttachmentId("big".into()),
            filename: "big.bin".into(),
            content_type: "application/octet-stream".into(),
            size: (MAX_ATTACHMENT_PERSIST_BYTES + 1) as u64,
            data: AttachmentData::Bytes(vec![0u8; MAX_ATTACHMENT_PERSIST_BYTES + 1]),
            source: None,
        });
        let persisted = PersistedComposeDraft::from_session(account, &s);
        assert!(persisted.attachments.is_empty());
    }

    #[test]
    fn pending_attachment_is_not_persisted() {
        let account = AccountId::new("acc");
        let mut s = session("File", "x");
        s.draft.attachments.push(FileAttachment {
            id: AttachmentId("pending".into()),
            filename: "soon.bin".into(),
            content_type: "application/octet-stream".into(),
            size: 0,
            data: AttachmentData::Pending,
            source: None,
        });
        let persisted = PersistedComposeDraft::from_session(account, &s);
        assert!(persisted.attachments.is_empty());
    }

    #[test]
    fn host_load_save_clear_roundtrip() {
        host_kv::reset();
        let acc = AccountId::new("host-acc");
        assert!(load_draft(&acc).is_none());

        save_draft(&acc, &session("Subj", "Hello"));
        let back = load_draft(&acc).expect("saved");
        assert_eq!(back.draft.subject, "Subj");
        assert_eq!(back.draft.plain_body, "Hello");

        clear_draft(&acc);
        assert!(load_draft(&acc).is_none());
        host_kv::reset();
    }

    #[test]
    fn imap_draft_id_roundtrips() {
        host_kv::reset();
        let acc = AccountId::new("host-acc");
        let mut s = session("Subj", "Hello");
        let mid = MessageId::new(mailiner_core::FolderId::new("Drafts"), "42");
        s.imap_draft = Some(mid.clone());
        save_draft(&acc, &s);
        let back = load_draft(&acc).expect("saved");
        assert_eq!(back.imap_draft, Some(mid.clone()));
        set_imap_draft(&acc, back.draft.id.as_str(), None);
        let cleared = load_draft(&acc).expect("still saved");
        assert!(cleared.imap_draft.is_none());
        set_imap_draft(&acc, back.draft.id.as_str(), Some(mid.clone()));
        assert_eq!(load_draft(&acc).and_then(|d| d.imap_draft), Some(mid));
        host_kv::reset();
    }

    #[test]
    fn retain_drops_unknown_accounts() {
        host_kv::reset();
        save_draft(&AccountId::new("keep"), &session("A", "a"));
        save_draft(&AccountId::new("gone"), &session("B", "b"));
        retain_drafts(&HashSet::from([AccountId::new("keep")]));
        assert!(load_draft(&AccountId::new("keep")).is_some());
        assert!(load_draft(&AccountId::new("gone")).is_none());
        host_kv::reset();
    }

    #[test]
    fn reply_headers_round_trip() {
        let account = AccountId::new("acc");
        let mut s = session("Re: Hello", "> quoted");
        s.title = "Reply".into();
        s.draft.in_reply_to = Some("<id@example.com>".into());
        s.draft.references = vec!["<root@example.com>".into(), "<id@example.com>".into()];
        s.draft
            .cc
            .push(ComposerAddress::email_only("cc@example.com"));
        let json = {
            let mut blob = DraftsBlob::empty();
            blob.upsert(PersistedComposeDraft::from_session(account.clone(), &s));
            blob.encode().unwrap()
        };
        let restored = DraftsBlob::decode(&json)
            .unwrap()
            .get(&account)
            .cloned()
            .unwrap()
            .into_session();
        assert_eq!(restored.title, "Reply");
        assert_eq!(
            restored.draft.in_reply_to.as_deref(),
            Some("<id@example.com>")
        );
        assert_eq!(restored.draft.references.len(), 2);
        assert_eq!(restored.draft.cc[0].email, "cc@example.com");
    }

    #[test]
    fn drafts_are_per_account() {
        let kv = MemoryKvStore::new();
        save_draft_in(&kv, &AccountId::new("a"), &session("A", "aa")).unwrap();
        save_draft_in(&kv, &AccountId::new("b"), &session("B", "bb")).unwrap();
        assert_eq!(
            load_draft_in(&kv, &AccountId::new("a"))
                .unwrap()
                .unwrap()
                .draft
                .subject,
            "A"
        );
        assert_eq!(
            load_draft_in(&kv, &AccountId::new("b"))
                .unwrap()
                .unwrap()
                .draft
                .subject,
            "B"
        );
        clear_draft_in(&kv, &AccountId::new("a")).unwrap();
        assert!(load_draft_in(&kv, &AccountId::new("a")).unwrap().is_none());
        assert!(load_draft_in(&kv, &AccountId::new("b")).unwrap().is_some());
    }

    #[test]
    fn persisted_from_is_restored() {
        let account = AccountId::new("acc");
        let mut s = session("Hi", "x");
        s.draft.from = Some(ComposerAddress {
            name: Some("Old".into()),
            email: "old@example.com".into(),
        });
        let persisted = PersistedComposeDraft::from_session(account, &s);
        let from = persisted.from.as_ref().expect("from");
        assert_eq!(from.email, "old@example.com");
        assert_eq!(from.name.as_deref(), Some("Old"));
        let restored = persisted.into_session().draft.from.expect("from");
        assert_eq!(restored.email, "old@example.com");
        assert_eq!(restored.name.as_deref(), Some("Old"));
    }

    #[test]
    fn decode_restores_legacy_from() {
        let json = r#"{"schema_version":1,"drafts":[{"account_id":"acc","title":"New message","draft_id":"d1","from":{"name":"Old","email":"old@example.com"},"to":[{"email":"you@example.com"}],"subject":"Hi","mode":"plain","plain_body":"x","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}]}"#;
        let restored = DraftsBlob::decode(json)
            .unwrap()
            .drafts
            .remove(0)
            .into_session();
        let from = restored.draft.from.expect("from");
        assert_eq!(from.email, "old@example.com");
        assert_eq!(from.name.as_deref(), Some("Old"));
        assert_eq!(restored.draft.subject, "Hi");
    }

    #[test]
    fn clear_draft_if_leaves_newer_id() {
        host_kv::reset();
        let acc = AccountId::new("acc");
        let first = session("A", "a");
        let first_id = first.draft.id.as_str().to_string();
        save_draft(&acc, &first);
        save_draft(&acc, &session("B", "b"));
        clear_draft_if(&acc, &first_id);
        assert_eq!(load_draft(&acc).unwrap().draft.subject, "B");
        let current = load_draft(&acc).unwrap().draft.id.as_str().to_string();
        clear_draft_if(&acc, &current);
        assert!(load_draft(&acc).is_none());
        host_kv::reset();
    }

    #[test]
    fn oversize_strips_only_the_draft_being_written() {
        let kv = MemoryKvStore::new();
        let a = AccountId::new("a");
        let b = AccountId::new("b");
        let attach = |id: &str, n: usize| FileAttachment {
            id: AttachmentId(id.into()),
            filename: format!("{id}.bin"),
            content_type: "application/octet-stream".into(),
            size: n as u64,
            data: AttachmentData::Bytes(vec![b'x'; n]),
            source: None,
        };
        // 256 + 256 + 238 KiB stays under the per-draft persist cap but two
        // such drafts exceed the shared blob cap.
        let fill = |s: &mut ComposeSession, prefix: &str| {
            s.draft
                .attachments
                .push(attach(&format!("{prefix}-1"), 256 * 1024));
            s.draft
                .attachments
                .push(attach(&format!("{prefix}-2"), 256 * 1024));
            s.draft
                .attachments
                .push(attach(&format!("{prefix}-3"), 220 * 1024));
        };
        let mut sa = session_for(a.clone(), "A", "aa");
        fill(&mut sa, "a");
        save_draft_in(&kv, &a, &sa).unwrap();
        assert_eq!(
            load_draft_in(&kv, &a)
                .unwrap()
                .unwrap()
                .draft
                .attachments
                .len(),
            3
        );

        let mut sb = session_for(b.clone(), "B", "bb");
        fill(&mut sb, "b");
        save_draft_in(&kv, &b, &sb).unwrap();
        let back_a = load_draft_in(&kv, &a).unwrap().unwrap();
        let back_b = load_draft_in(&kv, &b).unwrap().unwrap();
        assert_eq!(back_a.draft.attachments.len(), 3);
        assert_eq!(back_b.draft.attachments.len(), 0);
        assert_eq!(back_b.draft.subject, "B");
    }

    #[test]
    fn corrupt_blob_is_replaced_on_save() {
        let kv = MemoryKvStore::new();
        kv.set_item(DRAFTS_LOCAL_STORAGE_KEY, "not-json").unwrap();
        let account = AccountId::new("acc");
        save_draft_in(&kv, &account, &session("Hi", "x")).unwrap();
        let back = load_draft_in(&kv, &account).unwrap().unwrap();
        assert_eq!(back.draft.subject, "Hi");
    }
}
