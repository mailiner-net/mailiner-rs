//! Draft document model (recipients, body modes, attachments, inline images).

pub mod attachment;
pub mod convert;
pub mod draft;
pub mod recipients;

pub use attachment::{
    AttachmentData, AttachmentId, AttachmentSource, FileAttachment, InlineId, InlineImage,
};
pub use convert::{html_to_plain, plain_to_html};
pub use draft::{
    caps, is_valid_email_v1, validate_draft, BodyMode, ComposerAddress, DraftDocument, DraftId,
    DraftValidationError,
};
pub use recipients::{
    dedupe_addresses, emails_equal, exclude_self, flatten_addresses, try_composer_address,
};
