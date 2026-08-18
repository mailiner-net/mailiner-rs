//! RFC 5322 / MIME writer: headers, folding, RFC 2047, multipart.

use thiserror::Error;
use uuid::Uuid;

/// Errors while serializing a MIME message.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WriteError {
    /// A header name contained a colon, CR, or LF.
    #[error("invalid header name: {0}")]
    InvalidHeaderName(String),
    /// Multipart part is missing a boundary.
    #[error("multipart boundary must not be empty")]
    EmptyBoundary,
}

/// One MIME part: headers plus a leaf or nested multipart body.
#[derive(Debug, Clone)]
pub struct MimePart {
    /// Raw Unicode field values. For multipart parts, do **not** set `Content-Type`;
    /// [`serialize_message`] / the multipart writer synthesize it from [`MimeBody::Multipart`].
    pub headers: Vec<(String, String)>,
    /// Part body.
    pub body: MimeBody,
}

/// MIME part payload.
#[derive(Debug, Clone)]
pub enum MimeBody {
    /// Already transfer-encoded octets (quoted-printable or base64).
    Octets(Vec<u8>),
    /// Nested multipart.
    Multipart {
        /// e.g. `mixed`, `alternative`, `related`.
        subtype: String,
        /// Boundary token (without leading `--`).
        boundary: String,
        /// Child parts.
        parts: Vec<MimePart>,
    },
}

/// Serialize a complete RFC 5322 message (headers + root part). Always CRLF.
///
/// `headers` are message-level (From, To, Subject, Date, Message-ID, …) as raw
/// Unicode; this function is the only RFC 2047 encoder. `MIME-Version: 1.0` is
/// inserted if missing. Multipart `Content-Type` is taken from `root`, not from
/// a caller-supplied header.
pub fn serialize_message(
    headers: &[(String, String)],
    root: &MimePart,
) -> Result<Vec<u8>, WriteError> {
    let mut out = Vec::new();
    let mut saw_mime_version = false;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("mime-version") {
            saw_mime_version = true;
        }
        if name.eq_ignore_ascii_case("content-type") {
            // Writer owns Content-Type from `root`.
            continue;
        }
        write_header(&mut out, name, value)?;
    }
    if !saw_mime_version {
        write_header(&mut out, "MIME-Version", "1.0")?;
    }
    write_part_content_headers(&mut out, root)?;
    out.extend_from_slice(b"\r\n");
    write_part_body(&mut out, root)?;
    Ok(out)
}

/// Generate a MIME boundary: `=_mlnr_` plus 24 hex characters.
pub fn generate_boundary() -> String {
    let hex = Uuid::new_v4().simple().to_string();
    format!("=_mlnr_{}", &hex[..24])
}

/// Encode an unstructured header value (Subject, …). ASCII is left as-is;
/// anything else becomes RFC 2047 encoded-words (UTF-8 Q).
pub fn encode_unstructured(s: &str) -> String {
    if is_all_printable_ascii(s) {
        s.to_string()
    } else {
        encode_2047_q(s)
    }
}

/// Format `Name <email>` or just `email`. Display name is RFC 2047 when needed.
pub fn format_mailbox(name: Option<&str>, email: &str) -> String {
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        None => email.to_string(),
        Some(n) if is_all_printable_ascii(n) && !needs_quoted_display(n) => {
            format!("{n} <{email}>")
        }
        Some(n) if is_all_printable_ascii(n) => {
            format!("\"{}\" <{email}>", escape_quoted(n))
        }
        Some(n) => format!("{} <{email}>", encode_2047_q(n)),
    }
}

/// `Content-Disposition` with ASCII `filename=` and RFC 2231 `filename*` when needed.
pub fn format_disposition(kind: &str, filename: Option<&str>) -> String {
    let Some(name) = filename.map(str::trim).filter(|s| !s.is_empty()) else {
        return kind.to_string();
    };
    if is_all_printable_ascii(name) && !name.contains('"') && !name.contains('\\') && !name.contains(';')
    {
        return format!("{kind}; filename=\"{name}\"");
    }
    let encoded = rfc2231_encode(name);
    if is_all_printable_ascii(name) {
        format!("{kind}; filename=\"{}\"; filename*={encoded}", escape_quoted(name))
    } else {
        // ASCII fallback filename for old MUAs.
        let fallback: String = name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            })
            .collect();
        format!("{kind}; filename=\"{fallback}\"; filename*={encoded}")
    }
}

