//! Apply detect + decrypt/verify to a loaded message.

use mailiner_core::models::{
    primary_mime, LoadedMessage, MessageContent, PgpSignatureState, PgpViewState,
};
use pgp::composed::SignedPublicKey;

use crate::armor::ArmorKind;
use crate::decrypt::{
    decrypt_message, first_armor_payload, parse_public_key, parse_secret_key, public_from_secret,
    verify_detached, verify_signed_message, CryptoError, SecretKeyInput,
};
use crate::detect::detect_from_parts;
use crate::mime_inner::parts_from_decrypted;

/// Autocrypt / imported public key (binary or armored).
#[derive(Debug, Clone, Copy)]
pub struct PublicKeyInput<'a> {
    pub data: &'a [u8],
}

/// Decrypt and/or verify `loaded` in place. Updates [`LoadedMessage::pgp`].
pub fn process_loaded(
    loaded: &mut LoadedMessage,
    secrets: &[SecretKeyInput<'_>],
    publics: &[PublicKeyInput<'_>],
) -> PgpViewState {
    let det = detect_from_parts(&loaded.parts);
    if det.is_empty() {
        loaded.pgp = PgpViewState::default();
        return loaded.pgp.clone();
    }

    let pubkeys = collect_publics(secrets, publics);
    let mut state = PgpViewState {
        encrypted: det.encrypted(),
        signed: det.signed(),
        ..PgpViewState::default()
    };

    if det.encrypted() {
        match decrypt_loaded(loaded, secrets) {
            Ok(()) => {
                state.need_private_key = false;
            }
            Err(CryptoError::NeedKey) | Err(CryptoError::BadPassphrase) => {
                state.need_private_key = true;
            }
            Err(_) => {
                state.need_private_key = secrets.is_empty();
            }
        }
    }

    // Re-detect after decrypt (inner cleartext-signed is common).
    let after = detect_from_parts(&loaded.parts);
    if after.signed() || det.signed() {
        state.signed = true;
        match verify_loaded(loaded, &pubkeys) {
            Ok(signer) => {
                state.signature = PgpSignatureState::Valid;
                if !signer.is_empty() {
                    state.signer = Some(signer);
                }
            }
            Err(CryptoError::NeedPublicKey) => {
                state.signature = PgpSignatureState::NeedKey;
            }
            Err(_) => {
                state.signature = PgpSignatureState::Invalid;
            }
        }
    }

    loaded.pgp = state.clone();
    state
}

fn collect_publics(
    secrets: &[SecretKeyInput<'_>],
    publics: &[PublicKeyInput<'_>],
) -> Vec<SignedPublicKey> {
    let mut out = Vec::new();
    for s in secrets {
        if let Ok(sk) = parse_secret_key(s.armored) {
            out.push(public_from_secret(&sk));
        }
    }
    for p in publics {
        if let Ok(pk) = parse_public_key(p.data) {
            out.push(pk);
        }
    }
    out
}

fn decrypt_loaded(
    loaded: &mut LoadedMessage,
    secrets: &[SecretKeyInput<'_>],
) -> Result<(), CryptoError> {
    if let Some(cipher) = find_ciphertext(loaded) {
        let plain = decrypt_message(&cipher, secrets)?;
        replace_with_decrypted(loaded, &plain);
        return Ok(());
    }
    Err(CryptoError::NeedKey)
}

fn find_ciphertext(loaded: &LoadedMessage) -> Option<Vec<u8>> {
    // PGP/MIME: application/octet-stream next to application/pgp-encrypted,
    // or any part whose body is an armored PGP MESSAGE.
    let has_pgp_enc = loaded
        .parts
        .iter()
        .any(|p| primary_mime(&p.content_type).eq_ignore_ascii_case("application/pgp-encrypted"));
    for part in &loaded.parts {
        let mime = primary_mime(&part.content_type);
        let bytes = part_bytes(part);
        if bytes.is_empty() {
            continue;
        }
        if looks_like_pgp_message(&bytes) {
            return Some(bytes);
        }
        if has_pgp_enc && mime.eq_ignore_ascii_case("application/octet-stream") {
            return Some(bytes);
        }
    }
    None
}

fn looks_like_pgp_message(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|t| first_armor_payload(t, ArmorKind::Message))
        .is_some()
}

fn replace_with_decrypted(loaded: &mut LoadedMessage, plain: &[u8]) {
    for part in &mut loaded.parts {
        if part.is_display_part() || is_pgp_payload_part(part) {
            part.is_hidden = true;
        }
    }
    let mut fresh = parts_from_decrypted(&loaded.envelope_id, plain);
    loaded.parts.append(&mut fresh);
}

fn is_pgp_payload_part(part: &mailiner_core::models::MessagePart) -> bool {
    let mime = primary_mime(&part.content_type);
    mime.eq_ignore_ascii_case("application/pgp-encrypted")
        || mime.eq_ignore_ascii_case("application/pgp-signature")
        || mime.eq_ignore_ascii_case("application/octet-stream")
}

fn verify_loaded(
    loaded: &mut LoadedMessage,
    publics: &[SignedPublicKey],
) -> Result<String, CryptoError> {
    // Inline / cleartext first (text parts).
    for part in loaded.parts.iter_mut() {
        let MessageContent::Text(text) = &part.content else {
            continue;
        };
        if first_armor_payload(text, ArmorKind::SignedMessage).is_some()
            || (text.contains("BEGIN PGP MESSAGE") && text.contains("BEGIN PGP SIGNATURE"))
        {
            let (plain, signer) = verify_signed_message(text.as_bytes(), publics)?;
            if part.is_display_part() {
                part.content = MessageContent::Text(plain);
            }
            return Ok(signer);
        }
    }

    // PGP/MIME detached: signature part + first non-signature sibling bytes.
    if let Some((sig, signed)) = find_detached_pair(loaded) {
        return verify_detached(&sig, &signed, publics);
    }

    Err(CryptoError::NeedPublicKey)
}

fn find_detached_pair(loaded: &LoadedMessage) -> Option<(Vec<u8>, Vec<u8>)> {
    let sig_part = loaded.parts.iter().find(|p| {
        primary_mime(&p.content_type).eq_ignore_ascii_case("application/pgp-signature")
    })?;
    let sig = part_bytes(sig_part);
    if sig.is_empty() {
        return None;
    }
    let signed = loaded.parts.iter().find(|p| {
        p.id != sig_part.id
            && p.nested_in == sig_part.nested_in
            && !primary_mime(&p.content_type).eq_ignore_ascii_case("application/pgp-signature")
            && !part_bytes(p).is_empty()
    });
    let signed_bytes = signed
        .or_else(|| loaded.parts.iter().find(|p| p.is_display_part()))
        .map(part_bytes)
        .filter(|b| !b.is_empty())?;
    Some((sig, signed_bytes))
}

fn part_bytes(part: &mailiner_core::models::MessagePart) -> Vec<u8> {
    match &part.content {
        MessageContent::Text(t) => t.as_bytes().to_vec(),
        MessageContent::Binary(b) => b.clone(),
        MessageContent::Empty => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decrypt::inspect_secret_key;
    use chrono::Utc;
    use mailiner_core::ids::{FolderId, MessageId, MessagePartId};
    use mailiner_core::models::{MessagePart, PartKind, TransferEncoding};
    use pgp::composed::{
        EncryptionCaps, KeyType, MessageBuilder, SecretKeyParamsBuilder, SubkeyParamsBuilder,
    };
    use pgp::crypto::ecc_curve::ECCCurve;
    use pgp::crypto::hash::HashAlgorithm;
    use pgp::crypto::sym::SymmetricKeyAlgorithm;
    use rand::thread_rng;
    use smallvec::smallvec;

    fn sample_part(content: MessageContent) -> MessagePart {
        let now = Utc::now();
        MessagePart {
            id: MessagePartId::new("p1"),
            envelope_id: MessageId::new(FolderId::new("INBOX"), "1"),
            path: vec!["1".into()],
            kind: PartKind::TextPlain,
            content_type: "text/plain".into(),
            charset: Some("utf-8".into()),
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

    #[test]
    fn process_inline_encrypted_need_key() {
        let mut loaded = LoadedMessage {
            envelope_id: MessageId::new(FolderId::new("INBOX"), "1"),
            folder_id: FolderId::new("INBOX"),
            parts: vec![sample_part(MessageContent::Text(
                "-----BEGIN PGP MESSAGE-----\n\nww==\n-----END PGP MESSAGE-----".into(),
            ))],
            pgp: Default::default(),
        };
        let state = process_loaded(&mut loaded, &[], &[]);
        assert!(state.encrypted);
        assert!(state.need_private_key);
    }

    #[test]
    fn process_inline_encrypted_decrypts() {
        let mut rng = thread_rng();
        let mut key_params = SecretKeyParamsBuilder::default();
        key_params
            .key_type(KeyType::Ed25519Legacy)
            .can_certify(false)
            .can_sign(true)
            .primary_user_id("Alice <alice@example.com>".into())
            .preferred_symmetric_algorithms(smallvec![SymmetricKeyAlgorithm::AES128])
            .preferred_hash_algorithms(smallvec![HashAlgorithm::Sha256])
            .preferred_compression_algorithms(smallvec![])
            .passphrase(Some("pw".into()))
            .subkeys(vec![SubkeyParamsBuilder::default()
                .key_type(KeyType::ECDH(ECCCurve::Curve25519Legacy))
                .can_encrypt(EncryptionCaps::All)
                .passphrase(Some("pw".into()))
                .build()
                .expect("subkey")]);
        let secret = key_params
            .build()
            .expect("params")
            .generate(&mut rng)
            .expect("generate");
        let armored = secret.to_armored_string(Default::default()).expect("armor");
        inspect_secret_key(&armored, "pw").expect("import");
        let public = secret.to_public_key();
        const DATA: &[u8] = b"secret body";
        let mut builder = MessageBuilder::from_bytes("note.txt", DATA)
            .seipd_v1(&mut rng, SymmetricKeyAlgorithm::AES128);
        builder
            .encrypt_to_key(&mut rng, &public.public_subkeys[0])
            .expect("encrypt");
        let encrypted = builder
            .to_armored_string(&mut rng, Default::default())
            .expect("armor msg");
        let mut loaded = LoadedMessage {
            envelope_id: MessageId::new(FolderId::new("INBOX"), "1"),
            folder_id: FolderId::new("INBOX"),
            parts: vec![sample_part(MessageContent::Text(encrypted))],
            pgp: Default::default(),
        };
        let state = process_loaded(
            &mut loaded,
            &[SecretKeyInput {
                armored: &armored,
                passphrase: "pw",
            }],
            &[],
        );
        assert!(state.encrypted);
        assert!(!state.need_private_key);
        let text = loaded
            .parts
            .iter()
            .find(|p| !p.is_hidden)
            .and_then(|p| match &p.content {
                MessageContent::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .unwrap_or("");
        assert!(text.contains("secret body"), "decrypted={text:?}");
    }
}
