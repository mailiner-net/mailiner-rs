use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::error::{MailinerError, Result};
use crate::ids::{AccountId, FolderId, MessageId, MessagePartId};
use crate::models::{Account, AccountMetadata, Envelope, Folder, FolderMetadata, MessagePart};

#[async_trait]
pub trait Storage: Send + Sync {
    // Account operations
    async fn save_account(&self, account: &Account) -> Result<()>;
    async fn get_account(&self, id: &AccountId) -> Result<Account>;
    async fn list_accounts(&self) -> Result<Vec<Account>>;
    async fn delete_account(&self, id: &AccountId) -> Result<()>;

    // Folder operations
    async fn save_folder(&self, folder: &Folder) -> Result<()>;
    async fn get_folder(&self, id: &FolderId) -> Result<Folder>;
    async fn list_folders(&self, account_id: &AccountId) -> Result<Vec<Folder>>;
    async fn delete_folder(&self, id: &FolderId) -> Result<()>;

    // Envelope operations
    async fn save_envelope(&self, envelope: &Envelope) -> Result<()>;
    async fn get_envelope(&self, id: &MessageId) -> Result<Envelope>;
    async fn list_envelopes(&self, folder_id: &FolderId) -> Result<Vec<Envelope>>;
    async fn delete_envelope(&self, id: &MessageId) -> Result<()>;
    async fn update_envelope_flags(&self, id: &MessageId, flags: &[(&str, bool)]) -> Result<()>;

    // Message part operations
    async fn save_message_part(&self, part: &MessagePart) -> Result<()>;
    async fn get_message_part(&self, id: &MessagePartId) -> Result<MessagePart>;
    async fn list_message_parts(&self, envelope_id: &MessageId) -> Result<Vec<MessagePart>>;
    async fn delete_message_part(&self, id: &MessagePartId) -> Result<()>;

    // Metadata operations
    async fn save_account_metadata(&self, metadata: &AccountMetadata) -> Result<()>;
    async fn get_account_metadata(&self, account_id: &AccountId) -> Result<AccountMetadata>;
    async fn save_folder_metadata(&self, metadata: &FolderMetadata) -> Result<()>;
    async fn get_folder_metadata(&self, folder_id: &FolderId) -> Result<FolderMetadata>;
}

