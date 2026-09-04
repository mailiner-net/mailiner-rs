//! Extract ASCII-armored OpenPGP blocks from text.

/// Kind of armored OpenPGP block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmorKind {
    Message,
    SignedMessage,
    Signature,
    PublicKey,
    PrivateKey,
}

impl ArmorKind {
    fn from_begin(label: &str) -> Option<Self> {
        match label.trim() {
            "PGP MESSAGE" => Some(Self::Message),
            "PGP SIGNED MESSAGE" => Some(Self::SignedMessage),
            "PGP SIGNATURE" => Some(Self::Signature),
            "PGP PUBLIC KEY BLOCK" => Some(Self::PublicKey),
            "PGP PRIVATE KEY BLOCK" => Some(Self::PrivateKey),
            _ => None,
        }
    }

    fn end_label(self) -> &'static str {
        match self {
            Self::Message => "PGP MESSAGE",
            Self::SignedMessage => "PGP SIGNATURE",
            Self::Signature => "PGP SIGNATURE",
            Self::PublicKey => "PGP PUBLIC KEY BLOCK",
            Self::PrivateKey => "PGP PRIVATE KEY BLOCK",
        }
    }
}

/// One armored block, including the BEGIN/END lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmorBlock {
    pub kind: ArmorKind,
    pub text: String,
}

/// Find `-----BEGIN …-----` / `-----END …-----` blocks in `text`.
pub fn extract_armor_blocks(text: &str) -> Vec<ArmorBlock> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &text[i..];
        let Some(begin_rel) = rest.find("-----BEGIN ") else {
            break;
        };
        let begin = i + begin_rel;
        let after_begin = begin + "-----BEGIN ".len();
        let Some(label_end_rel) = text[after_begin..].find("-----") else {
            break;
        };
        let label = &text[after_begin..after_begin + label_end_rel];
        let Some(kind) = ArmorKind::from_begin(label) else {
            i = after_begin + label_end_rel + 5;
            continue;
        };
        let end_marker = format!("-----END {}-----", kind.end_label());
        let search_from = after_begin + label_end_rel + 5;
        let Some(end_rel) = text[search_from..].find(&end_marker) else {
            break;
        };
        let end = search_from + end_rel + end_marker.len();
        out.push(ArmorBlock {
            kind,
            text: text[begin..end].to_string(),
        });
        i = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_message_and_ignores_noise() {
        let text = "hello\n-----BEGIN PGP MESSAGE-----\n\nww==\n-----END PGP MESSAGE-----\nbye";
        let blocks = extract_armor_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, ArmorKind::Message);
        assert!(blocks[0].text.starts_with("-----BEGIN PGP MESSAGE-----"));
        assert!(blocks[0].text.ends_with("-----END PGP MESSAGE-----"));
    }

    #[test]
    fn extracts_cleartext_signed() {
        let text = "-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA256\n\nhi\n-----BEGIN PGP SIGNATURE-----\n\nww==\n-----END PGP SIGNATURE-----\n";
        let blocks = extract_armor_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, ArmorKind::SignedMessage);
    }

    #[test]
    fn skips_unknown_begin() {
        let text = "-----BEGIN CERTIFICATE-----\nMII=\n-----END CERTIFICATE-----\n";
        assert!(extract_armor_blocks(text).is_empty());
    }
}
