#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct MessageId(String);

impl From<String> for MessageId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

#[derive(PartialEq, Debug)]
pub struct Message {
    pub id: MessageId,
    pub subject: String,
    pub from: String,
    pub to: String,
    pub cc: String,
    pub bcc: String,
}
