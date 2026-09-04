use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::auth::AuthResults;
use crate::ids::{AccountId, FolderId, MessageId, MessagePartId};

/// How the message list is ordered for a selected folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MessageSort {
    /// Arrival order, newest first (UID descending; not the Date header).
    ///
    /// Persisted `"date"` still loads as Arrival (pre-rename pref / cache).
    #[default]
    #[serde(alias = "date")]
    Arrival,
    /// RFC 5322 Date header, newest first.
    ///
    /// Uses IMAP `SORT DATE` (RFC 5256) when the server advertises `SORT`.
    /// Without `SORT`, the index is arrival/UID order — same as [`Self::Arrival`].
    /// The current page is not re-sorted client-side (that would only order the
    /// fetched window, not the mailbox).
    #[serde(rename = "date_header")]
    Date,
    /// Unseen first, then seen; each group newest first.
    Unread,
    /// Largest `RFC822.SIZE` first. Requires IMAP `SORT`.
    Size,
    /// First From mailbox, A–Z. Requires IMAP `SORT`.
    Sender,
}

impl MessageSort {
    pub const ALL: [Self; 5] = [
        Self::Arrival,
        Self::Date,
        Self::Unread,
        Self::Size,
        Self::Sender,
    ];

    pub fn as_key(self) -> &'static str {
        match self {
            Self::Arrival => "arrival",
            Self::Date => "date_header",
            Self::Unread => "unread",
            Self::Size => "size",
            Self::Sender => "sender",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "arrival" | "date" => Some(Self::Arrival),
            "date_header" => Some(Self::Date),
            "unread" => Some(Self::Unread),
            "size" => Some(Self::Size),
            "sender" => Some(Self::Sender),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Arrival => "Arrival",
            Self::Date => "Date",
            Self::Unread => "Unread first",
            Self::Size => "Size",
            Self::Sender => "Sender",
        }
    }

    /// Size and Sender need RFC 5256 `SORT`. Date uses it when present, else arrival.
    pub fn needs_sort_capability(self) -> bool {
        matches!(self, Self::Size | Self::Sender)
    }
}

/// Quick list narrowing for the current folder. Active flags combine with AND.
///
/// `unread` / `flagged` map to IMAP `SEARCH UNSEEN` / `FLAGGED` and combine with
/// the mailbox search box. Attachment has no portable SEARCH key, so the chip
/// is applied client-side on known envelopes (`has:attachment` uses a HEADER
/// heuristic server-side).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MessageListFilter {
    #[serde(default)]
    pub unread: bool,
    #[serde(default)]
    pub flagged: bool,
    #[serde(default)]
    pub has_attachment: bool,
}

impl MessageListFilter {
    pub fn is_empty(self) -> bool {
        !self.unread && !self.flagged && !self.has_attachment
    }

    /// `true` when every active flag is satisfied (AND). Inactive flags are ignored.
    pub fn matches(self, is_read: bool, is_flagged: bool, has_attachments: bool) -> bool {
        if self.unread && is_read {
            return false;
        }
        if self.flagged && !is_flagged {
            return false;
        }
        if self.has_attachment && !has_attachments {
            return false;
        }
        true
    }

    /// IMAP `SEARCH` keys for criteria the server can evaluate (`None` = `ALL`).
    pub fn imap_search_query(self) -> Option<&'static str> {
        match (self.unread, self.flagged) {
            (false, false) => None,
            (true, false) => Some("UNSEEN"),
            (false, true) => Some("FLAGGED"),
            (true, true) => Some("UNSEEN FLAGGED"),
        }
    }
}

/// Result of preparing a folder for a paged list (after SELECT + optional SORT/SEARCH).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderListState {
    /// Length of the prepared list (after sort + list filters).
    pub total: usize,
    /// SELECT EXISTS / unfiltered folder size (for badges and cache).
    pub folder_total: usize,
    /// Whole-folder `UNSEEN` (`None` if SEARCH failed).
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

