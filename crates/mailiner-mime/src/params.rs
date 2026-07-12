//! MIME parameter normalization: uppercased keys, RFC 2231, MIME-words.

use std::collections::BTreeMap;

use crate::codec::{base64_decode, charset_decode, qp_decode};

/// Normalize raw parameter pairs into uppercased keys with decoded values.
///
/// Handles:
/// - Key uppercasing
/// - RFC 2231 continuations (`FILENAME*0*`, `FILENAME*1`, …)
/// - Percent-decoding with optional charset prefix (`utf-8''…`)
/// - MIME-word decoding (`=?UTF-8?B?…?=`, `=?UTF-8?Q?…?=`)
pub fn normalize_params<'a, K, V, I>(params: Option<I>) -> BTreeMap<String, String>
where
    K: AsRef<str> + 'a,
    V: AsRef<str> + 'a,
    I: IntoIterator<Item = &'a (K, V)>,
{
    let mut raw: BTreeMap<String, String> = BTreeMap::new();
    if let Some(iter) = params {
        for (k, v) in iter {
            raw.insert(k.as_ref().to_ascii_uppercase(), v.as_ref().to_string());
        }
    }

    // Collect RFC 2231 continuation families: NAME*0*, NAME*1, NAME*
    let mut continuations: BTreeMap<String, Vec<(usize, bool, String)>> = BTreeMap::new();
    let mut plain: BTreeMap<String, String> = BTreeMap::new();

    for (key, value) in raw {
        if let Some((base, encoded, idx)) = parse_rfc2231_key(&key) {
            continuations
                .entry(base)
                .or_default()
                .push((idx, encoded, value));
        } else {
            plain.insert(key, mime_words_decode(&value));
        }
    }

    for (base, mut parts) in continuations {
        parts.sort_by_key(|(i, _, _)| *i);
        let mut combined = String::new();
        let mut charset: Option<String> = None;
        for (i, encoded, value) in parts {
            let piece = if encoded {
                if i == 0 {
                    // charset'lang'value
                    let (cs, rest) = split_rfc2231_charset(&value);
                    charset = cs;
                    percent_decode(rest)
                } else {
                    percent_decode(&value)
                }
            } else {
                value
            };
            combined.push_str(&piece);
        }
        let decoded = if let Some(cs) = charset {
            // combined is raw bytes as latin1 string from percent_decode
            let bytes: Vec<u8> = combined.chars().map(|c| c as u8).collect();
            charset_decode(&bytes, &cs).unwrap_or(combined)
        } else {
            mime_words_decode(&combined)
        };
        plain.insert(base, decoded);
    }

    plain
}

fn parse_rfc2231_key(key: &str) -> Option<(String, bool, usize)> {
    // FILENAME*0* → (FILENAME, true, 0)
    // FILENAME*0 → (FILENAME, false, 0)
    // FILENAME* → (FILENAME, true, 0) single extended
    if !key.contains('*') {
        return None;
    }
    let parts: Vec<&str> = key.split('*').collect();
    // "FILENAME", "0", ""  or "FILENAME", "0" or "FILENAME", ""
    if parts.len() < 2 {
        return None;
    }
    let base = parts[0].to_string();
    if parts.len() == 2 && parts[1].is_empty() {
        // NAME*
        return Some((base, true, 0));
    }
    if parts.len() >= 2 {
        let idx = parts[1].parse::<usize>().unwrap_or(0);
        let encoded = parts.len() >= 3; // trailing * after index
        // FILENAME*0* → parts = ["FILENAME", "0", ""]
        let encoded = encoded || (parts.len() == 3);
        return Some((base, encoded, idx));
    }
    None
}

fn split_rfc2231_charset(value: &str) -> (Option<String>, &str) {
    // charset'language'value
    let mut parts = value.splitn(3, '\'');
    let cs = parts.next();
    let _lang = parts.next();
    if let Some(rest) = parts.next() {
        (cs.map(|s| s.to_string()), rest)
    } else {
        (None, value)
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(a), Some(b)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2])) {
                out.push((a << 4) | b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    // Interpret as latin1 so non-utf8 octets survive for charset_decode later
    out.iter().map(|&b| b as char).collect()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'A'..=b'F' => Some(b - b'A' + 10),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    }
}

