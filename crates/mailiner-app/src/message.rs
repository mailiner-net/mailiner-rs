use chrono::{DateTime, Utc};
use mailiner_core::{EmailAddr, EmailAddress, Envelope};

pub use mailiner_core::MessageId;

/// Distinct mid-saturation swatches for placeholder sender avatars.
const AVATAR_COLORS: &[&str] = &[
    "#4C6EF5", "#0CA678", "#F08C00", "#E64980", "#7048E8", "#1098AD", "#F76707", "#37B24D",
    "#C2255C", "#364FC7",
];

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
    pub is_read: bool,
    pub is_starred: bool,
    pub is_flagged: bool,
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

    pub fn cc_preview(&self) -> &str {
        preview_mailbox(self.cc.as_deref().unwrap_or(""))
    }

    pub fn bcc_preview(&self) -> &str {
        preview_mailbox(self.bcc.as_deref().unwrap_or(""))
    }

    /// Formatted Reply-To when the envelope has a non-empty address.
    pub fn reply_to(&self) -> Option<String> {
        self.envelope
            .reply_to
            .as_ref()
            .map(ToString::to_string)
            .filter(|s| !s.trim().is_empty())
    }

    /// CSS color for the list avatar; stable for the same sender.
    pub fn avatar_color(&self) -> &'static str {
        avatar_color_for(self.avatar_seed())
    }

    fn avatar_seed(&self) -> &str {
        if let Some(email) = first_from_email(self.envelope.from.as_ref())
            && !email.is_empty()
        {
            return email;
        }
        let preview = self.from_preview();
        if preview.is_empty() { "?" } else { preview }
    }
}

/// Hash `seed` (case-insensitive) onto [`AVATAR_COLORS`].
pub fn avatar_color_for(seed: &str) -> &'static str {
    let mut hash: u32 = 2_166_136_261;
    for b in seed.as_bytes() {
        hash ^= u32::from(b.to_ascii_lowercase());
        hash = hash.wrapping_mul(16_777_619);
    }
    AVATAR_COLORS[(hash as usize) % AVATAR_COLORS.len()]
}

fn first_from_email(from: Option<&EmailAddress>) -> Option<&str> {
    match from? {
        EmailAddress::List(list) => list.iter().find_map(|a| nonempty_email(a)),
        EmailAddress::Group(groups) => groups
            .iter()
            .flat_map(|g| g.members.iter())
            .find_map(nonempty_email),
    }
}

fn nonempty_email(addr: &EmailAddr) -> Option<&str> {
    addr.email.as_deref().filter(|s| !s.is_empty())
}

