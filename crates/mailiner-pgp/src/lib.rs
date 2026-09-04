//! OpenPGP detect, Autocrypt parse, and decrypt/verify for Mailiner.
//!
//! Crate choice: **rPGP** (`pgp` 0.20), the only widely used pure-Rust OpenPGP
//! stack that builds for `wasm32-unknown-unknown` (`--features wasm`, no `bzip2`).
//! Sequoia is native-only and is not used. S/MIME / PKCS7 is out of scope
//! (issue #81).

mod armor;
mod autocrypt;
mod decrypt;
mod detect;
mod mime_inner;
mod process;

pub use armor::{extract_armor_blocks, ArmorBlock, ArmorKind};
pub use autocrypt::{
    parse_autocrypt_header, parse_autocrypt_headers, parse_autocrypt_headers_bytes,
    AutocryptHeader, PreferEncrypt,
};
pub use decrypt::{inspect_secret_key, CryptoError, ImportedSecretKey, SecretKeyInput};
pub use detect::{
    detect_from_body, detect_from_content_type, detect_from_parts, detect_inline, PgpDetection,
    PgpFormat,
};
pub use process::{process_loaded, PublicKeyInput};

/// rPGP crate version this wrapper is written against.
pub const RPGP_CRATE: &str = "pgp";
pub const RPGP_VERSION: &str = "0.20";
