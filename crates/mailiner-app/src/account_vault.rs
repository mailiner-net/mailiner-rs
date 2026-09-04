//! Passphrase-wrapped account secrets (IMAP/SMTP passwords and proxy tokens).
//!
//! Browser builds use WebCrypto AES-GCM with a PBKDF2-SHA-256 key. Host unit
//! tests use the matching rustcrypto primitives so the on-disk format can be
//! exercised without a DOM.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use mailiner_core::ids::AccountId;
use serde::{Deserialize, Serialize};

use crate::account_config::AccountConfig;
use crate::account_store::AccountStoreError;

/// PBKDF2-HMAC-SHA-256 iteration count written by this build.
///
/// Stored on the vault so a later bump does not invalidate existing blobs.
pub const VAULT_PBKDF2_ITERATIONS: u32 = 210_000;

/// Reject a tampered vault that would DoS unlock (or a zero-iteration KDF).
const VAULT_PBKDF2_ITERATIONS_MIN: u32 = 1;
const VAULT_PBKDF2_ITERATIONS_MAX: u32 = 5_000_000;

pub const VAULT_KDF: &str = "pbkdf2-sha256";
pub const VAULT_CIPHER: &str = "aes-256-gcm";

pub const VAULT_SALT_LEN: usize = 16;
pub const VAULT_NONCE_LEN: usize = 12;
pub const VAULT_KEY_LEN: usize = 32;

/// Minimum unlock-passphrase length (Unicode scalar values).
pub const MIN_PASSPHRASE_CHARS: usize = 8;

/// Whether the persisted blob has a vault and whether this session can open it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultState {
    /// No passphrase; secrets sit in plaintext JSON.
    Plaintext,
    /// Vault present; session key is not in memory.
    Locked,
    /// Vault present; session key is in memory.
    Unlocked,
}

/// AES-GCM envelope stored next to the (redacted) account list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretsVault {
    pub kdf: String,
    pub iterations: u32,
    pub salt_b64: String,
    pub cipher: String,
    pub nonce_b64: String,
    pub ciphertext_b64: String,
}

/// Per-account secret fields encrypted inside [`SecretsVault`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountSecrets {
    pub id: AccountId,
    pub imap_password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smtp_password: Option<String>,
    pub proxy_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth2_tokens: Option<crate::account_config::Oauth2Tokens>,
}

/// Imported OpenPGP private key (ASCII-armor + key passphrase).
///
/// Stored inside the same vault as IMAP/SMTP passwords. Debug redacts material.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenPgpSecret {
    pub fingerprint: String,
    #[serde(default)]
    pub user_ids: Vec<String>,
    pub armored: String,
    pub passphrase: String,
}

impl fmt::Debug for OpenPgpSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenPgpSecret")
            .field("fingerprint", &self.fingerprint)
            .field("user_ids", &self.user_ids)
            .field("armored", &"***")
            .field("passphrase", &"***")
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretsPayload {
    pub accounts: Vec<AccountSecrets>,
    /// Absent on older vaults.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pgp_keys: Vec<OpenPgpSecret>,
}

/// Session key derived from the user passphrase. Never written to storage.
#[derive(Clone)]
pub struct VaultKey {
    inner: VaultKeyInner,
    salt: Vec<u8>,
    iterations: u32,
}

#[derive(Clone)]
enum VaultKeyInner {
    #[cfg(target_arch = "wasm32")]
    Web(web_sys::CryptoKey),
    #[cfg(not(target_arch = "wasm32"))]
    Raw([u8; VAULT_KEY_LEN]),
}

impl fmt::Debug for VaultKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VaultKey")
            .field("iterations", &self.iterations)
            .field("salt_len", &self.salt.len())
            .finish_non_exhaustive()
    }
}

impl VaultKey {
    /// Derive a new key with a fresh salt at the current iteration count.
    pub async fn generate(passphrase: &str) -> Result<Self, AccountStoreError> {
        validate_passphrase(passphrase)?;
        let salt = random_bytes(VAULT_SALT_LEN)?;
        Self::derive(passphrase, &salt, VAULT_PBKDF2_ITERATIONS).await
    }

