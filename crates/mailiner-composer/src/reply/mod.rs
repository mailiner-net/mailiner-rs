//! Reply / forward prefill, quoting, and cid rehydration.

pub mod cid;
pub mod prefill;
pub mod quote;
pub mod signature;

pub use cid::{
    bare_content_id, extract_cid_refs, normalize_cid, rehydrate_cids, CidRehydrateResult,
};
pub use prefill::{
    build_draft, discard_rich_quote, is_forwardable_attachment, ComposeIntent, PrefillError,
};
pub use quote::{attribution_line, quote_plain, subject_with_prefix};
pub use signature::{append_plain_signature, apply_plain_signature, SIGDASH};
