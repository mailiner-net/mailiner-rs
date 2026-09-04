//! Detect PGP/MIME vs inline OpenPGP (not S/MIME).

use mailiner_core::body::BodyPart;
use mailiner_core::models::{primary_mime, MessageContent, MessagePart};

use crate::armor::{extract_armor_blocks, ArmorKind};

/// How OpenPGP appears in a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PgpFormat {
    MimeEncrypted,
    MimeSigned,
    InlineEncrypted,
    InlineSigned,
}

/// Combined detection result.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PgpDetection {
    pub mime_encrypted: bool,
    pub mime_signed: bool,
    pub inline_encrypted: bool,
    pub inline_signed: bool,
}

impl PgpDetection {
    pub fn is_empty(self) -> bool {
        !self.mime_encrypted && !self.mime_signed && !self.inline_encrypted && !self.inline_signed
    }

    pub fn encrypted(self) -> bool {
        self.mime_encrypted || self.inline_encrypted
    }

    pub fn signed(self) -> bool {
        self.mime_signed || self.inline_signed
    }
}

/// Detect from a Content-Type value (and optional `protocol=` parameter).
///
/// S/MIME (`pkcs7-mime`, `pkcs7-signature`) is ignored.
pub fn detect_from_content_type(content_type: &str, protocol: Option<&str>) -> Option<PgpFormat> {
    if looks_like_smime(content_type, protocol) {
        return None;
    }
    let mime = primary_mime(content_type).to_ascii_lowercase();
    let proto = protocol.unwrap_or("").to_ascii_lowercase();
    if mime == "multipart/encrypted" {
        if proto.is_empty() || proto.contains("pgp-encrypted") {
            return Some(PgpFormat::MimeEncrypted);
        }
        return None;
    }
    if mime == "multipart/signed" {
        if proto.contains("pgp-signature") {
            return Some(PgpFormat::MimeSigned);
        }
        return None;
    }
    if mime == "application/pgp-encrypted" {
        return Some(PgpFormat::MimeEncrypted);
    }
    if mime == "application/pgp-signature" {
        return Some(PgpFormat::MimeSigned);
    }
    None
}

/// Scan body text for inline armor.
pub fn detect_inline(text: &str) -> Option<PgpFormat> {
    let blocks = extract_armor_blocks(text);
    if blocks.iter().any(|b| b.kind == ArmorKind::SignedMessage) {
        return Some(PgpFormat::InlineSigned);
    }
    if blocks.iter().any(|b| b.kind == ArmorKind::Message) {
        return Some(PgpFormat::InlineEncrypted);
    }
    None
}

/// Walk a BODYSTRUCTURE tree.
pub fn detect_from_body(root: &BodyPart) -> PgpDetection {
    let mut det = PgpDetection::default();
    walk_body(root, &mut det);
    det
}

fn walk_body(part: &BodyPart, det: &mut PgpDetection) {
    let protocol = part.parameters.get("PROTOCOL").map(String::as_str);
    match detect_from_content_type(&part.content_type(), protocol) {
        Some(PgpFormat::MimeEncrypted) => det.mime_encrypted = true,
        Some(PgpFormat::MimeSigned) => det.mime_signed = true,
        _ => {}
    }
    for child in &part.subparts {
        walk_body(child, det);
    }
    if let Some(nested) = &part.nested_message {
        walk_body(nested, det);
    }
}

/// Inspect parsed / loaded parts (including decoded text for inline armor).
pub fn detect_from_parts(parts: &[MessagePart]) -> PgpDetection {
    let mut det = PgpDetection::default();
    for part in parts {
        let protocol = protocol_from_content_type(&part.content_type);
        match detect_from_content_type(&part.content_type, protocol.as_deref()) {
            Some(PgpFormat::MimeEncrypted) => det.mime_encrypted = true,
            Some(PgpFormat::MimeSigned) => det.mime_signed = true,
            _ => {}
        }
        match &part.content {
            MessageContent::Text(t) => match detect_inline(t) {
                Some(PgpFormat::InlineEncrypted) => det.inline_encrypted = true,
                Some(PgpFormat::InlineSigned) => det.inline_signed = true,
                _ => {}
            },
            MessageContent::Binary(b) => {
                if let Ok(t) = std::str::from_utf8(b) {
                    match detect_inline(t) {
                        Some(PgpFormat::InlineEncrypted) => det.inline_encrypted = true,
                        Some(PgpFormat::InlineSigned) => det.inline_signed = true,
                        _ => {}
                    }
                }
            }
            MessageContent::Empty => {}
        }
    }
    det
}

