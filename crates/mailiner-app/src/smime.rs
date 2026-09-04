//! S/MIME detect / verify / certificate import (separate from OpenPGP).

use std::collections::BTreeMap;

use chrono::Utc;
use cms::cert::CertificateChoices;
use cms::content_info::ContentInfo;
use cms::signed_data::{
    EncapsulatedContentInfo, SignedData, SignerIdentifier, SignerInfo, SignerInfos,
};
use const_oid::ObjectIdentifier;
use der::asn1::OctetString;
use der::{Decode, DecodePem, Encode};
use digest::Digest;
use mailiner_core::ids::MessagePartId;
use mailiner_core::models::{
    MessageContent, MessagePart, PartKind, TransferEncoding, is_smime_mime,
};
use mailiner_mime::{
    SmimeDetection, SmimeRole, SmimeType, detect_smime, is_pkcs7_mime, is_pkcs7_signature,
    split_content_type,
};
use rsa::RsaPublicKey;
use rsa::pkcs1v15::{Signature as RsaSignature, VerifyingKey as RsaVerifyingKey};
use rsa::signature::Verifier as RsaVerifier;
use sha1::Sha1;
use sha2::{Sha256, Sha384, Sha512};
use spki::DecodePublicKey;
use x509_cert::Certificate;

use crate::account::Account;
use crate::account_config::SmimeIdentity;

/// Viewer banner for an S/MIME part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmimeBanner {
    /// Signed, but the cert is untrusted or verify is incomplete.
    Signed { signer: Option<String> },
    /// Cryptographic signature and trust checks succeeded.
    SignatureValid { signer: Option<String> },
    /// CMS parse or signature check failed.
    SignatureFailed { reason: String },
    /// Encrypted (or detached without a signer cert) and no matching identity.
    NeedCertificate { encrypted: bool },
}