fn write_part_content_headers(out: &mut Vec<u8>, part: &MimePart) -> Result<(), WriteError> {
    match &part.body {
        MimeBody::Multipart { subtype, boundary, .. } => {
            if boundary.is_empty() {
                return Err(WriteError::EmptyBoundary);
            }
            let ct = format!("multipart/{subtype}; boundary=\"{boundary}\"");
            write_header(out, "Content-Type", &ct)?;
            for (name, value) in &part.headers {
                if name.eq_ignore_ascii_case("content-type") {
                    continue;
                }
                write_header(out, name, value)?;
            }
        }
        MimeBody::Octets(_) => {
            for (name, value) in &part.headers {
                write_header(out, name, value)?;
            }
        }
    }
    Ok(())
}

fn write_part_body(out: &mut Vec<u8>, part: &MimePart) -> Result<(), WriteError> {
    match &part.body {
        MimeBody::Octets(bytes) => {
            out.extend_from_slice(bytes);
            if !bytes.ends_with(b"\r\n") {
                out.extend_from_slice(b"\r\n");
            }
        }
        MimeBody::Multipart {
            boundary, parts, ..
        } => {
            if boundary.is_empty() {
                return Err(WriteError::EmptyBoundary);
            }
            for child in parts {
                out.extend_from_slice(b"--");
                out.extend_from_slice(boundary.as_bytes());
                out.extend_from_slice(b"\r\n");
                write_part_content_headers(out, child)?;
                out.extend_from_slice(b"\r\n");
                write_part_body(out, child)?;
            }
            out.extend_from_slice(b"--");
            out.extend_from_slice(boundary.as_bytes());
            out.extend_from_slice(b"--\r\n");
        }
    }
    Ok(())
}

fn write_header(out: &mut Vec<u8>, name: &str, value: &str) -> Result<(), WriteError> {
    if name.is_empty()
        || name.bytes().any(|b| b == b':' || b == b'\r' || b == b'\n' || b.is_ascii_whitespace())
    {
        return Err(WriteError::InvalidHeaderName(name.to_string()));
    }
    let encoded = encode_unstructured(value);
    // Preferred 78-octet lines (RFC 5322 §2.2.3). Name + ": " counts.
    let prefix = format!("{name}: ");
    out.extend_from_slice(prefix.as_bytes());
    fold_into(out, &encoded, prefix.len());
    out.extend_from_slice(b"\r\n");
    Ok(())
}

fn fold_into(out: &mut Vec<u8>, value: &str, first_col: usize) {
    // Encoded-words already contain spaces between them; fold on those.
    // ASCII values fold on existing whitespace.
    let mut col = first_col;
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let remaining = &bytes[i..];
        let next_break = remaining
            .iter()
            .position(|&b| b == b' ')
            .unwrap_or(remaining.len());
        let token_len = if next_break == remaining.len() {
            remaining.len()
        } else {
            next_break + 1 // include the space with this token
        };
        if col > first_col && col + token_len > 78 {
            out.extend_from_slice(b"\r\n ");
            col = 1;
        }
        out.extend_from_slice(&remaining[..token_len]);
        col += token_len;
        i += token_len;
    }
}

fn is_all_printable_ascii(s: &str) -> bool {
    s.bytes()
        .all(|b| b == b'\t' || (32..=126).contains(&b))
}

fn needs_quoted_display(n: &str) -> bool {
    n.bytes().any(|b| matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b':' | b';' | b'@' | b'\\' | b',' | b'.' | b'"'
    )) || n.contains(' ')
}

