use serde::{Deserialize, Serialize};

use crate::kernel::model::{AccountId, FolderId};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub String);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub id: MessageId,
    pub folder_id: FolderId,
    pub subject: String,
    pub sender: String,
    pub recipients: Vec<String>,
    pub date: chrono::DateTime<chrono::Utc>,
    pub flags: MessageFlags,
    pub snippet: String,         // Message preview text
    pub has_attachments: bool,
    // Note: Message body is loaded separately
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct MessageFlags {
    pub read: bool,
    pub flagged: bool,
    pub answered: bool,
    pub forwarded: bool,
    pub draft: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MessageContent {
    pub id: MessageId,
    pub text_body: Option<String>,
    pub html_body: Option<String>,
    pub attachments: Vec<Attachment>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Attachment {
    pub name: String,
    pub mime_type: String,
    pub size: usize,
    pub content_id: Option<String>, // For inline attachments
    // Content is fetched separately to avoid excessive memory usage
}