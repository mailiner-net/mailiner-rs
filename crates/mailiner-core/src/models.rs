use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::body::BodyPart;
use crate::ids::{AccountId, FolderId, MessageId, MessagePartId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    pub name: String,
    pub email: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// How the message list is ordered for a selected folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MessageSort {
    /// Arrival / sequence order, newest first (no IMAP SORT required).
    #[default]
    Date,
    /// Unseen first, then seen; each group newest first.
    Unread,
    /// Largest `RFC822.SIZE` first. Requires IMAP `SORT`.
    Size,
    /// First From mailbox, A–Z. Requires IMAP `SORT`.
    Sender,
}

impl MessageSort {
    pub const ALL: [Self; 4] = [Self::Date, Self::Unread, Self::Size, Self::Sender];

    pub fn as_key(self) -> &'static str {
        match self {
            Self::Date => "date",
            Self::Unread => "unread",
            Self::Size => "size",
            Self::Sender => "sender",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "date" => Some(Self::Date),
            "unread" => Some(Self::Unread),
            "size" => Some(Self::Size),
            "sender" => Some(Self::Sender),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Date => "Date",
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    pub id: FolderId,
    pub account_id: AccountId,
    pub name: String,
    pub parent_id: Option<FolderId>,
    #[serde(default)]
    pub role: MailboxRole,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailAddr {
    pub name: Option<String>,
    pub email: Option<String>,
}

impl ToString for EmailAddr {
    fn to_string(&self) -> String {
        match (self.name.as_ref(), self.email.as_ref()) {
            (Some(name), Some(email)) => format!("{} <{}>", name, email),
            (Some(name), None) => format!("{}", name),
            (None, Some(email)) => format!("{}", email),
            (None, None) => String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub name: Option<String>,
    pub members: Vec<EmailAddr>,
}

impl ToString for Group {
    fn to_string(&self) -> String {
        match self.name.as_ref() {
            Some(name) => format!(
                "{}: {}",
                name,
                self.members
                    .iter()
                    .map(EmailAddr::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            None => self
                .members
                .iter()
                .map(EmailAddr::to_string)
                .collect::<Vec<_>>()
                .join(", "),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EmailAddress {
    List(Vec<EmailAddr>),
    Group(Vec<Group>),
}

impl ToString for EmailAddress {
    fn to_string(&self) -> String {
        match self {
            EmailAddress::List(list) => list.iter().map(|e| e.to_string()).collect(),
            EmailAddress::Group(group) => group.iter().map(|g| g.to_string()).collect(),
        }
    }
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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
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
    Attachment,
    #[default]
    Other,
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
    /// Undecoded FETCH payload. Cleared after successful decode.
    #[serde(skip)]
    pub raw_content: Option<Vec<u8>>,
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
}

/// Aggregate returned by the message load pipeline.
#[derive(Debug, Clone)]
pub struct LoadedMessage {
    pub envelope_id: MessageId,
    pub folder_id: FolderId,
    /// Full parsed part list (content + attachments + hidden inlines).
    pub parts: Vec<MessagePart>,
    /// Optional structure retained for debugging / re-parse.
    pub structure: Option<BodyPart>,
}

impl LoadedMessage {
    pub fn attachments(&self) -> impl Iterator<Item = &MessagePart> {
        self.parts
            .iter()
            .filter(|p| p.is_attachment && !p.is_hidden)
    }

    pub fn content_parts(&self) -> impl Iterator<Item = &MessagePart> {
        self.parts.iter().filter(|p| p.should_prefetch())
    }
}

/// Chunk of transfer-encoded bytes for streaming attachment download.
#[derive(Debug, Clone)]
pub struct PartChunk {
    pub data: Vec<u8>,
    /// Total expected transfer size if known (BODYSTRUCTURE octets).
    pub total_hint: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderMetadata {
    pub id: FolderId,
    pub total_messages: u64,
    pub unread_messages: u64,
    pub last_sync: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountMetadata {
    pub id: AccountId,
    pub last_sync: DateTime<Utc>,
    pub folders: Vec<FolderMetadata>,
}

#[cfg(test)]
mod tests {
    use super::MessageSort;

    #[test]
    fn message_sort_key_roundtrip() {
        for sort in MessageSort::ALL {
            assert_eq!(MessageSort::from_key(sort.as_key()), Some(sort));
        }
        assert!(MessageSort::from_key("nope").is_none());
        assert!(MessageSort::Size.needs_sort_capability());
        assert!(MessageSort::Sender.needs_sort_capability());
        assert!(!MessageSort::Date.needs_sort_capability());
        assert!(!MessageSort::Unread.needs_sort_capability());
    }
}
