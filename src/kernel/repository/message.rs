use crate::kernel::cache::CacheStorage;
use crate::kernel::model::{FolderId, Message, MessageContent, MessageFlags, MessageId};
use crate::kernel::Result;
use crate::kernel::MailinerError;
use async_trait::async_trait;
use std::sync::Arc;
#[async_trait]
pub trait MessageRepository: Send + Sync + 'static {
    async fn list_messages(
        &self,
        folder_id: &FolderId,
        offset: usize,
        limit: usize,
        sort: MessageSort,
    ) -> Result<Vec<Message>>;

    async fn get_message(&self, id: &MessageId) -> Result<Option<Message>>;

    async fn get_message_content(&self, id: &MessageId) -> Result<Option<MessageContent>>;

    async fn update_message_flags(&self, id: &MessageId, flags: MessageFlags) -> Result<()>;

    async fn move_message(&self, id: &MessageId, to_folder: &FolderId) -> Result<()>;

    async fn delete_message(&self, id: &MessageId) -> Result<()>;

    async fn search_messages(
        &self,
        folder_id: &FolderId,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Message>>;

    async fn sync_messages(&self, folder_id: &FolderId, limit: usize) -> Result<SyncResult>;
}

#[derive(Debug, Clone, Copy)]
pub enum MessageSort {
    DateAsc,
    DateDesc,
    SubjectAsc,
    SubjectDesc,
    SenderAsc,
    SenderDesc,
}

#[derive(Debug, Clone, Default)]
pub struct SyncResult {
    pub new_messages: Vec<Message>,
    pub updated_messages: Vec<Message>,
    pub deleted_message_ids: Vec<MessageId>,
}

pub struct CachedMessageRepository<B: MessageRepository, C: CacheStorage> {
    backend: Arc<B>,
    cache: C,
    ttl: std::time::Duration, // Time-to-live for cached items
}

impl<B: MessageRepository, C: CacheStorage> CachedMessageRepository<B, C> {
    pub fn new(backend: Arc<B>, cache: C) -> Self {
        Self {
            backend,
            cache,
            ttl: std::time::Duration::from_secs(300), // 5 minutes default
        }
    }

    fn message_list_key(
        &self,
        folder_id: &FolderId,
        offset: usize,
        limit: usize,
        sort: &MessageSort,
    ) -> String {
        format!(
            "messages:list:{}:{}:{}:{:?}",
            folder_id.0, offset, limit, sort
        )
    }

    fn message_key(&self, id: &MessageId) -> String {
        format!("messages:{}", id.0)
    }

    fn content_key(&self, id: &MessageId) -> String {
        format!("content:{}", id.0)
    }
}

#[async_trait::async_trait]
impl<B: MessageRepository, C: CacheStorage> MessageRepository for CachedMessageRepository<B, C> {
    async fn list_messages(
        &self,
        folder_id: &FolderId,
        offset: usize,
        limit: usize,
        sort: MessageSort,
    ) -> Result<Vec<Message>> {
        let cache_key = self.message_list_key(folder_id, offset, limit, &sort);

        // Try to get from cache first
        if let Some(cached) = self.cache.get(&cache_key).await? {
            if let Ok(messages) = serde_json::from_str::<Vec<Message>>(&cached) {
                return Ok(messages);
            }
        }

        // Cache miss, fetch from backend
        let messages = self
            .backend
            .list_messages(folder_id, offset, limit, sort)
            .await?;

        // Cache the result
        let json = serde_json::to_string(&messages)
            .map_err(|e| MailinerError::Cache(format!("Failed to serialize messages: {}", e)))?;

        let _ = self.cache.set(&cache_key, &json).await;

        Ok(messages)
    }

    async fn get_message(&self, id: &MessageId) -> Result<Option<Message>> {
        let cache_key = self.message_key(id);

        // Try to get from cache first
        if let Some(cached) = self.cache.get(&cache_key).await? {
            if let Ok(message) = serde_json::from_str::<Message>(&cached) {
                return Ok(Some(message));
            }
        }

        // Cache miss, fetch from backend
        let message = self.backend.get_message(id).await?;

        // Cache the result if found
        if let Some(ref message) = message {
            let json = serde_json::to_string(message)
                .map_err(|e| MailinerError::Cache(format!("Failed to serialize message: {}", e)))?;

            let _ = self.cache.set(&cache_key, &json).await;
        }

        Ok(message)
    }

