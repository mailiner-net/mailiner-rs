use chrono::{DateTime, Utc};
use mailiner_core::{EmailAddress, Envelope};
use std::fmt;

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct MessageId(String);

impl MessageId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for MessageId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(PartialEq, Debug, Clone)]
pub struct Message {
    pub id: MessageId,
    pub subject: String,
    pub from: String,
    pub to: String,
    pub cc: Option<String>,
    pub bcc: Option<String>,
    pub date: DateTime<Utc>,
    pub has_attachments: bool,
    /// Original IMAP envelope for reply/forward prefill.
    pub envelope: Envelope,
}

impl Message {
    /// Display name when the address is `Name <email>`, otherwise the raw string.
    pub fn from_preview(&self) -> &str {
        preview_mailbox(&self.from)
    }

    pub fn to_preview(&self) -> &str {
        preview_mailbox(&self.to)
    }
}

fn preview_mailbox(value: &str) -> &str {
    if let Some((name, rest)) = value.split_once(" <") {
        if !name.is_empty() && rest.ends_with('>') {
            return name;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::preview_mailbox;

    #[test]
    fn preview_strips_angle_addr() {
        assert_eq!(
            preview_mailbox("Mailiner Test <mailiner@dvratil.cz>"),
            "Mailiner Test"
        );
        assert_eq!(preview_mailbox("solo@example.com"), "solo@example.com");
    }
}

impl From<Envelope> for Message {
    fn from(envelope: Envelope) -> Self {
        Self {
            id: MessageId::from(envelope.id.to_string()),
            subject: envelope.subject.clone().unwrap_or_default(),
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
            date: envelope.date,
            has_attachments: envelope.has_attachments,
            envelope,
        }
    }
}
