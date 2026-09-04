//! Thin rPGP wrapper: import a secret key, decrypt, verify.

use pgp::composed::{
    CleartextSignedMessage, Deserializable, DetachedSignature, Message, SignedPublicKey,
    SignedSecretKey,
};
use pgp::types::{KeyDetails, Password};

use crate::armor::{extract_armor_blocks, ArmorKind};

/// Armored private key + the passphrase that unlocks it.
#[derive(Debug, Clone, Copy)]
pub struct SecretKeyInput<'a> {
    pub armored: &'a str,
    pub passphrase: &'a str,
}

/// Metadata extracted from an imported secret key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedSecretKey {
    pub fingerprint: String,
    pub user_ids: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CryptoError {
    #[error("could not parse OpenPGP key")]
    BadKey,
    #[error("incorrect key passphrase")]
    BadPassphrase,
    #[error("no matching private key")]
    NeedKey,
    #[error("decrypt failed")]
    DecryptFailed,
    #[error("signature verification failed")]
    VerifyFailed,
    #[error("no matching public key")]
    NeedPublicKey,
}

/// Parse an ASCII-armored secret key and check the passphrase by unlocking it.
pub fn inspect_secret_key(
    armored: &str,
    passphrase: &str,
) -> Result<ImportedSecretKey, CryptoError> {
    let key = parse_secret_key(armored)?;
    unlock_secret(&key, passphrase)?;
    Ok(imported_from(&key))
}

pub(crate) fn imported_from(key: &SignedSecretKey) -> ImportedSecretKey {
    ImportedSecretKey {
        fingerprint: fingerprint_hex(&key.fingerprint()),
        user_ids: user_ids_of(key),
    }
}

pub(crate) fn parse_secret_key(armored: &str) -> Result<SignedSecretKey, CryptoError> {
    SignedSecretKey::from_string(armored)
        .map(|(k, _)| k)
        .or_else(|_| SignedSecretKey::from_bytes(armored.as_bytes()))
        .map_err(|_| CryptoError::BadKey)
}

pub(crate) fn parse_public_key(data: &[u8]) -> Result<SignedPublicKey, CryptoError> {
    if let Ok(text) = std::str::from_utf8(data) {
        if text.contains("BEGIN PGP PUBLIC KEY BLOCK") {
            return SignedPublicKey::from_string(text)
                .map(|(k, _)| k)
                .map_err(|_| CryptoError::BadKey);
        }
    }
    SignedPublicKey::from_bytes(data).map_err(|_| CryptoError::BadKey)
}

pub(crate) fn public_from_secret(key: &SignedSecretKey) -> SignedPublicKey {
    key.to_public_key()
}

fn unlock_secret(key: &SignedSecretKey, passphrase: &str) -> Result<(), CryptoError> {
    let pw = Password::from(passphrase.to_string());
    let work = |_: &_, _: &_| Ok(());
    match key.primary_key.unlock(&pw, work) {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(_)) => return Err(CryptoError::BadPassphrase),
        Err(_) => {}
    }
    // Some keys only encrypt the subkeys; try the first secret subkey.
    let sub = key.secret_subkeys.first().ok_or(CryptoError::BadKey)?;
    match sub.unlock(&pw, work) {
        Ok(Ok(())) => Ok(()),
        _ => Err(CryptoError::BadPassphrase),
    }
}

