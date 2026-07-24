//! Shared account settings form fields and validation (onboarding + accounts UI).

use chrono::Utc;
use dioxus::prelude::*;

use crate::account::AccountId;
use crate::account_config::{AccountConfig, ImapSettings, ProxySettings};
use crate::connection::ConnectErrorKind;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FormPhase {
    Idle,
    Testing,
    Saving,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StatusLevel {
    Info,
    Success,
    Error,
}

#[derive(Clone, PartialEq, Eq)]
pub struct StatusMessage {
    pub level: StatusLevel,
    pub title: Option<String>,
    pub body: String,
}

impl StatusMessage {
    pub fn info(body: impl Into<String>) -> Self {
        Self {
            level: StatusLevel::Info,
            title: None,
            body: body.into(),
        }
    }
    pub fn success(body: impl Into<String>) -> Self {
        Self {
            level: StatusLevel::Success,
            title: None,
            body: body.into(),
        }
    }
    pub fn error(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            level: StatusLevel::Error,
            title: Some(title.into()),
            body: body.into(),
        }
    }
}

pub fn kind_label(kind: ConnectErrorKind) -> &'static str {
    match kind {
        ConnectErrorKind::NetworkOrProxy => "Network / proxy",
        ConnectErrorKind::TlsOrSni => "TLS / certificate",
        ConnectErrorKind::Auth => "Sign-in failed",
        ConnectErrorKind::Timeout => "Timed out",
        ConnectErrorKind::Cancelled => "Cancelled",
        ConnectErrorKind::Internal => "Error",
    }
}