fn protocol_from_content_type(content_type: &str) -> Option<String> {
    for param in content_type.split(';').skip(1) {
        let param = param.trim();
        let Some((name, value)) = param.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("protocol") {
            return Some(unquote(value.trim()).to_string());
        }
    }
    None
}

fn unquote(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s)
}

fn looks_like_smime(content_type: &str, protocol: Option<&str>) -> bool {
    let blob = format!(
        "{} {}",
        content_type.to_ascii_lowercase(),
        protocol.unwrap_or("").to_ascii_lowercase()
    );
    blob.contains("pkcs7") || blob.contains("x-pkcs7")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use mailiner_core::ids::{FolderId, MessageId, MessagePartId};
    use mailiner_core::models::{MessageContent, PartKind, TransferEncoding};

    #[test]
    fn mime_encrypted_from_content_type() {
        assert_eq!(
            detect_from_content_type(
                r#"multipart/encrypted; protocol="application/pgp-encrypted""#,
                Some("application/pgp-encrypted")
            ),
            Some(PgpFormat::MimeEncrypted)
        );
        assert_eq!(
            detect_from_content_type("application/pgp-encrypted", None),
            Some(PgpFormat::MimeEncrypted)
        );
    }

    #[test]
    fn mime_signed_requires_pgp_protocol() {
        assert_eq!(
            detect_from_content_type(
                r#"multipart/signed; protocol="application/pgp-signature""#,
                Some("application/pgp-signature")
            ),
            Some(PgpFormat::MimeSigned)
        );
        assert_eq!(
            detect_from_content_type(
                r#"multipart/signed; protocol="application/pkcs7-signature""#,
                Some("application/pkcs7-signature")
            ),
            None
        );
    }

    #[test]
    fn smime_pkcs7_mime_ignored() {
        assert_eq!(
            detect_from_content_type("application/pkcs7-mime", None),
            None
        );
        assert_eq!(
            detect_from_content_type("application/x-pkcs7-mime", None),
            None
        );
    }

    #[test]
    fn inline_message_vs_signed() {
        assert_eq!(
            detect_inline("-----BEGIN PGP MESSAGE-----\n\nww==\n-----END PGP MESSAGE-----"),
            Some(PgpFormat::InlineEncrypted)
        );
        assert_eq!(
            detect_inline(
                "-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA256\n\nhi\n-----BEGIN PGP SIGNATURE-----\n\nww==\n-----END PGP SIGNATURE-----"
            ),
            Some(PgpFormat::InlineSigned)
        );
        assert_eq!(detect_inline("just a regular email"), None);
    }

    #[test]
    fn body_structure_multipart_encrypted() {
        let root = BodyPart {
            type_: "multipart".into(),
            subtype: "encrypted".into(),
            parameters: [("PROTOCOL".into(), "application/pgp-encrypted".into())].into(),
            subparts: vec![
                BodyPart {
                    type_: "application".into(),
                    subtype: "pgp-encrypted".into(),
                    ..Default::default()
                },
                BodyPart {
                    type_: "application".into(),
                    subtype: "octet-stream".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let det = detect_from_body(&root);
        assert!(det.mime_encrypted);
        assert!(!det.mime_signed);
    }

    #[test]
    fn parts_detect_inline_in_text() {
        let part = sample_part(
            "text/plain",
            MessageContent::Text(
                "-----BEGIN PGP MESSAGE-----\n\nww==\n-----END PGP MESSAGE-----".into(),
            ),
        );
        let det = detect_from_parts(&[part]);
        assert!(det.inline_encrypted);
        assert!(!det.mime_encrypted);
    }

    fn sample_part(ct: &str, content: MessageContent) -> MessagePart {
        let now = Utc::now();
        MessagePart {
            id: MessagePartId::new("p1"),
            envelope_id: MessageId::new(FolderId::new("INBOX"), "1"),
            path: vec!["1".into()],
            kind: PartKind::TextPlain,
            content_type: ct.into(),
            charset: None,
            content_id: None,
            description: None,
            filename: None,
            encoding: TransferEncoding::SevenBit,
            original_size: None,
            size: 0,
            is_attachment: false,
            is_hidden: false,
            nested_in: None,
            nested_headers: None,
            content,
            created_at: now,
            updated_at: now,
        }
    }
}
