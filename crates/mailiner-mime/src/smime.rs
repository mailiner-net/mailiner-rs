//! S/MIME MIME-type detection (RFC 8551). No CMS parsing.

use std::collections::BTreeMap;

use mailiner_core::body::BodyPart;
use mailiner_core::models::primary_mime;

/// How the S/MIME payload is carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmimeRole {
    /// `application/pkcs7-mime` / `application/x-pkcs7-mime`.
    Pkcs7Mime,
    /// Detached `application/pkcs7-signature` / `application/x-pkcs7-signature`.
    DetachedSignature,
    /// `multipart/signed` with an S/MIME `protocol` parameter.
    MultipartSigned,
}

/// `smime-type` parameter (RFC 8551 §3.2), when present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmimeType {
    SignedData,
    EnvelopedData,
    CompressedData,
    CertsOnly,
    Unknown,
}

/// Result of inspecting a Content-Type (and optional BODYSTRUCTURE params).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmimeDetection {
    pub role: SmimeRole,
    pub smime_type: SmimeType,
}

impl SmimeDetection {
    pub fn is_signed(self) -> bool {
        matches!(
            self,
            Self {
                role: SmimeRole::DetachedSignature | SmimeRole::MultipartSigned,
                ..
            } | Self {
                role: SmimeRole::Pkcs7Mime,
                smime_type: SmimeType::SignedData | SmimeType::Unknown,
            }
        )
    }

    pub fn is_encrypted(self) -> bool {
        matches!(
            self,
            Self {
                role: SmimeRole::Pkcs7Mime,
                smime_type: SmimeType::EnvelopedData,
            }
        )
    }
}

/// Detect S/MIME from a Content-Type header or BODYSTRUCTURE type/subtype + params.
///
/// OpenPGP (`application/pgp-*`, `protocol=application/pgp-signature`) is ignored.
pub fn detect_smime(content_type: &str) -> Option<SmimeDetection> {
    let (mime, params) = split_content_type(content_type);
    detect_smime_parts(mime, &params)
}

/// Detect from a BODYSTRUCTURE node.
pub fn detect_smime_part(part: &BodyPart) -> Option<SmimeDetection> {
    detect_smime_parts(&part.content_type(), &part.parameters)
}

fn detect_smime_parts(mime: &str, params: &BTreeMap<String, String>) -> Option<SmimeDetection> {
    let mime = primary_mime(mime).trim();
    if mime.is_empty() {
        return None;
    }
    if is_pkcs7_mime(mime) {
        return Some(SmimeDetection {
            role: SmimeRole::Pkcs7Mime,
            smime_type: smime_type_from_params(params),
        });
    }
    if is_pkcs7_signature(mime) {
        return Some(SmimeDetection {
            role: SmimeRole::DetachedSignature,
            smime_type: SmimeType::SignedData,
        });
    }
    if is_smime_multipart_signed(mime, params.get("PROTOCOL").map(String::as_str)) {
        return Some(SmimeDetection {
            role: SmimeRole::MultipartSigned,
            smime_type: SmimeType::SignedData,
        });
    }
    None
}

pub fn is_pkcs7_mime(content_type: &str) -> bool {
    let mime = primary_mime(content_type);
    mime.eq_ignore_ascii_case("application/pkcs7-mime")
        || mime.eq_ignore_ascii_case("application/x-pkcs7-mime")
}

pub fn is_pkcs7_signature(content_type: &str) -> bool {
    let mime = primary_mime(content_type);
    mime.eq_ignore_ascii_case("application/pkcs7-signature")
        || mime.eq_ignore_ascii_case("application/x-pkcs7-signature")
}

/// `multipart/signed` whose `protocol` names an S/MIME signature part.
pub fn is_smime_multipart_signed(content_type: &str, protocol: Option<&str>) -> bool {
    let mime = primary_mime(content_type);
    if !mime.eq_ignore_ascii_case("multipart/signed") {
        return false;
    }
    protocol
        .map(unquote)
        .is_some_and(|p| is_pkcs7_signature(p.trim()))
}

pub fn is_smime_protocol(protocol: &str) -> bool {
    is_pkcs7_signature(unquote(protocol).trim())
}

fn smime_type_from_params(params: &BTreeMap<String, String>) -> SmimeType {
    let Some(raw) = params.get("SMIME-TYPE") else {
        return SmimeType::Unknown;
    };
    match unquote(raw).trim().to_ascii_lowercase().as_str() {
        "signed-data" => SmimeType::SignedData,
        "enveloped-data" | "authenveloped-data" => SmimeType::EnvelopedData,
        "compressed-data" => SmimeType::CompressedData,
        "certs-only" => SmimeType::CertsOnly,
        _ => SmimeType::Unknown,
    }
}

