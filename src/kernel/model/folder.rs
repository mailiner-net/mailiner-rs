use std::{convert::Infallible, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::kernel::model::AccountId;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FolderId(pub String);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Folder {
    pub id: FolderId,
    pub account_id: AccountId,
    pub name: String,
    pub unread_count: usize,
    pub total_messages: usize,
    pub folder_type: FolderType,
}

impl Into<String> for FolderId {
    fn into(self) -> String {
        self.0
    }
}

impl FolderId {
    pub fn as_string(&self) -> String {
        self.0.clone()
    }
}

impl FromStr for FolderId {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(FolderId(s.to_string()))
    }
}

impl ToString for FolderId {
    fn to_string(&self) -> String {
        self.0.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FolderType {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Junk,
    Archive,
    Custom(String),
}
