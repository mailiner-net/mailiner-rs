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
pub struct EmailAddr {
    pub name: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub name: Option<String>,
    pub members: Vec<EmailAddr>
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EmailAddress {
    List(Vec<EmailAddr>),
    Group(Vec<Group>)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub id: MessageId,
    pub account_id: AccountId,
    pub folder_id: FolderId,
    pub subject: Option<String>,
    pub from: Option<EmailAddress>,
    pub to: Option<EmailAddress>,
    pub cc: Option<EmailAddress>,
    pub bcc: Option<EmailAddress>,
    pub date: DateTime<Utc>,
    pub is_read: bool,
    pub is_starred: bool,
    pub is_flagged: bool,
    pub is_draft: bool,
    pub is_deleted: bool,
    pub has_attachments: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
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