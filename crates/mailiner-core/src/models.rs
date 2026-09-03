use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{AccountId, FolderId, MessageId, MessagePartId};

/// How the message list is ordered for a selected folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MessageSort {
    /// Arrival order, newest first (no IMAP `SORT DATE`; not the Date header).
    #[default]
    #[serde(alias = "date")]
    Arrival,
    /// Unseen first, then seen; each group newest first.
    Unread,
    /// Largest `RFC822.SIZE` first. Requires IMAP `SORT`.
    Size,
    /// First From mailbox, A–Z. Requires IMAP `SORT`.
    Sender,
}

impl MessageSort {
    pub const ALL: [Self; 4] = [Self::Arrival, Self::Unread, Self::Size, Self::Sender];

    pub fn as_key(self) -> &'static str {
        match self {
            Self::Arrival => "arrival",
            Self::Unread => "unread",
            Self::Size => "size",
            Self::Sender => "sender",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "arrival" | "date" => Some(Self::Arrival),
            "unread" => Some(Self::Unread),
            "size" => Some(Self::Size),
            "sender" => Some(Self::Sender),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Arrival => "Arrival",
            Self::Unread => "Unread first",
            Self::Size => "Size",
            Self::Sender => "Sender",
        }
    }

    /// Size and Sender need RFC 5256 `SORT`.
    pub fn needs_sort_capability(self) -> bool {
        matches!(self, Self::Size | Self::Sender)
    }
}

/// Result of preparing a folder for a paged list (after SELECT + optional SORT/SEARCH).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderListState {
    pub total: usize,
    /// `UNSEEN` from the just-selected mailbox (`None` if SEARCH failed).
    pub unread: Option<usize>,
    /// Sort actually applied (may fall back from the request).
    pub sort: MessageSort,
    pub supports_size_sender: bool,
}

/// IMAP `STATUS` totals for one mailbox (`MESSAGES` / `UNSEEN`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderCounts {
    pub total_messages: u64,
    pub unread_messages: u64,
}

/// Well-known mailbox role (RFC 6154 special-use, else name heuristics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MailboxRole {
    Inbox,
    Drafts,
    Sent,
    Outbox,
    Trash,
    #[default]
    Other,
}

impl MailboxRole {
    /// Inbox first, then Drafts, Sent, Outbox, Trash, then everything else.
    pub fn sort_rank(self) -> u8 {
        match self {
            Self::Inbox => 0,
            Self::Drafts => 1,
            Self::Sent => 2,
            Self::Outbox => 3,
            Self::Trash => 4,
            Self::Other => 5,
        }
    }

    /// Canonical label for special-use folders (`None` keeps the server name).
    pub fn label(self) -> Option<&'static str> {
        match self {
            Self::Inbox => Some("Inbox"),
            Self::Drafts => Some("Drafts"),
            Self::Sent => Some("Sent"),
            Self::Outbox => Some("Outbox"),
            Self::Trash => Some("Trash"),
            Self::Other => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub id: FolderId,
    pub account_id: AccountId,
    pub name: String,
    pub parent_id: Option<FolderId>,
    #[serde(default)]
    pub role: MailboxRole,
    /// False for `\\Noselect` / synthesized ancestors. Default true for older blobs.
    #[serde(default = "default_true")]
    pub selectable: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailAddr {
    pub name: Option<String>,
    pub email: Option<String>,
}

impl fmt::Display for EmailAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.name.as_ref(), self.email.as_ref()) {
            (Some(name), Some(email)) => write!(f, "{name} <{email}>"),
            (Some(name), None) => write!(f, "{name}"),
            (None, Some(email)) => write!(f, "{email}"),
            (None, None) => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub name: Option<String>,
    pub members: Vec<EmailAddr>,
}

impl fmt::Display for Group {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(name) = self.name.as_ref() {
            write!(f, "{name}: ")?;
        }
        for (i, member) in self.members.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{member}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EmailAddress {
    List(Vec<EmailAddr>),
    Group(Vec<Group>),
}

impl fmt::Display for EmailAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EmailAddress::List(list) => {
                for (i, addr) in list.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{addr}")?;
                }
            }
            EmailAddress::Group(groups) => {
                for (i, group) in groups.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{group}")?;
                }
            }
        }
        Ok(())
    }
}

/// IMAP flag names used by [`crate::connector::EmailConnector::update_envelope_flags`].
///
/// `Starred` is the custom `\Starred` atom; `Flagged` is standard `\Flagged`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvelopeFlag {
    Read,
    Flagged,
    Draft,
    Deleted,
    Starred,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub id: MessageId,
    pub account_id: AccountId,
    pub folder_id: FolderId,
    pub subject: Option<String>,
    pub from: Option<EmailAddress>,
    pub to: Option<EmailAddress>,
    pub cc: Option<EmailAddress>,
    pub bcc: Option<EmailAddress>,
    /// RFC 5322 `Reply-To`, when present.
    #[serde(default)]
    pub reply_to: Option<EmailAddress>,
    /// RFC 5322 `Message-ID` of this message (not the IMAP UID).
    #[serde(default)]
    pub rfc_message_id: Option<String>,
    /// RFC 5322 `In-Reply-To`.
    #[serde(default)]
    pub in_reply_to: Option<String>,
    /// RFC 5322 `References` chain (parent ids, oldest first).
    #[serde(default)]
    pub references: Vec<String>,
    pub date: DateTime<Utc>,
    pub is_read: bool,
    pub is_starred: bool,
    pub is_flagged: bool,
    pub is_draft: bool,
    pub is_deleted: bool,
    pub has_attachments: bool,
    /// RFC822.SIZE when the server sent it.
    #[serde(default)]
    pub size: Option<u64>,
}

