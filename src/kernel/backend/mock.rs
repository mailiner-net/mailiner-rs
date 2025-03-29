use crate::kernel::model::{
    Account, AccountId, Folder, FolderId, FolderType, Message, MessageContent, MessageFlags,
    MessageId,
};
use crate::kernel::repository::{
    AccountRepository, FolderRepository, MessageRepository, MessageSort, SyncResult,
};
use crate::kernel::Result;
use async_trait::async_trait;
use std::{collections::HashMap, sync::RwLock};

#[derive(Debug)]
pub struct MockBackend {
    accounts: RwLock<HashMap<AccountId, Account>>,
    folders: RwLock<HashMap<FolderId, Folder>>,
    messages: RwLock<HashMap<MessageId, Message>>,
    flags: RwLock<HashMap<MessageId, MessageFlags>>,
    contents: RwLock<HashMap<MessageId, MessageContent>>,
}

impl Default for MockBackend {
    fn default() -> Self {
        Self {
            accounts: RwLock::new(HashMap::from([(
                AccountId("1".into()),
                Account {
                    id: AccountId("1".into()),
                    name: "Test Account".into(),
                    email: "test@example.com".into(),
                    backend_type: super::BackendType::Mock,
                },
            )])),
            folders: RwLock::new(HashMap::from([(
                FolderId("INBOX".into()),
                Folder {
                    id: FolderId("INBOX".into()),
                    account_id: AccountId("1".into()),
                    name: "Inbox".into(),
                    unread_count: 0,
                    total_messages: 0,
                    folder_type: FolderType::Inbox,
                },
            )])),
            messages: RwLock::new(HashMap::new()),
            flags: RwLock::new(HashMap::new()),
            contents: RwLock::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl AccountRepository for MockBackend {
    async fn list_accounts(&self) -> Result<Vec<Account>> {
        Ok(self.accounts.read().unwrap().values().cloned().collect())
    }

    async fn get_account(&self, id: &AccountId) -> Result<Option<Account>> {
        Ok(self.accounts.read().unwrap().get(id).cloned())
    }

    async fn create_account(&self, account: Account) -> Result<Account> {
        self.accounts
            .write()
            .unwrap()
            .insert(account.id.clone(), account.clone());
        Ok(account)
    }

    async fn update_account(&self, account: Account) -> Result<Account> {
        self.accounts
            .write()
            .unwrap()
            .insert(account.id.clone(), account.clone());
        Ok(account)
    }

    async fn delete_account(&self, id: &AccountId) -> Result<()> {
        self.accounts.write().unwrap().remove(id);
        Ok(())
    }
}

#[async_trait]
impl FolderRepository for MockBackend {
    async fn list_folders(&self, account_id: &AccountId) -> Result<Vec<Folder>> {
        let folders = self
            .folders
            .read()
            .unwrap()
            .iter()
            .filter(|(_, folder)| folder.account_id == *account_id)
            .map(|(_, folder)| folder.clone())
            .collect();
        Ok(folders)
    }

    async fn get_folder(&self, id: &FolderId) -> Result<Option<Folder>> {
        Ok(self.folders.read().unwrap().get(id).cloned())
    }

    async fn create_folder(&self, folder: Folder) -> Result<Folder> {
        self.folders
            .write()
            .unwrap()
            .insert(folder.id.clone(), folder.clone());
        Ok(folder)
    }

    async fn update_folder(&self, folder: Folder) -> Result<Folder> {
        self.folders
            .write()
            .unwrap()
            .insert(folder.id.clone(), folder.clone());
        Ok(folder)
    }

    async fn delete_folder(&self, id: &FolderId) -> Result<()> {
        self.folders.write().unwrap().remove(id);
        Ok(())
    }

    async fn sync_folders(&self, account_id: &AccountId) -> Result<Vec<Folder>> {
        // TODO: Generate random folders
        self.list_folders(account_id).await
    }
}

#[async_trait]
impl MessageRepository for MockBackend {
    async fn list_messages(
        &self,
        folder_id: &FolderId,
        offset: usize,
        limit: usize,
        sort: MessageSort,
    ) -> Result<Vec<Message>> {
        let mut messages = self
            .messages
            .read()
            .unwrap()
            .iter()
            .filter_map(|(_, msg)| {
                if msg.folder_id == *folder_id {
                    Some(msg)
                } else {
                    None
                }
            })
            .cloned()
            .collect::<Vec<_>>();

        match sort {
            MessageSort::DateAsc => messages.sort_by_key(|msg| msg.date),
            MessageSort::DateDesc => messages.sort_by_key(|msg| msg.date),
            MessageSort::SubjectAsc => messages.sort_by_key(|msg| msg.subject.clone()),
            MessageSort::SubjectDesc => messages.sort_by_key(|msg| msg.subject.clone()),
            MessageSort::SenderAsc => messages.sort_by_key(|msg| msg.sender.clone()),
            MessageSort::SenderDesc => messages.sort_by_key(|msg| msg.sender.clone()),
        };

        Ok(messages.into_iter().skip(offset).take(limit).collect())
    }

    async fn get_message(&self, id: &MessageId) -> Result<Option<Message>> {
        let messages = self.messages.read().unwrap();
        if let Some(msg) = messages.get(id) {
            Ok(Some(msg.clone()))
        } else {
            Ok(None)
        }
    }

    async fn get_message_content(&self, id: &MessageId) -> Result<Option<MessageContent>> {
        let contents = self.contents.read().unwrap();
        if let Some(content) = contents.get(id) {
            Ok(Some(content.clone()))
        } else {
            Ok(None)
        }
    }

    async fn update_message_flags(&self, id: &MessageId, flags: MessageFlags) -> Result<()> {
        self.flags.write().unwrap().insert(id.clone(), flags);
        Ok(())
    }

    async fn move_message(&self, id: &MessageId, to_folder: &FolderId) -> Result<()> {
        let mut messages = self.messages.write().unwrap();
        if let Some(msg) = messages.get_mut(id) {
            msg.folder_id = to_folder.clone();
        }

        Ok(())
    }

    async fn delete_message(&self, id: &MessageId) -> Result<()> {
        self.messages.write().unwrap().remove(id);
        self.flags.write().unwrap().remove(id);
        self.contents.write().unwrap().remove(id);
        Ok(())
    }

    async fn search_messages(
        &self,
        _folder_id: &FolderId,
        _query: &str,
        _offset: usize,
        _limit: usize,
    ) -> Result<Vec<Message>> {
        // TODO: Implement search
        Ok(Vec::new())
    }

    async fn sync_messages(&self, _folder_id: &FolderId, _limit: usize) -> Result<SyncResult> {
        // TODO: Generate random messages
        Ok(SyncResult::default())
    }
}
