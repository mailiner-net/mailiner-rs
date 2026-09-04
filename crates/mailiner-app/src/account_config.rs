//! Account configuration model (connection settings + secrets).
//!
//! This is separate from the thin UI `account::Account` display type.

use chrono::{DateTime, Utc};
use mailiner_core::ids::AccountId;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
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

/// Extra From identity (name + email alias) on an account.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AccountIdentity {
    /// Display name shown in From. Empty is omitted on send.
    #[serde(default)]
    pub display_name: String,
    /// Mailbox address.
    pub email: String,
}

impl AccountIdentity {
    /// Construct a name + email identity.
    pub fn new(display_name: impl Into<String>, email: impl Into<String>) -> Self {
        Self {
            display_name: display_name.into(),
            email: email.into(),
        }
    }

    /// Trimmed name + email, or `None` if both are empty.
    pub fn trimmed(&self) -> Option<Self> {
        let display_name = self.display_name.trim().to_string();
        let email = self.email.trim().to_string();
        if display_name.is_empty() && email.is_empty() {
            None
        } else {
            Some(Self {
                display_name,
                email,
            })
        }
    }
}

/// Primary first, then extras (duplicates of the primary are skipped).
pub fn account_identities(
    name: &str,
    email: &str,
    extras: &[AccountIdentity],
) -> Vec<AccountIdentity> {
    let mut out = Vec::with_capacity(extras.len() + 1);
    out.push(AccountIdentity::new(name, email));
    for extra in extras {
        if identity_same(
            extra.display_name.trim(),
            extra.email.trim(),
            name.trim(),
            email.trim(),
        ) {
            continue;
        }
        out.push(extra.clone());
    }
    out
}

fn identity_same(name_a: &str, email_a: &str, name_b: &str, email_b: &str) -> bool {
    email_a.eq_ignore_ascii_case(email_b) && name_a.eq_ignore_ascii_case(name_b)
}

/// Skip blank rows, reject incomplete / invalid emails, drop duplicates.
pub fn normalize_identities(
    rows: impl IntoIterator<Item = AccountIdentity>,
    primary_name: &str,
    primary_email: &str,
) -> Result<Vec<AccountIdentity>, String> {
    let primary_name = primary_name.trim();
    let primary_email = primary_email.trim();
    let mut out = Vec::new();
    for (i, raw) in rows.into_iter().enumerate() {
        let Some(id) = raw.trimmed() else {
            continue;
        };
        if id.email.is_empty() {
            return Err(format!(
                "Identity {} needs an email address (or clear the name).",
                i + 1
            ));
        }
        if !id.email.contains('@') {
            return Err(format!(
                "Identity {} email must look like an address (user@example.com).",
                i + 1
            ));
        }
        if identity_same(&id.display_name, &id.email, primary_name, primary_email) {
            continue;
        }
        if out.iter().any(|existing: &AccountIdentity| {
            identity_same(
                &existing.display_name,
                &existing.email,
                &id.display_name,
                &id.email,
            )
        }) {
            continue;
        }
        out.push(id);
    }
    Ok(out)
}

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
    /// Extra From identities (name + email aliases). Primary is `display_name` + `email`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identities: Vec<AccountIdentity>,
    /// Optional plain-text signature appended when opening a compose draft.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    pub imap: ImapSettings,
    /// Optional until send is implemented; persisted for forward-compat.
    pub smtp: Option<SmtpSettings>,
    pub proxy: ProxySettings,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// How the IMAP session is wrapped in TLS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImapTlsMode {
    /// Implicit TLS (typically port 993).
    #[default]
    Implicit,
    /// STARTTLS after a plaintext greeting (typically port 143).
    StartTls,
    /// No TLS. LOGIN and mail travel in the clear (including through the proxy).
    None,
}

impl ImapTlsMode {
    /// `true` when the session is expected to be encrypted (implicit or STARTTLS).
    pub fn uses_tls(self) -> bool {
        !matches!(self, Self::None)
    }

