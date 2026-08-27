//! Pure MIME utilities for Mailiner: transfer-encoding decode, charset conversion,
//! BODYSTRUCTURE → part list parsing, parameter normalization, and RFC 5322 writing.

pub mod codec;
pub mod heuristics;
pub mod params;
pub mod parser;
pub mod writer;

pub use codec::{
    base64_decode, base64_encode, decode_content, decode_part_content, decode_transfer_encoding,
    decode_transfer_stream, qp_decode, qp_encode, DecodeError, DecodedContent,
    StreamingTransferDecoder, MAX_BINARY_DECODE_BYTES, MAX_TEXT_DECODE_BYTES,
};
pub use heuristics::{is_attachment, is_rich_part};
pub use params::{mime_words_decode, normalize_params};
pub use parser::{MessageParser, ATTACHMENT_MIME};
pub use writer::{
    encode_unstructured, format_disposition, format_folded_header, format_mailbox,
    generate_boundary, serialize_message, MimeBody, MimePart, WriteError,
};