/// RFC 2087 `STORAGE` quota (bytes). Hidden when the server has no QUOTA / no limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxQuota {
    pub used_bytes: u64,
    pub limit_bytes: u64,
}

impl MailboxQuota {
    /// e.g. `1.2 GB of 15 GB`.
    pub fn display(self) -> String {
        format!(
            "{} of {}",
            format_quota_bytes(self.used_bytes),
            format_quota_bytes(self.limit_bytes)
        )
    }

    pub fn used_percent(self) -> u64 {
        if self.limit_bytes == 0 {
            return 0;
        }
        // Round half up so 1.2/15 shows as 8%, not 7%.
        let numerator = u128::from(self.used_bytes) * 100 + u128::from(self.limit_bytes) / 2;
        (numerator / u128::from(self.limit_bytes)).min(u128::from(u64::MAX)) as u64
    }
}

/// 1024-based units labeled KB/MB/GB (mailbox-quota convention).
pub fn format_quota_bytes(bytes: u64) -> String {
    const UNIT: f64 = 1024.0;
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut idx = 0usize;
    while value >= UNIT && idx + 1 < units.len() {
        value /= UNIT;
        idx += 1;
    }
    if idx == 0 {
        format!("{bytes} B")
    } else if (value - value.round()).abs() < 0.05 {
        format!("{:.0} {}", value, units[idx])
    } else {
        format!("{:.1} {}", value, units[idx])
    }
}

/// Well-known mailbox role (RFC 6154 special-use, else name heuristics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MailboxRole {
    Inbox,
    Archive,
    Drafts,
    Sent,
    Outbox,
    Trash,
    Junk,
    #[default]
    Other,
}

impl MailboxRole {
    /// Inbox, Archive, Drafts, Sent, Outbox, Trash, Junk, then everything else.
    pub fn sort_rank(self) -> u8 {
        match self {
            Self::Inbox => 0,
            Self::Archive => 1,
            Self::Drafts => 2,
            Self::Sent => 3,
            Self::Outbox => 4,
            Self::Trash => 5,
            Self::Junk => 6,
            Self::Other => 7,
        }
    }

    /// Canonical label for special-use folders (`None` keeps the server name).
    pub fn label(self) -> Option<&'static str> {
        match self {
            Self::Inbox => Some("Inbox"),
            Self::Archive => Some("Archive"),
            Self::Drafts => Some("Drafts"),
            Self::Sent => Some("Sent"),
            Self::Outbox => Some("Outbox"),
            Self::Trash => Some("Trash"),
            Self::Junk => Some("Junk"),
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
    /// IMAP subscription (`LSUB`). Default true for older cache blobs.
    #[serde(default = "default_true")]
    pub subscribed: bool,
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

/// Small built-in set of custom IMAP keywords the UI can set and clear.
///
/// Atoms are `$Important`, `$Work`, `$Personal`, `$Todo`, and `$Later`.
/// Other keywords on a message are stored on [`Envelope::keywords`] and shown
/// read-only. Gmail label folders are out of scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImapKeyword {
    Important,
    Work,
    Personal,
    Todo,
    Later,
}

impl ImapKeyword {
    pub const ALL: [Self; 5] = [
        Self::Important,
        Self::Work,
        Self::Personal,
        Self::Todo,
        Self::Later,
    ];

    /// IMAP atom written by [`crate::connector::EmailConnector::update_envelope_flags`].
    pub fn atom(self) -> &'static str {
        match self {
            Self::Important => "$Important",
            Self::Work => "$Work",
            Self::Personal => "$Personal",
            Self::Todo => "$Todo",
            Self::Later => "$Later",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Important => "Important",
            Self::Work => "Work",
            Self::Personal => "Personal",
            Self::Todo => "To do",
            Self::Later => "Later",
        }
    }

    pub fn from_atom(atom: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|keyword| keyword.atom().eq_ignore_ascii_case(atom))
    }
}