/// Decrypt an armored or binary OpenPGP message.
pub fn decrypt_message(
    data: &[u8],
    secrets: &[SecretKeyInput<'_>],
) -> Result<Vec<u8>, CryptoError> {
    if secrets.is_empty() {
        return Err(CryptoError::NeedKey);
    }
    let mut last = CryptoError::NeedKey;
    for secret in secrets {
        let Ok(key) = parse_secret_key(secret.armored) else {
            last = CryptoError::BadKey;
            continue;
        };
        let pw = Password::from(secret.passphrase.to_string());
        match decrypt_with(&key, &pw, data) {
            Ok(plain) => return Ok(plain),
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn decrypt_with(key: &SignedSecretKey, pw: &Password, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let msg = parse_message(data)?;
    let mut decrypted = msg
        .decrypt(pw, key)
        .map_err(|_| CryptoError::DecryptFailed)?;
    read_message_bytes(&mut decrypted)
}

fn parse_message(data: &[u8]) -> Result<Message<'_>, CryptoError> {
    if let Ok(text) = std::str::from_utf8(data) {
        if text.contains("BEGIN PGP MESSAGE") || text.contains("BEGIN PGP SIGNED MESSAGE") {
            return Message::from_string(text)
                .map(|(m, _)| m)
                .map_err(|_| CryptoError::DecryptFailed);
        }
    }
    Message::from_bytes(data).map_err(|_| CryptoError::DecryptFailed)
}

fn read_message_bytes(msg: &mut Message<'_>) -> Result<Vec<u8>, CryptoError> {
    if let Ok(s) = msg.as_data_string() {
        return Ok(s.into_bytes());
    }
    msg.as_data_vec().map_err(|_| CryptoError::DecryptFailed)
}

/// Verify a cleartext-signed or inline-signed message. Returns the signed text.
pub fn verify_signed_message(
    data: &[u8],
    publics: &[SignedPublicKey],
) -> Result<(String, String), CryptoError> {
    if publics.is_empty() {
        return Err(CryptoError::NeedPublicKey);
    }
    if let Ok(text) = std::str::from_utf8(data) {
        if text.contains("BEGIN PGP SIGNED MESSAGE") {
            return verify_cleartext(text, publics);
        }
    }
    verify_inline_message(data, publics)
}

fn verify_cleartext(
    text: &str,
    publics: &[SignedPublicKey],
) -> Result<(String, String), CryptoError> {
    let (clear, _) =
        CleartextSignedMessage::from_string(text).map_err(|_| CryptoError::VerifyFailed)?;
    let mut last = CryptoError::NeedPublicKey;
    for pk in publics {
        match clear.verify(pk) {
            Ok(_) => {
                let signer = first_user_id(pk).unwrap_or_default();
                return Ok((clear.text().to_string(), signer));
            }
            Err(_) => last = CryptoError::VerifyFailed,
        }
    }
    Err(last)
}

fn verify_inline_message(
    data: &[u8],
    publics: &[SignedPublicKey],
) -> Result<(String, String), CryptoError> {
    let mut msg = parse_message(data).map_err(|_| CryptoError::VerifyFailed)?;
    let mut last = CryptoError::NeedPublicKey;
    for pk in publics {
        match msg.verify(pk) {
            Ok(_) => {
                let signer = first_user_id(pk).unwrap_or_default();
                let text = read_message_bytes(&mut msg)
                    .ok()
                    .and_then(|b| String::from_utf8(b).ok())
                    .unwrap_or_default();
                return Ok((text, signer));
            }
            Err(_) => last = CryptoError::VerifyFailed,
        }
    }
    Err(last)
}

/// Verify a detached signature over `signed_bytes`.
pub fn verify_detached(
    signature: &[u8],
    signed_bytes: &[u8],
    publics: &[SignedPublicKey],
) -> Result<String, CryptoError> {
    if publics.is_empty() {
        return Err(CryptoError::NeedPublicKey);
    }
    let sig = parse_detached(signature)?;
    let mut last = CryptoError::NeedPublicKey;
    for pk in publics {
        match sig.verify(pk, signed_bytes) {
            Ok(()) => return Ok(first_user_id(pk).unwrap_or_default()),
            Err(_) => last = CryptoError::VerifyFailed,
        }
    }
    Err(last)
}

fn parse_detached(data: &[u8]) -> Result<DetachedSignature, CryptoError> {
    if let Ok(text) = std::str::from_utf8(data) {
        if text.contains("BEGIN PGP SIGNATURE") {
            return DetachedSignature::from_string(text)
                .map(|(s, _)| s)
                .map_err(|_| CryptoError::VerifyFailed);
        }
    }
    DetachedSignature::from_bytes(data).map_err(|_| CryptoError::VerifyFailed)
}

pub(crate) fn first_user_id(pk: &SignedPublicKey) -> Option<String> {
    pk.details
        .users
        .first()
        .map(|u| String::from_utf8_lossy(u.id.id()).into_owned())
        .filter(|s| !s.is_empty())
}

fn user_ids_of(key: &SignedSecretKey) -> Vec<String> {
    key.details
        .users
        .iter()
        .map(|u| String::from_utf8_lossy(u.id.id()).into_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

fn fingerprint_hex(fp: &pgp::types::Fingerprint) -> String {
    format!("{fp}")
}

/// Pull the first PGP MESSAGE / SIGNED MESSAGE / SIGNATURE block as bytes.
pub fn first_armor_payload(text: &str, kind: ArmorKind) -> Option<String> {
    extract_armor_blocks(text)
        .into_iter()
        .find(|b| b.kind == kind)
        .map(|b| b.text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pgp::composed::{
        EncryptionCaps, KeyType, MessageBuilder, SecretKeyParamsBuilder, SubkeyParamsBuilder,
    };
    use pgp::crypto::ecc_curve::ECCCurve;
    use pgp::crypto::hash::HashAlgorithm;
    use pgp::crypto::sym::SymmetricKeyAlgorithm;
    use rand::thread_rng;
    use smallvec::smallvec;

    fn generate_keypair(passphrase: &str) -> (String, SignedSecretKey) {
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
            .passphrase(Some(passphrase.to_string()))
            .subkeys(vec![SubkeyParamsBuilder::default()
                .key_type(KeyType::ECDH(ECCCurve::Curve25519Legacy))
                .can_encrypt(EncryptionCaps::All)
                .passphrase(Some(passphrase.to_string()))
                .build()
                .expect("subkey")]);
        let secret = key_params
            .build()
            .expect("params")
            .generate(&mut rng)
            .expect("generate");
        let armored = secret.to_armored_string(Default::default()).expect("armor");
        (armored, secret)
    }

    #[test]
    fn inspect_accepts_passphrase_and_rejects_wrong() {
        let (armored, _) = generate_keypair("correct-horse");
        let info = inspect_secret_key(&armored, "correct-horse").expect("import");
        assert!(info
            .user_ids
            .iter()
            .any(|u| u.contains("alice@example.com")));
        assert!(!info.fingerprint.is_empty());
        assert_eq!(
            inspect_secret_key(&armored, "wrong-pass"),
            Err(CryptoError::BadPassphrase)
        );
    }

    #[test]
    fn decrypt_generated_message() {
        let (armored, secret) = generate_keypair("correct-horse");
        let public = secret.to_public_key();
        let mut rng = thread_rng();
        const DATA: &[u8] = b"Hello OpenPGP";
        let mut builder = MessageBuilder::from_bytes("note.txt", DATA)
            .seipd_v1(&mut rng, SymmetricKeyAlgorithm::AES128);
        builder
            .encrypt_to_key(&mut rng, &public.public_subkeys[0])
            .expect("encrypt");
        let encrypted = builder.to_vec(&mut rng).expect("serialize");
        let plain = decrypt_message(
            &encrypted,
            &[SecretKeyInput {
                armored: &armored,
                passphrase: "correct-horse",
            }],
        )
        .expect("decrypt");
        assert_eq!(plain, DATA);
    }

    #[test]
    fn decrypt_need_key_without_secrets() {
        assert_eq!(
            decrypt_message(
                b"-----BEGIN PGP MESSAGE-----\n\nww==\n-----END PGP MESSAGE-----",
                &[]
            ),
            Err(CryptoError::NeedKey)
        );
    }
}