    /// Form `<select>` value (`implicit` / `start_tls` / `none`).
    pub fn as_form_value(self) -> &'static str {
        match self {
            Self::Implicit => "implicit",
            Self::StartTls => "start_tls",
            Self::None => "none",
        }
    }

    /// Parse a form `<select>` value; unknown → Implicit.
    pub fn from_form_value(value: &str) -> Self {
        match value {
            "start_tls" => Self::StartTls,
            "none" => Self::None,
            _ => Self::Implicit,
        }
    }

    /// Connector connect path for this persisted mode.
    pub fn to_connector(self) -> mailiner_imap_connector::ImapTlsMode {
        match self {
            Self::Implicit => mailiner_imap_connector::ImapTlsMode::Implicit,
            Self::StartTls => mailiner_imap_connector::ImapTlsMode::StartTls,
            Self::None => mailiner_imap_connector::ImapTlsMode::None,
        }
    }
}

/// Map a v1 `use_tls` + port pair. `true` + 143 → [`ImapTlsMode::StartTls`].
pub fn imap_tls_mode_from_legacy(use_tls: bool, port: u16) -> ImapTlsMode {
    match (use_tls, port) {
        (false, _) => ImapTlsMode::None,
        (true, 143) => ImapTlsMode::StartTls,
        (true, _) => ImapTlsMode::Implicit,
    }
}

/// Nominal default port for an IMAP TLS mode (form auto-fill only).
pub fn default_port_for_imap_tls_mode(mode: ImapTlsMode) -> u16 {
    match mode {
        ImapTlsMode::Implicit => DEFAULT_IMAP_PORT,
        ImapTlsMode::StartTls | ImapTlsMode::None => 143,
    }
}

/// Rewrite IMAP port when TLS mode changes, only if it is still the previous default.
pub fn port_for_imap_tls_mode_change(port: &str, from: ImapTlsMode, to: ImapTlsMode) -> String {
    let trimmed = port.trim();
    let prev_default = default_port_for_imap_tls_mode(from).to_string();
    if trimmed.is_empty() || trimmed == prev_default {
        default_port_for_imap_tls_mode(to).to_string()
    } else {
        trimmed.to_string()
    }
}

/// IMAP connection settings.
#[derive(Clone, Serialize, PartialEq, Eq)]
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
    pub tls_mode: ImapTlsMode,
    /// Dual-written for schema-1 readers. Always derived from [`Self::tls_mode`].
    pub use_tls: bool,
}

impl ImapSettings {
    /// Construct settings and derive `use_tls` from `tls_mode`.
    pub fn new(
        host: String,
        port: u16,
        username: String,
        password: String,
        tls_mode: ImapTlsMode,
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

impl<'de> Deserialize<'de> for ImapSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            host: String,
            port: u16,
            username: String,
            password: String,
            /// Absent on v1 blobs. Must **not** use `#[serde(default)]` on the
            /// public field — missing key must stay distinguishable.
            tls_mode: Option<ImapTlsMode>,
            use_tls: Option<bool>,
        }
        let raw = Raw::deserialize(deserializer)?;
        let tls_mode = match raw.tls_mode {
            Some(mode) => mode,
            None => imap_tls_mode_from_legacy(raw.use_tls.unwrap_or(true), raw.port),
        };
        Ok(ImapSettings::new(
            raw.host,
            raw.port,
            raw.username,
            raw.password,
            tls_mode,
        ))
    }
}

impl fmt::Debug for ImapSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImapSettings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"***")
            .field("tls_mode", &self.tls_mode)
            .field("use_tls", &self.use_tls)
            .finish()
    }
}

/// How the SMTP session is wrapped in TLS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SmtpTlsMode {
    /// Implicit TLS (typically port 465).
    #[default]
    Implicit,
    /// STARTTLS after a plaintext greeting (typically port 587).
    StartTls,
    /// No TLS. AUTH and DATA travel in the clear (including through the proxy).
    None,
}

impl SmtpTlsMode {
    /// `true` when the session is expected to be encrypted (implicit or STARTTLS).
    pub fn uses_tls(self) -> bool {
        !matches!(self, Self::None)
    }