/// IMAP flag names used by [`crate::connector::EmailConnector::update_envelope_flags`].
///
/// `Starred` is the custom `\Starred` atom; `Flagged` is standard `\Flagged`.
/// [`Self::Keyword`] is one of the built-in custom keywords.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvelopeFlag {
    Read,
    Answered,
    Flagged,
    Draft,
    Deleted,
    Starred,
    Keyword(ImapKeyword),
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
    /// IMAP `\Answered`. Default false for older cached envelopes.
    #[serde(default)]
    pub is_answered: bool,
    pub is_starred: bool,
    pub is_flagged: bool,
    pub is_draft: bool,
    pub is_deleted: bool,
    /// Custom IMAP keywords (not system flags or `\Starred`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    pub has_attachments: bool,
    /// RFC822.SIZE when the server sent it.
    #[serde(default)]
    pub size: Option<u64>,
    /// Cached list-preview snippet (not IMAP ENVELOPE). Short cleaned text.
    #[serde(default)]
    pub snippet: Option<String>,
    /// SPF / DKIM / DMARC from Authentication-Results (not locally verified).
    #[serde(default, skip_serializing_if = "AuthResults::is_empty")]
    pub auth_results: AuthResults,
}

impl Envelope {
    pub fn has_keyword(&self, keyword: ImapKeyword) -> bool {
        self.keywords
            .iter()
            .any(|atom| ImapKeyword::from_atom(atom) == Some(keyword))
    }

    pub fn set_keyword(&mut self, keyword: ImapKeyword, on: bool) {
        self.keywords
            .retain(|atom| ImapKeyword::from_atom(atom) != Some(keyword));
        if on {
            self.keywords.push(keyword.atom().to_string());
        }
    }
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
    /// `text/calendar` / `application/ics` invite (card in the viewer).
    Calendar,
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    /// Section of the enclosing `message/rfc822` part, if this part is nested.
    #[serde(default)]
    pub nested_in: Option<String>,
    /// IMAP envelope of this `message/rfc822` part (From/To/Subject/Date).
    #[serde(default)]
    pub nested_headers: Option<NestedMessageHeaders>,
    pub content: MessageContent,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Viewer headers for a nested `message/rfc822` (from IMAP BODYSTRUCTURE ENVELOPE).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct NestedMessageHeaders {
    pub subject: Option<String>,
    pub from: Option<EmailAddress>,
    pub to: Option<EmailAddress>,
    pub cc: Option<EmailAddress>,
    pub date: Option<DateTime<Utc>>,
}

/// Type/subtype of a Content-Type value, ignoring parameters.
pub fn primary_mime(content_type: &str) -> &str {
    content_type.split(';').next().unwrap_or("").trim()
}

pub fn is_rfc822_mime(content_type: &str) -> bool {
    primary_mime(content_type).eq_ignore_ascii_case("message/rfc822")
}

/// `text/calendar` and the common `.ics` application types.
pub fn is_calendar_mime(content_type: &str) -> bool {
    let mime = primary_mime(content_type);
    mime.eq_ignore_ascii_case("text/calendar")
        || mime.eq_ignore_ascii_case("application/ics")
        || mime.eq_ignore_ascii_case("application/x-ics")
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
    ///
    /// Calendar invites are fetched even when listed as attachments so the
    /// viewer can render a title / time / organizer card.
    pub fn should_prefetch(&self) -> bool {
        self.is_hidden
            || !self.is_attachment
            || self.kind == PartKind::Calendar
            || is_calendar_mime(&self.content_type)
    }

    pub fn is_calendar(&self) -> bool {
        self.kind == PartKind::Calendar || is_calendar_mime(&self.content_type)
    }

    /// Visible body (not an attachment and not a cid-only inline).
    pub fn is_display_part(&self) -> bool {
        !self.is_attachment && !self.is_hidden
    }

    pub fn is_rfc822(&self) -> bool {
        is_rfc822_mime(&self.content_type)
    }

    /// Top-level part of the outer message (not inside a `message/rfc822`).
    pub fn is_top_level(&self) -> bool {
        self.nested_in.is_none()
    }

    /// True when this part belongs to `nested_in` (`None` = outer message).
    pub fn in_scope(&self, nested_in: Option<&str>) -> bool {
        self.nested_in.as_deref() == nested_in
    }
}

/// OpenPGP state after detect / decrypt / verify (no crypto material).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PgpViewState {
    pub encrypted: bool,
    pub signed: bool,
    pub signature: PgpSignatureState,
    /// User id of a valid signer, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signer: Option<String>,
    /// Encrypted, but no matching private key is available.
    #[serde(default)]
    pub need_private_key: bool,
}

