//! Account configuration model (connection settings + secrets).
//!
//! This is separate from `mailiner_core::models::Account` (mail-cache metadata)
//! and from the thin UI `account::Account` display type.

use chrono::{DateTime, Utc};
use mailiner_core::ids::AccountId;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Schema version for migrations of persisted account-store blobs.
pub const ACCOUNT_STORE_SCHEMA_VERSION: u32 = 1;

/// Encode query component values per RFC 3986 (unreserved left as-is).
/// Unreserved = ALPHA / DIGIT / "-" / "." / "_" / "~"
const QUERY_VALUE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Errors produced while validating or building account config URLs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountConfigError {
    /// IMAP or proxy remote host is empty.
    EmptyHost,
    /// Proxy `base_url` is empty or missing a scheme.
    InvalidProxyUrl(String),
    /// Proxy scheme is not `ws` or `wss`.
    InvalidProxyScheme(String),
}

impl fmt::Display for AccountConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyHost => write!(f, "host must not be empty"),
            Self::InvalidProxyUrl(msg) => write!(f, "invalid proxy URL: {msg}"),
            Self::InvalidProxyScheme(scheme) => {
                write!(f, "proxy scheme must be ws or wss, got {scheme:?}")
            }
        }
    }
}

impl std::error::Error for AccountConfigError {}

/// Full account configuration including connection secrets.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountConfig {
    pub id: AccountId,
    /// User-visible label (e.g. "Work", "Valhalla").
    pub display_name: String,
    /// Primary mailbox address (From: / identity).
    pub email: String,
    pub imap: ImapSettings,
    /// Optional until send is implemented; persisted for forward-compat.
    pub smtp: Option<SmtpSettings>,
    pub proxy: ProxySettings,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// IMAP connection settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImapSettings {
    /// Hostname for TLS SNI and display (e.g. "imap.example.com").
    pub host: String,
    /// Nominal IMAP port (993). Used to build proxy `remote=` when
    /// `ProxySettings.remote_host/port` are unset; not used for direct TCP in WASM.
    pub port: u16,
    /// LOGIN username (often same as email).
    pub username: String,
    /// Password / app password.
    pub password: String,
    /// Reserved; v1 only supports implicit TLS over the proxy stream. Default true.
    pub use_tls: bool,
}

/// SMTP connection settings (optional until send is implemented).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SmtpSettings {
    pub host: String,
    pub port: u16,
    pub username: String,
    /// If None, reuse IMAP password at send time.
    pub password: Option<String>,
    pub use_tls: bool,
}

/// WebSocket TCP-proxy settings used to reach IMAP from the browser.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProxySettings {
    /// e.g. "ws://localhost:9400/proxy" or "wss://proxy.example/proxy"
    pub base_url: String,
    /// Shared secret for the proxy (`token` query param). Empty if proxy open.
    pub token: String,
    /// Override remote host for proxy (defaults to `imap.host`).
    pub remote_host: Option<String>,
    /// Override remote port for proxy (defaults to `imap.port`).
    pub remote_port: Option<u16>,
}

impl ProxySettings {
    /// Build full WebSocket URL for `WebSocketStream`.
    ///
    /// Encoding policy:
    /// - Percent-encode `token` and remote host (non-unreserved chars).
    /// - `remote` value is `{encoded_host}:{port}` with port as decimal digits.
    /// - Reject empty IMAP/remote host.
    /// - Do not append a second `?` if `base_url` already has a query; use `&`.
    /// - Trim a single trailing `/` on the path when there is no query string.
    /// - Scheme must be `ws` or `wss`.
    pub fn websocket_url(&self, imap: &ImapSettings) -> Result<String, AccountConfigError> {
        let remote_host = self
            .remote_host
            .as_deref()
            .unwrap_or(imap.host.as_str())
            .trim();
        if remote_host.is_empty() {
            return Err(AccountConfigError::EmptyHost);
        }
        let remote_port = self.remote_port.unwrap_or(imap.port);

        let mut base = self.base_url.trim().to_string();
        if base.is_empty() {
            return Err(AccountConfigError::InvalidProxyUrl(
                "base_url is empty".into(),
            ));
        }

        let scheme = base
            .split_once("://")
            .map(|(s, _)| s)
            .ok_or_else(|| AccountConfigError::InvalidProxyUrl("missing scheme".into()))?;
        if scheme != "ws" && scheme != "wss" {
            return Err(AccountConfigError::InvalidProxyScheme(scheme.to_string()));
        }

        // Trim a single trailing slash only when there is no existing query.
        if !base.contains('?') && base.ends_with('/') {
            base.pop();
        }

        let encoded_token = utf8_percent_encode(&self.token, QUERY_VALUE).to_string();
        let encoded_host = utf8_percent_encode(remote_host, QUERY_VALUE).to_string();
        let remote = format!("{encoded_host}:{remote_port}");

        let sep = if base.contains('?') { '&' } else { '?' };
        Ok(format!("{base}{sep}token={encoded_token}&remote={remote}"))
    }