/// Suggest `imap.{domain}` as a **placeholder only** (never auto-submitted).
pub fn email_to_imap_host_hint(email: &str) -> String {
    email
        .rsplit_once('@')
        .map(|(_, domain)| domain.trim())
        .filter(|d| !d.is_empty() && d.contains('.'))
        .map(|d| format!("imap.{d}"))
        .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
pub fn build_config_from_form(
    account_id: &AccountId,
    display_name: &str,
    email: &str,
    imap_host: &str,
    imap_port: &str,
    imap_username: &str,
    imap_password: &str,
    proxy_base_url: &str,
    proxy_token: &str,
    remote_host: &str,
    remote_port: &str,
    created_at: chrono::DateTime<Utc>,
) -> Result<AccountConfig, String> {
    let display_name = display_name.trim();
    let email = email.trim();
    let host = imap_host.trim().to_string();
    let username = imap_username.trim();
    // Do not trim passwords (spaces may be intentional).
    let password = imap_password;
    let proxy_base = proxy_base_url.trim();

    if display_name.is_empty() {
        return Err("Display name is required.".into());
    }
    if email.is_empty() {
        return Err("Email is required.".into());
    }
    if !email.contains('@') {
        return Err("Email must look like an address (user@example.com).".into());
    }
    if host.is_empty() {
        return Err("IMAP host is required.".into());
    }
    let port: u16 = imap_port
        .trim()
        .parse()
        .map_err(|_| "IMAP port must be a number between 1 and 65535.".to_string())?;
    if port == 0 {
        return Err("IMAP port must be a number between 1 and 65535.".into());
    }
    if username.is_empty() {
        return Err("IMAP username is required.".into());
    }
    if password.is_empty() {
        return Err("IMAP password is required.".into());
    }
    if proxy_base.is_empty() {
        return Err("Proxy base URL is required (e.g. ws://localhost:9400/proxy).".into());
    }

    let remote_host = {
        let t = remote_host.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    };
    let remote_port = {
        let t = remote_port.trim();
        if t.is_empty() {
            None
        } else {
            let p: u16 = t
                .parse()
                .map_err(|_| "Remote port must be a number between 1 and 65535.".to_string())?;
            if p == 0 {
                return Err("Remote port must be a number between 1 and 65535.".into());
            }
            Some(p)
        }
    };

    let now = Utc::now();
    let config = AccountConfig {
        id: account_id.clone(),
        display_name: display_name.to_string(),
        email: email.to_string(),
        imap: ImapSettings {
            host,
            port,
            username: username.to_string(),
            password: password.to_string(),
            use_tls: true,
        },
        smtp: None,
        proxy: ProxySettings {
            base_url: proxy_base.to_string(),
            token: proxy_token.to_string(),
            remote_host,
            remote_port,
        },
        created_at,
        updated_at: now,
    };

    config
        .validate()
        .map_err(|e| format!("Invalid settings: {e}"))?;

    Ok(config)
}

/// Whether connection-related fields differ (triggers connect-before-persist on edit).
pub fn credentials_changed(old: &AccountConfig, new: &AccountConfig) -> bool {
    old.imap.host != new.imap.host
        || old.imap.port != new.imap.port
        || old.imap.username != new.imap.username
        || old.imap.password != new.imap.password
        || old.imap.use_tls != new.imap.use_tls
        || old.proxy != new.proxy
}

#[component]
pub fn FormField(
    label: String,
    id: String,
    value: String,
    oninput: EventHandler<String>,
    #[props(default = "text".to_string())] input_type: String,
    #[props(default)] placeholder: String,
    #[props(default)] autocomplete: String,
    #[props(default)] disabled: bool,
) -> Element {
    rsx! {
        div {
            class: "onboarding-field",
            label {
                r#for: "{id}",
                "{label}"
            }
            input {
                id: "{id}",
                name: "{id}",
                r#type: "{input_type}",
                value: "{value}",
                placeholder: "{placeholder}",
                autocomplete: "{autocomplete}",
                disabled: disabled,
                oninput: move |e| oninput.call(e.value()),
            }
        }
    }
}

/// Identity + IMAP + proxy fieldsets (no SMTP). Shared by onboarding and accounts.
#[component]
pub fn AccountConnectionFields(
    id_prefix: String,
    display_name: String,
    email: String,
    imap_host: String,
    imap_port: String,
    imap_username: String,
    imap_password: String,
    proxy_base_url: String,
    proxy_token: String,
    remote_host: String,
    remote_port: String,
    set_display_name: EventHandler<String>,
    set_email: EventHandler<String>,
    set_imap_host: EventHandler<String>,
    set_imap_port: EventHandler<String>,
    set_imap_username: EventHandler<String>,
    set_imap_password: EventHandler<String>,
    set_proxy_base_url: EventHandler<String>,
    set_proxy_token: EventHandler<String>,
    set_remote_host: EventHandler<String>,
    set_remote_port: EventHandler<String>,
    busy: bool,
    #[props(default)] open_advanced: bool,
) -> Element {
    let host_placeholder = email_to_imap_host_hint(&email);
    let insecure_proxy = {
        ProxySettings {
            base_url: proxy_base_url.clone(),
            token: String::new(),
            remote_host: None,
            remote_port: None,
        }
        .is_insecure_remote_ws()
    };

    rsx! {
        fieldset {
            class: "onboarding-section",
            legend { "Identity" }
            FormField {
                label: "Display name",
                id: "{id_prefix}-display-name",
                value: display_name,
                oninput: move |v| set_display_name.call(v),
                autocomplete: "name",
                disabled: busy,
            }
            FormField {
                label: "Email",
                id: "{id_prefix}-email",
                value: email,
                oninput: move |v| set_email.call(v),
                input_type: "email",
                autocomplete: "email",
                disabled: busy,
            }
        }

        fieldset {
            class: "onboarding-section",
            legend { "IMAP" }
            FormField {
                label: "Host",
                id: "{id_prefix}-imap-host",
                value: imap_host,
                oninput: move |v| set_imap_host.call(v),
                placeholder: host_placeholder.clone(),
                autocomplete: "off",
                disabled: busy,
            }
            FormField {
                label: "Port",
                id: "{id_prefix}-imap-port",
                value: imap_port,
                oninput: move |v| set_imap_port.call(v),
                input_type: "number",
                autocomplete: "off",
                disabled: busy,
            }
            FormField {
                label: "Username",
                id: "{id_prefix}-imap-user",
                value: imap_username,
                oninput: move |v| set_imap_username.call(v),
                autocomplete: "username",
                disabled: busy,
            }
            FormField {
                label: "Password",
                id: "{id_prefix}-imap-password",
                value: imap_password,
                oninput: move |v| set_imap_password.call(v),
                input_type: "password",
                autocomplete: "current-password",
                disabled: busy,
            }
        }

        fieldset {
            class: "onboarding-section",
            legend { "Proxy (WebSocket TCP proxy)" }
            p {
                class: "bootstrap-muted",
                "Browsers cannot open plain TCP. Mailiner reaches IMAP through a \
                 WebSocket proxy (e.g. ws-tcp-proxy)."
            }
            FormField {
                label: "Proxy base URL",
                id: "{id_prefix}-proxy-url",
                value: proxy_base_url,
                oninput: move |v| set_proxy_base_url.call(v),
                placeholder: "ws://localhost:9400/proxy",
                autocomplete: "off",
                disabled: busy,
            }
            FormField {
                label: "Proxy token",
                id: "{id_prefix}-proxy-token",
                value: proxy_token,
                oninput: move |v| set_proxy_token.call(v),
                input_type: "password",
                autocomplete: "off",
                disabled: busy,
            }
            details {
                class: "onboarding-advanced",
                open: open_advanced,
                summary { "Advanced: remote override" }
                FormField {
                    label: "Remote host (optional)",
                    id: "{id_prefix}-remote-host",
                    value: remote_host,
                    oninput: move |v| set_remote_host.call(v),
                    placeholder: "Defaults to IMAP host",
                    autocomplete: "off",
                    disabled: busy,
                }
                FormField {
                    label: "Remote port (optional)",
                    id: "{id_prefix}-remote-port",
                    value: remote_port,
                    oninput: move |v| set_remote_port.call(v),
                    placeholder: "Defaults to IMAP port",
                    input_type: "number",
                    autocomplete: "off",
                    disabled: busy,
                }
            }
        }

        if insecure_proxy {
            p {
                class: "onboarding-warning",
                role: "alert",
                "This proxy URL uses unencrypted WebSocket to a non-local host. \
                 The proxy token can be sniffed. Prefer wss://."
            }
        }

        p {
            class: "onboarding-disclosure bootstrap-muted",
            "Your IMAP password is stored only in this browser on this device. \
             Mailiner has no server account. Anyone with access to this browser \
             profile (or a compromised page on this origin) can read it. Use a \
             private device; clear site data to remove it."
        }
    }
}

#[component]
pub fn FormStatusBanner(message: Option<StatusMessage>) -> Element {
    let Some(msg) = message else {
        return rsx! {};
    };
    rsx! {
        p {
            class: match msg.level {
                StatusLevel::Error => "onboarding-status onboarding-status-error",
                StatusLevel::Success => "onboarding-status onboarding-status-success",
                StatusLevel::Info => "onboarding-status onboarding-status-info",
            },
            role: "status",
            if let Some(title) = &msg.title {
                strong { "{title}: " }
            }
            "{msg.body}"
        }
    }
}