/// Decode MIME encoded-words in a header value (RFC 2047).
pub fn mime_words_decode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' && i + 1 < bytes.len() && bytes[i + 1] == b'?' {
            if let Some((decoded, end)) = try_decode_mime_word(&input[i..]) {
                out.push_str(&decoded);
                i += end;
                // skip a single space between adjacent mime words
                if i < bytes.len() && bytes[i] == b' ' {
                    // look ahead for next =?
                    if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                        i += 1;
                    }
                }
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn try_decode_mime_word(s: &str) -> Option<(String, usize)> {
    // =?charset?Q|B?text?=
    if !s.starts_with("=?") {
        return None;
    }
    let rest = &s[2..];
    let charset_end = rest.find('?')?;
    let charset = &rest[..charset_end];
    let rest = &rest[charset_end + 1..];
    let enc_end = rest.find('?')?;
    let enc = rest[..enc_end].to_ascii_uppercase();
    let rest = &rest[enc_end + 1..];
    let text_end = rest.find("?=")?;
    let text = &rest[..text_end];
    let total_len = 2 + charset_end + 1 + enc_end + 1 + text_end + 2;

    let raw = match enc.as_str() {
        "B" => base64_decode(text.as_bytes()).ok()?,
        "Q" => {
            // Q-encoding: _ is space, =XX hex, otherwise literal
            let mut buf = Vec::new();
            let tb = text.as_bytes();
            let mut j = 0;
            while j < tb.len() {
                match tb[j] {
                    b'_' => {
                        buf.push(b' ');
                        j += 1;
                    }
                    b'=' if j + 2 < tb.len() => {
                        if let (Some(a), Some(b)) = (from_hex(tb[j + 1]), from_hex(tb[j + 2])) {
                            buf.push((a << 4) | b);
                            j += 3;
                        } else {
                            buf.push(tb[j]);
                            j += 1;
                        }
                    }
                    c => {
                        buf.push(c);
                        j += 1;
                    }
                }
            }
            buf
        }
        _ => return None,
    };

    let decoded = charset_decode(&raw, charset).unwrap_or_else(|_| {
        raw.iter().map(|&b| b as char).collect()
    });
    Some((decoded, total_len))
}

// silence unused qp in params (used by codec re-export path)
#[allow(dead_code)]
fn _qp_touch() {
    let _ = qp_decode;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_ascii_filename() {
        let pairs = vec![("filename".to_string(), "report.pdf".to_string())];
        let m = normalize_params(Some(&pairs));
        assert_eq!(m.get("FILENAME").map(|s| s.as_str()), Some("report.pdf"));
    }

    #[test]
    fn rfc2231_utf8_filename() {
        let pairs = vec![(
            "filename*".to_string(),
            "UTF-8''%C3%A9.pdf".to_string(),
        )];
        let m = normalize_params(Some(&pairs));
        assert_eq!(m.get("FILENAME").map(|s| s.as_str()), Some("é.pdf"));
    }

    #[test]
    fn rfc2231_continuations() {
        let pairs = vec![
            ("filename*0*".to_string(), "UTF-8''a%20".to_string()),
            ("filename*1*".to_string(), "b.pdf".to_string()),
        ];
        let m = normalize_params(Some(&pairs));
        assert_eq!(m.get("FILENAME").map(|s| s.as_str()), Some("a b.pdf"));
    }

    #[test]
    fn mime_word_b() {
        // "café" in UTF-8 base64
        let s = "=?UTF-8?B?Y2Fmw6k=?=";
        assert_eq!(mime_words_decode(s), "café");
    }

    #[test]
    fn mime_word_q() {
        let s = "=?UTF-8?Q?Hello_World?=";
        assert_eq!(mime_words_decode(s), "Hello World");
    }
}