impl PgpViewState {
    pub fn is_active(&self) -> bool {
        self.encrypted || self.signed || self.need_private_key
    }
}

/// Local signature check for OpenPGP (not S/MIME).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PgpSignatureState {
    #[default]
    None,
    Valid,
    Invalid,
    NeedKey,
}

/// Aggregate returned by the message load pipeline.
#[derive(Debug, Clone)]
pub struct LoadedMessage {
    pub envelope_id: MessageId,
    pub folder_id: FolderId,
    /// Full parsed part list (content + attachments + hidden inlines).
    pub parts: Vec<MessagePart>,
    /// Detect / decrypt / verify result. Default is inactive.
    pub pgp: PgpViewState,
}

impl LoadedMessage {
    pub fn attachments(&self) -> impl Iterator<Item = &MessagePart> {
        self.parts
            .iter()
            .filter(|p| p.is_top_level() && p.is_attachment && !p.is_hidden)
    }

    pub fn content_parts(&self) -> impl Iterator<Item = &MessagePart> {
        self.parts
            .iter()
            .filter(|p| p.is_top_level() && p.is_display_part())
    }

    /// Attachments of the outer message (`None`) or a nested `message/rfc822`.
    pub fn attachments_in_scope<'a>(
        &'a self,
        nested_in: Option<&'a str>,
    ) -> impl Iterator<Item = &'a MessagePart> + 'a {
        self.parts
            .iter()
            .filter(move |p| p.in_scope(nested_in) && p.is_attachment && !p.is_hidden)
    }

    /// Parts that belong to the nested `message/rfc822` at `section`.
    pub fn nested_parts<'a>(
        &'a self,
        section: &'a str,
    ) -> impl Iterator<Item = &'a MessagePart> + 'a {
        self.parts
            .iter()
            .filter(move |p| p.nested_in.as_deref() == Some(section))
    }

    pub fn rfc822_part(&self, section: &str) -> Option<&MessagePart> {
        self.parts
            .iter()
            .find(|p| p.is_rfc822() && p.section() == section)
    }
}

/// Decoded prefix of the first text part (`BODY.PEEK[section]<0.N>`).
///
/// Missing from a [`crate::connector::EmailConnector::fetch_text_prefixes`]
/// map means the peek failed and the caller should retry. An empty `text`
/// means there is no preview (no text part).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPrefix {
    pub text: String,
    pub is_html: bool,
}