    /// Form `<select>` value (`implicit` / `start_tls` / `none`).
    pub fn as_form_value(self) -> &'static str {
        match self {
            Self::Implicit => "implicit",
            Self::StartTls => "start_tls",
            Self::None => "none",
        }
    }

    /// Parse a form `<select>` value; unknown → Implicit.
    pub fn from_form_value(value: &str) -> Self {
        match value {
            "start_tls" => Self::StartTls,
            "none" => Self::None,
            _ => Self::Implicit,
        }
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

/// Rewrite SMTP port when TLS mode changes, only if it is still the previous default.
pub fn port_for_tls_mode_change(port: &str, from: SmtpTlsMode, to: SmtpTlsMode) -> String {
    let trimmed = port.trim();
    let prev_default = default_port_for_tls_mode(from).to_string();
    if trimmed.is_empty() || trimmed == prev_default {
        default_port_for_tls_mode(to).to_string()
    } else {
        trimmed.to_string()
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
    /// Override proxy dial host (defaults to [`Self::host`]). TLS SNI stays `host`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_host: Option<String>,
    /// Override proxy dial port (defaults to [`Self::port`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_port: Option<u16>,
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
            remote_host: None,
            remote_port: None,
        }
    }

    /// Host the proxy should dial. TLS SNI / EHLO stay on [`Self::host`].
    pub fn dial_host(&self) -> &str {
        self.remote_host
            .as_deref()
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .unwrap_or(self.host.as_str())
    }

    /// Port the proxy should dial. Defaults to [`Self::port`].
    pub fn dial_port(&self) -> u16 {
        self.remote_port.unwrap_or(self.port)
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
            #[serde(default)]
            remote_host: Option<String>,
            #[serde(default)]
            remote_port: Option<u16>,
        }
        let raw = Raw::deserialize(deserializer)?;
        let tls_mode = match raw.tls_mode {
            Some(mode) => mode,
            None => tls_mode_from_legacy(raw.use_tls.unwrap_or(true), raw.port),
        };
        let mut smtp = SmtpSettings::new(raw.host, raw.port, raw.username, raw.password, tls_mode);
        smtp.remote_host = raw
            .remote_host
            .map(|h| h.trim().to_string())
            .filter(|h| !h.is_empty());
        smtp.remote_port = raw.remote_port;
        Ok(smtp)
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
            .field("remote_host", &self.remote_host)
            .field("remote_port", &self.remote_port)
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
    /// SMTP must call this with the SMTP dial target ([`SmtpSettings::dial_host`]
    /// / [`SmtpSettings::dial_port`]) — never IMAP `remote_host` / `remote_port`.
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

    /// SMTP proxy URL. Dial uses `smtp.remote_*` when set; TLS SNI stays `smtp.host`.
    pub fn websocket_url_for_smtp(
        &self,
        smtp: &SmtpSettings,
    ) -> Result<String, AccountConfigError> {
        self.websocket_url_for(smtp.dial_host(), smtp.dial_port())
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
            signature: self.signature.clone(),
            identities: self.identities.clone(),
        }
    }

    /// Primary From identity (`display_name` + `email`).
    pub fn primary_identity(&self) -> AccountIdentity {
        AccountIdentity::new(self.display_name.clone(), self.email.clone())
    }

    /// Primary first, then extras.
    pub fn all_identities(&self) -> Vec<AccountIdentity> {
        account_identities(&self.display_name, &self.email, &self.identities)
    }

    /// Replace extras after [`normalize_identities`].
    pub fn with_identities(
        mut self,
        rows: impl IntoIterator<Item = AccountIdentity>,
    ) -> Result<Self, String> {
        self.identities = normalize_identities(rows, &self.display_name, &self.email)?;
        Ok(self)
    }

    /// Clear IMAP/SMTP passwords and the proxy token in place.
    ///
    /// Also strips a `token` query param and `userinfo` from `proxy.base_url`
    /// so a public export cannot leak a credential that was baked into the URL.
    pub fn redact_secrets(&mut self) {
        self.imap.password.clear();
        if let Some(smtp) = self.smtp.as_mut() {
            smtp.password = None;
        }
        self.proxy.token.clear();
        self.proxy.base_url = strip_embedded_proxy_secrets(&self.proxy.base_url);
    }

    /// Clone with IMAP/SMTP passwords and the proxy token removed.
    pub fn without_secrets(&self) -> Self {
        let mut out = self.clone();
        out.redact_secrets();
        out
    }

    /// Validate required fields that would break connect / URL building.
    ///
    /// PR1 checks hosts + proxy URL only. Fuller form validation (email, username,
    /// password non-empty) lands with the onboarding UI (PR5).
    pub fn validate(&self) -> Result<(), AccountConfigError> {
        if self.imap.host.trim().is_empty() {
            return Err(AccountConfigError::EmptyHost);
        }
        if let Some(ref smtp) = self.smtp {
            if smtp.host.trim().is_empty() {
                return Err(AccountConfigError::EmptyHost);
            }
            self.proxy.websocket_url_for_smtp(smtp)?;
        }
        // Ensure proxy URL can be built (also validates scheme / remote host).
        self.proxy.websocket_url(&self.imap)?;
        Ok(())
    }
}