fn escape_quoted(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn encode_2047_q(s: &str) -> String {
    // Split into encoded-words of at most 75 octets including chrome.
    // Chrome: "=?UTF-8?Q?" (10) + "?=" (2) = 12; payload ≤ 63.
    const PAYLOAD: usize = 63;
    let mut words = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;

    let flush = |buf: &mut String, len: &mut usize, words: &mut Vec<String>| {
        if buf.is_empty() {
            return;
        }
        words.push(format!("=?UTF-8?Q?{buf}?="));
        buf.clear();
        *len = 0;
    };

    for ch in s.chars() {
        let piece = q_encode_char(ch);
        if current_len + piece.len() > PAYLOAD && !current.is_empty() {
            flush(&mut current, &mut current_len, &mut words);
        }
        current.push_str(&piece);
        current_len += piece.len();
    }
    flush(&mut current, &mut current_len, &mut words);
    words.join(" ")
}

fn q_encode_char(ch: char) -> String {
    if ch == ' ' {
        return "_".to_string();
    }
    if ch.is_ascii() && ch.is_ascii_graphic() && ch != '=' && ch != '?' && ch != '_' {
        return ch.to_string();
    }
    let mut tmp = [0u8; 4];
    let encoded = ch.encode_utf8(&mut tmp);
    encoded
        .as_bytes()
        .iter()
        .map(|b| format!("={b:02X}"))
        .collect()
}

fn rfc2231_encode(s: &str) -> String {
    let mut out = String::from("UTF-8''");
    for b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(*b, b'-' | b'.' | b'_' | b'~') {
            out.push(*b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{base64_encode, qp_encode};

    fn has_crlf_only(bytes: &[u8]) -> bool {
        !bytes.contains(&b'\n') || bytes.windows(2).filter(|w| w[1] == b'\n').all(|w| w[0] == b'\r')
    }

    #[test]
    fn crlf_everywhere() {
        let root = MimePart {
            headers: vec![
                ("Content-Type".into(), "text/plain; charset=UTF-8".into()),
                ("Content-Transfer-Encoding".into(), "quoted-printable".into()),
            ],
            body: MimeBody::Octets(qp_encode(b"Hello")),
        };
        let msg = serialize_message(
            &[
                ("From".into(), "me@example.com".into()),
                ("To".into(), "you@example.com".into()),
                ("Subject".into(), "Hi".into()),
            ],
            &root,
        )
        .unwrap();
        assert!(has_crlf_only(&msg), "bare LF in {:?}", String::from_utf8_lossy(&msg));
        assert!(msg.windows(2).any(|w| w == b"\r\n"));
    }

    #[test]
    fn rfc2047_subject() {
        let encoded = encode_unstructured("Café résumé");
        assert!(encoded.starts_with("=?UTF-8?Q?"), "{encoded}");
        assert!(encoded.contains("=C3=A9") || encoded.contains("=C3=A9"));
    }

    #[test]
    fn folds_long_subject() {
        let long = "A ".repeat(60) + "end";
        let root = MimePart {
            headers: vec![("Content-Type".into(), "text/plain; charset=UTF-8".into())],
            body: MimeBody::Octets(b"x".to_vec()),
        };
        let msg = serialize_message(&[("Subject".into(), long)], &root).unwrap();
        let text = String::from_utf8(msg).unwrap();
        assert!(text.contains("\r\n "), "expected folded Subject:\n{text}");
    }

    #[test]
    fn multipart_alternative() {
        let boundary = generate_boundary();
        assert!(boundary.starts_with("=_mlnr_"));
        assert_eq!(boundary.len(), "=_mlnr_".len() + 24);

        let plain = MimePart {
            headers: vec![
                ("Content-Type".into(), "text/plain; charset=UTF-8".into()),
                ("Content-Transfer-Encoding".into(), "quoted-printable".into()),
            ],
            body: MimeBody::Octets(qp_encode(b"plain")),
        };
        let html = MimePart {
            headers: vec![
                ("Content-Type".into(), "text/html; charset=UTF-8".into()),
                ("Content-Transfer-Encoding".into(), "quoted-printable".into()),
            ],
            body: MimeBody::Octets(qp_encode(b"<p>html</p>")),
        };
        let root = MimePart {
            headers: vec![],
            body: MimeBody::Multipart {
                subtype: "alternative".into(),
                boundary: boundary.clone(),
                parts: vec![plain, html],
            },
        };
        let msg = serialize_message(&[("From".into(), "a@b.co".into())], &root).unwrap();
        let s = String::from_utf8(msg).unwrap();
        assert!(s.contains("MIME-Version: 1.0"));
        assert!(s.contains("Content-Type: multipart/alternative;"));
        assert!(s.contains(&format!("boundary=\"{boundary}\"")));
        assert!(s.contains(&format!("--{boundary}\r\n")));
        assert!(s.contains(&format!("--{boundary}--\r\n")));
        assert!(!s.contains("Bcc:"));
    }

    #[test]
    fn mixed_attachment_and_rfc2231_filename() {
        let mix = generate_boundary();
        let body = MimePart {
            headers: vec![
                ("Content-Type".into(), "text/plain; charset=UTF-8".into()),
                ("Content-Transfer-Encoding".into(), "quoted-printable".into()),
            ],
            body: MimeBody::Octets(qp_encode(b"hi")),
        };
        let att = MimePart {
            headers: vec![
                ("Content-Type".into(), "application/pdf".into()),
                ("Content-Transfer-Encoding".into(), "base64".into()),
                (
                    "Content-Disposition".into(),
                    format_disposition("attachment", Some("café.pdf")),
                ),
            ],
            body: MimeBody::Octets(base64_encode(b"%PDF")),
        };
        let root = MimePart {
            headers: vec![],
            body: MimeBody::Multipart {
                subtype: "mixed".into(),
                boundary: mix,
                parts: vec![body, att],
            },
        };
        let msg = serialize_message(&[("Subject".into(), "file".into())], &root).unwrap();
        let s = String::from_utf8(msg).unwrap();
        assert!(s.contains("filename*="));
        assert!(s.contains("UTF-8''caf%C3%A9.pdf"));
    }

    #[test]
    fn related_with_cid() {
        let rel = generate_boundary();
        let html = MimePart {
            headers: vec![
                ("Content-Type".into(), "text/html; charset=UTF-8".into()),
                ("Content-Transfer-Encoding".into(), "quoted-printable".into()),
            ],
            body: MimeBody::Octets(qp_encode(b"<img src=\"cid:img1@mailiner\">")),
        };
        let img = MimePart {
            headers: vec![
                ("Content-Type".into(), "image/png".into()),
                ("Content-Transfer-Encoding".into(), "base64".into()),
                ("Content-ID".into(), "<img1@mailiner>".into()),
                (
                    "Content-Disposition".into(),
                    format_disposition("inline", Some("dot.png")),
                ),
            ],
            body: MimeBody::Octets(base64_encode(b"PNG")),
        };
        let root = MimePart {
            headers: vec![],
            body: MimeBody::Multipart {
                subtype: "related".into(),
                boundary: rel,
                parts: vec![html, img],
            },
        };
        let msg = serialize_message(&[], &root).unwrap();
        let s = String::from_utf8(msg).unwrap();
        assert!(s.contains("Content-ID: <img1@mailiner>"));
        assert!(s.contains("cid:img1@mailiner"));
    }

    #[test]
    fn format_mailbox_ascii_and_unicode() {
        assert_eq!(format_mailbox(None, "a@b.co"), "a@b.co");
        assert_eq!(format_mailbox(Some("Ada"), "a@b.co"), "Ada <a@b.co>");
        let u = format_mailbox(Some("Åsa"), "a@b.co");
        assert!(u.starts_with("=?UTF-8?Q?"), "{u}");
        assert!(u.ends_with(" <a@b.co>"));
    }

    #[test]
    fn boundary_unique() {
        let a = generate_boundary();
        let b = generate_boundary();
        assert_ne!(a, b);
    }
}
