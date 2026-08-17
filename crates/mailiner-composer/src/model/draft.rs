//! Central draft document and validation.

use chrono::{DateTime, Utc};

use crate::identity::FromIdentity;
use crate::model::attachment::{AttachmentId, FileAttachment, InlineId, InlineImage};

/// Logical body representation for the draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BodyMode {
    /// Controlled textarea body.
    Plain,
    /// Shadow-hosted contenteditable HTML fragment.
    #[default]
    Rich,
}

/// Stable draft session id (uuid v4 string).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DraftId(pub String);

impl DraftId {
    /// Allocate a new random draft id.
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    /// Borrow the underlying string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for DraftId {
    fn default() -> Self {
        Self::new()
    }
}

/// Compose-time address. Email is required for a valid outbound recipient.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerAddress {
    /// Optional display name.
    pub name: Option<String>,
    /// Required mailbox address.
    pub email: String,
}

impl ComposerAddress {
    /// Construct from email only.
    pub fn email_only(email: impl Into<String>) -> Self {
        Self {
            name: None,
            email: email.into(),
        }
    }
}

/// In-memory compose draft.
#[derive(Debug, Clone)]
pub struct DraftDocument {
    /// Session id; remount key for the shell.
    pub id: DraftId,
    /// From header (usually filled from [`FromIdentity`]).
    pub from: Option<ComposerAddress>,
    /// To recipients.
    pub to: Vec<ComposerAddress>,
    /// Cc recipients.
    pub cc: Vec<ComposerAddress>,
    /// Bcc recipients (envelope only at send; omitted from RFC 5322 headers).
    pub bcc: Vec<ComposerAddress>,
    /// Subject line (may be empty).
    pub subject: String,
    /// Which body field is authoritative.
    pub mode: BodyMode,
    /// Authoritative when `mode == Plain`.
    pub plain_body: String,
    /// Authoritative when `mode == Rich` (HTML fragment, not full document).
    pub html_body: String,
    /// When true, `plain_body` must not be used for export without recomputing.
    pub plain_cache_dirty: bool,
    /// Non-inline file attachments.
    pub attachments: Vec<FileAttachment>,
    /// Images referenced from `html_body` via cid/blob/data while editing.
    pub inline_images: Vec<InlineImage>,
    /// In-Reply-To message-id (angle brackets optional; normalize at export).
    pub in_reply_to: Option<String>,
    /// References chain.
    pub references: Vec<String>,
    /// Soft warnings from prefill (e.g. stripped cids).
    pub prefill_warnings: Vec<String>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Last mutation time.
    pub updated_at: DateTime<Utc>,
}

