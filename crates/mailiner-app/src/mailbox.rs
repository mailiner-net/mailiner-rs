use mailiner_core::{Folder, FolderId};

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct MailboxId(String);

impl From<String> for MailboxId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

impl From<FolderId> for MailboxId {
    fn from(id: FolderId) -> Self {
        Self(id.to_string())
    }
}


pub struct MailboxNode {
    pub id: MailboxId,
    pub name: String,
    pub parent: Option<MailboxId>,
    pub children: Vec<MailboxId>,
    pub unread_count: usize,
    pub total_count: usize,
}

impl From<Folder> for MailboxNode {
    fn from(folder: Folder) -> Self {
        Self {
            id: folder.id.into(),
            name: folder.name,
            parent: folder.parent_id.map(|id| id.into()),
            children: vec![],
            unread_count: 0,
            total_count: 0,
        }
    }
}