    /// Derive a key from an existing vault's salt and iteration count.
    pub async fn derive(
        passphrase: &str,
        salt: &[u8],
        iterations: u32,
    ) -> Result<Self, AccountStoreError> {
        if passphrase.is_empty() {
            return Err(AccountStoreError::WrongPassphrase);
        }
        if salt.len() != VAULT_SALT_LEN {
            return Err(AccountStoreError::Serialization(format!(
                "vault salt must be {VAULT_SALT_LEN} bytes"
            )));
        }
        if !(VAULT_PBKDF2_ITERATIONS_MIN..=VAULT_PBKDF2_ITERATIONS_MAX).contains(&iterations) {
            return Err(AccountStoreError::Serialization(format!(
                "unsupported vault PBKDF2 iterations {iterations}"
            )));
        }
        let inner = derive_key_inner(passphrase, salt, iterations).await?;
        Ok(Self {
            inner,
            salt: salt.to_vec(),
            iterations,
        })
    }
}

/// Reject a passphrase that is too short to wrap secrets.
pub fn validate_passphrase(passphrase: &str) -> Result<(), AccountStoreError> {
    if passphrase.chars().count() < MIN_PASSPHRASE_CHARS {
        return Err(AccountStoreError::InvalidPassphrase);
    }
    Ok(())
}

/// Pull IMAP/SMTP passwords and proxy tokens out of the in-memory configs.
pub fn extract_secrets(accounts: &[AccountConfig]) -> SecretsPayload {
    SecretsPayload {
        accounts: accounts
            .iter()
            .map(|a| AccountSecrets {
                id: a.id.clone(),
                imap_password: a.imap.password.clone(),
                smtp_password: a.smtp.as_ref().and_then(|s| s.password.clone()),
                proxy_token: a.proxy.token.clone(),
                oauth2_tokens: a.oauth2.as_ref().map(|o| o.tokens.clone()),
            })
            .collect(),
        pgp_keys: Vec::new(),
    }
}

/// Restore secret fields after decrypting the vault.
pub fn apply_secrets(accounts: &mut [AccountConfig], payload: &SecretsPayload) {
    for acc in accounts.iter_mut() {
        if let Some(s) = payload.accounts.iter().find(|s| s.id == acc.id) {
            acc.imap.password = s.imap_password.clone();
            if let Some(smtp) = acc.smtp.as_mut() {
                smtp.password = s.smtp_password.clone();
            }
            acc.proxy.token = s.proxy_token.clone();
            if let Some(oauth) = acc.oauth2.as_mut()
                && let Some(tokens) = s.oauth2_tokens.clone()
            {
                oauth.tokens = tokens;
            }
        }
    }
}

/// Encrypt [`SecretsPayload`] with a new nonce. Salt stays on the key.
pub async fn encrypt_secrets(
    key: &VaultKey,
    payload: &SecretsPayload,
) -> Result<SecretsVault, AccountStoreError> {
    let plaintext =
        serde_json::to_vec(payload).map_err(|e| AccountStoreError::Serialization(e.to_string()))?;
    let nonce = random_bytes(VAULT_NONCE_LEN)?;
    let ciphertext = aes_gcm_encrypt(key, &nonce, &plaintext).await?;
    Ok(SecretsVault {
        kdf: VAULT_KDF.into(),
        iterations: key.iterations,
        salt_b64: B64.encode(&key.salt),
        cipher: VAULT_CIPHER.into(),
        nonce_b64: B64.encode(&nonce),
        ciphertext_b64: B64.encode(&ciphertext),
    })
}

/// Decrypt a vault. Authentication failure is [`AccountStoreError::WrongPassphrase`].
pub async fn decrypt_secrets(
    key: &VaultKey,
    vault: &SecretsVault,
) -> Result<SecretsPayload, AccountStoreError> {
    validate_vault_meta(vault)?;
    let nonce = decode_exact(&vault.nonce_b64, VAULT_NONCE_LEN, "nonce")?;
    let ciphertext = B64
        .decode(vault.ciphertext_b64.as_bytes())
        .map_err(|e| AccountStoreError::Serialization(format!("vault ciphertext: {e}")))?;
    if ciphertext.is_empty() {
        return Err(AccountStoreError::Serialization(
            "vault ciphertext is empty".into(),
        ));
    }
    let plaintext = aes_gcm_decrypt(key, &nonce, &ciphertext).await?;
    serde_json::from_slice(&plaintext)
        .map_err(|e| AccountStoreError::Serialization(format!("vault payload: {e}")))
}

/// Decode salt bytes from a persisted vault (before deriving the session key).
pub fn decode_vault_salt(vault: &SecretsVault) -> Result<Vec<u8>, AccountStoreError> {
    validate_vault_meta(vault)?;
    decode_exact(&vault.salt_b64, VAULT_SALT_LEN, "salt")
}

