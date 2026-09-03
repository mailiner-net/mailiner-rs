//! Convert a raw RFC 822 message (IMAP `BODY.PEEK[]`) to text.

/// Decode raw message octets for display as preformatted text.
///
/// Valid UTF-8 is kept as-is. Anything else is mapped byte-for-byte (Latin-1)
/// so every octet stays visible. CRLF / lone CR become `\n` for `<pre>`.
pub fn source_bytes_to_text(bytes: &[u8]) -> String {
    let raw = match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    };
    normalize_source_newlines(&raw)
}

fn normalize_source_newlines(raw: &str) -> String {
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
    fn utf8_source_keeps_text() {
        let raw = b"From: Alice <alice@example.com>\r\nSubject: Hello\r\n\r\nHi\r\n";
        assert_eq!(
            source_bytes_to_text(raw),
            "From: Alice <alice@example.com>\nSubject: Hello\n\nHi\n"
        );
    }

    #[test]
    fn lf_only_source_unchanged() {
        let raw = b"Subject: already lf\n\nbody\n";
        assert_eq!(source_bytes_to_text(raw), "Subject: already lf\n\nbody\n");
    }

    #[test]
    fn lone_cr_becomes_newline() {
        let raw = b"Subject: old mac\r\rbody\r";
        assert_eq!(source_bytes_to_text(raw), "Subject: old mac\n\nbody\n");
    }

    #[test]
    fn invalid_utf8_is_latin1() {
        // 0xE9 is "é" in Latin-1 and invalid as a standalone UTF-8 sequence.
        let raw = b"Subject: caf\xE9\r\n\r\nbody\r\n";
        assert_eq!(source_bytes_to_text(raw), "Subject: café\n\nbody\n");
    }

    #[test]
    fn empty_source() {
        assert_eq!(source_bytes_to_text(b""), "");
    }

    #[test]
    fn mock_connector_source_decode() {
        let text = source_bytes_to_text(mailiner_core::mock_rfc822());
        assert!(text.contains("From: sender@example.com"));
        assert!(text.contains("Subject: Test Message"));
        assert!(text.contains("Hello from MockConnector."));
        assert!(!text.contains('\r'));
        assert!(text.contains("\n\n"));
    }
}