// In-memory implementation for testing
pub struct InMemoryStorage {
    accounts: Arc<RwLock<HashMap<AccountId, Account>>>,
    folders: Arc<RwLock<HashMap<FolderId, Folder>>>,
    envelopes: Arc<RwLock<HashMap<MessageId, Envelope>>>,
    message_parts: Arc<RwLock<HashMap<MessagePartId, MessagePart>>>,
    account_metadata: Arc<RwLock<HashMap<AccountId, AccountMetadata>>>,
    folder_metadata: Arc<RwLock<HashMap<FolderId, FolderMetadata>>>,
}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self {
            accounts: Arc::new(RwLock::new(HashMap::new())),
            folders: Arc::new(RwLock::new(HashMap::new())),
            envelopes: Arc::new(RwLock::new(HashMap::new())),
            message_parts: Arc::new(RwLock::new(HashMap::new())),
            account_metadata: Arc::new(RwLock::new(HashMap::new())),
            folder_metadata: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl Storage for InMemoryStorage {
    async fn save_account(&self, account: &Account) -> Result<()> {
        self.accounts.write().await.insert(account.id.clone(), account.clone());
        Ok(())
    }

    async fn get_account(&self, id: &AccountId) -> Result<Account> {
        self.accounts.read().await.get(id).cloned().ok_or_else(|| MailinerError::NotFound(format!("Account {}", id)))
    }

    async fn list_accounts(&self) -> Result<Vec<Account>> {
        Ok(self.accounts.read().await.values().cloned().collect())
    }

    async fn delete_account(&self, id: &AccountId) -> Result<()> {
        self.accounts.write().await.remove(id).ok_or_else(|| MailinerError::NotFound(format!("Account {}", id)))?;
        Ok(())
    }

    async fn save_folder(&self, folder: &Folder) -> Result<()> {
        self.folders.write().await.insert(folder.id.clone(), folder.clone());
        Ok(())
    }

    async fn get_folder(&self, id: &FolderId) -> Result<Folder> {
        self.folders.read().await.get(id).cloned().ok_or_else(|| MailinerError::NotFound(format!("Folder {}", id)))
    }

    async fn list_folders(&self, account_id: &AccountId) -> Result<Vec<Folder>> {
        Ok(self.folders.read().await.values().filter(|f| f.account_id == *account_id).cloned().collect())
    }

    async fn delete_folder(&self, id: &FolderId) -> Result<()> {
        self.folders.write().await.remove(id).ok_or_else(|| MailinerError::NotFound(format!("Folder {}", id)))?;
        Ok(())
    }

    async fn save_envelope(&self, envelope: &Envelope) -> Result<()> {
        self.envelopes.write().await.insert(envelope.id.clone(), envelope.clone());
        Ok(())
    }

    async fn get_envelope(&self, id: &MessageId) -> Result<Envelope> {
        self.envelopes.read().await.get(id).cloned().ok_or_else(|| MailinerError::NotFound(format!("Envelope {}", id)))
    }

    async fn list_envelopes(&self, folder_id: &FolderId) -> Result<Vec<Envelope>> {
        Ok(self.envelopes.read().await.values().filter(|e| e.folder_id == *folder_id).cloned().collect())
    }

    async fn delete_envelope(&self, id: &MessageId) -> Result<()> {
        self.envelopes.write().await.remove(id).ok_or_else(|| MailinerError::NotFound(format!("Envelope {}", id)))?;
        Ok(())
    }

    async fn update_envelope_flags(&self, id: &MessageId, flags: &[(&str, bool)]) -> Result<()> {
        let mut envelopes = self.envelopes.write().await;
        let envelope = envelopes.get_mut(id).ok_or_else(|| MailinerError::NotFound(format!("Envelope {}", id)))?;
        
        for (flag, value) in flags {
            match *flag {
                "is_read" => envelope.is_read = *value,
                "is_starred" => envelope.is_starred = *value,
                "is_flagged" => envelope.is_flagged = *value,
                "is_draft" => envelope.is_draft = *value,
                "is_deleted" => envelope.is_deleted = *value,
                _ => return Err(MailinerError::InvalidData(format!("Unknown flag: {}", flag))),
            }
        }
        Ok(())
    }

    async fn save_message_part(&self, part: &MessagePart) -> Result<()> {
        self.message_parts.write().await.insert(part.id.clone(), part.clone());
        Ok(())
    }

    async fn get_message_part(&self, id: &MessagePartId) -> Result<MessagePart> {
        self.message_parts.read().await.get(id).cloned().ok_or_else(|| MailinerError::NotFound(format!("Message part {}", id)))
    }

    async fn list_message_parts(&self, envelope_id: &MessageId) -> Result<Vec<MessagePart>> {
        Ok(self.message_parts.read().await.values().filter(|p| p.envelope_id == *envelope_id).cloned().collect())
    }

    async fn delete_message_part(&self, id: &MessagePartId) -> Result<()> {
        self.message_parts.write().await.remove(id).ok_or_else(|| MailinerError::NotFound(format!("Message part {}", id)))?;
        Ok(())
    }

    async fn save_account_metadata(&self, metadata: &AccountMetadata) -> Result<()> {
        self.account_metadata.write().await.insert(metadata.id.clone(), metadata.clone());
        Ok(())
    }

    async fn get_account_metadata(&self, account_id: &AccountId) -> Result<AccountMetadata> {
        self.account_metadata.read().await.get(account_id).cloned().ok_or_else(|| MailinerError::NotFound(format!("Account metadata {}", account_id)))
    }

    async fn save_folder_metadata(&self, metadata: &FolderMetadata) -> Result<()> {
        self.folder_metadata.write().await.insert(metadata.id.clone(), metadata.clone());
        Ok(())
    }

    async fn get_folder_metadata(&self, folder_id: &FolderId) -> Result<FolderMetadata> {
        self.folder_metadata.read().await.get(folder_id).cloned().ok_or_else(|| MailinerError::NotFound(format!("Folder metadata {}", folder_id)))
    }
} 