//! File attachments and inline images held on a draft.

/// Stable attachment id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AttachmentId(pub String);

impl AttachmentId {
    /// New random id.
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl Default for AttachmentId {
    fn default() -> Self {
        Self::new()
    }
}

/// Stable inline image id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InlineId(pub String);

impl InlineId {
    /// New random id.
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl Default for InlineId {
    fn default() -> Self {
        Self::new()
    }
}

/// Payload bytes or still-loading placeholder.
#[derive(Debug, Clone)]
pub enum AttachmentData {
    /// Fully buffered (v1 default).
    Bytes(Vec<u8>),
    /// Placeholder until read completes; never left Pending on successful validate.
    Pending,
}

/// Non-inline file attachment.
#[derive(Debug, Clone)]
pub struct FileAttachment {
    /// Id.
    pub id: AttachmentId,
    /// Filename for Content-Disposition.
    pub filename: String,
    /// MIME type.
    pub content_type: String,
    /// Declared size (may match bytes len).
    pub size: u64,
    /// Payload.
    pub data: AttachmentData,
}

/// Inline image referenced from HTML.
#[derive(Debug, Clone)]
pub struct InlineImage {
    /// Id.
    pub id: InlineId,
    /// Content-ID without angle brackets (e.g. `img-uuid@mailiner`).
    pub content_id: String,
    /// MIME type (`image/png`, …).
    pub content_type: String,
    /// Optional filename.
    pub filename: Option<String>,
    /// Payload.
    pub data: AttachmentData,
    /// blob: or data: URL used inside contenteditable while editing.
    pub edit_url: Option<String>,
}
