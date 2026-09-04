//! Export account settings and wipe Mailiner-owned `localStorage` keys.
//!
//! Envelope/part cache in IndexedDB is wiped via [`crate::mail_cache::MailCache::clear_all`].

use chrono::{DateTime, Utc};
use mailiner_core::ids::AccountId;
use serde::{Deserialize, Serialize};

use crate::account_config::{ACCOUNT_STORE_SCHEMA_VERSION, AccountConfig};
use crate::account_store::{ACCOUNTS_LOCAL_STORAGE_KEY, AccountStoreError};
use crate::address_book::ADDRESS_BOOK_LOCAL_STORAGE_KEY;
use crate::layout::{FOLDER_WIDTH_KEY, LIST_HEIGHT_KEY, LIST_WIDTH_KEY};
use crate::mail_cache::MAIL_CACHE_LOCAL_STORAGE_KEY;
use crate::mail_rules::{MAIL_RULES_APPLIED_KEY, MAIL_RULES_KEY};
use crate::outbox_store::OUTBOX_LOCAL_STORAGE_KEY;
use crate::recipient_suggest::RECENT_RECIPIENTS_LOCAL_STORAGE_KEY;
use crate::ui_prefs::{
    ACK_UNREAD_KEY, COMPOSE_PLACEMENT_KEY, LAST_MAILBOX_KEY, MAIL_LAYOUT_KEY,
    MESSAGE_LIST_VIEW_KEY, MESSAGE_SORT_KEY, PINNED_MESSAGES_KEY, SAVED_SEARCHES_KEY,
    SHORTCUT_MAP_KEY, SNOOZED_MESSAGES_KEY,
};

/// Prefix of every Mailiner-owned `localStorage` key.
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
pub const MAILINER_STORAGE_PREFIX: &str = "mailiner.";

/// `format` field on [`AccountsExport`].
pub const ACCOUNTS_EXPORT_FORMAT: &str = "mailiner.accounts.export";

/// Keys Mailiner is known to write. Used as a fallback if enumeration misses one.
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
pub const KNOWN_MAILINER_STORAGE_KEYS: &[&str] = &[
    ACCOUNTS_LOCAL_STORAGE_KEY,
    ADDRESS_BOOK_LOCAL_STORAGE_KEY,
    RECENT_RECIPIENTS_LOCAL_STORAGE_KEY,
    MAIL_CACHE_LOCAL_STORAGE_KEY,
    OUTBOX_LOCAL_STORAGE_KEY,
    MESSAGE_SORT_KEY,
    MESSAGE_LIST_VIEW_KEY,
    LAST_MAILBOX_KEY,
    ACK_UNREAD_KEY,
    COMPOSE_PLACEMENT_KEY,
    MAIL_LAYOUT_KEY,
    SHORTCUT_MAP_KEY,
    SAVED_SEARCHES_KEY,
    PINNED_MESSAGES_KEY,
    SNOOZED_MESSAGES_KEY,
    MAIL_RULES_KEY,
    MAIL_RULES_APPLIED_KEY,
    FOLDER_WIDTH_KEY,
    LIST_HEIGHT_KEY,
    LIST_WIDTH_KEY,
    E2E_SKIP_CONNECT_KEY,
];

/// Playwright e2e: skip live IMAP (no WebSocket, no auto-reconnect).
pub const E2E_SKIP_CONNECT_KEY: &str = "mailiner.e2e.skipConnect";

/// `true` when [`E2E_SKIP_CONNECT_KEY`] is `1` or `true`.
pub fn e2e_skip_connect() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .and_then(|s| s.get_item(E2E_SKIP_CONNECT_KEY).ok().flatten())
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        false
    }
}

/// Downloadable account list. `includes_secrets` is the only difference between
/// the public export and the warned full backup.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountsExport {
    pub format: String,
    pub schema_version: u32,
    pub includes_secrets: bool,
    pub exported_at: DateTime<Utc>,
    pub active_account_id: Option<AccountId>,
    pub accounts: Vec<AccountConfig>,
}

impl AccountsExport {
    pub fn new(
        accounts: impl IntoIterator<Item = AccountConfig>,
        active_account_id: Option<AccountId>,
        includes_secrets: bool,
        exported_at: DateTime<Utc>,
    ) -> Self {
        let accounts: Vec<AccountConfig> = if includes_secrets {
            accounts.into_iter().collect()
        } else {
            accounts.into_iter().map(|c| c.without_secrets()).collect()
        };
        Self {
            format: ACCOUNTS_EXPORT_FORMAT.into(),
            schema_version: ACCOUNT_STORE_SCHEMA_VERSION,
            includes_secrets,
            exported_at,
            active_account_id,
            accounts,
        }
    }

    pub fn to_pretty_json(&self) -> Result<String, AccountStoreError> {
        serde_json::to_string_pretty(self)
            .map_err(|e| AccountStoreError::Serialization(e.to_string()))
    }
}