impl TextPrefix {
    pub fn empty() -> Self {
        Self {
            text: String::new(),
            is_html: false,
        }
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
    use super::{
        format_quota_bytes, EmailAddr, EmailAddress, Envelope, ImapKeyword, MailboxQuota,
        MailboxRole, MessageListFilter, MessageSort,
    };

    #[test]
    fn archive_role_ranks_after_inbox() {
        assert!(MailboxRole::Inbox.sort_rank() < MailboxRole::Archive.sort_rank());
        assert!(MailboxRole::Archive.sort_rank() < MailboxRole::Drafts.sort_rank());
        assert_eq!(MailboxRole::Archive.label(), Some("Archive"));
    }

    #[test]
    fn junk_role_ranks_after_trash() {
        assert!(MailboxRole::Trash.sort_rank() < MailboxRole::Junk.sort_rank());
        assert!(MailboxRole::Junk.sort_rank() < MailboxRole::Other.sort_rank());
        assert_eq!(MailboxRole::Junk.label(), Some("Junk"));
    }

    #[test]
    fn rfc822_mime_ignores_parameters() {
        assert!(super::is_rfc822_mime("message/rfc822"));
        assert!(super::is_rfc822_mime("Message/RFC822; name=note.eml"));
        assert!(!super::is_rfc822_mime("text/plain"));
        assert_eq!(
            super::primary_mime("message/rfc822; name=x"),
            "message/rfc822"
        );
    }

    #[test]
    fn calendar_mime_detects_common_types() {
        assert!(super::is_calendar_mime("text/calendar"));
        assert!(super::is_calendar_mime("TEXT/CALENDAR; method=REQUEST"));
        assert!(super::is_calendar_mime("application/ics"));
        assert!(super::is_calendar_mime(
            "application/x-ics; name=invite.ics"
        ));
        assert!(!super::is_calendar_mime("text/plain"));
        assert!(!super::is_calendar_mime("application/pdf"));
    }

    #[test]
    fn quota_display_matches_issue_example() {
        let quota = MailboxQuota {
            used_bytes: 12 * 1024 * 1024 * 1024 / 10,
            limit_bytes: 15 * 1024 * 1024 * 1024,
        };
        assert_eq!(quota.display(), "1.2 GB of 15 GB");
        assert_eq!(quota.used_percent(), 8);
        assert_eq!(
            MailboxQuota {
                used_bytes: u64::MAX / 2,
                limit_bytes: u64::MAX,
            }
            .used_percent(),
            50
        );
    }

    #[test]
    fn quota_bytes_units() {
        assert_eq!(format_quota_bytes(0), "0 B");
        assert_eq!(format_quota_bytes(500), "500 B");
        assert_eq!(format_quota_bytes(10 * 1024), "10 KB");
        assert_eq!(format_quota_bytes(512 * 1024), "512 KB");
        assert_eq!(format_quota_bytes(1024 * 1024), "1 MB");
    }

    #[test]
    fn message_sort_key_roundtrip() {
        for sort in MessageSort::ALL {
            assert_eq!(MessageSort::from_key(sort.as_key()), Some(sort));
        }
        assert!(MessageSort::from_key("nope").is_none());
        assert!(MessageSort::Size.needs_sort_capability());
        assert!(MessageSort::Sender.needs_sort_capability());
        assert!(!MessageSort::Arrival.needs_sort_capability());
        assert!(!MessageSort::Date.needs_sort_capability());
        assert_eq!(MessageSort::from_key("date"), Some(MessageSort::Arrival));
        assert_eq!(
            MessageSort::from_key("date_header"),
            Some(MessageSort::Date)
        );
        assert_eq!(MessageSort::Date.as_key(), "date_header");
        assert_eq!(MessageSort::Date.label(), "Date");
        assert!(!MessageSort::Unread.needs_sort_capability());
    }

    #[test]
    fn envelope_answered_defaults_when_missing() {
        let json = r#"{
            "id":{"folder_id":"INBOX","uid":"1"},
            "account_id":"a",
            "folder_id":"INBOX",
            "subject":null,
            "from":null,
            "to":null,
            "cc":null,
            "bcc":null,
            "date":"2026-01-01T00:00:00Z",
            "is_read":false,
            "is_starred":false,
            "is_flagged":false,
            "is_draft":false,
            "is_deleted":false,
            "has_attachments":false,
            "created_at":"2026-01-01T00:00:00Z",
            "updated_at":"2026-01-01T00:00:00Z"
        }"#;
        let env: Envelope = serde_json::from_str(json).expect("legacy envelope");
        assert!(!env.is_answered);
        assert!(env.auth_results.is_empty());
        assert!(env.keywords.is_empty());
    }

