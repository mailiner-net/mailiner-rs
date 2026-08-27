//! Quoted-printable decode (RFC 2045 §6.7).

use super::DecodeError;

/// Decode quoted-printable wire bytes into raw octets (no charset).
///
/// Policy:
/// - Strip trailing whitespace on each line.
/// - Remove soft line breaks (`=\r\n`, `=\n`, trailing `=` at EOL).
/// - Decode `=XX` hex sequences.
/// - Invalid `=XX`: treat `=` as literal and continue (lenient, common MUA behavior).
pub fn qp_decode(raw: &[u8]) -> Result<Vec<u8>, DecodeError> {
    // Work on bytes; treat input as Latin-1-ish wire text.
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    let bytes = raw;

    while i < bytes.len() {
        match bytes[i] {
            b'=' => {
                // Soft line break: =\r\n or =\n or = at end of input
                if i + 1 >= bytes.len() {
                    // trailing lone '=' at EOF — soft break / ignore
                    break;
                }
                let n1 = bytes[i + 1];
                if n1 == b'\n' {
                    i += 2;
                    continue;
                }
                if n1 == b'\r' {
                    if i + 2 < bytes.len() && bytes[i + 2] == b'\n' {
                        i += 3;
                    } else {
                        i += 2;
                    }
                    continue;
                }
                // =XX hex
                if i + 2 < bytes.len() {
                    let h1 = bytes[i + 1];
                    let h2 = bytes[i + 2];
                    if let (Some(a), Some(b)) = (from_hex(h1), from_hex(h2)) {
                        out.push((a << 4) | b);
                        i += 3;
                        continue;
                    }
                }
                // Lenient: literal '='
                out.push(b'=');
                i += 1;
            }
            b'\r' => {
                // Normalize CRLF / CR to LF in output? QP decode typically keeps
                // hard line breaks as the original line ending bytes after soft-break
                // removal. Keep as-is for hard breaks.
                out.push(b'\r');
                i += 1;
            }
            b'\n' => {
                // Strip trailing whitespace already handled per-line below via scan.
                out.push(b'\n');
                i += 1;
            }
            b' ' | b'\t' => {
                // Collect run of trailing WS until EOL — if EOL, drop; else keep.
                let start = i;
                while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
                    i += 1;
                }
                if i >= bytes.len() || bytes[i] == b'\r' || bytes[i] == b'\n' {
                    // trailing WS on line — drop
                    continue;
                }
                // not at EOL — keep the whitespace
                out.extend_from_slice(&bytes[start..i]);
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }

    Ok(out)
}

/// Encode raw octets as quoted-printable (RFC 2045 §6.7).
///
/// Soft-wraps at 76 columns with `=\r\n`. Hard newlines in `raw` should already
/// be CRLF; a lone `LF` is emitted as a hard CRLF so WASM `\n` bodies stay legal.
pub fn qp_encode(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + raw.len() / 8);
    let mut col = 0usize;
    let mut i = 0;
    while i < raw.len() {
        let b = raw[i];
        if b == b'\r' && i + 1 < raw.len() && raw[i + 1] == b'\n' {
            out.extend_from_slice(b"\r\n");
            col = 0;
            i += 2;
            continue;
        }
        if b == b'\n' {
            out.extend_from_slice(b"\r\n");
            col = 0;
            i += 1;
            continue;
        }
        if b == b'\r' {
            out.extend_from_slice(b"\r\n");
            col = 0;
            i += 1;
            continue;
        }

        let encode = b == b'=' || !(32..=126).contains(&b);
        let token: Vec<u8> = if encode {
            format!("={b:02X}").into_bytes()
        } else {
            vec![b]
        };

        // Leave room for a soft break (`=\r\n` is 3). Encoded tokens are 3 bytes.
        if col + token.len() > 75 {
            out.extend_from_slice(b"=\r\n");
            col = 0;
        }
        out.extend_from_slice(&token);
        col += token.len();
        i += 1;
    }
    out
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'A'..=b'F' => Some(b - b'A' + 10),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_break_crlf() {
        let raw = b"surely=\r\nmathematics";
        assert_eq!(qp_decode(raw).unwrap(), b"surelymathematics");
    }

    #[test]
    fn soft_break_lf() {
        let raw = b"surely=\nmathematics";
        assert_eq!(qp_decode(raw).unwrap(), b"surelymathematics");
    }

    #[test]
    fn hex_utf8_pound() {
        // £ in UTF-8 is C2 A3
        let raw = b"=C2=A3";
        assert_eq!(qp_decode(raw).unwrap(), [0xC2, 0xA3]);
    }

    #[test]
    fn trailing_ws_stripped() {
        let raw = b"hello   \r\nworld";
        assert_eq!(qp_decode(raw).unwrap(), b"hello\r\nworld");
    }

    #[test]
    fn invalid_hex_lenient() {
        let raw = b"=ZZ";
        // literal '=' then Z Z
        assert_eq!(qp_decode(raw).unwrap(), b"=ZZ");
    }

    #[test]
    fn plain_ascii() {
        assert_eq!(qp_decode(b"hello").unwrap(), b"hello");
    }

    #[test]
    fn mid_line_spaces_kept() {
        assert_eq!(qp_decode(b"a b c").unwrap(), b"a b c");
    }

    #[test]
    fn encode_roundtrip_ascii() {
        let raw = b"hello world";
        assert_eq!(qp_decode(&qp_encode(raw)).unwrap(), raw);
    }

    #[test]
    fn encode_utf8_and_equals() {
        let raw = "café = tea".as_bytes();
        let enc = qp_encode(raw);
        assert_eq!(qp_decode(&enc).unwrap(), raw);
        assert!(enc.windows(3).any(|w| w == b"=C3"));
        assert!(enc.windows(3).any(|w| w == b"=3D"));
    }

    #[test]
    fn encode_soft_wraps() {
        let raw = vec![b'a'; 200];
        let enc = qp_encode(&raw);
        assert!(enc.windows(3).any(|w| w == b"=\r\n"));
        assert_eq!(qp_decode(&enc).unwrap(), raw);
    }
}