/// Suggested download name for an accounts export.
pub fn accounts_export_filename(includes_secrets: bool, exported_at: DateTime<Utc>) -> String {
    let day = exported_at.format("%Y-%m-%d");
    if includes_secrets {
        format!("mailiner-accounts-full-backup-{day}.json")
    } else {
        format!("mailiner-accounts-{day}.json")
    }
}

/// True when `key` is owned by Mailiner (`mailiner.` prefix).
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
pub fn is_mailiner_storage_key(key: &str) -> bool {
    key.starts_with(MAILINER_STORAGE_PREFIX)
}

/// Filter an arbitrary key list down to Mailiner-owned keys (stable input order).
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
pub fn mailiner_keys_to_clear<S: AsRef<str>>(keys: impl IntoIterator<Item = S>) -> Vec<String> {
    keys.into_iter()
        .map(|k| k.as_ref().to_string())
        .filter(|k| is_mailiner_storage_key(k))
        .collect()
}

/// Remove every Mailiner-owned key in `keys` via `remove`.
///
/// Non-`mailiner.*` keys are ignored. Continues after an individual `remove`
/// failure so one blocked key cannot leave the rest behind. Returns the number
/// of successful removals, or the first error after attempting every key.
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
pub fn remove_mailiner_keys(
    keys: &[String],
    mut remove: impl FnMut(&str) -> Result<(), AccountStoreError>,
) -> Result<usize, AccountStoreError> {
    let mut n = 0;
    let mut first_err = None;
    for key in mailiner_keys_to_clear(keys.iter().map(String::as_str)) {
        match remove(&key) {
            Ok(()) => n += 1,
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
    }
    match first_err {
        Some(e) => Err(e),
        None => Ok(n),
    }
}

/// Delete every `mailiner.*` key in `window.localStorage` (no-op on host).
pub fn clear_mailiner_local_storage() -> Result<usize, AccountStoreError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Ok(0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let window = web_sys::window().ok_or(AccountStoreError::Unavailable)?;
        let storage = window
            .local_storage()
            .map_err(|_| AccountStoreError::Unavailable)?
            .ok_or(AccountStoreError::Unavailable)?;
        let len = storage
            .length()
            .map_err(|_| AccountStoreError::Unavailable)?;
        let mut keys = Vec::with_capacity(len as usize + KNOWN_MAILINER_STORAGE_KEYS.len());
        for i in 0..len {
            if let Ok(Some(k)) = storage.key(i) {
                keys.push(k);
            }
        }
        for known in KNOWN_MAILINER_STORAGE_KEYS {
            if !keys.iter().any(|k| k == known) {
                keys.push((*known).to_string());
            }
        }
        remove_mailiner_keys(&keys, |k| {
            storage
                .remove_item(k)
                .map_err(|_| AccountStoreError::Unavailable)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use mailiner_core::ids::AccountId;

    use crate::account_config::{ImapSettings, ProxySettings, SmtpSettings, SmtpTlsMode};

    fn ts() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 3, 12, 0, 0).unwrap()
    }

    fn sample() -> AccountConfig {
        AccountConfig {
            id: AccountId::new("acc-1"),
            display_name: "Work".into(),
            email: "user@example.com".into(),
            identities: Vec::new(),
            signature: None,
            imap: ImapSettings::new(
                "imap.example.com".into(),
                993,
                "user@example.com".into(),
                "s3cret".into(),
                crate::account_config::ImapTlsMode::Implicit,
            ),
            smtp: Some(SmtpSettings::new(
                "smtp.example.com".into(),
                465,
                "user@example.com".into(),
                Some("smtp-secret".into()),
                SmtpTlsMode::Implicit,
            )),
            proxy: ProxySettings {
                base_url: "wss://proxy.example/proxy".into(),
                token: "proxy-token".into(),
                remote_host: None,
                remote_port: None,
            },
            extra_ca_pems: Vec::new(),
            created_at: ts(),
            updated_at: ts(),
        }
    }

    #[test]
    fn public_export_json_omits_secrets() {
        let export =
            AccountsExport::new(vec![sample()], Some(AccountId::new("acc-1")), false, ts());
        assert!(!export.includes_secrets);
        assert!(export.accounts[0].imap.password.is_empty());
        assert!(export.accounts[0].smtp.as_ref().unwrap().password.is_none());
        assert!(export.accounts[0].proxy.token.is_empty());
        let json = export.to_pretty_json().unwrap();
        assert!(json.contains("\"format\": \"mailiner.accounts.export\""));
        assert!(json.contains("\"includes_secrets\": false"));
        assert!(json.contains("imap.example.com"));
        assert!(json.contains("smtp.example.com"));
        assert!(json.contains("wss://proxy.example/proxy"));
        assert!(!json.contains("s3cret"), "imap password leaked: {json}");
        assert!(
            !json.contains("smtp-secret"),
            "smtp password leaked: {json}"
        );
        assert!(!json.contains("proxy-token"), "proxy token leaked: {json}");
    }

    #[test]
    fn full_backup_json_keeps_secrets() {
        let export = AccountsExport::new(vec![sample()], None, true, ts());
        assert!(export.includes_secrets);
        let json = export.to_pretty_json().unwrap();
        assert!(json.contains("\"includes_secrets\": true"));
        assert!(json.contains("s3cret"));
        assert!(json.contains("smtp-secret"));
        assert!(json.contains("proxy-token"));
    }

    #[test]
    fn export_filename_distinguishes_backup() {
        assert_eq!(
            accounts_export_filename(false, ts()),
            "mailiner-accounts-2026-09-03.json"
        );
        assert_eq!(
            accounts_export_filename(true, ts()),
            "mailiner-accounts-full-backup-2026-09-03.json"
        );
    }

    #[test]
    fn known_keys_are_all_mailiner_prefixed() {
        assert!(!KNOWN_MAILINER_STORAGE_KEYS.is_empty());
        for key in KNOWN_MAILINER_STORAGE_KEYS {
            assert!(
                is_mailiner_storage_key(key),
                "known key missing prefix: {key}"
            );
        }
        assert!(KNOWN_MAILINER_STORAGE_KEYS.contains(&ACCOUNTS_LOCAL_STORAGE_KEY));
        assert!(KNOWN_MAILINER_STORAGE_KEYS.contains(&ADDRESS_BOOK_LOCAL_STORAGE_KEY));
        assert!(KNOWN_MAILINER_STORAGE_KEYS.contains(&RECENT_RECIPIENTS_LOCAL_STORAGE_KEY));
        assert!(KNOWN_MAILINER_STORAGE_KEYS.contains(&MAIL_CACHE_LOCAL_STORAGE_KEY));
        assert!(KNOWN_MAILINER_STORAGE_KEYS.contains(&OUTBOX_LOCAL_STORAGE_KEY));
        assert!(KNOWN_MAILINER_STORAGE_KEYS.contains(&LAST_MAILBOX_KEY));
        assert!(KNOWN_MAILINER_STORAGE_KEYS.contains(&ACK_UNREAD_KEY));
        assert!(KNOWN_MAILINER_STORAGE_KEYS.contains(&MESSAGE_SORT_KEY));
        assert!(KNOWN_MAILINER_STORAGE_KEYS.contains(&MESSAGE_LIST_VIEW_KEY));
        assert!(KNOWN_MAILINER_STORAGE_KEYS.contains(&COMPOSE_PLACEMENT_KEY));
        assert!(KNOWN_MAILINER_STORAGE_KEYS.contains(&MAIL_LAYOUT_KEY));
        assert!(KNOWN_MAILINER_STORAGE_KEYS.contains(&SHORTCUT_MAP_KEY));
        assert!(KNOWN_MAILINER_STORAGE_KEYS.contains(&SAVED_SEARCHES_KEY));
        assert!(KNOWN_MAILINER_STORAGE_KEYS.contains(&PINNED_MESSAGES_KEY));
        assert!(KNOWN_MAILINER_STORAGE_KEYS.contains(&SNOOZED_MESSAGES_KEY));
        assert!(KNOWN_MAILINER_STORAGE_KEYS.contains(&MAIL_RULES_KEY));
        assert!(KNOWN_MAILINER_STORAGE_KEYS.contains(&MAIL_RULES_APPLIED_KEY));
        assert!(KNOWN_MAILINER_STORAGE_KEYS.contains(&FOLDER_WIDTH_KEY));
        assert!(KNOWN_MAILINER_STORAGE_KEYS.contains(&LIST_HEIGHT_KEY));
        assert!(KNOWN_MAILINER_STORAGE_KEYS.contains(&LIST_WIDTH_KEY));
        assert!(KNOWN_MAILINER_STORAGE_KEYS.contains(&E2E_SKIP_CONNECT_KEY));
    }

    #[test]
    fn clear_keys_helper_drops_only_mailiner_prefix() {
        let keys = [
            "mailiner.accounts.v1".into(),
            "unrelated".into(),
            "mailiner.cache.v1".into(),
            "other.app".into(),
            "mailiner.ui.lastMailbox.v1".into(),
            "Mailiner.not-ours".into(),
        ];
        let filtered = mailiner_keys_to_clear(keys.iter().map(String::as_str));
        assert_eq!(
            filtered,
            vec![
                "mailiner.accounts.v1",
                "mailiner.cache.v1",
                "mailiner.ui.lastMailbox.v1",
            ]
        );

        let mut removed = Vec::new();
        let n = remove_mailiner_keys(&keys, |k| {
            removed.push(k.to_string());
            Ok(())
        })
        .unwrap();
        assert_eq!(n, 3);
        assert_eq!(removed, filtered);
    }

    #[test]
    fn remove_mailiner_keys_propagates_error() {
        let keys = vec!["mailiner.accounts.v1".into(), "mailiner.cache.v1".into()];
        let mut seen = Vec::new();
        let err = remove_mailiner_keys(&keys, |k| {
            seen.push(k.to_string());
            Err(AccountStoreError::Unavailable)
        })
        .unwrap_err();
        assert_eq!(err, AccountStoreError::Unavailable);
        assert_eq!(seen, keys);
    }
}