fn validate_vault_meta(vault: &SecretsVault) -> Result<(), AccountStoreError> {
    if vault.kdf != VAULT_KDF {
        return Err(AccountStoreError::Serialization(format!(
            "unsupported vault kdf {:?}",
            vault.kdf
        )));
    }
    if vault.cipher != VAULT_CIPHER {
        return Err(AccountStoreError::Serialization(format!(
            "unsupported vault cipher {:?}",
            vault.cipher
        )));
    }
    if !(VAULT_PBKDF2_ITERATIONS_MIN..=VAULT_PBKDF2_ITERATIONS_MAX).contains(&vault.iterations) {
        return Err(AccountStoreError::Serialization(format!(
            "unsupported vault PBKDF2 iterations {}",
            vault.iterations
        )));
    }
    Ok(())
}

fn decode_exact(b64: &str, expected: usize, label: &str) -> Result<Vec<u8>, AccountStoreError> {
    let bytes = B64
        .decode(b64.as_bytes())
        .map_err(|e| AccountStoreError::Serialization(format!("vault {label}: {e}")))?;
    if bytes.len() != expected {
        return Err(AccountStoreError::Serialization(format!(
            "vault {label} must be {expected} bytes"
        )));
    }
    Ok(bytes)
}

// ── Crypto backends ─────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn random_bytes(n: usize) -> Result<Vec<u8>, AccountStoreError> {
    use rand::RngCore;
    let mut buf = vec![0u8; n];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    Ok(buf)
}

#[cfg(target_arch = "wasm32")]
fn random_bytes(n: usize) -> Result<Vec<u8>, AccountStoreError> {
    let crypto = web_crypto()?;
    let mut buf = vec![0u8; n];
    crypto
        .get_random_values_with_u8_array(&mut buf)
        .map_err(|_| AccountStoreError::Other("WebCrypto getRandomValues failed".into()))?;
    Ok(buf)
}

#[cfg(not(target_arch = "wasm32"))]
async fn derive_key_inner(
    passphrase: &str,
    salt: &[u8],
    iterations: u32,
) -> Result<VaultKeyInner, AccountStoreError> {
    use pbkdf2::pbkdf2_hmac;
    use sha2::Sha256;
    let mut key = [0u8; VAULT_KEY_LEN];
    pbkdf2_hmac::<Sha256>(passphrase.as_bytes(), salt, iterations, &mut key);
    Ok(VaultKeyInner::Raw(key))
}

#[cfg(target_arch = "wasm32")]
async fn derive_key_inner(
    passphrase: &str,
    salt: &[u8],
    iterations: u32,
) -> Result<VaultKeyInner, AccountStoreError> {
    use js_sys::{Array, Uint8Array};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::JsValue;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{AesKeyGenParams, Pbkdf2Params};

    let subtle = web_crypto()?.subtle();
    let raw = Uint8Array::from(passphrase.as_bytes());
    let import_usages = Array::of1(&JsValue::from_str("deriveKey"));
    let imported = JsFuture::from(
        subtle
            .import_key_with_str(
                "raw",
                raw.unchecked_ref(),
                "PBKDF2",
                false,
                &import_usages.into(),
            )
            .map_err(js_err("WebCrypto importKey"))?,
    )
    .await
    .map_err(js_err("WebCrypto importKey"))?;
    let base_key: web_sys::CryptoKey = imported.dyn_into().map_err(|_| {
        AccountStoreError::Other("WebCrypto importKey returned a non-CryptoKey".into())
    })?;

    let salt_arr = Uint8Array::from(salt);
    let pbkdf = Pbkdf2Params::new(
        "PBKDF2",
        &JsValue::from_str("SHA-256"),
        iterations,
        salt_arr.unchecked_ref(),
    );
    let aes = AesKeyGenParams::new("AES-GCM", 256);
    let usages = Array::of2(&JsValue::from_str("encrypt"), &JsValue::from_str("decrypt"));
    let derived = JsFuture::from(
        subtle
            .derive_key_with_object_and_object(
                pbkdf.as_ref(),
                &base_key,
                aes.as_ref(),
                false,
                &usages.into(),
            )
            .map_err(js_err("WebCrypto deriveKey"))?,
    )
    .await
    .map_err(js_err("WebCrypto deriveKey"))?;
    let key: web_sys::CryptoKey = derived.dyn_into().map_err(|_| {
        AccountStoreError::Other("WebCrypto deriveKey returned a non-CryptoKey".into())
    })?;
    Ok(VaultKeyInner::Web(key))
}

