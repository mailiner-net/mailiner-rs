use mailiner_core::{EmailAddress, Envelope};

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
    pub cc: Option<String>,
    pub bcc: Option<String>,
}

impl From<Envelope> for Message {
    fn from(envelope: Envelope) -> Self {
        Self {
            id: MessageId::from(envelope.id.to_string()),
            subject: envelope.subject.unwrap_or_default(),
            from: envelope
                .from
                .as_ref()
                .map(EmailAddress::to_string)
                .unwrap_or_default(),
            to: envelope
                .to
                .as_ref()
                .map(EmailAddress::to_string)
                .unwrap_or_default(),
            cc: envelope.cc.as_ref().map(EmailAddress::to_string),
            bcc: envelope.bcc.as_ref().map(EmailAddress::to_string),
        }
    }
}
