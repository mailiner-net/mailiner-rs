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
    /// Proxy `base_url` is empty, missing a scheme, or otherwise malformed.
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
///
/// `Debug` is derived; secret-bearing nested fields implement redacting `Debug`
/// so passwords and proxy tokens never appear in logs/panic messages.
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
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
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

impl fmt::Debug for ImapSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImapSettings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"***")
            .field("use_tls", &self.use_tls)
            .finish()
    }
}

/// How the SMTP session is wrapped in TLS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SmtpTlsMode {
    /// Implicit TLS (typically port 465). v1 send/test path.
    #[default]
    Implicit,
    /// STARTTLS after a plaintext greeting (typically port 587). Persisted, not spoken in v1.
    StartTls,
    /// No TLS. Persisted, refused at send/test.
    None,
}

impl SmtpTlsMode {
    /// `true` when the session is expected to be encrypted (implicit or STARTTLS).
    pub fn uses_tls(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Map a v1 `use_tls` + port pair. `true` + 587 → [`SmtpTlsMode::StartTls`].
pub fn tls_mode_from_legacy(use_tls: bool, port: u16) -> SmtpTlsMode {
    match (use_tls, port) {
        (false, _) => SmtpTlsMode::None,
        (true, 587) => SmtpTlsMode::StartTls,
        (true, _) => SmtpTlsMode::Implicit,
    }
}

/// Nominal default port for a TLS mode (form auto-fill only).
pub fn default_port_for_tls_mode(mode: SmtpTlsMode) -> u16 {
    match mode {
        SmtpTlsMode::Implicit => DEFAULT_SMTP_PORT,
        SmtpTlsMode::StartTls => 587,
        SmtpTlsMode::None => 25,
    }
}

/// SMTP connection settings (optional until send is implemented).
#[derive(Clone, Serialize, PartialEq, Eq)]
pub struct SmtpSettings {
    pub host: String,
    pub port: u16,
    pub username: String,
    /// If None, reuse IMAP password at send time.
    pub password: Option<String>,
    pub tls_mode: SmtpTlsMode,
    /// Dual-written for schema-1 readers. Always derived from [`Self::tls_mode`].
    pub use_tls: bool,
}

impl SmtpSettings {
    /// Construct settings and derive `use_tls` from `tls_mode`.
    pub fn new(
        host: String,
        port: u16,
        username: String,
        password: Option<String>,
        tls_mode: SmtpTlsMode,
    ) -> Self {
        Self {
            host,
            port,
            username,
            password,
            tls_mode,
            use_tls: tls_mode.uses_tls(),
        }
    }
}

impl<'de> Deserialize<'de> for SmtpSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            host: String,
            port: u16,
            username: String,
            password: Option<String>,
            /// Absent on v1 blobs. Must **not** use `#[serde(default)]` on the
            /// public field — missing key must stay distinguishable.
            tls_mode: Option<SmtpTlsMode>,
            use_tls: Option<bool>,
        }
        let raw = Raw::deserialize(deserializer)?;
        let tls_mode = match raw.tls_mode {
            Some(mode) => mode,
            None => tls_mode_from_legacy(raw.use_tls.unwrap_or(true), raw.port),
        };
        Ok(SmtpSettings::new(
            raw.host,
            raw.port,
            raw.username,
            raw.password,
            tls_mode,
        ))
    }
}

impl fmt::Debug for SmtpSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmtpSettings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .field("tls_mode", &self.tls_mode)
            .field("use_tls", &self.use_tls)
            .finish()
    }
}

/// WebSocket TCP-proxy settings used to reach IMAP from the browser.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
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