#[cfg(not(target_arch = "wasm32"))]
async fn aes_gcm_encrypt(
    key: &VaultKey,
    nonce: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, AccountStoreError> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};
    let VaultKeyInner::Raw(raw) = &key.inner;
    let cipher = Aes256Gcm::new_from_slice(raw)
        .map_err(|_| AccountStoreError::Other("invalid AES-GCM key".into()))?;
    let nonce = Nonce::from_slice(nonce);
    cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| AccountStoreError::Other("AES-GCM encrypt failed".into()))
}

#[cfg(target_arch = "wasm32")]
async fn aes_gcm_encrypt(
    key: &VaultKey,
    nonce: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, AccountStoreError> {
    let VaultKeyInner::Web(crypto_key) = &key.inner;
    web_aes_gcm(crypto_key, nonce, plaintext, true).await
}

#[cfg(not(target_arch = "wasm32"))]
async fn aes_gcm_decrypt(
    key: &VaultKey,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, AccountStoreError> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};
    let VaultKeyInner::Raw(raw) = &key.inner;
    let cipher = Aes256Gcm::new_from_slice(raw)
        .map_err(|_| AccountStoreError::Other("invalid AES-GCM key".into()))?;
    let nonce = Nonce::from_slice(nonce);
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| AccountStoreError::WrongPassphrase)
}

