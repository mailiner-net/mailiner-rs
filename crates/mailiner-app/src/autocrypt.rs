//! Persist Autocrypt peer public keys for later encrypt-on-send.

use serde::{Deserialize, Serialize};

use crate::account_store::{AccountStoreError, MemoryKvStore, StringKvStore, WebLocalStorage};
use mailiner_pgp::{AutocryptHeader, PreferEncrypt, parse_autocrypt_headers_bytes};

/// `localStorage` key for Autocrypt peers (public keys only).
pub const AUTOCRYPT_LOCAL_STORAGE_KEY: &str = "mailiner.autocrypt.v1";
pub const AUTOCRYPT_SCHEMA_VERSION: u32 = 1;
const MAX_PEERS: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredAutocryptPeer {
    pub addr: String,
    pub prefer_encrypt: String,
    pub keydata_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct AutocryptBlob {
    schema_version: u32,
    #[serde(default)]
    peers: Vec<StoredAutocryptPeer>,
}

impl AutocryptBlob {
    fn empty() -> Self {
        Self {
            schema_version: AUTOCRYPT_SCHEMA_VERSION,
            peers: Vec::new(),
        }
    }
}

fn open_kv() -> Result<Box<dyn StringKvStore>, AccountStoreError> {
    match WebLocalStorage::try_open() {
        Ok(s) => Ok(Box::new(s)),
        Err(AccountStoreError::Unavailable) => Ok(Box::new(MemoryKvStore::new())),
        Err(e) => Err(e),
    }
}

fn load_blob(kv: &dyn StringKvStore) -> AutocryptBlob {
    match kv.get_item(AUTOCRYPT_LOCAL_STORAGE_KEY) {
        Ok(Some(s)) if !s.trim().is_empty() => {
            serde_json::from_str(&s).unwrap_or_else(|_| AutocryptBlob::empty())
        }
        _ => AutocryptBlob::empty(),
    }
}

fn save_blob(kv: &dyn StringKvStore, blob: &AutocryptBlob) -> Result<(), AccountStoreError> {
    let json =
        serde_json::to_string(blob).map_err(|e| AccountStoreError::Serialization(e.to_string()))?;
    kv.set_item(AUTOCRYPT_LOCAL_STORAGE_KEY, &json)
}

/// Remember Autocrypt headers from a raw RFC 5322 header block.
pub fn remember_from_headers(bytes: &[u8]) {
    let parsed = parse_autocrypt_headers_bytes(bytes);
    if parsed.is_empty() {
        return;
    }
    let Ok(kv) = open_kv() else {
        return;
    };
    let mut blob = load_blob(kv.as_ref());
    for header in parsed {
        upsert_peer(&mut blob, &header);
    }
    let _ = save_blob(kv.as_ref(), &blob);
}

fn upsert_peer(blob: &mut AutocryptBlob, header: &AutocryptHeader) {
    let addr = header.addr.trim().to_ascii_lowercase();
    if addr.is_empty() {
        return;
    }
    let stored = StoredAutocryptPeer {
        addr: addr.clone(),
        prefer_encrypt: header.prefer_encrypt.as_str().to_string(),
        keydata_b64: base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &header.keydata,
        ),
    };
    if let Some(slot) = blob
        .peers
        .iter_mut()
        .find(|p| p.addr.eq_ignore_ascii_case(&addr))
    {
        *slot = stored;
        return;
    }
    if blob.peers.len() >= MAX_PEERS {
        blob.peers.remove(0);
    }
    blob.peers.push(stored);
}

/// Public key blobs for signature verification / later encrypt-on-send.
pub fn public_key_blobs() -> Vec<Vec<u8>> {
    let Ok(kv) = open_kv() else {
        return Vec::new();
    };
    let blob = load_blob(kv.as_ref());
    blob.peers
        .iter()
        .filter_map(|p| {
            base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                p.keydata_b64.as_bytes(),
            )
            .ok()
        })
        .collect()
}

pub fn prefer_encrypt_label(p: PreferEncrypt) -> &'static str {
    p.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use mailiner_pgp::parse_autocrypt_header;

    #[test]
    fn upsert_replaces_same_addr() {
        let mut blob = AutocryptBlob::empty();
        let key1 = base64::engine::general_purpose::STANDARD.encode(b"key-one");
        let key2 = base64::engine::general_purpose::STANDARD.encode(b"key-two");
        let a = parse_autocrypt_header(&format!("addr=Alice@Example.com; keydata={key1}")).unwrap();
        let b = parse_autocrypt_header(&format!(
            "addr=alice@example.com; prefer-encrypt=mutual; keydata={key2}"
        ))
        .unwrap();
        upsert_peer(&mut blob, &a);
        upsert_peer(&mut blob, &b);
        assert_eq!(blob.peers.len(), 1);
        assert_eq!(blob.peers[0].prefer_encrypt, "mutual");
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(blob.peers[0].keydata_b64.as_bytes())
                .unwrap(),
            b"key-two"
        );
    }
}
