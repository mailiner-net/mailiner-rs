//! Base64 decode for MIME transfer encoding.

use super::DecodeError;
use base64::{engine::general_purpose::STANDARD, Engine};

/// Decode base64 wire bytes into raw octets.
///
/// - Strips all whitespace before decode.
/// - Accepts missing `=` padding by normalizing length mod 4.
/// - Hard-fails on illegal alphabet characters.
pub fn base64_decode(raw: &[u8]) -> Result<Vec<u8>, DecodeError> {
    let mut cleaned = Vec::with_capacity(raw.len());
    for &b in raw {
        match b {
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => cleaned.push(b),
        }
    }

    // Normalize padding
    let rem = cleaned.len() % 4;
    if rem != 0 {
        cleaned.extend(std::iter::repeat_n(b'=', 4 - rem));
    }

    STANDARD
        .decode(&cleaned)
        .map_err(|e| DecodeError::Base64(e.to_string()))
}

/// Encode raw octets as base64 with 76-column CRLF wrapping (RFC 2045).
pub fn base64_encode(raw: &[u8]) -> Vec<u8> {
    let enc = STANDARD.encode(raw);
    let bytes = enc.as_bytes();
    if bytes.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(bytes.len() + bytes.len() / 19);
    for (i, chunk) in bytes.chunks(76).enumerate() {
        if i > 0 {
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(chunk);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic() {
        assert_eq!(base64_decode(b"YQ==").unwrap(), b"a");
    }

    #[test]
    fn missing_padding() {
        // "hi" = aGk=
        assert_eq!(base64_decode(b"aGk").unwrap(), b"hi");
    }

    #[test]
    fn with_crlf_folding() {
        let raw = b"dGVyZSDD\r\nlcOEw5bDlQ=="; // tere ÕÄÖÕ
        let out = base64_decode(raw).unwrap();
        assert_eq!(
            out,
            [116, 101, 114, 101, 32, 195, 149, 195, 132, 195, 150, 195, 149]
        );
    }

    #[test]
    fn illegal_alphabet() {
        assert!(matches!(
            base64_decode(b"@@@@"),
            Err(DecodeError::Base64(_))
        ));
    }

    #[test]
    fn empty() {
        assert_eq!(base64_decode(b"").unwrap(), b"");
    }

    #[test]
    fn encode_roundtrip() {
        let raw = b"PNGDATA";
        assert_eq!(base64_decode(&base64_encode(raw)).unwrap(), raw);
    }

    #[test]
    fn encode_wraps_at_76() {
        let raw = vec![0u8; 80];
        let enc = base64_encode(&raw);
        assert!(enc.windows(2).any(|w| w == b"\r\n"));
        assert_eq!(base64_decode(&enc).unwrap(), raw);
    }
}
