//! Incremental transfer-encoding decode for attachment download streams.

use super::{base64_decode, qp_decode, DecodeError};
use mailiner_core::models::TransferEncoding;

/// Streaming transfer-encoding decoder.
///
/// Feed wire octets with [`push`]; call [`finish`] when the stream ends.
/// Incomplete sequences at chunk boundaries are held in a small remainder buffer.
pub struct StreamingTransferDecoder {
    inner: Inner,
}

enum Inner {
    /// 7bit / 8bit / binary / other — pass through.
    Identity,
    Base64 { pending: Vec<u8> },
    Qp { pending: Vec<u8> },
}

impl StreamingTransferDecoder {
    pub fn new(encoding: TransferEncoding) -> Self {
        let inner = match encoding {
            TransferEncoding::Base64 => Inner::Base64 {
                pending: Vec::with_capacity(4),
            },
            TransferEncoding::QuotedPrintable => Inner::Qp {
                pending: Vec::with_capacity(8),
            },
            TransferEncoding::SevenBit
            | TransferEncoding::EightBit
            | TransferEncoding::Binary
            | TransferEncoding::Other => Inner::Identity,
        };
        Self { inner }
    }

    /// Decode as much as possible from `chunk`, returning newly decoded octets.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<u8>, DecodeError> {
        match &mut self.inner {
            Inner::Identity => Ok(chunk.to_vec()),
            Inner::Base64 { pending } => push_base64(pending, chunk),
            Inner::Qp { pending } => push_qp(pending, chunk),
        }
    }

    /// Flush any remainder after the last chunk.
    pub fn finish(self) -> Result<Vec<u8>, DecodeError> {
        match self.inner {
            Inner::Identity => Ok(Vec::new()),
            Inner::Base64 { pending } => finish_base64(pending),
            Inner::Qp { pending } => {
                if pending.is_empty() {
                    Ok(Vec::new())
                } else {
                    // Decode leftover as a final QP fragment (handles trailing soft `=`).
                    qp_decode(&pending)
                }
            }
        }
    }
}

fn is_b64_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

fn push_base64(pending: &mut Vec<u8>, chunk: &[u8]) -> Result<Vec<u8>, DecodeError> {
    for &b in chunk {
        if is_b64_ws(b) {
            continue;
        }
        pending.push(b);
    }

    let complete = pending.len() / 4 * 4;
    if complete == 0 {
        return Ok(Vec::new());
    }

    let to_decode: Vec<u8> = pending.drain(..complete).collect();
    // STANDARD decode; groups of 4 are complete (may include padding in middle of stream —
    // padding should only appear at end, but if present mid-stream base64 crate may error).
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &to_decode)
        .map_err(|e| DecodeError::Base64(e.to_string()))
}

fn finish_base64(mut pending: Vec<u8>) -> Result<Vec<u8>, DecodeError> {
    if pending.is_empty() {
        return Ok(Vec::new());
    }
    let rem = pending.len() % 4;
    if rem != 0 {
        pending.extend(std::iter::repeat(b'=').take(4 - rem));
    }
    base64_decode(&pending)
}

/// Append chunk to pending and QP-decode a safe prefix, leaving an incomplete tail.
fn push_qp(pending: &mut Vec<u8>, chunk: &[u8]) -> Result<Vec<u8>, DecodeError> {
    pending.extend_from_slice(chunk);
    let safe = qp_safe_prefix_len(pending);
    if safe == 0 {
        return Ok(Vec::new());
    }
    let to_decode: Vec<u8> = pending.drain(..safe).collect();
    qp_decode(&to_decode)
}

