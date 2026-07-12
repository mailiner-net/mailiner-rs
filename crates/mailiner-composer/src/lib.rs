//! # mailiner-composer
//!
//! Email composition for Mailiner: draft model, plain/rich body editors, composer shell,
//! reply/forward prefill, and export orchestration.
//!
//! This crate is intentionally free of IMAP/SMTP transport. Wire send lives in the app
//! (and later `mailiner-core` connector extensions).
//!
//! ## Module map
//!
//! | Module | Role |
//! |--------|------|
//! | [`model`] | `DraftDocument`, recipients, attachments, body conversion |
//! | [`editor`] | Plain textarea + shadow-hosted contenteditable (web) |
//! | [`shell`] | Full `EmailComposer` UI |
//! | [`reply`] | Reply/forward prefill, quotes, cid rehydration |
//! | [`sanitize`] | Edit/export HTML policy wrappers |
//! | [`export`] | Validate + build MIME (`prepare_submit`) |
//! | [`identity`] | From-address identity for the draft |

#![deny(missing_docs)]

pub mod editor;
pub mod export;
pub mod identity;
pub mod model;
pub mod reply;
pub mod sanitize;
pub mod shell;

pub use identity::FromIdentity;
pub use model::{
    caps, dedupe_addresses, emails_equal, exclude_self, flatten_addresses, html_to_plain,
    is_valid_email_v1, plain_to_html, try_composer_address, validate_draft, AttachmentData,
    AttachmentId, BodyMode, ComposerAddress, DraftDocument, DraftId, DraftValidationError,
    FileAttachment, InlineId, InlineImage,
};
pub use reply::{
    attribution_line, build_draft, quote_plain, subject_with_prefix, ComposeIntent, PrefillError,
};
pub use sanitize::{sanitize_for_edit, sanitize_for_export};

/// Crate version string (for diagnostics / about UI).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn crate_version_is_nonempty() {
        assert!(!super::VERSION.is_empty(), "VERSION must not be empty");
    }
}
