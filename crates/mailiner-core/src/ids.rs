use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccountId(String);

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct FolderId(String);

/// Folder-scoped IMAP UID. The same numeric UID in two mailboxes is two ids.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct MessageId {
    folder_id: FolderId,
    uid: String,
}

/// [`MessageId::try_new`] rejected an empty folder id or UID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyMessageId;

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct MessagePartId(String);

impl AccountId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FolderId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl MessageId {
    /// Build an id. Folder and UID must both be non-empty.
    pub fn try_new(folder_id: FolderId, uid: impl Into<String>) -> Result<Self, EmptyMessageId> {
        let uid = uid.into();
        if folder_id.as_str().is_empty() || uid.is_empty() {
            Err(EmptyMessageId)
        } else {
            Ok(Self { folder_id, uid })
        }
    }

    /// Build an id. Panics if folder or UID is empty.
    pub fn new(folder_id: FolderId, uid: impl Into<String>) -> Self {
        Self::try_new(folder_id, uid).expect("MessageId folder and uid must be non-empty")
    }

    pub fn folder_id(&self) -> &FolderId {
        &self.folder_id
    }

    /// IMAP UID atom (FETCH / STORE / MOVE). Not unique across folders.
    pub fn as_uid(&self) -> &str {
        &self.uid
    }
}

impl AsRef<FolderId> for MessageId {
    fn as_ref(&self) -> &FolderId {
        &self.folder_id
    }
}

impl MessagePartId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for FolderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Unit separator: mailbox names may contain `#`, `/`, `:`.
        write!(f, "{}\u{1f}{}", self.folder_id.as_str(), self.uid)
    }
}

impl fmt::Display for MessagePartId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_uid_different_folders_are_distinct() {
        let inbox = MessageId::new(FolderId::new("INBOX"), "12");
        let sent = MessageId::new(FolderId::new("Sent"), "12");
        assert_ne!(inbox, sent);
        assert_eq!(inbox.as_uid(), "12");
        assert_eq!(sent.as_uid(), "12");
        assert_eq!(inbox.folder_id().as_str(), "INBOX");
    }

    #[test]
    fn rejects_empty_folder_or_uid() {
        assert_eq!(
            MessageId::try_new(FolderId::new(""), "1"),
            Err(EmptyMessageId)
        );
        assert_eq!(
            MessageId::try_new(FolderId::new("INBOX"), ""),
            Err(EmptyMessageId)
        );
    }
}
