//! Reply / forward prefill, quoting, and cid rehydration.

pub mod cid;
pub mod prefill;
pub mod quote;

pub use prefill::{build_draft, ComposeIntent, PrefillError};
pub use quote::{attribution_line, quote_plain, subject_with_prefix};
