//! Convert a raw RFC 5322 header block (IMAP `BODY.PEEK[HEADER]`) to text.

/// Decode raw header octets for display as preformatted text.
///
/// Valid UTF-8 is kept as-is. Anything else is mapped byte-for-byte (Latin-1)
/// so every octet stays visible. CRLF / lone CR become `\n` for `<pre>`.
pub fn headers_bytes_to_text(bytes: &[u8]) -> String {
    let raw = match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    };
    normalize_header_newlines(&raw)
}

fn normalize_header_newlines(raw: &str) -> String {
    if !raw.contains('\r') {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_headers_keep_text() {
        let raw = b"From: Alice <alice@example.com>\r\nSubject: Hello\r\n\r\n";
        assert_eq!(
            headers_bytes_to_text(raw),
            "From: Alice <alice@example.com>\nSubject: Hello\n\n"
        );
    }

    #[test]
    fn lf_only_headers_unchanged() {
        let raw = b"Subject: already lf\n";
        assert_eq!(headers_bytes_to_text(raw), "Subject: already lf\n");
    }

    #[test]
    fn lone_cr_becomes_newline() {
        let raw = b"Subject: old mac\rNext: yes\r";
        assert_eq!(headers_bytes_to_text(raw), "Subject: old mac\nNext: yes\n");
    }

    #[test]
    fn invalid_utf8_is_latin1() {
        // 0xE9 is "é" in Latin-1 and invalid as a standalone UTF-8 sequence.
        let raw = b"Subject: caf\xE9\r\n";
        assert_eq!(headers_bytes_to_text(raw), "Subject: café\n");
    }

    #[test]
    fn empty_headers() {
        assert_eq!(headers_bytes_to_text(b""), "");
    }

    #[test]
    fn mock_connector_headers_decode() {
        let text = headers_bytes_to_text(mailiner_core::MOCK_RFC822_HEADERS);
        assert!(text.contains("From: Sender <sender@example.com>"));
        assert!(text.contains("Subject: Test Message"));
        assert!(!text.contains('\r'));
        assert!(text.ends_with("\n\n"));
    }
}
