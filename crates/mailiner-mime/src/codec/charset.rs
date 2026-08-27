//! Charset conversion aligned with the TypeScript mimetree charset fallback chain.

use super::DecodeError;
use encoding_rs::Encoding;

/// Decode bytes to Unicode using the TS-aligned fallback chain:
/// requested (lenient) → UTF-8 (strict) → ISO-8859-15 (lenient) → latin1 byte→char.
pub fn charset_decode(buf: &[u8], from_charset: &str) -> Result<String, DecodeError> {
    if buf.is_empty() {
        return Ok(String::new());
    }

    // 1) Requested charset (fatal: false — accept replacements)
    if let Some(enc) = normalize_label(from_charset) {
        let (cow, _used, _had_errors) = enc.decode(buf);
        return Ok(cow.into_owned());
    }

    // 2) UTF-8 fatal:true — reject if errors
    {
        let (cow, _used, had_errors) = encoding_rs::UTF_8.decode(buf);
        if !had_errors {
            return Ok(cow.into_owned());
        }
    }

    // 3) ISO-8859-15 lenient
    let (cow, _used, _had_errors) = encoding_rs::ISO_8859_15.decode(buf);
    Ok(cow.into_owned())
}

/// Last-resort lossy latin1 (each byte → U+00xx), like TS `arr2str`.
#[allow(dead_code)]
pub fn latin1_lossy(buf: &[u8]) -> String {
    buf.iter().map(|&b| b as char).collect()
}

fn normalize_label(cs: &str) -> Option<&'static Encoding> {
    Encoding::for_label(cs.trim().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_ok() {
        assert_eq!(charset_decode("café".as_bytes(), "utf-8").unwrap(), "café");
    }

    #[test]
    fn iso_8859_1_e9() {
        let buf = [0xE9u8]; // é in ISO-8859-1
        let s = charset_decode(&buf, "iso-8859-1").unwrap();
        assert_eq!(s, "é");
    }

    #[test]
    fn windows_1252() {
        let buf = [0x80u8]; // euro in windows-1252
        let s = charset_decode(&buf, "windows-1252").unwrap();
        assert_eq!(s, "€");
    }

    #[test]
    fn invalid_utf8_label_still_decodes_lenient() {
        // labeled utf-8 but invalid sequence — encoding_rs replaces
        let buf = [0xFFu8];
        let s = charset_decode(&buf, "utf-8").unwrap();
        assert!(!s.is_empty());
    }

    #[test]
    fn unknown_label_falls_through() {
        let buf = b"hello";
        let s = charset_decode(buf, "x-totally-unknown").unwrap();
        assert_eq!(s, "hello");
    }
}
