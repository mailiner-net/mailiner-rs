//! Autocrypt Level 1 header parse (`Autocrypt: addr=…; keydata=…`).

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

/// `prefer-encrypt` attribute (RFC 8281 / Autocrypt Level 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreferEncrypt {
    #[default]
    Nopreference,
    Mutual,
}

impl PreferEncrypt {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Nopreference => "nopreference",
            Self::Mutual => "mutual",
        }
    }

    pub fn parse(s: &str) -> Self {
        if s.trim().eq_ignore_ascii_case("mutual") {
            Self::Mutual
        } else {
            Self::Nopreference
        }
    }
}

/// One Autocrypt header (critical attributes only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutocryptHeader {
    pub addr: String,
    pub prefer_encrypt: PreferEncrypt,
    /// Decoded `keydata` (binary transferable public key).
    pub keydata: Vec<u8>,
}

/// Parse a single Autocrypt header value (after unfolding).
pub fn parse_autocrypt_header(value: &str) -> Option<AutocryptHeader> {
    let mut addr = None;
    let mut prefer = PreferEncrypt::Nopreference;
    let mut keydata_b64 = String::new();
    for attr in split_attrs(value) {
        let Some((name, raw)) = attr.split_once('=') else {
            continue;
        };
        let name = name.trim();
        let val = unquote(raw.trim());
        if name.eq_ignore_ascii_case("addr") {
            addr = Some(val.to_string());
        } else if name.eq_ignore_ascii_case("prefer-encrypt") {
            prefer = PreferEncrypt::parse(val);
        } else if name.eq_ignore_ascii_case("keydata") {
            keydata_b64.push_str(val);
        }
    }
    let addr = addr.filter(|s| !s.is_empty())?;
    let cleaned: String = keydata_b64
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    let keydata = B64.decode(cleaned.as_bytes()).ok()?;
    if keydata.is_empty() {
        return None;
    }
    Some(AutocryptHeader {
        addr,
        prefer_encrypt: prefer,
        keydata,
    })
}

/// Parse every `Autocrypt:` field in a raw header block.
pub fn parse_autocrypt_headers(raw: &str) -> Vec<AutocryptHeader> {
    unfold_header_fields(raw)
        .into_iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("Autocrypt"))
        .filter_map(|(_, value)| parse_autocrypt_header(&value))
        .collect()
}

/// Latin-1 fallback for invalid UTF-8 (same as AuthResults).
pub fn parse_autocrypt_headers_bytes(bytes: &[u8]) -> Vec<AutocryptHeader> {
    match std::str::from_utf8(bytes) {
        Ok(s) => parse_autocrypt_headers(s),
        Err(_) => {
            let raw: String = bytes.iter().map(|&b| b as char).collect();
            parse_autocrypt_headers(&raw)
        }
    }
}

fn split_attrs(value: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_quote = false;
    for (i, c) in value.char_indices() {
        match c {
            '"' => in_quote = !in_quote,
            ';' if !in_quote => {
                let piece = value[start..i].trim();
                if !piece.is_empty() {
                    out.push(piece);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    let piece = value[start..].trim();
    if !piece.is_empty() {
        out.push(piece);
    }
    out
}

fn unquote(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s)
}

fn unfold_header_fields(raw: &str) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    let mut name = String::new();
    let mut value = String::new();
    let mut have = false;

    for line in raw.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            break;
        }
        let folded = line.starts_with([' ', '\t']);
        if folded {
            if have {
                value.push(' ');
                value.push_str(line.trim_start());
            }
            continue;
        }
        if have {
            fields.push((std::mem::take(&mut name), std::mem::take(&mut value)));
            have = false;
        }
        let Some((n, v)) = line.split_once(':') else {
            continue;
        };
        name = n.trim().to_string();
        value = v.trim_start().to_string();
        have = true;
    }
    if have {
        fields.push((name, value));
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_level1_header() {
        let key = B64.encode(b"dummy-openpgp-key");
        let value = format!("addr=alice@example.com; prefer-encrypt=mutual; keydata={key}");
        let parsed = parse_autocrypt_header(&value).expect("parse");
        assert_eq!(parsed.addr, "alice@example.com");
        assert_eq!(parsed.prefer_encrypt, PreferEncrypt::Mutual);
        assert_eq!(parsed.keydata, b"dummy-openpgp-key");
    }

    #[test]
    fn default_prefer_encrypt_is_nopreference() {
        let key = B64.encode(b"k");
        let parsed = parse_autocrypt_header(&format!("addr=bob@ex.com; keydata={key}")).unwrap();
        assert_eq!(parsed.prefer_encrypt, PreferEncrypt::Nopreference);
    }

    #[test]
    fn folded_keydata_from_header_block() {
        let key = B64.encode(b"folded-key-bytes");
        let (a, b) = key.split_at(key.len() / 2);
        let block = format!(
            "From: Alice <alice@example.com>\r\n\
             Autocrypt: addr=alice@example.com; prefer-encrypt=mutual;\r\n\
             \tkeydata={a}\r\n\
             \t{b}\r\n\
             Subject: hi\r\n\r\n"
        );
        let parsed = parse_autocrypt_headers(&block);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].addr, "alice@example.com");
        assert_eq!(parsed[0].keydata, b"folded-key-bytes");
    }

    #[test]
    fn rejects_missing_addr_or_keydata() {
        assert!(parse_autocrypt_header("prefer-encrypt=mutual; keydata=YQ==").is_none());
        assert!(parse_autocrypt_header("addr=alice@example.com").is_none());
    }

    #[test]
    fn ignores_non_autocrypt_headers() {
        let block = "Subject: hi\r\nFrom: a@b.c\r\n\r\n";
        assert!(parse_autocrypt_headers(block).is_empty());
    }
}
