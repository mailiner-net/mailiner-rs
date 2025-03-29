use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{AccountId, FolderId, MessageId, MessagePartId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    pub name: String,
    pub email: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    pub id: FolderId,
    pub account_id: AccountId,
    pub name: String,
    pub parent_id: Option<FolderId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailAddress {
    pub name: Option<String>,
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub id: MessageId,
    pub account_id: AccountId,
    pub folder_id: FolderId,
    pub subject: String,
    pub from: EmailAddress,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub date: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
    pub is_read: bool,
    pub is_starred: bool,
    pub is_flagged: bool,
    pub is_draft: bool,
    pub is_deleted: bool,
    pub has_attachments: bool,
    pub message_structure: MessageStructure,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageStructure {
    Simple(MessagePartId),
    Multipart {
        parts: Vec<MessagePartId>,
        boundary: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePart {
    pub id: MessagePartId,
    pub envelope_id: MessageId,
    pub content_type: String,
    pub filename: Option<String>,
    pub size: u64,
    pub is_attachment: bool,
    pub content: MessageContent,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageContent {
    Text(String),
    Html(String),
    Binary(Vec<u8>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderMetadata {
    pub id: FolderId,
    pub total_messages: u64,
    pub unread_messages: u64,
    pub last_sync: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountMetadata {
    pub id: AccountId,
    pub last_sync: DateTime<Utc>,
    pub folders: Vec<FolderMetadata>,
} 