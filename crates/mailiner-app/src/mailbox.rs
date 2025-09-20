#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct MailboxId(String);

impl From<String> for MailboxId {
    fn from(id: String) -> Self {
        Self(id)
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