impl SmimeBanner {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Signed { .. } => "Signed",
            Self::SignatureValid { .. } => "Signature valid",
            Self::SignatureFailed { .. } => "Signature failed",
            Self::NeedCertificate { .. } => "Need certificate",
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::Signed { signer } => match signer {
                Some(s) => format!("S/MIME signed by {s}. Certificate is not in your trust store."),
                None => "S/MIME signed. Certificate is not in your trust store.".into(),
            },
            Self::SignatureValid { signer } => match signer {
                Some(s) => format!("S/MIME signature verified ({s})."),
                None => "S/MIME signature verified.".into(),
            },
            Self::SignatureFailed { reason } => format!("S/MIME signature could not be verified: {reason}"),
            Self::NeedCertificate { encrypted: true } => {
                "This message is S/MIME encrypted. Import your certificate and private key in account settings.".into()
            }
            Self::NeedCertificate { encrypted: false } => {
                "S/MIME signed, but the signer certificate is missing. Import it in account settings.".into()
            }
        }
    }

    pub fn tone(&self) -> SmimeTone {
        match self {
            Self::SignatureValid { .. } => SmimeTone::Ok,
            Self::Signed { .. } => SmimeTone::Info,
            Self::NeedCertificate { .. } => SmimeTone::Warn,
            Self::SignatureFailed { .. } => SmimeTone::Fail,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmimeTone {
    Ok,
    Info,
    Warn,
    Fail,
}

/// Map a verify outcome to a banner (unit-tested independently of CMS).
pub fn banner_from_outcome(outcome: SmimeOutcome) -> SmimeBanner {
    match outcome {
        SmimeOutcome::DetectedSigned { signer } => SmimeBanner::Signed { signer },
        SmimeOutcome::Valid { signer } => SmimeBanner::SignatureValid { signer },
        SmimeOutcome::Failed { reason } => SmimeBanner::SignatureFailed { reason },
        SmimeOutcome::NeedCert { encrypted } => SmimeBanner::NeedCertificate { encrypted },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmimeOutcome {
    DetectedSigned { signer: Option<String> },
    Valid { signer: Option<String> },
    Failed { reason: String },
    NeedCert { encrypted: bool },
}

/// Trust material available in the viewer (no private keys).
#[derive(Debug, Clone, Default)]
pub struct SmimeTrust {
    pub extra_ca_pems: Vec<String>,
    pub identities: Vec<SmimeIdentity>,
}

impl SmimeTrust {
    pub fn from_account(account: &Account) -> Self {
        Self {
            extra_ca_pems: account.extra_ca_pems.clone(),
            identities: account.smime_identities.clone(),
        }
    }

    fn anchors(&self) -> Vec<Certificate> {
        let mut out = Vec::new();
        for pem in &self.extra_ca_pems {
            out.extend(parse_certs_pem(pem));
        }
        for id in &self.identities {
            out.extend(parse_certs_pem(&id.cert_pem));
        }
        out
    }
}

/// Inspect loaded parts, expand opaque signed-data, and return a banner.
pub fn evaluate_parts(parts: &mut Vec<MessagePart>, trust: &SmimeTrust) -> Option<SmimeBanner> {
    let detected = detect_from_parts(parts)?;
    if detected.is_encrypted() {
        return Some(banner_from_outcome(SmimeOutcome::NeedCert {
            encrypted: true,
        }));
    }

    let cms = first_cms_bytes(parts);
    let detached = detected.role == SmimeRole::MultipartSigned
        || detected.role == SmimeRole::DetachedSignature;
    let content = if detached {
        detached_signed_bytes(parts)
    } else {
        None
    };

    let Some(cms) = cms else {
        return Some(banner_from_outcome(SmimeOutcome::DetectedSigned {
            signer: None,
        }));
    };

    match verify_cms(&cms, content.as_deref(), trust) {
        Ok(result) => {
            if let Some(inner) = result.inner_content.as_ref() {
                expand_inner_parts(parts, inner);
            }
            Some(banner_from_outcome(result.outcome))
        }
        Err(reason) => Some(banner_from_outcome(SmimeOutcome::Failed { reason })),
    }
}

pub fn detect_from_parts(parts: &[MessagePart]) -> Option<SmimeDetection> {
    let has_sig = parts.iter().any(|p| is_pkcs7_signature(&p.content_type));
    let has_body = parts.iter().any(|p| !is_smime_mime(&p.content_type));
    if has_sig && has_body {
        return Some(SmimeDetection {
            role: SmimeRole::MultipartSigned,
            smime_type: SmimeType::SignedData,
        });
    }
    let mut found = None;
    for part in parts {
        if let Some(d) = detect_smime(&part.content_type) {
            if d.role == SmimeRole::Pkcs7Mime {
                return Some(d);
            }
            if found.is_none() {
                found = Some(d);
            }
        }
    }
    found
}

fn first_cms_bytes(parts: &[MessagePart]) -> Option<Vec<u8>> {
    parts.iter().find_map(|p| {
        if !is_smime_mime(&p.content_type) {
            return None;
        }
        match &p.content {
            MessageContent::Binary(b) if !b.is_empty() => Some(unwrap_cms(b)),
            MessageContent::Text(t) if !t.is_empty() => Some(unwrap_cms(t.as_bytes())),
            _ => None,
        }
    })
}

fn detached_signed_bytes(parts: &[MessagePart]) -> Option<Vec<u8>> {
    let content = parts.iter().find(|p| !is_smime_mime(&p.content_type))?;
    Some(reconstruct_entity(content))
}

fn reconstruct_entity(part: &MessagePart) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"Content-Type: ");
    out.extend_from_slice(part.content_type.as_bytes());
    out.extend_from_slice(b"\r\nContent-Transfer-Encoding: ");
    let enc = match part.encoding {
        TransferEncoding::SevenBit => "7bit",
        TransferEncoding::EightBit => "8bit",
        TransferEncoding::Binary => "binary",
        TransferEncoding::Base64 => "base64",
        TransferEncoding::QuotedPrintable => "quoted-printable",
        TransferEncoding::Other => "7bit",
    };
    out.extend_from_slice(enc.as_bytes());
    out.extend_from_slice(b"\r\n\r\n");
    match &part.content {
        MessageContent::Text(t) => out.extend_from_slice(t.as_bytes()),
        MessageContent::Binary(b) => out.extend_from_slice(b),
        MessageContent::Empty => {}
    }
    out
}

struct VerifyResult {
    outcome: SmimeOutcome,
    inner_content: Option<Vec<u8>>,
}

fn verify_cms(
    raw: &[u8],
    detached: Option<&[u8]>,
    trust: &SmimeTrust,
) -> Result<VerifyResult, String> {
    let info = ContentInfo::from_der(raw).map_err(|e| format!("invalid CMS: {e}"))?;
    if info.content_type == ID_ENVELOPED_DATA {
        return Ok(VerifyResult {
            outcome: SmimeOutcome::NeedCert { encrypted: true },
            inner_content: None,
        });
    }
    if info.content_type != ID_SIGNED_DATA {
        return Err(format!(
            "unsupported CMS content type {}",
            info.content_type
        ));
    }
    let signed: SignedData = info
        .content
        .decode_as()
        .map_err(|e| format!("invalid SignedData: {e}"))?;
    let inner = econtent_bytes(&signed.encap_content_info);
    let signed_content = detached.or(inner.as_deref());
    let bag = collect_certs(&signed, trust);
    let infos = signer_info_slice(&signed.signer_infos);
    if infos.is_empty() {
        return Err("SignedData has no signerInfos".into());
    }

    let mut last_fail = None;
    let mut last_need_cert = false;
    let mut last_signed = None;
    for si in infos {
        match verify_signer(&signed, si, signed_content, &bag, trust) {
            Ok(outcome) => match outcome {
                SmimeOutcome::Valid { .. } => {
                    return Ok(VerifyResult {
                        outcome,
                        inner_content: inner.clone(),
                    });
                }
                SmimeOutcome::DetectedSigned { .. } => last_signed = Some(outcome),
                SmimeOutcome::NeedCert { .. } => last_need_cert = true,
                SmimeOutcome::Failed { reason } => last_fail = Some(reason),
            },
            Err(reason) => last_fail = Some(reason),
        }
    }
    if let Some(outcome) = last_signed {
        return Ok(VerifyResult {
            outcome,
            inner_content: inner,
        });
    }
    if last_need_cert {
        return Ok(VerifyResult {
            outcome: SmimeOutcome::NeedCert { encrypted: false },
            inner_content: inner,
        });
    }
    Err(last_fail.unwrap_or_else(|| "signature verification failed".into()))
}

fn signer_info_slice(infos: &SignerInfos) -> Vec<&SignerInfo> {
    infos.0.iter().collect()
}

fn econtent_bytes(info: &EncapsulatedContentInfo) -> Option<Vec<u8>> {
    let any = info.econtent.as_ref()?;
    let der = any.to_der().ok()?;
    if let Ok(os) = OctetString::from_der(&der) {
        return Some(os.as_bytes().to_vec());
    }
    Some(any.value().to_vec())
}

fn collect_certs(signed: &SignedData, trust: &SmimeTrust) -> Vec<Certificate> {
    let mut out = Vec::new();
    if let Some(set) = &signed.certificates {
        for choice in set.0.iter() {
            if let CertificateChoices::Certificate(cert) = choice {
                out.push(cert.clone());
            }
        }
    }
    for id in &trust.identities {
        out.extend(parse_certs_pem(&id.cert_pem));
    }
    out
}

fn verify_signer(
    _signed: &SignedData,
    si: &SignerInfo,
    content: Option<&[u8]>,
    bag: &[Certificate],
    trust: &SmimeTrust,
) -> Result<SmimeOutcome, String> {
    let Some(cert) = find_signer_cert(si, bag) else {
        return Ok(SmimeOutcome::NeedCert { encrypted: false });
    };
    let signer = cert_label(&cert);
    let digest_oid = si.digest_alg.oid;
    let to_be_signed = if let Some(attrs) = &si.signed_attrs {
        if let Some(content) = content {
            let digest = digest_bytes(digest_oid, content)?;
            let md = message_digest_attr(attrs).ok_or("missing messageDigest signed attribute")?;
            if md != digest {
                return Err("message digest does not match signed content".into());
            }
        }
        attrs
            .to_der()
            .map_err(|e| format!("encode signed attributes: {e}"))?
    } else {
        content.ok_or("detached signature has no content")?.to_vec()
    };

    verify_signature(
        &cert,
        si.signature_algorithm.oid,
        digest_oid,
        &to_be_signed,
        si.signature.as_bytes(),
    )?;

    if cert_is_trusted(&cert, bag, trust) {
        Ok(SmimeOutcome::Valid {
            signer: Some(signer),
        })
    } else {
        Ok(SmimeOutcome::DetectedSigned {
            signer: Some(signer),
        })
    }
}

fn find_signer_cert(si: &SignerInfo, bag: &[Certificate]) -> Option<Certificate> {
    match &si.sid {
        SignerIdentifier::IssuerAndSerialNumber(isn) => bag
            .iter()
            .find(|c| {
                c.tbs_certificate.issuer == isn.issuer
                    && c.tbs_certificate.serial_number == isn.serial_number
            })
            .cloned(),
        SignerIdentifier::SubjectKeyIdentifier(ski) => bag
            .iter()
            .find(|c| subject_key_id(c).is_some_and(|id| id.0.as_bytes() == ski.0.as_bytes()))
            .cloned(),
    }
}

fn subject_key_id(cert: &Certificate) -> Option<x509_cert::ext::pkix::SubjectKeyIdentifier> {
    use x509_cert::ext::pkix::SubjectKeyIdentifier;
    cert.tbs_certificate
        .extensions
        .as_ref()?
        .iter()
        .find_map(|ext| {
            if ext.extn_id == const_oid::db::rfc5280::ID_CE_SUBJECT_KEY_IDENTIFIER {
                SubjectKeyIdentifier::from_der(ext.extn_value.as_bytes()).ok()
            } else {
                None
            }
        })
}

fn message_digest_attr(attrs: &cms::signed_data::SignedAttributes) -> Option<Vec<u8>> {
    for attr in attrs.iter() {
        if attr.oid == ID_MESSAGE_DIGEST {
            let any = attr.values.iter().next()?;
            if let Ok(os) = der::asn1::OctetString::from_der(&any.to_der().ok()?) {
                return Some(os.as_bytes().to_vec());
            }
            // Some encoders store the OCTET STRING payload in Any without wrapping.
            return Some(any.value().to_vec());
        }
    }
    None
}

fn digest_bytes(oid: ObjectIdentifier, data: &[u8]) -> Result<Vec<u8>, String> {
    if oid == ID_SHA_1 {
        return Ok(Sha1::digest(data).to_vec());
    }
    if oid == ID_SHA_256 {
        return Ok(Sha256::digest(data).to_vec());
    }
    if oid == ID_SHA_384 {
        return Ok(Sha384::digest(data).to_vec());
    }
    if oid == ID_SHA_512 {
        return Ok(Sha512::digest(data).to_vec());
    }
    Err(format!("unsupported digest {oid}"))
}

fn verify_signature(
    cert: &Certificate,
    sig_oid: ObjectIdentifier,
    digest_oid: ObjectIdentifier,
    tbs: &[u8],
    signature: &[u8],
) -> Result<(), String> {
    let spki = &cert.tbs_certificate.subject_public_key_info;
    let spki_der = spki.to_der().map_err(|e| format!("encode SPKI: {e}"))?;
    if is_rsa_oid(sig_oid) || is_rsa_oid(spki.algorithm.oid) {
        let key =
            RsaPublicKey::from_public_key_der(&spki_der).map_err(|e| format!("RSA key: {e}"))?;
        return verify_rsa(&key, digest_oid, tbs, signature);
    }
    if is_ecdsa_oid(sig_oid) || spki.algorithm.oid == ID_EC_PUBLIC_KEY {
        return verify_ecdsa(&spki_der, digest_oid, tbs, signature);
    }
    Err(format!("unsupported signature algorithm {sig_oid}"))
}

fn is_rsa_oid(oid: ObjectIdentifier) -> bool {
    oid == ID_RSA_ENCRYPTION
        || oid == ID_SHA1_WITH_RSA
        || oid == ID_SHA256_WITH_RSA
        || oid == ID_SHA384_WITH_RSA
        || oid == ID_SHA512_WITH_RSA
}

fn is_ecdsa_oid(oid: ObjectIdentifier) -> bool {
    oid == ID_ECDSA_SHA1
        || oid == ID_ECDSA_SHA256
        || oid == ID_ECDSA_SHA384
        || oid == ID_ECDSA_SHA512
}

fn verify_rsa(
    key: &RsaPublicKey,
    digest_oid: ObjectIdentifier,
    tbs: &[u8],
    signature: &[u8],
) -> Result<(), String> {
    let sig = RsaSignature::try_from(signature).map_err(|e| format!("RSA signature: {e}"))?;
    let ok = if digest_oid == ID_SHA_256 {
        RsaVerifyingKey::<Sha256>::new(key.clone())
            .verify(tbs, &sig)
            .is_ok()
    } else if digest_oid == ID_SHA_384 {
        RsaVerifyingKey::<Sha384>::new(key.clone())
            .verify(tbs, &sig)
            .is_ok()
    } else if digest_oid == ID_SHA_512 {
        RsaVerifyingKey::<Sha512>::new(key.clone())
            .verify(tbs, &sig)
            .is_ok()
    } else if digest_oid == ID_SHA_1 {
        RsaVerifyingKey::<Sha1>::new(key.clone())
            .verify(tbs, &sig)
            .is_ok()
    } else {
        return Err(format!("unsupported RSA digest {digest_oid}"));
    };
    if ok {
        Ok(())
    } else {
        Err("RSA signature mismatch".into())
    }
}

fn verify_ecdsa(
    spki_der: &[u8],
    digest_oid: ObjectIdentifier,
    tbs: &[u8],
    signature: &[u8],
) -> Result<(), String> {
    if (digest_oid == ID_SHA_256 || digest_oid == ID_SHA_1)
        && let Ok(key) = p256::ecdsa::VerifyingKey::from_public_key_der(spki_der)
    {
        let sig = p256::ecdsa::Signature::from_der(signature)
            .or_else(|_| p256::ecdsa::Signature::from_slice(signature))
            .map_err(|e| format!("ECDSA P-256 signature: {e}"))?;
        return p256::ecdsa::signature::Verifier::verify(&key, tbs, &sig)
            .map_err(|_| "ECDSA P-256 signature mismatch".into());
    }
    if (digest_oid == ID_SHA_384 || digest_oid == ID_SHA_512)
        && let Ok(key) = p384::ecdsa::VerifyingKey::from_public_key_der(spki_der)
    {
        let sig = p384::ecdsa::Signature::from_der(signature)
            .or_else(|_| p384::ecdsa::Signature::from_slice(signature))
            .map_err(|e| format!("ECDSA P-384 signature: {e}"))?;
        return p384::ecdsa::signature::Verifier::verify(&key, tbs, &sig)
            .map_err(|_| "ECDSA P-384 signature mismatch".into());
    }
    Err("unsupported ECDSA key or digest".into())
}

fn cert_is_trusted(cert: &Certificate, bag: &[Certificate], trust: &SmimeTrust) -> bool {
    let anchors = trust.anchors();
    if anchors.is_empty() {
        return false;
    }
    if anchors.iter().any(|a| certs_eq(a, cert)) {
        return true;
    }
    // One-level issuer check against extra CAs / imported certs.
    let mut chain = bag.to_vec();
    chain.extend(anchors.iter().cloned());
    let mut current = cert.clone();
    for _ in 0..8 {
        if anchors.iter().any(|a| {
            certs_eq(a, &current)
                || names_eq(&a.tbs_certificate.subject, &current.tbs_certificate.subject)
        }) {
            return true;
        }
        let issuer = current.tbs_certificate.issuer.clone();
        let Some(next) = chain
            .iter()
            .find(|c| names_eq(&c.tbs_certificate.subject, &issuer))
        else {
            return false;
        };
        if certs_eq(next, &current) {
            return anchors
                .iter()
                .any(|a| names_eq(&a.tbs_certificate.subject, &issuer));
        }
        if verify_cert_signed_by(&current, next).is_err() {
            return false;
        }
        current = next.clone();
    }
    false
}

fn certs_eq(a: &Certificate, b: &Certificate) -> bool {
    a.tbs_certificate.serial_number == b.tbs_certificate.serial_number
        && names_eq(&a.tbs_certificate.issuer, &b.tbs_certificate.issuer)
}

fn names_eq(a: &x509_cert::name::Name, b: &x509_cert::name::Name) -> bool {
    a == b
}

fn verify_cert_signed_by(child: &Certificate, issuer: &Certificate) -> Result<(), String> {
    let tbs = child
        .tbs_certificate
        .to_der()
        .map_err(|e| format!("encode TBS: {e}"))?;
    verify_signature(
        issuer,
        child.signature_algorithm.oid,
        digest_oid_for_sig(child.signature_algorithm.oid),
        &tbs,
        child.signature.raw_bytes(),
    )
}

fn digest_oid_for_sig(sig_oid: ObjectIdentifier) -> ObjectIdentifier {
    if sig_oid == ID_SHA1_WITH_RSA || sig_oid == ID_ECDSA_SHA1 {
        ID_SHA_1
    } else if sig_oid == ID_SHA384_WITH_RSA || sig_oid == ID_ECDSA_SHA384 {
        ID_SHA_384
    } else if sig_oid == ID_SHA512_WITH_RSA || sig_oid == ID_ECDSA_SHA512 {
        ID_SHA_512
    } else {
        ID_SHA_256
    }
}

fn cert_label(cert: &Certificate) -> String {
    if let Some(email) = cert_email(cert) {
        return email;
    }
    let s = cert.tbs_certificate.subject.to_string();
    if s.is_empty() {
        "unknown signer".into()
    } else {
        s
    }
}

fn cert_email(cert: &Certificate) -> Option<String> {
    use x509_cert::ext::pkix::SubjectAltName;
    if let Some(exts) = &cert.tbs_certificate.extensions {
        for ext in exts {
            if ext.extn_id == const_oid::db::rfc5280::ID_CE_SUBJECT_ALT_NAME
                && let Ok(san) = SubjectAltName::from_der(ext.extn_value.as_bytes())
            {
                for name in san.0.iter() {
                    if let x509_cert::ext::pkix::name::GeneralName::Rfc822Name(n) = name {
                        return Some(n.to_string());
                    }
                }
            }
        }
    }
    for rdn in cert.tbs_certificate.subject.0.iter() {
        for ava in rdn.0.iter() {
            if (ava.oid == const_oid::db::rfc3280::EMAIL_ADDRESS
                || ava.oid == const_oid::db::rfc3280::EMAIL)
                && let Ok(s) = ava.value.decode_as::<der::asn1::Ia5StringRef<'_>>()
            {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn parse_certs_pem(pem: &str) -> Vec<Certificate> {
    let trimmed = pem.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut rest = trimmed;
    while let Some(start) = rest.find("-----BEGIN CERTIFICATE-----") {
        let chunk = &rest[start..];
        let Some(end) = chunk.find("-----END CERTIFICATE-----") else {
            break;
        };
        let one = &chunk[..end + "-----END CERTIFICATE-----".len()];
        match Certificate::from_pem(one.as_bytes()) {
            Ok(cert) => out.push(cert),
            Err(_) => break,
        }
        rest = &chunk[end + "-----END CERTIFICATE-----".len()..];
    }
    out
}

fn unwrap_cms(raw: &[u8]) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(raw) else {
        return raw.to_vec();
    };
    let t = text.trim();
    if !(t.contains("BEGIN PKCS7") || t.contains("BEGIN CMS") || t.contains("BEGIN CERTIFICATE")) {
        return raw.to_vec();
    }
    let mut b64 = String::new();
    let mut in_body = false;
    for line in t.lines() {
        let line = line.trim();
        if line.starts_with("-----BEGIN") {
            in_body = true;
            continue;
        }
        if line.starts_with("-----END") {
            break;
        }
        if in_body {
            b64.push_str(line);
        }
    }
    mailiner_mime::base64_decode(b64.as_bytes()).unwrap_or_else(|_| raw.to_vec())
}

fn expand_inner_parts(parts: &mut Vec<MessagePart>, inner: &[u8]) {
    let extracted = parse_inner_mime(inner);
    if extracted.is_empty() {
        return;
    }
    for part in parts.iter_mut() {
        if is_pkcs7_mime(&part.content_type) {
            part.is_hidden = true;
        }
    }
    let envelope = parts
        .first()
        .map(|p| p.envelope_id.clone())
        .unwrap_or_else(|| mailiner_core::MessageId::new(mailiner_core::FolderId::new(""), ""));
    let now = Utc::now();
    for (i, (ct, content)) in extracted.into_iter().enumerate() {
        let kind = if primary_is(&ct, "text/html") {
            PartKind::TextHtml
        } else {
            PartKind::TextPlain
        };
        parts.push(MessagePart {
            id: MessagePartId::new(format!(".smime.inner.{i}")),
            envelope_id: envelope.clone(),
            path: vec!["smime".into(), format!("{}", i + 1)],
            kind,
            content_type: ct,
            charset: Some("utf-8".into()),
            content_id: None,
            description: None,
            filename: None,
            encoding: TransferEncoding::SevenBit,
            original_size: None,
            size: match &content {
                MessageContent::Text(t) => t.len() as u64,
                MessageContent::Binary(b) => b.len() as u64,
                MessageContent::Empty => 0,
            },
            is_attachment: false,
            is_hidden: false,
            nested_in: None,
            nested_headers: None,
            content,
            created_at: now,
            updated_at: now,
        });
    }
}

fn primary_is(content_type: &str, want: &str) -> bool {
    mailiner_core::primary_mime(content_type).eq_ignore_ascii_case(want)
}

fn parse_inner_mime(bytes: &[u8]) -> Vec<(String, MessageContent)> {
    let text = String::from_utf8_lossy(bytes);
    let (headers, body) = split_header_block(&text);
    if headers.is_empty() && !looks_like_headers(&text) {
        return vec![("text/plain".into(), MessageContent::Text(text.into_owned()))];
    }
    let ct = header_value(&headers, "content-type").unwrap_or("text/plain");
    let cte = header_value(&headers, "content-transfer-encoding").unwrap_or("7bit");
    let (mime, params) = split_content_type(ct);
    if (mime.eq_ignore_ascii_case("multipart/alternative")
        || mime.eq_ignore_ascii_case("multipart/mixed")
        || mime.eq_ignore_ascii_case("multipart/related"))
        && let Some(boundary) = params.get("BOUNDARY")
    {
        return split_multipart(body, boundary);
    }
    let decoded = decode_inner_body(body.as_bytes(), cte, mime);
    vec![(mime.to_string(), decoded)]
}

fn looks_like_headers(text: &str) -> bool {
    text.contains("Content-Type:") || text.contains("content-type:")
}

fn split_header_block(text: &str) -> (BTreeMap<String, String>, &str) {
    let idx = text
        .find("\r\n\r\n")
        .map(|i| (i, 4))
        .or_else(|| text.find("\n\n").map(|i| (i, 2)));
    let Some((i, sep)) = idx else {
        return (BTreeMap::new(), text);
    };
    let mut headers = BTreeMap::new();
    let mut current: Option<(String, String)> = None;
    for line in text[..i].lines() {
        if let Some(rest) = line.strip_prefix(' ').or_else(|| line.strip_prefix('\t')) {
            if let Some((_, v)) = current.as_mut() {
                v.push(' ');
                v.push_str(rest.trim());
            }
            continue;
        }
        if let Some((k, v)) = current.take() {
            headers.insert(k, v);
        }
        if let Some((k, v)) = line.split_once(':') {
            current = Some((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    if let Some((k, v)) = current {
        headers.insert(k, v);
    }
    (headers, &text[i + sep..])
}

fn header_value<'a>(headers: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    headers.get(name).map(String::as_str)
}

fn split_multipart(body: &str, boundary: &str) -> Vec<(String, MessageContent)> {
    let delim = format!("--{boundary}");
    let mut out = Vec::new();
    for chunk in body.split(&delim).skip(1) {
        let chunk = chunk.trim_start_matches("\r\n").trim_start_matches('\n');
        if chunk.starts_with("--") {
            break;
        }
        out.extend(parse_inner_mime(chunk.as_bytes()));
    }
    out
}

fn decode_inner_body(body: &[u8], cte: &str, mime: &str) -> MessageContent {
    match mailiner_mime::decode_content(body, cte, mime, Some("utf-8")) {
        Ok(mailiner_mime::DecodedContent::Text(t)) => MessageContent::Text(t),
        Ok(mailiner_mime::DecodedContent::Binary(b)) => String::from_utf8(b)
            .map(MessageContent::Text)
            .unwrap_or_else(|e| MessageContent::Binary(e.into_bytes())),
        Err(_) => MessageContent::Text(String::from_utf8_lossy(body).into_owned()),
    }
}

/// Import PEM or PKCS#12. `password` is used for PKCS#12 / encrypted keys.
pub fn import_smime_material(bytes: &[u8], password: &str) -> Result<SmimeIdentity, String> {
    if looks_like_pkcs12(bytes) {
        return import_pkcs12(bytes, password);
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "certificate must be PEM text or PKCS#12".to_string())?;
    import_pem(text, password)
}

fn looks_like_pkcs12(bytes: &[u8]) -> bool {
    if bytes.len() < 4 {
        return false;
    }
    // DER SEQUENCE, typical PKCS#12 start.
    bytes[0] == 0x30 && !bytes.windows(11).any(|w| w == b"-----BEGIN")
}

fn import_pem(text: &str, password: &str) -> Result<SmimeIdentity, String> {
    let certs = parse_certs_pem(text);
    if certs.is_empty() {
        return Err("no certificates found in PEM data".into());
    }
    let leaf = &certs[0];
    let label = cert_label(leaf);
    let cert_pem = pem_join(&certs)?;
    let key_pem = extract_key_pem(text, password)?;
    if let Some(ref key) = key_pem {
        validate_key_pem(key)?;
    }
    Ok(SmimeIdentity::new(
        label,
        cert_pem,
        key_pem.unwrap_or_default(),
    ))
}

fn pem_join(certs: &[Certificate]) -> Result<String, String> {
    let mut out = String::new();
    for cert in certs {
        let der = cert
            .to_der()
            .map_err(|e| format!("encode certificate: {e}"))?;
        out.push_str(&pem_block("CERTIFICATE", &der));
    }
    Ok(out)
}

fn pem_block(label: &str, der: &[u8]) -> String {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or(""));
        out.push('\n');
    }
    out.push_str("-----END ");
    out.push_str(label);
    out.push_str("-----\n");
    out
}

fn extract_key_pem(text: &str, password: &str) -> Result<Option<String>, String> {
    const LABELS: &[&str] = &[
        "PRIVATE KEY",
        "RSA PRIVATE KEY",
        "EC PRIVATE KEY",
        "ENCRYPTED PRIVATE KEY",
    ];
    for label in LABELS {
        let begin = format!("-----BEGIN {label}-----");
        let end = format!("-----END {label}-----");
        if let (Some(s), Some(e)) = (text.find(&begin), text.find(&end)) {
            let block = text[s..e + end.len()].trim().to_string();
            if *label == "ENCRYPTED PRIVATE KEY" {
                if password.is_empty() {
                    return Err("encrypted private key requires a password".into());
                }
                let decrypted = decrypt_pkcs8_pem(&block, password)?;
                return Ok(Some(decrypted));
            }
            return Ok(Some(format!("{block}\n")));
        }
    }
    Ok(None)
}

fn decrypt_pkcs8_pem(_pem: &str, _password: &str) -> Result<String, String> {
    Err("encrypted PEM keys are not supported; import PKCS#12 (.p12/.pfx) instead".into())
}

fn validate_key_pem(pem: &str) -> Result<(), String> {
    use pkcs8::DecodePrivateKey;
    use rsa::pkcs1::DecodeRsaPrivateKey;
    if rsa::RsaPrivateKey::from_pkcs8_pem(pem).is_ok()
        || rsa::RsaPrivateKey::from_pkcs1_pem(pem).is_ok()
        || p256::SecretKey::from_pkcs8_pem(pem).is_ok()
        || p384::SecretKey::from_pkcs8_pem(pem).is_ok()
    {
        return Ok(());
    }
    Err("unrecognized private key PEM".into())
}

fn import_pkcs12(bytes: &[u8], password: &str) -> Result<SmimeIdentity, String> {
    if password.is_empty() {
        return Err("PKCS#12 import requires a password".into());
    }
    let store = p12_keystore::KeyStore::from_pkcs12(bytes, password)
        .map_err(|e| format!("invalid PKCS#12: {e}"))?;
    if let Some((alias, chain)) = store.private_key_chain() {
        let certs = chain.chain();
        if certs.is_empty() {
            return Err("PKCS#12 has no certificate".into());
        }
        let mut pem = String::new();
        let mut label = alias.to_string();
        for (i, cert) in certs.iter().enumerate() {
            let der = cert.as_der().to_vec();
            if i == 0
                && let Ok(parsed) = Certificate::from_der(&der)
            {
                label = cert_label(&parsed);
            }
            pem.push_str(&pem_block("CERTIFICATE", &der));
        }
        let key_pem = pem_block("PRIVATE KEY", chain.key());
        validate_key_pem(&key_pem)?;
        return Ok(SmimeIdentity::new(label, pem, key_pem));
    }
    // Trust-store (certs only).
    for (_alias, entry) in store.entries() {
        if let p12_keystore::KeyStoreEntry::Certificate(cert) = entry {
            let der = cert.as_der();
            let parsed = Certificate::from_der(der).map_err(|e| format!("PKCS#12 cert: {e}"))?;
            return Ok(SmimeIdentity::new(
                cert_label(&parsed),
                pem_block("CERTIFICATE", der),
                "",
            ));
        }
    }
    Err("PKCS#12 contained no certificate".into())
}

// CMS / PKCS#7 OIDs
const ID_SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
const ID_ENVELOPED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.3");
const ID_MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const ID_SHA_1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.14.3.2.26");
const ID_SHA_256: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const ID_SHA_384: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.2");
const ID_SHA_512: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.3");
const ID_RSA_ENCRYPTION: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const ID_SHA1_WITH_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.5");
const ID_SHA256_WITH_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.11");
const ID_SHA384_WITH_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.12");
const ID_SHA512_WITH_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.13");
const ID_EC_PUBLIC_KEY: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");
const ID_ECDSA_SHA1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.1");
const ID_ECDSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");
const ID_ECDSA_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.3");
const ID_ECDSA_SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.4");

#[cfg(test)]
mod tests {
    use super::*;
    use mailiner_core::ids::{FolderId, MessageId};
    use mailiner_core::models::MessagePart;

    fn dummy_part(ct: &str, content: MessageContent) -> MessagePart {
        let now = Utc::now();
        MessagePart {
            id: MessagePartId::new("p"),
            envelope_id: MessageId::new(FolderId::new("INBOX"), "1"),
            path: vec!["1".into()],
            kind: PartKind::Attachment,
            content_type: ct.into(),
            charset: None,
            content_id: None,
            description: None,
            filename: None,
            encoding: TransferEncoding::Base64,
            original_size: None,
            size: 0,
            is_attachment: true,
            is_hidden: false,
            nested_in: None,
            nested_headers: None,
            content,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn banner_state_helpers() {
        assert_eq!(
            banner_from_outcome(SmimeOutcome::DetectedSigned { signer: None }).label(),
            "Signed"
        );
        assert_eq!(
            banner_from_outcome(SmimeOutcome::Valid {
                signer: Some("ada@example.com".into())
            })
            .label(),
            "Signature valid"
        );
        assert_eq!(
            banner_from_outcome(SmimeOutcome::Failed {
                reason: "nope".into()
            })
            .label(),
            "Signature failed"
        );
        assert_eq!(
            banner_from_outcome(SmimeOutcome::NeedCert { encrypted: true }).label(),
            "Need certificate"
        );
        assert_eq!(
            banner_from_outcome(SmimeOutcome::NeedCert { encrypted: false }).tone(),
            SmimeTone::Warn
        );
        assert_eq!(
            banner_from_outcome(SmimeOutcome::Valid { signer: None }).tone(),
            SmimeTone::Ok
        );
        assert_eq!(
            banner_from_outcome(SmimeOutcome::Failed { reason: "x".into() }).tone(),
            SmimeTone::Fail
        );
        assert!(
            banner_from_outcome(SmimeOutcome::NeedCert { encrypted: true })
                .detail()
                .contains("encrypted")
        );
    }

    #[test]
    fn detect_pkcs7_mime_vs_signed_from_parts() {
        let mime = dummy_part(
            "application/pkcs7-mime; smime-type=signed-data",
            MessageContent::Binary(vec![0x30, 0x00]),
        );
        let d = detect_from_parts(&[mime]).unwrap();
        assert_eq!(d.role, SmimeRole::Pkcs7Mime);
        assert!(d.is_signed());

        let body = dummy_part("text/plain", MessageContent::Text("hi".into()));
        let mut body = body;
        body.kind = PartKind::TextPlain;
        body.is_attachment = false;
        let sig = dummy_part(
            "application/pkcs7-signature",
            MessageContent::Binary(vec![1]),
        );
        let d = detect_from_parts(&[body, sig]).unwrap();
        assert_eq!(d.role, SmimeRole::MultipartSigned);
    }

    #[test]
    fn reject_garbage_cms() {
        let mut parts = vec![dummy_part(
            "application/pkcs7-mime; smime-type=signed-data",
            MessageContent::Binary(b"this is not cms".to_vec()),
        )];
        let banner = evaluate_parts(&mut parts, &SmimeTrust::default()).unwrap();
        assert!(matches!(banner, SmimeBanner::SignatureFailed { .. }));
    }

    #[test]
    fn enveloped_needs_certificate() {
        let mut parts = vec![dummy_part(
            "application/pkcs7-mime; smime-type=enveloped-data",
            MessageContent::Empty,
        )];
        let banner = evaluate_parts(&mut parts, &SmimeTrust::default()).unwrap();
        assert!(matches!(
            banner,
            SmimeBanner::NeedCertificate { encrypted: true }
        ));
    }

    #[test]
    fn import_pem_rejects_garbage() {
        let err = import_smime_material(b"not a cert", "").unwrap_err();
        assert!(
            err.contains("no certificates") || err.contains("PEM"),
            "{err}"
        );
    }

    #[test]
    fn import_pem_roundtrip_cert_and_key() {
        let (identity, _, _) = rsa_signed_fixture();
        let mut blob = identity.cert_pem.clone();
        blob.push_str(&identity.key_pem);
        let imported = import_smime_material(blob.as_bytes(), "").unwrap();
        assert!(imported.label.contains("Ada") || imported.label.contains("ada"));
        assert!(imported.has_private_key());
        assert!(imported.cert_pem.contains("BEGIN CERTIFICATE"));
    }

    #[test]
    fn import_and_verify_rsa_signed_data() {
        let (identity, cms, inner) = rsa_signed_fixture();
        let mut trust = SmimeTrust::default();
        trust.identities.push(identity.public_only());
        let mut parts = vec![dummy_part(
            "application/pkcs7-mime; smime-type=signed-data",
            MessageContent::Binary(cms),
        )];
        let banner = evaluate_parts(&mut parts, &trust).unwrap();
        assert!(
            matches!(banner, SmimeBanner::SignatureValid { .. }),
            "{banner:?}"
        );
        assert!(
            parts.iter().any(|p| matches!(
                &p.content,
                MessageContent::Text(t) if t.contains(&inner)
            )),
            "inner signed content should be expanded"
        );
    }

    #[test]
    fn verify_without_trust_is_signed_not_valid() {
        let (_identity, cms, _) = rsa_signed_fixture();
        let mut parts = vec![dummy_part(
            "application/pkcs7-mime; smime-type=signed-data",
            MessageContent::Binary(cms),
        )];
        let banner = evaluate_parts(&mut parts, &SmimeTrust::default()).unwrap();
        assert!(matches!(banner, SmimeBanner::Signed { .. }), "{banner:?}");
    }

    fn rsa_signed_fixture() -> (SmimeIdentity, Vec<u8>, String) {
        use cms::builder::{SignedDataBuilder, SignerInfoBuilder};
        use cms::signed_data::EncapsulatedContentInfo;
        use der::asn1::OctetString;
        use pkcs8::{EncodePrivateKey, LineEnding};
        use rsa::RsaPrivateKey;
        use rsa::pkcs1v15::SigningKey;
        use spki::AlgorithmIdentifierOwned;

        let mut params =
            rcgen::CertificateParams::new(vec!["ada@example.com".into()]).expect("params");
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Ada Lovelace");
        let key = RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).expect("rsa key");
        let key_pem = key.to_pkcs8_pem(LineEnding::LF).expect("pem").to_string();
        let kp = rcgen::KeyPair::from_pem(&key_pem).expect("rcgen key");
        let rc_cert = params.self_signed(&kp).expect("self-signed");
        let cert = Certificate::from_der(rc_cert.der()).expect("parse cert");
        let signing = SigningKey::<Sha256>::new(key.clone());

        let inner = "Content-Type: text/plain; charset=utf-8\r\n\r\nHello S/MIME\r\n".to_string();
        let os = OctetString::new(inner.as_bytes()).expect("octet");
        let econtent = der::Any::from_der(&os.to_der().expect("octet der")).expect("any");
        let encap = EncapsulatedContentInfo {
            econtent_type: const_oid::db::rfc5911::ID_DATA,
            econtent: Some(econtent),
        };
        let digest_alg = AlgorithmIdentifierOwned {
            oid: ID_SHA_256,
            parameters: None,
        };
        let sib = SignerInfoBuilder::new(
            &signing,
            SignerIdentifier::IssuerAndSerialNumber(cms::cert::IssuerAndSerialNumber {
                issuer: cert.tbs_certificate.issuer.clone(),
                serial_number: cert.tbs_certificate.serial_number.clone(),
            }),
            digest_alg.clone(),
            &encap,
            None,
        )
        .expect("signer info");
        let mut sdb = SignedDataBuilder::new(&encap);
        sdb.add_digest_algorithm(digest_alg).expect("digest alg");
        sdb.add_certificate(CertificateChoices::Certificate(cert.clone()))
            .expect("cert");
        sdb.add_signer_info::<_, rsa::pkcs1v15::Signature>(sib)
            .expect("signer");
        let cms = sdb.build().expect("signed data").to_der().expect("der");

        let key_pem = key.to_pkcs8_pem(LineEnding::LF).expect("pem").to_string();
        let cert_pem = pem_block("CERTIFICATE", &cert.to_der().expect("cert der"));
        let identity = SmimeIdentity::new("Ada Lovelace", cert_pem, key_pem);
        (identity, cms, "Hello S/MIME".into())
    }
}