pub(crate) fn preview_mailbox(value: &str) -> &str {
    if let Some((name, rest)) = value.split_once(" <") {
        if !name.is_empty() && rest.ends_with('>') {
            return name;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_strips_angle_addr() {
        assert_eq!(
            preview_mailbox("Mailiner Test <mailiner@dvratil.cz>"),
            "Mailiner Test"
        );
        assert_eq!(preview_mailbox("solo@example.com"), "solo@example.com");
        assert_eq!(
            preview_mailbox("Ada <ada@example.com>, Bob <bob@example.com>"),
            "Ada"
        );
    }

    #[test]
    fn cc_bcc_reply_to_preview_from_envelope() {
        use mailiner_core::{AccountId, FolderId};

        let now = DateTime::from_timestamp(0, 0).unwrap();
        let mut envelope = Envelope {
            id: MessageId::new(FolderId::new("INBOX"), "1"),
            account_id: AccountId::new("acc"),
            folder_id: FolderId::new("INBOX"),
            subject: Some("s".into()),
            from: None,
            to: None,
            cc: Some(EmailAddress::List(vec![EmailAddr {
                name: Some("Cc Name".into()),
                email: Some("cc@example.com".into()),
            }])),
            bcc: Some(EmailAddress::List(vec![EmailAddr {
                name: Some("Bcc Name".into()),
                email: Some("bcc@example.com".into()),
            }])),
            reply_to: Some(EmailAddress::List(vec![EmailAddr {
                name: Some("Reply Name".into()),
                email: Some("reply@example.com".into()),
            }])),
            rfc_message_id: None,
            in_reply_to: None,
            references: vec![],
            date: now,
            is_read: false,
            is_starred: false,
            is_flagged: false,
            is_draft: false,
            is_deleted: false,
            has_attachments: false,
            size: None,
        };
        let msg = Message::from(envelope.clone());
        assert_eq!(msg.cc.as_deref(), Some("Cc Name <cc@example.com>"));
        assert_eq!(msg.cc_preview(), "Cc Name");
        assert_eq!(msg.bcc.as_deref(), Some("Bcc Name <bcc@example.com>"));
        assert_eq!(msg.bcc_preview(), "Bcc Name");
        assert_eq!(
            msg.reply_to().as_deref(),
            Some("Reply Name <reply@example.com>")
        );
        assert_eq!(
            preview_mailbox(msg.reply_to().as_deref().unwrap()),
            "Reply Name"
        );

        envelope.cc = Some(EmailAddress::List(vec![]));
        envelope.bcc = Some(EmailAddress::List(vec![EmailAddr {
            name: None,
            email: None,
        }]));
        envelope.reply_to = Some(EmailAddress::List(vec![]));
        let empty = Message::from(envelope);
        assert_eq!(empty.cc.as_deref(), Some(""));
        assert!(empty.cc.as_deref().is_some_and(|s| s.trim().is_empty()));
        assert!(empty.bcc.as_deref().is_some_and(|s| s.trim().is_empty()));
        assert!(empty.reply_to().is_none());
    }

    #[test]
    fn from_envelope_keeps_star_and_flag() {
        use mailiner_core::{AccountId, FolderId};

        let now = DateTime::from_timestamp(0, 0).unwrap();
        let envelope = Envelope {
            id: MessageId::new(FolderId::new("INBOX"), "1"),
            account_id: AccountId::new("acc"),
            folder_id: FolderId::new("INBOX"),
            subject: Some("s".into()),
            from: None,
            to: None,
            cc: None,
            bcc: None,
            reply_to: None,
            rfc_message_id: None,
            in_reply_to: None,
            references: vec![],
            date: now,
            is_read: false,
            is_starred: true,
            is_flagged: true,
            is_draft: false,
            is_deleted: false,
            has_attachments: false,
            size: None,
        };
        let msg = Message::from(envelope);
        assert!(msg.is_starred);
        assert!(msg.is_flagged);
    }

    #[test]
    fn avatar_color_is_stable_and_case_insensitive() {
        let a = avatar_color_for("me@dvratil.cz");
        assert_eq!(a, avatar_color_for("ME@dvratil.cz"));
        assert!(AVATAR_COLORS.contains(&a));
    }

    #[test]
    fn avatar_palette_spreads_across_seeds() {
        let seen: std::collections::HashSet<_> = (0..40)
            .map(|i| avatar_color_for(&format!("user{i}@example.com")))
            .collect();
        assert!(
            seen.len() >= 4,
            "expected several colors from the palette, got {seen:?}"
        );
    }

    #[test]
    fn first_from_email_reads_list_and_skips_empty() {
        let from = EmailAddress::List(vec![
            EmailAddr {
                name: Some("No Mail".into()),
                email: Some("".into()),
            },
            EmailAddr {
                name: Some("Dan".into()),
                email: Some("me@dvratil.cz".into()),
            },
        ]);
        assert_eq!(first_from_email(Some(&from)), Some("me@dvratil.cz"));
        assert!(first_from_email(None).is_none());
    }
}

impl From<Envelope> for Message {
    fn from(envelope: Envelope) -> Self {
        Self {
            id: envelope.id.clone(),
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
            is_read: envelope.is_read,
            is_starred: envelope.is_starred,
            is_flagged: envelope.is_flagged,
            envelope,
        }
    }
}