#[cfg(target_arch = "wasm32")]
async fn aes_gcm_decrypt(
    key: &VaultKey,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, AccountStoreError> {
    let VaultKeyInner::Web(crypto_key) = &key.inner;
    web_aes_gcm(crypto_key, nonce, ciphertext, false).await
}

#[cfg(target_arch = "wasm32")]
async fn web_aes_gcm(
    key: &web_sys::CryptoKey,
    nonce: &[u8],
    data: &[u8],
    encrypt: bool,
) -> Result<Vec<u8>, AccountStoreError> {
    use js_sys::Uint8Array;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::AesGcmParams;

    let iv = Uint8Array::from(nonce);
    let params = AesGcmParams::new("AES-GCM", iv.unchecked_ref());
    let subtle = web_crypto()?.subtle();
    let promise = if encrypt {
        subtle.encrypt_with_object_and_u8_array(params.as_ref(), key, data)
    } else {
        subtle.decrypt_with_object_and_u8_array(params.as_ref(), key, data)
    }
    .map_err(|e| {
        if encrypt {
            js_err("WebCrypto encrypt")(e)
        } else {
            AccountStoreError::WrongPassphrase
        }
    })?;
    let js = JsFuture::from(promise).await.map_err(|e| {
        if encrypt {
            js_err("WebCrypto encrypt")(e)
        } else {
            AccountStoreError::WrongPassphrase
        }
    })?;
    let buf = js_sys::Uint8Array::new(&js);
    let mut out = vec![0u8; buf.length() as usize];
    buf.copy_to(&mut out);
    Ok(out)
}

#[cfg(target_arch = "wasm32")]
fn web_crypto() -> Result<web_sys::Crypto, AccountStoreError> {
    web_sys::window()
        .ok_or(AccountStoreError::Unavailable)?
        .crypto()
        .map_err(|_| AccountStoreError::Other("WebCrypto is unavailable".into()))
}

#[cfg(target_arch = "wasm32")]
fn js_err(op: &'static str) -> impl Fn(wasm_bindgen::JsValue) -> AccountStoreError {
    move |_| AccountStoreError::Other(format!("{op} failed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    use crate::account_config::{ImapSettings, ProxySettings, SmtpSettings, SmtpTlsMode};

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    fn sample(id: &str, imap_pw: &str, smtp_pw: Option<&str>, token: &str) -> AccountConfig {
        let ts = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        AccountConfig {
            id: AccountId::new(id),
            display_name: id.into(),
            email: format!("{id}@example.com"),
            identities: Vec::new(),
            signature: None,
            auth_kind: crate::account_config::AuthKind::Password,
            oauth2: None,
            imap: ImapSettings::new(
                "imap.example.com".into(),
                993,
                format!("{id}@example.com"),
                imap_pw.into(),
                crate::account_config::ImapTlsMode::Implicit,
            ),
            smtp: Some(SmtpSettings::new(
                "smtp.example.com".into(),
                465,
                format!("{id}@example.com"),
                smtp_pw.map(str::to_string),
                SmtpTlsMode::Implicit,
            )),
            proxy: ProxySettings {
                base_url: "wss://proxy.example/proxy".into(),
                token: token.into(),
                remote_host: None,
                remote_port: None,
            },
            extra_ca_pems: Vec::new(),
            created_at: ts,
            updated_at: ts,
        }
    }

    #[test]
    fn validate_passphrase_rejects_short() {
        assert_eq!(
            validate_passphrase("short"),
            Err(AccountStoreError::InvalidPassphrase)
        );
        assert!(validate_passphrase("long-enough").is_ok());
    }

    #[test]
    fn extract_and_apply_roundtrip() {
        let original = sample("a1", "imap-secret", Some("smtp-secret"), "proxy-token");
        let payload = extract_secrets(std::slice::from_ref(&original));
        let mut restored = original.clone();
        restored.redact_secrets();
        assert!(restored.imap.password.is_empty());
        apply_secrets(std::slice::from_mut(&mut restored), &payload);
        assert_eq!(restored.imap.password, "imap-secret");
        assert_eq!(
            restored.smtp.as_ref().and_then(|s| s.password.as_deref()),
            Some("smtp-secret")
        );
        assert_eq!(restored.proxy.token, "proxy-token");
    }

    #[test]
    fn extract_and_apply_roundtrip_oauth_tokens() {
        let mut original = sample("a1", "", None, "proxy-token");
        original.auth_kind = crate::account_config::AuthKind::Oauth2;
        original.oauth2 = Some(crate::account_config::Oauth2Settings {
            provider: crate::account_config::Oauth2Provider::Google,
            client_id: "cid".into(),
            tenant: None,
            tokens: crate::account_config::Oauth2Tokens {
                access_token: "ya29.secret".into(),
                refresh_token: Some("1//refresh".into()),
                expires_at: None,
            },
        });
        let payload = extract_secrets(std::slice::from_ref(&original));
        assert_eq!(
            payload.accounts[0]
                .oauth2_tokens
                .as_ref()
                .map(|t| t.access_token.as_str()),
            Some("ya29.secret")
        );
        let mut restored = original.clone();
        restored.redact_secrets();
        assert!(
            restored
                .oauth2
                .as_ref()
                .is_some_and(|o| o.tokens.access_token.is_empty())
        );
        apply_secrets(std::slice::from_mut(&mut restored), &payload);
        assert_eq!(
            restored
                .oauth2
                .as_ref()
                .map(|o| o.tokens.access_token.as_str()),
            Some("ya29.secret")
        );
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        block_on(async {
            let key = VaultKey::generate("correct-horse").await.unwrap();
            let payload = extract_secrets(&[sample(
                "a1",
                "imap-secret",
                Some("smtp-secret"),
                "proxy-token",
            )]);
            let vault = encrypt_secrets(&key, &payload).await.unwrap();
            assert_eq!(vault.kdf, VAULT_KDF);
            assert_eq!(vault.cipher, VAULT_CIPHER);
            assert_eq!(vault.iterations, VAULT_PBKDF2_ITERATIONS);
            assert!(!vault.ciphertext_b64.contains("imap-secret"));
            let back = decrypt_secrets(&key, &vault).await.unwrap();
            assert_eq!(back, payload);
        });
    }

    #[test]
    fn wrong_passphrase_fails_auth() {
        block_on(async {
            let key = VaultKey::generate("correct-horse").await.unwrap();
            let payload = extract_secrets(&[sample("a1", "imap-secret", None, "tok")]);
            let vault = encrypt_secrets(&key, &payload).await.unwrap();
            let salt = decode_vault_salt(&vault).unwrap();
            let other = VaultKey::derive("wrong-pass-word", &salt, vault.iterations)
                .await
                .unwrap();
            let err = decrypt_secrets(&other, &vault).await.unwrap_err();
            assert_eq!(err, AccountStoreError::WrongPassphrase);
        });
    }

    #[test]
    fn vault_key_debug_hides_material() {
        let key = block_on(VaultKey::generate("correct-horse")).unwrap();
        let dbg = format!("{key:?}");
        assert!(!dbg.contains("correct-horse"), "passphrase leaked: {dbg}");
        assert!(dbg.contains("VaultKey"));
    }

    #[test]
    fn decode_rejects_unknown_kdf() {
        let vault = SecretsVault {
            kdf: "scrypt".into(),
            iterations: VAULT_PBKDF2_ITERATIONS,
            salt_b64: B64.encode([0u8; VAULT_SALT_LEN]),
            cipher: VAULT_CIPHER.into(),
            nonce_b64: B64.encode([0u8; VAULT_NONCE_LEN]),
            ciphertext_b64: B64.encode([1u8; 32]),
        };
        let err = validate_vault_meta(&vault).unwrap_err();
        assert!(format!("{err}").contains("kdf"));
    }
}
