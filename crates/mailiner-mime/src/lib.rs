//! Pure MIME utilities for Mailiner: transfer-encoding decode, charset conversion.
//!
//! PR1 ships codecs only (string encoding names). Later PRs add parsers and wire
//! codecs to `mailiner-core` types.

pub mod codec;

pub use codec::{
    decode_content, decode_transfer_encoding, DecodeError, DecodedContent, MAX_BINARY_DECODE_BYTES,
    MAX_TEXT_DECODE_BYTES,
};