/// Length of a prefix that can be QP-decoded without needing more input.
///
/// Holds back:
/// - a trailing `=` or incomplete soft-break / hex (`=X`, `=\r` awaiting optional `\n`)
/// - trailing spaces/tabs that might still be end-of-line whitespace
fn qp_safe_prefix_len(buf: &[u8]) -> usize {
    if buf.is_empty() {
        return 0;
    }

    let mut end = buf.len();

    // Incomplete escape / soft-break at the end.
    if buf[end - 1] == b'=' {
        // Lone `=` — could be soft-break or start of =XX.
        end -= 1;
    } else if end >= 2 && buf[end - 2] == b'=' {
        let h = buf[end - 1];
        match h {
            // `=\n` is a complete soft break.
            b'\n' => {}
            // `=\r` may still be followed by `\n` — hold both until more input.
            b'\r' => {
                end -= 2;
            }
            // First hex digit of =XX (or invalid that needs second byte for lenient path).
            _ => {
                end -= 2;
            }
        }
    }

    // Trailing WS may be end-of-line whitespace (must drop) or mid-line (keep).
    // Without the following EOL/non-WS we cannot decide — hold it.
    while end > 0 && (buf[end - 1] == b' ' || buf[end - 1] == b'\t') {
        end -= 1;
    }

    end
}

/// Decode an entire attachment from an iterator of wire chunks (test / non-web helper).
pub fn decode_transfer_stream<I>(
    encoding: TransferEncoding,
    chunks: I,
) -> Result<Vec<u8>, DecodeError>
where
    I: IntoIterator<Item = Vec<u8>>,
{
    let mut dec = StreamingTransferDecoder::new(encoding);
    let mut out = Vec::new();
    for chunk in chunks {
        out.extend_from_slice(&dec.push(&chunk)?);
    }
    out.extend_from_slice(&dec.finish()?);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_chunked_matches_oneshot() {
        let full = b"dGVzdCBhdHRhY2htZW50IGRhdGE="; // "test attachment data"
        let oneshot = base64_decode(full).unwrap();

        // Awkward splits including mid-quartet and whitespace folds.
        let chunks = vec![
            b"dGVz".to_vec(),
            b"dCBh\r\n".to_vec(),
            b"dHRhY2htZW50IGRhdGE".to_vec(),
            b"=".to_vec(),
        ];
        let streamed = decode_transfer_stream(TransferEncoding::Base64, chunks).unwrap();
        assert_eq!(streamed, oneshot);
        assert_eq!(streamed, b"test attachment data");
    }

    #[test]
    fn base64_single_byte_chunks() {
        let full = b"YQ==";
        let chunks: Vec<Vec<u8>> = full.iter().map(|&b| vec![b]).collect();
        let streamed = decode_transfer_stream(TransferEncoding::Base64, chunks).unwrap();
        assert_eq!(streamed, b"a");
    }

    #[test]
    fn qp_chunked_soft_break() {
        let chunks = vec![b"surely=".to_vec(), b"\r\nmathematics".to_vec()];
        let streamed = decode_transfer_stream(TransferEncoding::QuotedPrintable, chunks).unwrap();
        assert_eq!(streamed, b"surelymathematics");
    }

    #[test]
    fn qp_chunked_hex_split() {
        let chunks = vec![b"=C".to_vec(), b"2=A3".to_vec()];
        let streamed = decode_transfer_stream(TransferEncoding::QuotedPrintable, chunks).unwrap();
        assert_eq!(streamed, [0xC2, 0xA3]);
    }

    #[test]
    fn identity_passthrough() {
        let chunks = vec![b"hello ".to_vec(), b"world".to_vec()];
        let streamed = decode_transfer_stream(TransferEncoding::SevenBit, chunks).unwrap();
        assert_eq!(streamed, b"hello world");
    }

    #[test]
    fn qp_trailing_ws_across_chunks() {
        // "hello   \r\nworld" with WS split from EOL
        let chunks = vec![b"hello   ".to_vec(), b"\r\nworld".to_vec()];
        let streamed = decode_transfer_stream(TransferEncoding::QuotedPrintable, chunks).unwrap();
        assert_eq!(streamed, b"hello\r\nworld");
    }

    #[test]
    fn qp_soft_break_crlf_split() {
        let chunks = vec![b"ab=\r".to_vec(), b"\ncd".to_vec()];
        let streamed = decode_transfer_stream(TransferEncoding::QuotedPrintable, chunks).unwrap();
        assert_eq!(streamed, b"abcd");
    }
}