    async fn get_message_content(&self, id: &MessageId) -> Result<Option<MessageContent>> {
        let cache_key = self.content_key(id);

        // Try to get from cache first
        if let Some(cached) = self.cache.get(&cache_key).await? {
            if let Ok(content) = serde_json::from_str::<MessageContent>(&cached) {
                return Ok(Some(content));
            }
        }

        // Cache miss, fetch from backend
        let content = self.backend.get_message_content(id).await?;

        // Cache the result if found
        if let Some(ref content) = content {
            let json = serde_json::to_string(content)
                .map_err(|e| MailinerError::Cache(format!("Failed to serialize content: {}", e)))?;

            let _ = self.cache.set(&cache_key, &json).await;
        }

        Ok(content)
    }

    async fn move_message(&self, id: &MessageId, to_folder: &FolderId) -> Result<()> {
        let cache_key = self.message_key(id);
        let cached_msg = self.cache.get(&cache_key).await?;

        self.backend.move_message(id, to_folder).await;

        // If we had the message cached, just update the folder_id and re-insert it back into the cache
        if let Some(mut cached_msg) = cached_msg {
            let mut msg: Message = serde_json::from_str(&cached_msg).map_err(|e| {
                MailinerError::Cache(format!("Failed to deserialize cached message: {}", e))
            })?;
            msg.folder_id = to_folder.clone();
            let json = serde_json::to_string(&msg)
                .map_err(|e| MailinerError::Cache(format!("Failed to serialize message: {}", e)))?;
            let _ = self.cache.set(&cache_key, &json).await;
        }

        Ok(())
    }

    async fn delete_message(&self, id: &MessageId) -> Result<()> {
        let cache_key = self.message_key(id);
        let _ = self.cache.remove(&cache_key).await;

        self.backend.delete_message(id).await
    }

    async fn search_messages(
        &self,
        folder_id: &FolderId,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Message>> {
        // TODO: Implement caching interaction - we normally only get list of UIDs from search,
        // so we should only fetch messages not in the cache.

        self.backend
            .search_messages(folder_id, query, offset, limit)
            .await
    }

    async fn update_message_flags(&self, id: &MessageId, flags: MessageFlags) -> Result<()> {
        // Invalidate the message in cache before updating
        let cache_key = self.message_key(id);
        let _ = self.cache.remove(&cache_key).await;

        // Update in backend
        self.backend.update_message_flags(id, flags).await
    }

    async fn sync_messages(&self, folder_id: &FolderId, limit: usize) -> Result<SyncResult> {
        // Fetch sync results from backend
        let result = self.backend.sync_messages(folder_id, limit).await?;

        // Invalidate related cache keys
        let prefix = format!("messages:list:{}", folder_id.0);
        let keys = self.cache.keys(&prefix).await?;

        for key in keys {
            let _ = self.cache.remove(&key).await;
        }

        // Cache the new/updated messages
        for message in &result.new_messages {
            let cache_key = self.message_key(&message.id);
            let json = serde_json::to_string(message)
                .map_err(|e| MailinerError::Cache(format!("Failed to serialize message: {}", e)))?;

            let _ = self.cache.set(&cache_key, &json).await;
        }

        for message in &result.updated_messages {
            let cache_key = self.message_key(&message.id);
            let json = serde_json::to_string(message)
                .map_err(|e| MailinerError::Cache(format!("Failed to serialize message: {}", e)))?;

            let _ = self.cache.set(&cache_key, &json).await;
        }

        // Also remove deleted messages from cache
        for id in &result.deleted_message_ids {
            let cache_key = self.message_key(id);
            let content_key = self.content_key(id);

            let _ = self.cache.remove(&cache_key).await;
            let _ = self.cache.remove(&content_key).await;
        }

        Ok(result)
    }
}