/// MIME Content-Transfer-Encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TransferEncoding {
    #[default]
    SevenBit,
    EightBit,
    Binary,
    Base64,
    QuotedPrintable,
    /// Unknown; treat as binary passthrough.
    Other,
}

impl TransferEncoding {
    pub fn from_wire(s: &str) -> Self {
        match s.trim().to_ascii_uppercase().as_str() {
            "" | "7BIT" => Self::SevenBit,
            "8BIT" => Self::EightBit,
            "BINARY" => Self::Binary,
            "BASE64" => Self::Base64,
            "QUOTED-PRINTABLE" => Self::QuotedPrintable,
            _ => Self::Other,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SevenBit => "7BIT",
            Self::EightBit => "8BIT",
            Self::Binary => "BINARY",
            Self::Base64 => "BASE64",
            Self::QuotedPrintable => "QUOTED-PRINTABLE",
            Self::Other => "OTHER",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PartKind {
    TextPlain,
    TextHtml,
    Image,
    #[default]
    Attachment,
}

/// Decoded part payload. HTML is still text; use [`PartKind::TextHtml`] to distinguish.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum MessageContent {
    /// Decoded Unicode text (plain or HTML source).
    Text(String),
    /// Decoded binary (images, PDFs, …).
    Binary(Vec<u8>),
    /// Not yet fetched / not applicable.
    #[default]
    Empty,
}

/// Viewer-oriented message part (parser / loader / formatter).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePart {
    /// Stable logical id for UI keys, e.g. `".alternative.1.html"`.
    pub id: MessagePartId,
    pub envelope_id: MessageId,
    /// IMAP section path segments. Join with `'.'` for FETCH.
    pub path: Vec<String>,
    pub kind: PartKind,
    pub content_type: String,
    pub charset: Option<String>,
    pub content_id: Option<String>,
    pub description: Option<String>,
    pub filename: Option<String>,
    pub encoding: TransferEncoding,
    /// BODYSTRUCTURE size in transfer-encoded octets (if known).
    pub original_size: Option<u64>,
    /// Display size (for base64 attachments: ~ original_size / 1.37).
    pub size: u64,
    pub is_attachment: bool,
    /// True for cid-inlined images (and parts hidden after cid resolution).
    pub is_hidden: bool,
    pub content: MessageContent,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl MessagePart {
    pub fn section(&self) -> String {
        if self.path.is_empty() {
            "TEXT".to_string()
        } else {
            self.path.join(".")
        }
    }

    /// Content parts that should be prefetched for display.
    pub fn should_prefetch(&self) -> bool {
        self.is_hidden || !self.is_attachment
    }

    /// Visible body (not an attachment and not a cid-only inline).
    pub fn is_display_part(&self) -> bool {
        !self.is_attachment && !self.is_hidden
    }
}

/// Aggregate returned by the message load pipeline.
#[derive(Debug, Clone)]
pub struct LoadedMessage {
    pub envelope_id: MessageId,
    pub folder_id: FolderId,
    /// Full parsed part list (content + attachments + hidden inlines).
    pub parts: Vec<MessagePart>,
}

impl LoadedMessage {
    pub fn attachments(&self) -> impl Iterator<Item = &MessagePart> {
        self.parts
            .iter()
            .filter(|p| p.is_attachment && !p.is_hidden)
    }

    pub fn content_parts(&self) -> impl Iterator<Item = &MessagePart> {
        self.parts.iter().filter(|p| p.is_display_part())
    }
}

/// Chunk of transfer-encoded bytes for streaming attachment download.
#[derive(Debug, Clone)]
pub struct PartChunk {
    pub data: Vec<u8>,
    /// Total expected transfer size if known (BODYSTRUCTURE octets).
    pub total_hint: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::{EmailAddr, EmailAddress, MessageSort};

    #[test]
    fn message_sort_key_roundtrip() {
        for sort in MessageSort::ALL {
            assert_eq!(MessageSort::from_key(sort.as_key()), Some(sort));
        }
        assert!(MessageSort::from_key("nope").is_none());
        assert!(MessageSort::Size.needs_sort_capability());
        assert!(MessageSort::Sender.needs_sort_capability());
        assert!(!MessageSort::Arrival.needs_sort_capability());
        assert_eq!(MessageSort::from_key("date"), Some(MessageSort::Arrival));
        assert!(!MessageSort::Unread.needs_sort_capability());
    }

    #[test]
    fn email_address_list_joins_with_comma() {
        let addr = EmailAddress::List(vec![
            EmailAddr {
                name: Some("Alice".into()),
                email: Some("a@x.com".into()),
            },
            EmailAddr {
                name: Some("Bob".into()),
                email: Some("b@y.com".into()),
            },
        ]);
        assert_eq!(addr.to_string(), "Alice <a@x.com>, Bob <b@y.com>");
    }
}