    #[test]
    fn imap_keyword_atom_roundtrip() {
        for keyword in ImapKeyword::ALL {
            assert_eq!(ImapKeyword::from_atom(keyword.atom()), Some(keyword));
        }
        assert_eq!(
            ImapKeyword::from_atom("$important"),
            Some(ImapKeyword::Important)
        );
        assert!(ImapKeyword::from_atom("$label1").is_none());
        assert_eq!(ImapKeyword::Todo.label(), "To do");
    }

    #[test]
    fn envelope_set_keyword_replaces_case_variant() {
        let mut env: Envelope = serde_json::from_str(
            r#"{
            "id":{"folder_id":"INBOX","uid":"1"},
            "account_id":"a",
            "folder_id":"INBOX",
            "subject":null,
            "from":null,
            "to":null,
            "cc":null,
            "bcc":null,
            "date":"2026-01-01T00:00:00Z",
            "is_read":false,
            "is_starred":false,
            "is_flagged":false,
            "is_draft":false,
            "is_deleted":false,
            "has_attachments":false,
            "keywords":["$important","ProjectX"]
        }"#,
        )
        .expect("envelope with keywords");
        assert!(env.has_keyword(ImapKeyword::Important));
        assert!(!env.has_keyword(ImapKeyword::Work));
        env.set_keyword(ImapKeyword::Important, false);
        assert_eq!(env.keywords, vec!["ProjectX".to_string()]);
        env.set_keyword(ImapKeyword::Work, true);
        assert_eq!(
            env.keywords,
            vec!["ProjectX".to_string(), "$Work".to_string()]
        );
        env.set_keyword(ImapKeyword::Work, true);
        assert_eq!(
            env.keywords,
            vec!["ProjectX".to_string(), "$Work".to_string()]
        );
    }

    #[test]
    fn message_sort_serde_keeps_date_alias_as_arrival() {
        assert_eq!(
            serde_json::from_str::<MessageSort>("\"date\"").unwrap(),
            MessageSort::Arrival
        );
        assert_eq!(
            serde_json::from_str::<MessageSort>("\"date_header\"").unwrap(),
            MessageSort::Date
        );
        assert_eq!(
            serde_json::to_string(&MessageSort::Date).unwrap(),
            "\"date_header\""
        );
        assert_eq!(
            serde_json::to_string(&MessageSort::Arrival).unwrap(),
            "\"arrival\""
        );
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

    #[test]
    fn list_filter_empty_matches_everything() {
        let f = MessageListFilter::default();
        assert!(f.is_empty());
        assert!(f.matches(true, false, false));
        assert!(f.matches(false, true, true));
        assert!(f.imap_search_query().is_none());
    }

    #[test]
    fn list_filter_unread_is_unseen_only() {
        let f = MessageListFilter {
            unread: true,
            ..MessageListFilter::default()
        };
        assert!(f.matches(false, false, false));
        assert!(!f.matches(true, false, false));
        assert!(f.matches(false, true, true));
        assert_eq!(f.imap_search_query(), Some("UNSEEN"));
    }

    #[test]
    fn list_filter_flagged_requires_flag() {
        let f = MessageListFilter {
            flagged: true,
            ..MessageListFilter::default()
        };
        assert!(f.matches(true, true, false));
        assert!(!f.matches(false, false, true));
        assert_eq!(f.imap_search_query(), Some("FLAGGED"));
    }

    #[test]
    fn list_filter_attachment_requires_attachment() {
        let f = MessageListFilter {
            has_attachment: true,
            ..MessageListFilter::default()
        };
        assert!(f.matches(true, false, true));
        assert!(!f.matches(false, true, false));
        // No portable IMAP SEARCH key for attachments.
        assert!(f.imap_search_query().is_none());
    }

    #[test]
    fn list_filter_combinable_and() {
        let f = MessageListFilter {
            unread: true,
            flagged: true,
            has_attachment: true,
        };
        assert!(f.matches(false, true, true));
        assert!(!f.matches(true, true, true));
        assert!(!f.matches(false, false, true));
        assert!(!f.matches(false, true, false));
        assert_eq!(f.imap_search_query(), Some("UNSEEN FLAGGED"));
    }
}