impl DraftDocument {
    /// Empty draft for a new compose (Rich by default).
    pub fn new_empty(identity: &FromIdentity) -> Self {
        let now = Utc::now();
        Self {
            id: DraftId::new(),
            from: Some(ComposerAddress {
                name: if identity.display_name.is_empty() {
                    None
                } else {
                    Some(identity.display_name.clone())
                },
                email: identity.email.clone(),
            }),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: String::new(),
            mode: BodyMode::Rich,
            plain_body: String::new(),
            html_body: String::new(),
            plain_cache_dirty: false,
            attachments: Vec::new(),
            inline_images: Vec::new(),
            in_reply_to: None,
            references: Vec::new(),
            prefill_warnings: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Touch `updated_at`.
    pub fn touch(&mut self) {
        self.updated_at = Utc::now();
    }
}

/// Hard caps for draft payload (design v1).
pub mod caps {
    /// Single file attachment max (bytes).
    pub const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
    /// Single inline image max (bytes).
    pub const MAX_INLINE_BYTES: u64 = 25 * 1024 * 1024;
    /// Total draft payload max (bytes).
    pub const MAX_DRAFT_BYTES: u64 = 40 * 1024 * 1024;
    /// Max file attachments.
    pub const MAX_ATTACHMENTS: usize = 20;
    /// Max inline images.
    pub const MAX_INLINES: usize = 30;
    /// Max HTML body UTF-8 length.
    pub const MAX_HTML_LEN: usize = 3 * 512 * 1024; // 1.5 MiB
    /// Max plain body UTF-8 length.
    pub const MAX_PLAIN_LEN: usize = 3 * 512 * 1024; // 1.5 MiB
}

/// Validation failure for a draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftValidationError {
    /// To is empty.
    EmptyTo,
    /// Address failed `is_valid_email_v1`.
    InvalidEmail {
        /// Header field name.
        field: &'static str,
        /// Index in the field list.
        index: usize,
        /// Offending value.
        value: String,
    },
    /// Attachment still pending read.
    PendingAttachment {
        /// Attachment id.
        id: AttachmentId,
    },
    /// Inline still pending read.
    PendingInline {
        /// Inline id.
        id: InlineId,
    },
    /// Single file over cap.
    OversizeFile {
        /// Attachment id.
        id: AttachmentId,
        /// Actual size.
        size: u64,
        /// Cap.
        max: u64,
    },
    /// Total draft over cap.
    OversizeDraft {
        /// Total bytes.
        total: u64,
        /// Cap.
        max: u64,
    },
    /// HTML body too large.
    OversizeHtml {
        /// Length.
        len: usize,
        /// Cap.
        max: usize,
    },
    /// Plain body too large.
    OversizePlain {
        /// Length.
        len: usize,
        /// Cap.
        max: usize,
    },
    /// Single inline image over cap.
    OversizeInline {
        /// Inline id.
        id: InlineId,
        /// Actual size.
        size: u64,
        /// Cap.
        max: u64,
    },
    /// Too many attachments.
    TooManyAttachments {
        /// Count.
        count: usize,
        /// Cap.
        max: usize,
    },
    /// Too many inlines.
    TooManyInlines {
        /// Count.
        count: usize,
        /// Cap.
        max: usize,
    },
    /// Missing From.
    MissingFrom,
    /// Inline reference orphan (reserved for later export path).
    OrphanInlineReference,
}

/// v1 email syntax (not full RFC 5322): after trim,
/// - non-empty
/// - contains exactly one `@`
/// - local and domain parts both non-empty
/// - no whitespace anywhere
/// - domain contains at least one `.`
/// Reject angle brackets and commas in the email field.
pub fn is_valid_email_v1(email: &str) -> bool {
    let email = email.trim();
    if email.is_empty() {
        return false;
    }
    if email.chars().any(|c| c.is_whitespace() || c == '<' || c == '>' || c == ',') {
        return false;
    }
    let mut parts = email.split('@');
    let local = match parts.next() {
        Some(l) if !l.is_empty() => l,
        _ => return false,
    };
    let domain = match parts.next() {
        Some(d) if !d.is_empty() => d,
        _ => return false,
    };
    if parts.next().is_some() {
        return false;
    }
    if local.is_empty() || domain.is_empty() {
        return false;
    }
    if !domain.contains('.') {
        return false;
    }
    // Domain labels non-empty around dots.
    if domain.split('.').any(|l| l.is_empty()) {
        return false;
    }
    true
}

fn validate_addr_list(
    field: &'static str,
    list: &[ComposerAddress],
    out: &mut Vec<DraftValidationError>,
) {
    for (index, a) in list.iter().enumerate() {
        // Reject untrimmed storage so headers never carry accidental spaces.
        if a.email != a.email.trim() || !is_valid_email_v1(&a.email) {
            out.push(DraftValidationError::InvalidEmail {
                field,
                index,
                value: a.email.clone(),
            });
        }
    }
}

fn attachment_bytes(a: &FileAttachment) -> Option<u64> {
    match &a.data {
        crate::model::attachment::AttachmentData::Bytes(b) => Some(b.len() as u64),
        crate::model::attachment::AttachmentData::Pending => None,
    }
}

/// Validate draft for send/export. Empty subject is allowed (soft product choice).
pub fn validate_draft(
    draft: &DraftDocument,
    identity: &FromIdentity,
) -> Result<(), Vec<DraftValidationError>> {
    let mut errs = Vec::new();

    if draft.from.is_none() && identity.email.trim().is_empty() {
        errs.push(DraftValidationError::MissingFrom);
    } else if let Some(from) = &draft.from {
        if !is_valid_email_v1(&from.email) {
            errs.push(DraftValidationError::InvalidEmail {
                field: "from",
                index: 0,
                value: from.email.clone(),
            });
        }
    } else if !is_valid_email_v1(&identity.email) {
        errs.push(DraftValidationError::InvalidEmail {
            field: "from",
            index: 0,
            value: identity.email.clone(),
        });
    }

    if draft.to.is_empty() {
        errs.push(DraftValidationError::EmptyTo);
    }
    validate_addr_list("to", &draft.to, &mut errs);
    validate_addr_list("cc", &draft.cc, &mut errs);
    validate_addr_list("bcc", &draft.bcc, &mut errs);

    if draft.attachments.len() > caps::MAX_ATTACHMENTS {
        errs.push(DraftValidationError::TooManyAttachments {
            count: draft.attachments.len(),
            max: caps::MAX_ATTACHMENTS,
        });
    }
    if draft.inline_images.len() > caps::MAX_INLINES {
        errs.push(DraftValidationError::TooManyInlines {
            count: draft.inline_images.len(),
            max: caps::MAX_INLINES,
        });
    }

    if draft.html_body.len() > caps::MAX_HTML_LEN {
        errs.push(DraftValidationError::OversizeHtml {
            len: draft.html_body.len(),
            max: caps::MAX_HTML_LEN,
        });
    }
    if draft.plain_body.len() > caps::MAX_PLAIN_LEN {
        errs.push(DraftValidationError::OversizePlain {
            len: draft.plain_body.len(),
            max: caps::MAX_PLAIN_LEN,
        });
    }

    let mut total = draft.plain_body.len() as u64 + draft.html_body.len() as u64;

    for a in &draft.attachments {
        match attachment_bytes(a) {
            None => errs.push(DraftValidationError::PendingAttachment { id: a.id.clone() }),
            Some(sz) => {
                if sz > caps::MAX_FILE_BYTES {
                    errs.push(DraftValidationError::OversizeFile {
                        id: a.id.clone(),
                        size: sz,
                        max: caps::MAX_FILE_BYTES,
                    });
                }
                total = total.saturating_add(sz);
            }
        }
    }
    for img in &draft.inline_images {
        match &img.data {
            crate::model::attachment::AttachmentData::Pending => {
                errs.push(DraftValidationError::PendingInline { id: img.id.clone() });
            }
            crate::model::attachment::AttachmentData::Bytes(b) => {
                let sz = b.len() as u64;
                if sz > caps::MAX_INLINE_BYTES {
                    errs.push(DraftValidationError::OversizeInline {
                        id: img.id.clone(),
                        size: sz,
                        max: caps::MAX_INLINE_BYTES,
                    });
                }
                total = total.saturating_add(sz);
            }
        }
    }

    if total > caps::MAX_DRAFT_BYTES {
        errs.push(DraftValidationError::OversizeDraft {
            total,
            max: caps::MAX_DRAFT_BYTES,
        });
    }

    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_v1_rules() {
        assert!(is_valid_email_v1("a@b.co"));
        assert!(is_valid_email_v1("  user.name+tag@example.com "));
        assert!(!is_valid_email_v1("a@b"));
        assert!(!is_valid_email_v1("a@@b.com"));
        assert!(!is_valid_email_v1("a b@c.com"));
        assert!(!is_valid_email_v1("<a@b.com>"));
        assert!(!is_valid_email_v1("a@b.com,c@d.com"));
        assert!(!is_valid_email_v1(""));
    }

    #[test]
    fn validate_empty_to() {
        let id = FromIdentity::new("Me", "me@example.com");
        let d = DraftDocument::new_empty(&id);
        let err = validate_draft(&d, &id).unwrap_err();
        assert!(err.contains(&DraftValidationError::EmptyTo));
    }

    #[test]
    fn validate_ok_minimal() {
        let id = FromIdentity::new("Me", "me@example.com");
        let mut d = DraftDocument::new_empty(&id);
        d.to.push(ComposerAddress::email_only("you@example.com"));
        assert!(validate_draft(&d, &id).is_ok());
    }
}