/// Drop `userinfo` and a `token` query param from a proxy `base_url`.
fn strip_embedded_proxy_secrets(url: &str) -> String {
    let (without_frag, frag) = match url.split_once('#') {
        Some((head, tail)) => (head, Some(tail)),
        None => (url, None),
    };
    let (base, query) = match without_frag.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (without_frag, None),
    };
    let mut base = strip_url_userinfo(base);
    if let Some(query) = query {
        let kept: Vec<&str> = query
            .split('&')
            .filter(|pair| {
                let key = pair.split('=').next().unwrap_or("");
                !percent_decode_str(key)
                    .decode_utf8()
                    .map(|key| key.eq_ignore_ascii_case("token"))
                    .unwrap_or(false)
            })
            .collect();
        if !kept.is_empty() {
            base.push('?');
            base.push_str(&kept.join("&"));
        }
    }
    if let Some(frag) = frag {
        base.push('#');
        base.push_str(frag);
    }
    base
}

fn strip_url_userinfo(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    let authority_end = rest
        .find(|ch| matches!(ch, '/' | '?' | '#'))
        .unwrap_or(rest.len());
    let (authority, suffix) = rest.split_at(authority_end);
    match authority.rsplit_once('@') {
        Some((_, host)) => format!("{scheme}://{host}{suffix}"),
        None => url.to_string(),
    }
}

/// Trim surrounding whitespace; empty / whitespace-only → `None`.
pub fn normalize_signature(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Default IMAP port used when the form leaves port blank (implicit TLS).
pub const DEFAULT_IMAP_PORT: u16 = 993;

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
    let port_trim = port.trim();
    let parsed_port: u16 = if port_trim.is_empty() {
        DEFAULT_SMTP_PORT
    } else {
        port_trim.parse().unwrap_or(DEFAULT_SMTP_PORT)
    };
    let tls_mode = tls_mode_from_legacy(use_tls, parsed_port);
    optional_smtp_from_tls_mode(host, port, username, password, tls_mode)
}

