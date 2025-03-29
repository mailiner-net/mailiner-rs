use crate::kernel::backend::BackendType;
use crate::kernel::backend::MockBackend;
use crate::kernel::model::{
    Account, AccountId, Folder, FolderId, Message, MessageContent, MessageId,
};
use crate::kernel::repository::{
    AccountRepository, CachedMessageRepository, FolderRepository, MessageRepository, MessageSort,
};
use crate::kernel::{MailinerError, Result};
use std::sync::Arc;

use super::cache::WebLocalStorage;

pub struct EmailService {
    account_repo: Arc<dyn AccountRepository>,
    folder_repo: Arc<dyn FolderRepository>,
    message_repo: Arc<dyn MessageRepository>,
}

impl EmailService {
    pub fn new(
        account_repo: Arc<dyn AccountRepository>,
        folder_repo: Arc<dyn FolderRepository>,
        message_repo: Arc<dyn MessageRepository>,
    ) -> Self {
        Self {
            account_repo,
            folder_repo,
            message_repo,
        }
    }

    pub async fn list_accounts(&self) -> Result<Vec<Account>> {
        self.account_repo.list_accounts().await
    }

    pub async fn list_folders(&self, account_id: &AccountId) -> Result<Vec<Folder>> {
        self.folder_repo.list_folders(account_id).await
    }

    pub async fn list_messages_paginated(
        &self,
        folder_id: &FolderId,
        page: usize,
        page_size: usize,
        sort: MessageSort,
    ) -> Result<Vec<Message>> {
        let offset = page * page_size;
        self.message_repo
            .list_messages(folder_id, offset, page_size, sort)
            .await
    }

    pub async fn get_message_with_content(
        &self,
        id: &MessageId,
    ) -> Result<Option<(Message, MessageContent)>> {
        let message = match self.message_repo.get_message(id).await? {
            Some(msg) => msg,
            None => return Ok(None),
        };

        let content = match self.message_repo.get_message_content(id).await? {
            Some(cnt) => cnt,
            None => return Ok(None),
        };

        Ok(Some((message, content)))
    }

    pub async fn mark_as_read(&self, id: &MessageId) -> Result<()> {
        let message = match self.message_repo.get_message(id).await? {
            Some(msg) => msg,
            None => return Err(MailinerError::NotFound),
        };

        let mut flags = message.flags.clone();
        if !flags.read {
            flags.read = true;
            self.message_repo.update_message_flags(id, flags).await?;
        }

        Ok(())
    }
}

pub struct EmailServiceFactory;

impl EmailServiceFactory {
    pub fn create_service(backend_type: BackendType, _config: &str) -> Result<Arc<EmailService>> {
        match backend_type {
            BackendType::Imap => todo!("IMAP backend not implemented"),
            BackendType::Jmap => todo!("JMAP backend not implemented"),
            BackendType::Mock => Self::create_mock_service(),
        }
    }

    fn create_mock_service() -> Result<Arc<EmailService>> {
        // Create a mock backend for testing
        let mock_backend = Arc::new(MockBackend::default());

        let cache = WebLocalStorage;

        let account_repo: Arc<dyn AccountRepository> = Arc::clone(&mock_backend) as Arc<dyn AccountRepository>;
        let folder_repo: Arc<dyn FolderRepository> = Arc::clone(&mock_backend) as Arc<dyn FolderRepository>;
        let message_repo: Arc<dyn MessageRepository> =
            Arc::new(CachedMessageRepository::new(mock_backend, cache));

        // Create the service
        let service = Arc::new(EmailService::new(account_repo, folder_repo, message_repo));

        Ok(service)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
}