impl fmt::Debug for ProxySettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxySettings")
            .field("base_url", &self.base_url)
            .field("token", &"***")
            .field("remote_host", &self.remote_host)
            .field("remote_port", &self.remote_port)
            .finish()
    }
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
    /// - Scheme must be `ws` or `wss` (compared case-insensitively).
    /// - Reject `#fragment` on `base_url` (would swallow query params).
    pub fn websocket_url(&self, imap: &ImapSettings) -> Result<String, AccountConfigError> {
        let remote_host = self
            .remote_host
            .as_deref()
            .unwrap_or(imap.host.as_str())
            .trim();
        let remote_port = self.remote_port.unwrap_or(imap.port);
        self.websocket_url_for(remote_host, remote_port)
    }

    /// Build a proxy WebSocket URL for an arbitrary `remote=host:port`.
    ///
    /// SMTP must call this with `smtp.host` / `smtp.port` — never IMAP
    /// `remote_host` / `remote_port` overrides.
    pub fn websocket_url_for(
        &self,
        remote_host: &str,
        remote_port: u16,
    ) -> Result<String, AccountConfigError> {
        let remote_host = remote_host.trim();
        if remote_host.is_empty() {
            return Err(AccountConfigError::EmptyHost);
        }

        let mut base = self.base_url.trim().to_string();
        if base.is_empty() {
            return Err(AccountConfigError::InvalidProxyUrl(
                "base_url is empty".into(),
            ));
        }
        if base.contains('#') {
            return Err(AccountConfigError::InvalidProxyUrl(
                "base_url must not contain a fragment".into(),
            ));
        }

        let scheme = base
            .split_once("://")
            .map(|(s, _)| s)
            .ok_or_else(|| AccountConfigError::InvalidProxyUrl("missing scheme".into()))?;
        if !scheme.eq_ignore_ascii_case("ws") && !scheme.eq_ignore_ascii_case("wss") {
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

    /// True when `base_url` is `ws://` (case-insensitive) and the host is not loopback.
    ///
    /// Loopback hosts: `localhost`, `127.0.0.1`, `::1`. IPv6 authorities must be
    /// bracketed per RFC 3986 (e.g. `ws://[::1]:9400/proxy`).
    pub fn is_insecure_remote_ws(&self) -> bool {
        let base = self.base_url.trim();
        // Case-insensitive `ws://` prefix (5 chars).
        let rest = if base.len() >= 5 && base.as_bytes()[..5].eq_ignore_ascii_case(b"ws://") {
            &base[5..]
        } else {
            return false;
        };
        // Path starts after first `/`; host[:port] is before that.
        // Also stop at `?` / `#` if present without a path.
        let host_port = rest.split(['/', '?', '#']).next().unwrap_or(rest);
        // Drop userinfo if present (`user@host`).
        let host_port = host_port.rsplit('@').next().unwrap_or(host_port);
        let host = if let Some(inner) = host_port.strip_prefix('[') {
            // IPv6 literal: [::1]:port — RFC 3986 requires brackets.
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
    ///
    /// Secret exclusion is structural: UI [`crate::account::Account`] only has
    /// non-secret display fields. If that type gains fields, extend the mapping and tests.
    pub fn to_ui_account(&self) -> crate::account::Account {
        crate::account::Account {
            id: self.id.clone(),
            name: self.display_name.clone(),
            email: self.email.clone(),
            host: self.imap.host.clone(),
        }
    }

    /// Validate required fields that would break connect / URL building.
    ///
    /// PR1 checks hosts + proxy URL only. Fuller form validation (email, username,
    /// password non-empty) lands with the onboarding UI (PR5).
    pub fn validate(&self) -> Result<(), AccountConfigError> {
        if self.imap.host.trim().is_empty() {
            return Err(AccountConfigError::EmptyHost);
        }
        if let Some(ref smtp) = self.smtp
            && smtp.host.trim().is_empty()
        {
            return Err(AccountConfigError::EmptyHost);
        }
        // Ensure proxy URL can be built (also validates scheme / remote host).
        self.proxy.websocket_url(&self.imap)?;
        Ok(())
    }
}

/// Default SMTP port used when the form leaves port blank but host is set.
pub const DEFAULT_SMTP_PORT: u16 = 465;

/// Build optional SMTP settings from form field strings.
///
/// - All fields empty / default → `Ok(None)` (section unused).
/// - Any non-default field set without a host → error (partial fill requires host).
/// - Host set → `Ok(Some(...))`; blank port defaults to [`DEFAULT_SMTP_PORT`];
///   blank password → `None` (reuse IMAP password at send time later).
pub fn optional_smtp_from_fields(
    host: &str,
    port: &str,
    username: &str,
    password: &str,
    use_tls: bool,
) -> Result<Option<SmtpSettings>, String> {
    let host = host.trim();
    let username = username.trim();
    // Do not trim passwords (spaces may be intentional).
    let password_raw = password;
    let port_trim = port.trim();

    let port_is_default = port_trim.is_empty() || port_trim == DEFAULT_SMTP_PORT.to_string();
    let password_empty = password_raw.is_empty();
    let section_empty =
        host.is_empty() && username.is_empty() && password_empty && port_is_default && use_tls;

    if section_empty {
        return Ok(None);
    }

    if host.is_empty() {
        return Err("SMTP host is required when other SMTP fields are filled. \
             Clear SMTP fields to skip outbound settings."
            .into());
    }

    let port: u16 = if port_trim.is_empty() {
        DEFAULT_SMTP_PORT
    } else {
        port_trim
            .parse()
            .map_err(|_| "SMTP port must be a number between 1 and 65535.".to_string())?
    };
    if port == 0 {
        return Err("SMTP port must be a number between 1 and 65535.".into());
    }

    let password = if password_empty {
        None
    } else {
        Some(password_raw.to_string())
    };

    let tls_mode = tls_mode_from_legacy(use_tls, port);
    Ok(Some(SmtpSettings::new(
        host.to_string(),
        port,
        username.to_string(),
        password,
        tls_mode,
    )))
}

/// SMTP LOGIN username: SMTP username if set, else IMAP username, else account email.
pub fn smtp_username(config: &AccountConfig) -> String {
    if let Some(smtp) = &config.smtp {
        let u = smtp.username.trim();
        if !u.is_empty() {
            return smtp.username.clone();
        }
    }
    let imap_user = config.imap.username.trim();
    if !imap_user.is_empty() {
        return config.imap.username.clone();
    }
    config.email.clone()
}

/// SMTP password: explicit SMTP secret if non-empty, else IMAP password.
pub fn smtp_password(config: &AccountConfig) -> String {
    if let Some(smtp) = &config.smtp {
        if let Some(p) = &smtp.password {
            if !p.is_empty() {
                return p.clone();
            }
        }
    }
    config.imap.password.clone()
}

/// EHLO domain: account email domain, else `smtp.host`. Never `127.0.0.1`.
pub fn ehlo_domain(config: &AccountConfig) -> String {
    if let Some((_, domain)) = config.email.rsplit_once('@') {
        let d = domain.trim().to_ascii_lowercase();
        if !d.is_empty() && d.contains('.') && d != "localhost" && d != "127.0.0.1" {
            return d;
        }
    }
    config
        .smtp
        .as_ref()
        .map(|s| s.host.clone())
        .filter(|h| !h.trim().is_empty() && h != "127.0.0.1")
        .unwrap_or_else(|| "localhost".into())
}

/// Optional onboarding form prefill (debug / `dev-defaults` only).
///
/// **Never** used to auto-connect or write the account store. Empty-store
/// bootstrap always goes to onboarding; these values only seed form fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevFormPrefill {
    pub display_name: String,
    pub email: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_username: String,
    /// May be empty — never auto-connects.
    pub imap_password: String,
    pub proxy_base_url: String,
    pub proxy_token: String,
    pub remote_host: String,
    /// Empty string means “use IMAP port”.
    pub remote_port: String,
}

impl Default for DevFormPrefill {
    fn default() -> Self {
        Self {
            display_name: String::new(),
            email: String::new(),
            imap_host: String::new(),
            imap_port: 993,
            imap_username: String::new(),
            imap_password: String::new(),
            proxy_base_url: String::new(),
            proxy_token: String::new(),
            remote_host: String::new(),
            remote_port: String::new(),
        }
    }
}

/// Prefill values for the first-run form.
///
/// Under `debug_assertions` or feature `dev-defaults`, proxy defaults to a local
/// `ws-tcp-proxy` URL and optional `MAILINER_DEV_*` env vars (compile-time
/// `option_env!`) seed the remaining fields. In release without the feature,
/// returns empty fields with IMAP port `993` only.
///
/// Does **not** auto-connect and does **not** require a build-time password.
pub fn dev_form_prefill() -> DevFormPrefill {
    #[cfg(any(debug_assertions, feature = "dev-defaults"))]
    {
        let port: u16 = option_env!("MAILINER_DEV_IMAP_PORT")
            .and_then(|p| p.parse().ok())
            .unwrap_or(993);
        DevFormPrefill {
            display_name: option_env!("MAILINER_DEV_DISPLAY_NAME")
                .unwrap_or("")
                .to_string(),
            email: option_env!("MAILINER_DEV_EMAIL").unwrap_or("").to_string(),
            imap_host: option_env!("MAILINER_DEV_IMAP_HOST")
                .unwrap_or("")
                .to_string(),
            imap_port: port,
            imap_username: option_env!("MAILINER_DEV_IMAP_USER")
                .unwrap_or("")
                .to_string(),
            imap_password: option_env!("MAILINER_DEV_IMAP_PASSWORD")
                .unwrap_or("")
                .to_string(),
            proxy_base_url: option_env!("MAILINER_DEV_PROXY_URL")
                .unwrap_or("ws://localhost:9400/proxy")
                .to_string(),
            proxy_token: option_env!("MAILINER_DEV_PROXY_TOKEN")
                .unwrap_or("testtoken")
                .to_string(),
            remote_host: option_env!("MAILINER_DEV_PROXY_REMOTE_HOST")
                .unwrap_or("")
                .to_string(),
            remote_port: option_env!("MAILINER_DEV_PROXY_REMOTE_PORT")
                .unwrap_or("")
                .to_string(),
        }
    }
    #[cfg(not(any(debug_assertions, feature = "dev-defaults")))]
    {
        DevFormPrefill::default()
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
    fn websocket_url_accepts_uppercase_scheme() {
        let mut proxy = sample_proxy();
        proxy.base_url = "WS://localhost:9400/proxy".into();
        let url = proxy.websocket_url(&sample_imap()).unwrap();
        assert!(
            url.starts_with("WS://localhost:9400/proxy?token="),
            "url={url}"
        );

        proxy.base_url = "WSS://proxy.example/proxy".into();
        let url = proxy.websocket_url(&sample_imap()).unwrap();
        assert!(
            url.starts_with("WSS://proxy.example/proxy?token="),
            "url={url}"
        );
    }

    #[test]
    fn websocket_url_rejects_fragment() {
        let mut proxy = sample_proxy();
        proxy.base_url = "ws://localhost:9400/proxy#frag".into();
        match proxy.websocket_url(&sample_imap()) {
            Err(AccountConfigError::InvalidProxyUrl(msg)) => {
                assert!(msg.contains("fragment"), "msg={msg}");
            }
            other => panic!("expected InvalidProxyUrl, got {other:?}"),
        }
    }

    #[test]
    fn websocket_url_rejects_empty_base() {
        let mut proxy = sample_proxy();
        proxy.base_url = "  ".into();
        match proxy.websocket_url(&sample_imap()) {
            Err(AccountConfigError::InvalidProxyUrl(msg)) => {
                assert!(msg.contains("empty"), "msg={msg}");
            }
            other => panic!("expected InvalidProxyUrl, got {other:?}"),
        }
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
    fn is_insecure_remote_ws_loopback_ipv6_bracketed() {
        let mut proxy = sample_proxy();
        proxy.base_url = "ws://[::1]:9400/proxy".into();
        assert!(
            !proxy.is_insecure_remote_ws(),
            "bracketed ::1 should be loopback"
        );

        proxy.base_url = "ws://[::1]/proxy".into();
        assert!(!proxy.is_insecure_remote_ws());

        // Case-insensitive scheme
        proxy.base_url = "WS://[::1]:9400/proxy".into();
        assert!(!proxy.is_insecure_remote_ws());
    }

    #[test]
    fn is_insecure_remote_ws_case_insensitive_scheme() {
        let mut proxy = sample_proxy();
        proxy.base_url = "WS://proxy.example/proxy".into();
        assert!(proxy.is_insecure_remote_ws());

        proxy.base_url = "WS://localhost:9400/proxy".into();
        assert!(!proxy.is_insecure_remote_ws());
    }

    #[test]
    fn is_insecure_remote_ws_userinfo_localhost() {
        let mut proxy = sample_proxy();
        proxy.base_url = "ws://user@localhost:9400/proxy".into();
        assert!(!proxy.is_insecure_remote_ws());
    }

    #[test]
    fn debug_redacts_passwords_and_token() {
        let mut config = sample_config();
        config.smtp = Some(SmtpSettings::new(
            "smtp.example.com".into(),
            465,
            "user@example.com".into(),
            Some("smtp-secret".into()),
            SmtpTlsMode::Implicit,
        ));
        let dbg = format!("{config:?}");
        assert!(!dbg.contains("s3cret"), "imap password leaked: {dbg}");
        assert!(!dbg.contains("smtp-secret"), "smtp password leaked: {dbg}");
        assert!(!dbg.contains("testtoken"), "proxy token leaked: {dbg}");
        assert!(
            dbg.contains("password: \"***\""),
            "missing redaction: {dbg}"
        );
        assert!(
            dbg.contains("token: \"***\""),
            "missing token redaction: {dbg}"
        );
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
        config.smtp = Some(SmtpSettings::new(
            "smtp.example.com".into(),
            465,
            "user@example.com".into(),
            None,
            SmtpTlsMode::Implicit,
        ));
        let json = serde_json::to_string(&config).unwrap();
        let back: AccountConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.imap.password, "s3cret");
        assert!(back.smtp.as_ref().unwrap().password.is_none());
    }

    #[test]
    fn to_ui_account_strips_secrets() {
        // Secret exclusion is structural: UI Account has only non-secret display fields.
        // This test documents the mapping; the type system prevents secret fields.
        let config = sample_config();
        let ui = config.to_ui_account();
        assert_eq!(ui.id, config.id);
        assert_eq!(ui.name, "Work");
        assert_eq!(ui.email, "user@example.com");
        assert_eq!(ui.host, config.imap.host);
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
    fn validate_success_and_empty_smtp_host() {
        let config = sample_config();
        assert!(config.validate().is_ok());

        let mut config = sample_config();
        config.smtp = Some(SmtpSettings::new(
            "".into(),
            465,
            "u".into(),
            None,
            SmtpTlsMode::Implicit,
        ));
        assert_eq!(
            config.validate().unwrap_err(),
            AccountConfigError::EmptyHost
        );
    }

    #[test]
    fn schema_version_is_one() {
        assert_eq!(ACCOUNT_STORE_SCHEMA_VERSION, 1);
    }

    #[test]
    fn optional_smtp_empty_section_is_none() {
        assert_eq!(
            optional_smtp_from_fields("", "", "", "", true).unwrap(),
            None
        );
        assert_eq!(
            optional_smtp_from_fields("", "465", "", "", true).unwrap(),
            None
        );
        assert_eq!(
            optional_smtp_from_fields("  ", "465", "  ", "", true).unwrap(),
            None
        );
    }

    #[test]
    fn optional_smtp_partial_requires_host() {
        let err = optional_smtp_from_fields("", "465", "user", "", true).unwrap_err();
        assert!(err.contains("SMTP host"), "err={err}");

        let err = optional_smtp_from_fields("", "", "", "secret", true).unwrap_err();
        assert!(err.contains("SMTP host"), "err={err}");

        let err = optional_smtp_from_fields("", "587", "", "", true).unwrap_err();
        assert!(err.contains("SMTP host"), "err={err}");

        let err = optional_smtp_from_fields("", "", "", "", false).unwrap_err();
        assert!(err.contains("SMTP host"), "err={err}");
    }

    #[test]
    fn optional_smtp_with_host_defaults_and_password() {
        let smtp = optional_smtp_from_fields("smtp.example.com", "", "u@ex.com", "", true)
            .unwrap()
            .expect("Some");
        assert_eq!(smtp.host, "smtp.example.com");
        assert_eq!(smtp.port, DEFAULT_SMTP_PORT);
        assert_eq!(smtp.username, "u@ex.com");
        assert!(smtp.password.is_none());
        assert!(smtp.use_tls);
        assert_eq!(smtp.tls_mode, SmtpTlsMode::Implicit);

        let smtp = optional_smtp_from_fields("smtp.example.com", "587", "u", "pw", false)
            .unwrap()
            .expect("Some");
        assert_eq!(smtp.port, 587);
        assert_eq!(smtp.password.as_deref(), Some("pw"));
        assert!(!smtp.use_tls);
        assert_eq!(smtp.tls_mode, SmtpTlsMode::None);

        let smtp = optional_smtp_from_fields("smtp.example.com", "587", "", "", true)
            .unwrap()
            .expect("Some");
        assert_eq!(smtp.tls_mode, SmtpTlsMode::StartTls);
        assert!(smtp.use_tls);
    }

    #[test]
    fn v1_json_587_use_tls_true_becomes_starttls() {
        let json = r#"{
            "host": "smtp.example.com",
            "port": 587,
            "username": "u",
            "password": null,
            "use_tls": true
        }"#;
        let smtp: SmtpSettings = serde_json::from_str(json).unwrap();
        assert_eq!(smtp.tls_mode, SmtpTlsMode::StartTls);
        assert!(smtp.use_tls);
        let out = serde_json::to_string(&smtp).unwrap();
        assert!(out.contains("\"tls_mode\":\"start_tls\""));
        assert!(out.contains("\"use_tls\":true"));
    }

    #[test]
    fn websocket_url_for_smtp_ignores_imap_overrides() {
        let mut proxy = sample_proxy();
        proxy.remote_host = Some("imap-override.example".into());
        proxy.remote_port = Some(993);
        let url = proxy
            .websocket_url_for("smtp.example.com", 465)
            .unwrap();
        assert!(url.contains("remote=smtp.example.com:465"), "{url}");
        assert!(!url.contains("imap-override"));
    }

    #[test]
    fn credential_helpers_fallback() {
        let mut config = sample_config();
        config.smtp = Some(SmtpSettings::new(
            "smtp.example.com".into(),
            465,
            "".into(),
            None,
            SmtpTlsMode::Implicit,
        ));
        assert_eq!(smtp_username(&config), "user@example.com");
        assert_eq!(smtp_password(&config), "s3cret");
        assert_eq!(ehlo_domain(&config), "example.com");
    }

    #[test]
    fn optional_smtp_rejects_invalid_port() {
        let err = optional_smtp_from_fields("smtp.example.com", "0", "", "", true).unwrap_err();
        assert!(err.contains("port"), "err={err}");
        let err = optional_smtp_from_fields("smtp.example.com", "nope", "", "", true).unwrap_err();
        assert!(err.contains("port"), "err={err}");
    }
}