/// Like [`optional_smtp_from_fields`] but with an explicit TLS mode (settings `<select>`).
pub fn optional_smtp_from_tls_mode(
    host: &str,
    port: &str,
    username: &str,
    password: &str,
    tls_mode: SmtpTlsMode,
) -> Result<Option<SmtpSettings>, String> {
    let host = host.trim();
    let username = username.trim();
    let password_raw = password;
    let port_trim = port.trim();
    let default_port = default_port_for_tls_mode(tls_mode);
    let port_is_default = port_trim.is_empty() || port_trim == default_port.to_string();
    let password_empty = password_raw.is_empty();
    let section_empty = host.is_empty()
        && username.is_empty()
        && password_empty
        && port_is_default
        && tls_mode == SmtpTlsMode::Implicit;

    if section_empty {
        return Ok(None);
    }
    if host.is_empty() {
        return Err("SMTP host is required when other SMTP fields are filled. \
             Clear SMTP fields to skip outbound settings."
            .into());
    }
    let port: u16 = if port_trim.is_empty() {
        default_port
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
            imap_port: DEFAULT_IMAP_PORT,
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
            .unwrap_or(DEFAULT_IMAP_PORT);
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
        ImapSettings::new(
            "imap.example.com".into(),
            DEFAULT_IMAP_PORT,
            "user@example.com".into(),
            "s3cret".into(),
            ImapTlsMode::Implicit,
        )
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
            identities: Vec::new(),
            signature: None,
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
        assert!(ui.signature.is_none());
        assert!(ui.identities.is_empty());
    }

    #[test]
    fn serde_old_config_without_signature_defaults_none() {
        let json = serde_json::to_string(&sample_config()).unwrap();
        assert!(
            !json.contains("signature"),
            "None signature should be omitted: {json}"
        );
        let back: AccountConfig = serde_json::from_str(&json).unwrap();
        assert!(back.signature.is_none());

        let old = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "display_name": "Work",
            "email": "user@example.com",
            "imap": {
                "host": "imap.example.com",
                "port": 993,
                "username": "user@example.com",
                "password": "s3cret",
                "use_tls": true
            },
            "smtp": null,
            "proxy": {
                "base_url": "ws://localhost:9400/proxy",
                "token": "testtoken",
                "remote_host": null,
                "remote_port": null
            },
            "created_at": "2024-06-15T12:00:00Z",
            "updated_at": "2024-06-15T12:00:00Z"
        }"#;
        let back: AccountConfig = serde_json::from_str(old).expect("old blob without signature");
        assert!(back.signature.is_none());
        assert!(back.identities.is_empty());
        assert_eq!(back.email, "user@example.com");
    }

    #[test]
    fn serde_roundtrip_signature() {
        let mut config = sample_config();
        config.signature = Some("Jane Doe\nExample Corp".into());
        let json = serde_json::to_string(&config).unwrap();
        assert!(
            json.contains("\"signature\":\"Jane Doe\\nExample Corp\""),
            "signature missing: {json}"
        );
        let back: AccountConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.signature.as_deref(), Some("Jane Doe\nExample Corp"));
        assert_eq!(back.to_ui_account().signature, back.signature);
    }

    #[test]
    fn serde_old_config_without_identities_defaults_empty() {
        let json = serde_json::to_string(&sample_config()).unwrap();
        assert!(
            !json.contains("identities"),
            "empty identities should be omitted: {json}"
        );
        let back: AccountConfig = serde_json::from_str(&json).unwrap();
        assert!(back.identities.is_empty());
    }

    #[test]
    fn serde_roundtrip_identities() {
        let mut config = sample_config();
        config.identities = vec![AccountIdentity::new("Support", "support@example.com")];
        let json = serde_json::to_string(&config).unwrap();
        assert!(
            json.contains("\"email\":\"support@example.com\""),
            "identity missing: {json}"
        );
        let back: AccountConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.identities, config.identities);
        assert_eq!(back.to_ui_account().identities, back.identities);
        assert_eq!(back.all_identities().len(), 2);
        assert_eq!(back.all_identities()[0].email, "user@example.com");
        assert_eq!(back.all_identities()[1].email, "support@example.com");
    }

    #[test]
    fn normalize_identities_skips_blank_and_primary() {
        let rows = vec![
            AccountIdentity::new("", ""),
            AccountIdentity::new("  ", "  "),
            AccountIdentity::new("Work", "user@example.com"),
            AccountIdentity::new(" Support ", " support@example.com "),
            AccountIdentity::new("Support", "support@example.com"),
        ];
        let out = normalize_identities(rows, "Work", "user@example.com").unwrap();
        assert_eq!(
            out,
            vec![AccountIdentity::new("Support", "support@example.com")]
        );
    }

    #[test]
    fn normalize_identities_rejects_name_without_email() {
        let err = normalize_identities(
            vec![AccountIdentity::new("Support", "")],
            "Work",
            "user@example.com",
        )
        .unwrap_err();
        assert!(err.contains("email"), "{err}");
    }

    #[test]
    fn normalize_identities_rejects_invalid_email() {
        let err = normalize_identities(
            vec![AccountIdentity::new("Support", "not-an-email")],
            "Work",
            "user@example.com",
        )
        .unwrap_err();
        assert!(err.contains("look like an address"), "{err}");
    }

    #[test]
    fn with_identities_stores_normalized_extras() {
        let config = sample_config()
            .with_identities(vec![
                AccountIdentity::new("", ""),
                AccountIdentity::new("Alias", "alias@example.com"),
            ])
            .unwrap();
        assert_eq!(
            config.identities,
            vec![AccountIdentity::new("Alias", "alias@example.com")]
        );
    }

    #[test]
    fn normalize_signature_empty_is_none() {
        assert_eq!(normalize_signature(""), None);
        assert_eq!(normalize_signature("   \n\t  "), None);
        assert_eq!(
            normalize_signature("  Jane Doe\nExample Corp  \n"),
            Some("Jane Doe\nExample Corp".into())
        );
    }

    #[test]
    fn without_secrets_redacts_passwords_and_token() {
        let mut config = sample_config();
        config.smtp = Some(SmtpSettings::new(
            "smtp.example.com".into(),
            465,
            "user@example.com".into(),
            Some("smtp-secret".into()),
            SmtpTlsMode::Implicit,
        ));
        let redacted = config.without_secrets();
        assert!(redacted.imap.password.is_empty());
        assert!(redacted.smtp.as_ref().unwrap().password.is_none());
        assert!(redacted.proxy.token.is_empty());

        // Non-secret connection fields stay put.
        assert_eq!(redacted.display_name, "Work");
        assert_eq!(redacted.email, "user@example.com");
        assert_eq!(redacted.imap.host, "imap.example.com");
        assert_eq!(redacted.imap.port, 993);
        assert_eq!(redacted.imap.username, "user@example.com");
        assert_eq!(redacted.smtp.as_ref().unwrap().host, "smtp.example.com");
        assert_eq!(redacted.smtp.as_ref().unwrap().port, 465);
        assert_eq!(redacted.smtp.as_ref().unwrap().username, "user@example.com");
        assert_eq!(redacted.proxy.base_url, "ws://localhost:9400/proxy");

        // Source is unchanged; serialized redacted JSON must not contain secrets.
        assert_eq!(config.imap.password, "s3cret");
        assert_eq!(
            config.smtp.as_ref().and_then(|s| s.password.as_deref()),
            Some("smtp-secret")
        );
        assert_eq!(config.proxy.token, "testtoken");
        let json = serde_json::to_string(&redacted).unwrap();
        assert!(!json.contains("s3cret"), "imap password leaked: {json}");
        assert!(
            !json.contains("smtp-secret"),
            "smtp password leaked: {json}"
        );
        assert!(!json.contains("testtoken"), "proxy token leaked: {json}");
    }

    #[test]
    fn without_secrets_strips_token_baked_into_proxy_url() {
        let mut config = sample_config();
        config.proxy.base_url = "wss://user:pw@proxy.example/proxy?foo=1&token=leaked&bar=2".into();
        let redacted = config.without_secrets();
        assert_eq!(
            redacted.proxy.base_url,
            "wss://proxy.example/proxy?foo=1&bar=2"
        );
        let json = serde_json::to_string(&redacted).unwrap();
        assert!(!json.contains("leaked"), "url token leaked: {json}");
        assert!(!json.contains("user:pw"), "userinfo leaked: {json}");
        assert_eq!(
            config.proxy.base_url,
            "wss://user:pw@proxy.example/proxy?foo=1&token=leaked&bar=2"
        );
    }

    #[test]
    fn without_secrets_strips_percent_encoded_token_query_key() {
        let mut config = sample_config();
        config.proxy.base_url = "wss://proxy.example/proxy?%74oken=leaked&foo=1".into();
        let redacted = config.without_secrets();
        assert_eq!(redacted.proxy.base_url, "wss://proxy.example/proxy?foo=1");
        let json = serde_json::to_string(&redacted).unwrap();
        assert!(!json.contains("leaked"), "encoded token leaked: {json}");
    }

    #[test]
    fn without_secrets_keeps_at_sign_in_proxy_path() {
        let mut config = sample_config();
        config.proxy.base_url = "wss://proxy.example/u@v/proxy?foo=1".into();
        let redacted = config.without_secrets();
        assert_eq!(
            redacted.proxy.base_url,
            "wss://proxy.example/u@v/proxy?foo=1"
        );
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
        let url = proxy.websocket_url_for("smtp.example.com", 465).unwrap();
        assert!(url.contains("remote=smtp.example.com:465"), "{url}");
        assert!(!url.contains("imap-override"));
    }

    fn sample_smtp() -> SmtpSettings {
        SmtpSettings::new(
            "smtp.example.com".into(),
            465,
            "user@example.com".into(),
            None,
            SmtpTlsMode::Implicit,
        )
    }

    #[test]
    fn smtp_dial_defaults_to_host_port() {
        let smtp = sample_smtp();
        assert_eq!(smtp.dial_host(), "smtp.example.com");
        assert_eq!(smtp.dial_port(), 465);
        let url = sample_proxy().websocket_url_for_smtp(&smtp).unwrap();
        assert!(url.contains("remote=smtp.example.com:465"), "{url}");
    }

    #[test]
    fn smtp_dial_uses_remote_override() {
        let mut smtp = sample_smtp();
        smtp.remote_host = Some("smtp-backend.internal".into());
        smtp.remote_port = Some(2525);
        assert_eq!(smtp.dial_host(), "smtp-backend.internal");
        assert_eq!(smtp.dial_port(), 2525);
        assert_eq!(smtp.host, "smtp.example.com");
        assert_eq!(smtp.port, 465);

        let mut proxy = sample_proxy();
        proxy.remote_host = Some("imap-override.example".into());
        proxy.remote_port = Some(993);
        let url = proxy.websocket_url_for_smtp(&smtp).unwrap();
        assert!(url.contains("remote=smtp-backend.internal:2525"), "{url}");
        assert!(!url.contains("smtp.example.com"));
        assert!(!url.contains("imap-override"));
    }

    #[test]
    fn smtp_dial_blank_remote_host_falls_back() {
        let mut smtp = sample_smtp();
        smtp.remote_host = Some("   ".into());
        smtp.remote_port = Some(587);
        assert_eq!(smtp.dial_host(), "smtp.example.com");
        assert_eq!(smtp.dial_port(), 587);
    }

    #[test]
    fn smtp_serde_missing_remote_is_none() {
        let json = r#"{
            "host": "smtp.example.com",
            "port": 465,
            "username": "u",
            "password": null,
            "use_tls": true
        }"#;
        let smtp: SmtpSettings = serde_json::from_str(json).unwrap();
        assert!(smtp.remote_host.is_none());
        assert!(smtp.remote_port.is_none());
        let out = serde_json::to_string(&smtp).unwrap();
        assert!(!out.contains("remote_host"), "{out}");
        assert!(!out.contains("remote_port"), "{out}");
    }

    #[test]
    fn smtp_serde_roundtrip_remote_override() {
        let mut smtp = sample_smtp();
        smtp.remote_host = Some("smtp-dial.example".into());
        smtp.remote_port = Some(2525);
        let json = serde_json::to_string(&smtp).unwrap();
        assert!(
            json.contains("\"remote_host\":\"smtp-dial.example\""),
            "{json}"
        );
        assert!(json.contains("\"remote_port\":2525"), "{json}");
        let back: SmtpSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, smtp);
    }

    #[test]
    fn ehlo_domain_uses_smtp_host_not_remote() {
        let mut config = sample_config();
        config.email = "user@localhost".into();
        let mut smtp = sample_smtp();
        smtp.remote_host = Some("127.0.0.1".into());
        config.smtp = Some(smtp);
        assert_eq!(ehlo_domain(&config), "smtp.example.com");
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

    #[test]
    fn optional_smtp_from_tls_mode_writes_mode_and_use_tls() {
        let smtp =
            optional_smtp_from_tls_mode("smtp.example.com", "", "u", "", SmtpTlsMode::Implicit)
                .unwrap()
                .expect("Some");
        assert_eq!(smtp.tls_mode, SmtpTlsMode::Implicit);
        assert!(smtp.use_tls);
        assert_eq!(smtp.port, DEFAULT_SMTP_PORT);

        let smtp =
            optional_smtp_from_tls_mode("smtp.example.com", "", "", "", SmtpTlsMode::StartTls)
                .unwrap()
                .expect("Some");
        assert_eq!(smtp.tls_mode, SmtpTlsMode::StartTls);
        assert!(smtp.use_tls);
        assert_eq!(smtp.port, 587);

        let smtp =
            optional_smtp_from_tls_mode("smtp.example.com", "465", "", "", SmtpTlsMode::StartTls)
                .unwrap()
                .expect("Some");
        assert_eq!(smtp.tls_mode, SmtpTlsMode::StartTls);
        assert!(smtp.use_tls);
        assert_eq!(smtp.port, 465);

        let smtp = optional_smtp_from_tls_mode("smtp.example.com", "", "", "", SmtpTlsMode::None)
            .unwrap()
            .expect("Some");
        assert_eq!(smtp.tls_mode, SmtpTlsMode::None);
        assert!(!smtp.use_tls);
        assert_eq!(smtp.port, 25);
    }

    #[test]
    fn port_for_tls_mode_change_only_rewrites_previous_default() {
        assert_eq!(
            port_for_tls_mode_change("465", SmtpTlsMode::Implicit, SmtpTlsMode::StartTls),
            "587"
        );
        assert_eq!(
            port_for_tls_mode_change("587", SmtpTlsMode::StartTls, SmtpTlsMode::None),
            "25"
        );
        assert_eq!(
            port_for_tls_mode_change("", SmtpTlsMode::Implicit, SmtpTlsMode::None),
            "25"
        );
        assert_eq!(
            port_for_tls_mode_change("2525", SmtpTlsMode::Implicit, SmtpTlsMode::StartTls),
            "2525"
        );
        assert_eq!(
            port_for_tls_mode_change("587", SmtpTlsMode::Implicit, SmtpTlsMode::StartTls),
            "587"
        );
    }

    #[test]
    fn imap_tls_mode_from_legacy_maps_use_tls_and_port() {
        assert_eq!(imap_tls_mode_from_legacy(true, 993), ImapTlsMode::Implicit);
        assert_eq!(imap_tls_mode_from_legacy(true, 143), ImapTlsMode::StartTls);
        assert_eq!(imap_tls_mode_from_legacy(false, 993), ImapTlsMode::None);
        assert_eq!(imap_tls_mode_from_legacy(false, 143), ImapTlsMode::None);
        assert_eq!(imap_tls_mode_from_legacy(true, 9930), ImapTlsMode::Implicit);
    }

    #[test]
    fn v1_json_143_use_tls_true_becomes_starttls() {
        let json = r#"{
            "host": "imap.example.com",
            "port": 143,
            "username": "u",
            "password": "pw",
            "use_tls": true
        }"#;
        let imap: ImapSettings = serde_json::from_str(json).unwrap();
        assert_eq!(imap.tls_mode, ImapTlsMode::StartTls);
        assert!(imap.use_tls);
        let out = serde_json::to_string(&imap).unwrap();
        assert!(out.contains("\"tls_mode\":\"start_tls\""));
        assert!(out.contains("\"use_tls\":true"));
    }

    #[test]
    fn v1_json_993_use_tls_true_stays_implicit() {
        let json = r#"{
            "host": "imap.example.com",
            "port": 993,
            "username": "u",
            "password": "pw",
            "use_tls": true
        }"#;
        let imap: ImapSettings = serde_json::from_str(json).unwrap();
        assert_eq!(imap.tls_mode, ImapTlsMode::Implicit);
        assert!(imap.use_tls);
    }

    #[test]
    fn v1_json_use_tls_false_becomes_none() {
        let json = r#"{
            "host": "imap.example.com",
            "port": 143,
            "username": "u",
            "password": "pw",
            "use_tls": false
        }"#;
        let imap: ImapSettings = serde_json::from_str(json).unwrap();
        assert_eq!(imap.tls_mode, ImapTlsMode::None);
        assert!(!imap.use_tls);
        let out = serde_json::to_string(&imap).unwrap();
        assert!(out.contains("\"tls_mode\":\"none\""));
        assert!(out.contains("\"use_tls\":false"));
    }

    #[test]
    fn imap_tls_mode_to_connector_maps_each_variant() {
        use mailiner_imap_connector::ImapTlsMode as ConnectorMode;
        assert_eq!(
            ImapTlsMode::Implicit.to_connector(),
            ConnectorMode::Implicit
        );
        assert_eq!(
            ImapTlsMode::StartTls.to_connector(),
            ConnectorMode::StartTls
        );
        assert_eq!(ImapTlsMode::None.to_connector(), ConnectorMode::None);
    }

    #[test]
    fn port_for_imap_tls_mode_change_only_rewrites_previous_default() {
        assert_eq!(
            port_for_imap_tls_mode_change("993", ImapTlsMode::Implicit, ImapTlsMode::StartTls),
            "143"
        );
        assert_eq!(
            port_for_imap_tls_mode_change("143", ImapTlsMode::StartTls, ImapTlsMode::Implicit),
            "993"
        );
        assert_eq!(
            port_for_imap_tls_mode_change("", ImapTlsMode::Implicit, ImapTlsMode::None),
            "143"
        );
        assert_eq!(
            port_for_imap_tls_mode_change("1993", ImapTlsMode::Implicit, ImapTlsMode::StartTls),
            "1993"
        );
        assert_eq!(
            port_for_imap_tls_mode_change("143", ImapTlsMode::Implicit, ImapTlsMode::StartTls),
            "143"
        );
    }

    #[test]
    fn form_value_roundtrip_tls_mode() {
        for mode in [
            SmtpTlsMode::Implicit,
            SmtpTlsMode::StartTls,
            SmtpTlsMode::None,
        ] {
            assert_eq!(SmtpTlsMode::from_form_value(mode.as_form_value()), mode);
        }
        assert_eq!(SmtpTlsMode::from_form_value("bogus"), SmtpTlsMode::Implicit);
    }

    #[test]
    fn form_value_roundtrip_imap_tls_mode() {
        for mode in [
            ImapTlsMode::Implicit,
            ImapTlsMode::StartTls,
            ImapTlsMode::None,
        ] {
            assert_eq!(ImapTlsMode::from_form_value(mode.as_form_value()), mode);
        }
        assert_eq!(ImapTlsMode::from_form_value("bogus"), ImapTlsMode::Implicit);
    }
}