    /// True when `base_url` is `ws://` and the host is not a loopback address.
    pub fn is_insecure_remote_ws(&self) -> bool {
        let base = self.base_url.trim();
        let Some(rest) = base.strip_prefix("ws://") else {
            return false;
        };
        // Path starts after first `/`; host[:port] is before that.
        let host_port = rest.split('/').next().unwrap_or(rest);
        // Drop userinfo if present (`user@host`).
        let host_port = host_port.rsplit('@').next().unwrap_or(host_port);
        let host = if let Some(inner) = host_port.strip_prefix('[') {
            // IPv6 literal: [::1]:port
            inner.split(']').next().unwrap_or(inner)
        } else {
            host_port.split(':').next().unwrap_or(host_port)
        };
        let host = host.to_ascii_lowercase();
        !(host == "localhost" || host == "127.0.0.1" || host == "::1")
    }
}

impl AccountConfig {
    /// Map to the thin UI account type (no secrets).
    pub fn to_ui_account(&self) -> crate::account::Account {
        crate::account::Account {
            id: self.id.clone(),
            name: self.display_name.clone(),
            email: self.email.clone(),
        }
    }

    /// Validate required fields that would break connect / URL building.
    pub fn validate(&self) -> Result<(), AccountConfigError> {
        if self.imap.host.trim().is_empty() {
            return Err(AccountConfigError::EmptyHost);
        }
        if let Some(ref smtp) = self.smtp {
            if smtp.host.trim().is_empty() {
                return Err(AccountConfigError::EmptyHost);
            }
        }
        // Ensure proxy URL can be built (also validates scheme / remote host).
        self.proxy.websocket_url(&self.imap)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample_imap() -> ImapSettings {
        ImapSettings {
            host: "imap.example.com".into(),
            port: 993,
            username: "user@example.com".into(),
            password: "s3cret".into(),
            use_tls: true,
        }
    }

    fn sample_proxy() -> ProxySettings {
        ProxySettings {
            base_url: "ws://localhost:9400/proxy".into(),
            token: "testtoken".into(),
            remote_host: None,
            remote_port: None,
        }
    }

    fn sample_config() -> AccountConfig {
        let ts = Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap();
        AccountConfig {
            id: AccountId::new("550e8400-e29b-41d4-a716-446655440000"),
            display_name: "Work".into(),
            email: "user@example.com".into(),
            imap: sample_imap(),
            smtp: None,
            proxy: sample_proxy(),
            created_at: ts,
            updated_at: ts,
        }
    }

    #[test]
    fn websocket_url_basic() {
        let proxy = sample_proxy();
        let imap = sample_imap();
        let url = proxy.websocket_url(&imap).unwrap();
        assert_eq!(
            url,
            "ws://localhost:9400/proxy?token=testtoken&remote=imap.example.com:993"
        );
    }

    #[test]
    fn websocket_url_encodes_token_reserved_chars() {
        let mut proxy = sample_proxy();
        // Space, ampersand, equals, slash, plus — must be percent-encoded.
        proxy.token = "a b&c=d/e+f".into();
        let url = proxy.websocket_url(&sample_imap()).unwrap();
        assert!(
            url.contains("token=a%20b%26c%3Dd%2Fe%2Bf"),
            "token not encoded: {url}"
        );
        assert!(!url.contains("token=a b"), "raw space leaked: {url}");
    }

    #[test]
    fn websocket_url_encodes_host_reserved_chars() {
        let mut proxy = sample_proxy();
        proxy.remote_host = Some("host with space.example".into());
        proxy.remote_port = Some(993);
        let url = proxy.websocket_url(&sample_imap()).unwrap();
        assert!(
            url.contains("remote=host%20with%20space.example:993"),
            "host not encoded: {url}"
        );
    }

    #[test]
    fn websocket_url_uses_remote_overrides() {
        let mut proxy = sample_proxy();
        proxy.remote_host = Some("other.example".into());
        proxy.remote_port = Some(143);
        let url = proxy.websocket_url(&sample_imap()).unwrap();
        assert!(url.ends_with("remote=other.example:143"), "url={url}");
    }

    #[test]
    fn websocket_url_appends_with_ampersand_when_query_present() {
        let mut proxy = sample_proxy();
        proxy.base_url = "ws://localhost:9400/proxy?foo=1".into();
        let url = proxy.websocket_url(&sample_imap()).unwrap();
        assert!(
            url.starts_with("ws://localhost:9400/proxy?foo=1&token="),
            "url={url}"
        );
        assert!(!url.contains("?token="), "second ? introduced: {url}");
    }

    #[test]
    fn websocket_url_trims_trailing_slash() {
        let mut proxy = sample_proxy();
        proxy.base_url = "ws://localhost:9400/proxy/".into();
        let url = proxy.websocket_url(&sample_imap()).unwrap();
        assert!(url.starts_with("ws://localhost:9400/proxy?"), "url={url}");
    }

    #[test]
    fn websocket_url_rejects_empty_host() {
        let proxy = sample_proxy();
        let mut imap = sample_imap();
        imap.host = "  ".into();
        assert_eq!(
            proxy.websocket_url(&imap).unwrap_err(),
            AccountConfigError::EmptyHost
        );
    }

    #[test]
    fn websocket_url_rejects_http_scheme() {
        let mut proxy = sample_proxy();
        proxy.base_url = "http://localhost:9400/proxy".into();
        match proxy.websocket_url(&sample_imap()) {
            Err(AccountConfigError::InvalidProxyScheme(s)) => assert_eq!(s, "http"),
            other => panic!("expected InvalidProxyScheme, got {other:?}"),
        }
    }

    #[test]
    fn websocket_url_accepts_wss() {
        let mut proxy = sample_proxy();
        proxy.base_url = "wss://proxy.example/proxy".into();
        let url = proxy.websocket_url(&sample_imap()).unwrap();
        assert!(url.starts_with("wss://proxy.example/proxy?token="));
    }

    #[test]
    fn is_insecure_remote_ws_detects_non_loopback() {
        let mut proxy = sample_proxy();
        proxy.base_url = "ws://proxy.example/proxy".into();
        assert!(proxy.is_insecure_remote_ws());

        proxy.base_url = "ws://localhost:9400/proxy".into();
        assert!(!proxy.is_insecure_remote_ws());

        proxy.base_url = "ws://127.0.0.1:9400/proxy".into();
        assert!(!proxy.is_insecure_remote_ws());

        proxy.base_url = "wss://proxy.example/proxy".into();
        assert!(!proxy.is_insecure_remote_ws());
    }

    #[test]
    fn serde_roundtrip_account_id_and_datetime() {
        let config = sample_config();
        let json = serde_json::to_string(&config).expect("serialize");
        // AccountId is a newtype string
        assert!(
            json.contains("\"id\":\"550e8400-e29b-41d4-a716-446655440000\""),
            "id format: {json}"
        );
        // DateTime<Utc> as RFC 3339
        assert!(
            json.contains("2024-06-15T12:00:00Z") || json.contains("2024-06-15T12:00:00+00:00"),
            "datetime format: {json}"
        );

        let back: AccountConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, config);
        assert_eq!(back.id.as_str(), "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(
            back.created_at,
            Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap()
        );
    }

    #[test]
    fn serde_preserves_password_and_optional_smtp() {
        let mut config = sample_config();
        config.smtp = Some(SmtpSettings {
            host: "smtp.example.com".into(),
            port: 465,
            username: "user@example.com".into(),
            password: None,
            use_tls: true,
        });
        let json = serde_json::to_string(&config).unwrap();
        let back: AccountConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.imap.password, "s3cret");
        assert!(back.smtp.as_ref().unwrap().password.is_none());
    }

    #[test]
    fn to_ui_account_strips_secrets() {
        let config = sample_config();
        let ui = config.to_ui_account();
        assert_eq!(ui.id, config.id);
        assert_eq!(ui.name, "Work");
        assert_eq!(ui.email, "user@example.com");
    }

    #[test]
    fn validate_rejects_empty_imap_host() {
        let mut config = sample_config();
        config.imap.host.clear();
        assert_eq!(
            config.validate().unwrap_err(),
            AccountConfigError::EmptyHost
        );
    }

    #[test]
    fn schema_version_is_one() {
        assert_eq!(ACCOUNT_STORE_SCHEMA_VERSION, 1);
    }
}
