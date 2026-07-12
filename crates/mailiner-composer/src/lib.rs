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
//!
//! Scaffold only in PR 1 — behavior lands in subsequent PRs.

#![deny(missing_docs)]

pub mod editor;
pub mod export;
pub mod identity;
pub mod model;
pub mod reply;
pub mod sanitize;
pub mod shell;

/// Crate version string (for diagnostics / about UI).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn crate_version_is_nonempty() {
        assert!(!super::VERSION.is_empty(), "VERSION must not be empty");
    }
}