fn unquote(value: &str) -> &str {
    let t = value.trim();
    t.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(t)
}

/// Split `type/subtype; k=v; …` into the type and uppercased parameters.
pub fn split_content_type(content_type: &str) -> (&str, BTreeMap<String, String>) {
    let trimmed = content_type.trim();
    let Some((mime, rest)) = trimmed.split_once(';') else {
        return (trimmed, BTreeMap::new());
    };
    let mut params = BTreeMap::new();
    for piece in rest.split(';') {
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        let Some((k, v)) = piece.split_once('=') else {
            continue;
        };
        params.insert(k.trim().to_ascii_uppercase(), unquote(v.trim()).to_string());
    }
    (mime.trim(), params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_pkcs7_mime_signed_data() {
        let d = detect_smime("application/pkcs7-mime; smime-type=signed-data; name=smime.p7m")
            .expect("signed-data");
        assert_eq!(d.role, SmimeRole::Pkcs7Mime);
        assert_eq!(d.smime_type, SmimeType::SignedData);
        assert!(d.is_signed());
        assert!(!d.is_encrypted());
    }

    #[test]
    fn detect_pkcs7_mime_enveloped() {
        let d =
            detect_smime("application/x-pkcs7-mime; smime-type=enveloped-data").expect("enveloped");
        assert_eq!(d.role, SmimeRole::Pkcs7Mime);
        assert_eq!(d.smime_type, SmimeType::EnvelopedData);
        assert!(d.is_encrypted());
        assert!(!d.is_signed());
    }

    #[test]
    fn detect_pkcs7_mime_without_smime_type_is_unknown_signed() {
        let d = detect_smime("application/pkcs7-mime").expect("pkcs7-mime");
        assert_eq!(d.smime_type, SmimeType::Unknown);
        assert!(d.is_signed());
    }

    #[test]
    fn detect_detached_signature() {
        let d = detect_smime("application/pkcs7-signature; name=smime.p7s").expect("p7s");
        assert_eq!(d.role, SmimeRole::DetachedSignature);
        assert!(d.is_signed());
        assert!(detect_smime("application/x-pkcs7-signature").is_some());
    }

    #[test]
    fn detect_multipart_signed_with_quoted_protocol() {
        let d = detect_smime(
            r#"multipart/signed; protocol="application/pkcs7-signature"; micalg=sha-256"#,
        )
        .expect("multipart/signed");
        assert_eq!(d.role, SmimeRole::MultipartSigned);
        assert!(d.is_signed());
    }

    #[test]
    fn detect_multipart_signed_x_pkcs7() {
        assert!(detect_smime("multipart/signed; protocol=application/x-pkcs7-signature").is_some());
    }

    #[test]
    fn reject_garbage_and_openpgp() {
        assert!(detect_smime("").is_none());
        assert!(detect_smime("   ").is_none());
        assert!(detect_smime("not a mime type").is_none());
        assert!(detect_smime("application/octet-stream").is_none());
        assert!(detect_smime("text/plain").is_none());
        assert!(detect_smime("multipart/mixed").is_none());
        assert!(detect_smime("multipart/signed").is_none());
        assert!(detect_smime("multipart/signed; protocol=application/pgp-signature").is_none());
        assert!(detect_smime("application/pgp-signature").is_none());
        assert!(detect_smime("application/pgp-encrypted").is_none());
        assert!(detect_smime("application/pkcs7").is_none());
    }

    #[test]
    fn detect_from_body_part_uses_uppercased_params() {
        let part = BodyPart {
            type_: "multipart".into(),
            subtype: "signed".into(),
            parameters: [
                ("PROTOCOL".into(), "application/pkcs7-signature".into()),
                ("MICALG".into(), "sha-256".into()),
            ]
            .into(),
            ..Default::default()
        };
        let d = detect_smime_part(&part).expect("signed part");
        assert_eq!(d.role, SmimeRole::MultipartSigned);

        let enveloped = BodyPart {
            type_: "application".into(),
            subtype: "pkcs7-mime".into(),
            parameters: [("SMIME-TYPE".into(), "enveloped-data".into())].into(),
            ..Default::default()
        };
        assert!(detect_smime_part(&enveloped).unwrap().is_encrypted());
    }
}
