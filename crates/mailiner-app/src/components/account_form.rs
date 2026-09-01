//! Shared account settings form fields and validation (onboarding + accounts UI).

use chrono::Utc;
use dioxus::prelude::*;
use uuid::Uuid;

use crate::account::AccountId;
use crate::account_config::{
    AccountConfig, DEFAULT_SMTP_PORT, ImapSettings, ProxySettings, optional_smtp_from_fields,
};
use crate::connection::ConnectErrorKind;
use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::send::{SendState, send_kind_label};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FormPhase {
    Idle,
    Testing,
    TestingSmtp,
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
    smtp_host: &str,
    smtp_port: &str,
    smtp_username: &str,
    smtp_password: &str,
    smtp_use_tls: bool,
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

    let smtp = optional_smtp_from_fields(
        smtp_host,
        smtp_port,
        smtp_username,
        smtp_password,
        smtp_use_tls,
    )?;

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
        smtp,
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

/// Consume a terminal Test SMTP outcome for `rid` into the form banner.
pub fn apply_smtp_test_outcome(
    mut ctx: AppContext,
    rid: &AccountId,
    mut phase: Signal<FormPhase>,
    mut test_request_id: Signal<Option<AccountId>>,
    mut status_message: Signal<Option<StatusMessage>>,
) {
    let outcome = ctx.smtp_test_status.read().get(rid).cloned();
    match outcome {
        Some(SendState::Sending { .. }) | Some(SendState::Idle) | None => {}
        Some(SendState::Sent { .. }) => {
            phase.set(FormPhase::Idle);
            ctx.smtp_test_status.write().remove(rid);
            test_request_id.set(None);
            status_message.set(Some(StatusMessage::success("SMTP sign-in succeeded.")));
        }
        Some(SendState::Failed { kind, message, .. }) => {
            phase.set(FormPhase::Idle);
            ctx.smtp_test_status.write().remove(rid);
            test_request_id.set(None);
            status_message.set(Some(StatusMessage::error(send_kind_label(kind), message)));
        }
    }
}

/// Kick off `TestSmtpConnection` for a built account config.
///
/// No-ops unless the form is Idle. Missing SMTP host is a validation error.
pub fn start_smtp_test(
    config: Result<AccountConfig, String>,
    mut phase: Signal<FormPhase>,
    mut test_request_id: Signal<Option<AccountId>>,
    mut status_message: Signal<Option<StatusMessage>>,
    core_tx: Coroutine<CoreEvent>,
) {
    if !matches!(phase(), FormPhase::Idle) {
        return;
    }
    match config {
        Ok(config) => {
            if config.smtp.is_none() {
                status_message.set(Some(StatusMessage::error(
                    "SMTP",
                    "Fill in an SMTP host first.",
                )));
                return;
            }
            let request_id = AccountId::new(Uuid::new_v4().to_string());
            test_request_id.set(Some(request_id.clone()));
            phase.set(FormPhase::TestingSmtp);
            status_message.set(Some(StatusMessage::info("Testing SMTP…")));
            core_tx.send(CoreEvent::TestSmtpConnection { request_id, config });
        }
        Err(msg) => {
            status_message.set(Some(StatusMessage::error("Validation", &msg)));
        }
    }
}

/// Drop ephemeral Test SMTP / IMAP-test keys when the form unmounts so a
/// completed result cannot linger in `AppContext` after navigation.
pub fn use_form_test_status_cleanup(
    mut ctx: AppContext,
    test_request_id: Signal<Option<AccountId>>,
) {
    use_drop(move || {
        if let Some(rid) = test_request_id.peek().clone() {
            ctx.smtp_test_abandoned.write().insert(rid.clone());
            ctx.smtp_test_status.write().remove(&rid);
            ctx.connection_states.write().remove(&rid);
        }
    });
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

/// Identity + IMAP + proxy fieldsets. Shared by onboarding and accounts.
/// SMTP is a separate [`AccountSmtpFields`] section.
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

/// Optional SMTP fields (collapsed advanced).
///
/// Empty section persists as `smtp: None`. Leave password blank to reuse IMAP password later.
#[component]
pub fn AccountSmtpFields(
    id_prefix: String,
    smtp_host: String,
    smtp_port: String,
    smtp_username: String,
    smtp_password: String,
    smtp_use_tls: bool,
    set_smtp_host: EventHandler<String>,
    set_smtp_port: EventHandler<String>,
    set_smtp_username: EventHandler<String>,
    set_smtp_password: EventHandler<String>,
    set_smtp_use_tls: EventHandler<bool>,
    busy: bool,
    #[props(default)] open: bool,
) -> Element {
    let port_placeholder = DEFAULT_SMTP_PORT.to_string();
    let warn_starttls = smtp_use_tls && smtp_port.trim() == "587";
    let warn_plain = !smtp_use_tls;
    rsx! {
        fieldset {
            class: "onboarding-section",
            legend { "SMTP (sending)" }
            p {
                class: "onboarding-notice",
                role: "note",
                "Used when you click Send. Leave password empty to reuse the IMAP password. \
                 Implicit TLS (port 465), STARTTLS (port 587), or plaintext."
            }
            details {
                class: "onboarding-advanced",
                open: open,
                summary { "Optional SMTP settings" }
                FormField {
                    label: "SMTP host",
                    id: "{id_prefix}-smtp-host",
                    value: smtp_host,
                    oninput: move |v| set_smtp_host.call(v),
                    placeholder: "smtp.example.com",
                    autocomplete: "off",
                    disabled: busy,
                }
                FormField {
                    label: "SMTP port",
                    id: "{id_prefix}-smtp-port",
                    value: smtp_port,
                    oninput: move |v| set_smtp_port.call(v),
                    placeholder: port_placeholder,
                    input_type: "number",
                    autocomplete: "off",
                    disabled: busy,
                }
                FormField {
                    label: "SMTP username",
                    id: "{id_prefix}-smtp-user",
                    value: smtp_username,
                    oninput: move |v| set_smtp_username.call(v),
                    autocomplete: "off",
                    disabled: busy,
                }
                FormField {
                    label: "SMTP password",
                    id: "{id_prefix}-smtp-password",
                    value: smtp_password,
                    oninput: move |v| set_smtp_password.call(v),
                    input_type: "password",
                    placeholder: "Leave empty to reuse IMAP password",
                    autocomplete: "off",
                    disabled: busy,
                }
                div {
                    class: "onboarding-field onboarding-checkbox-field",
                    label {
                        class: "onboarding-checkbox-label",
                        input {
                            id: "{id_prefix}-smtp-tls",
                            name: "{id_prefix}-smtp-tls",
                            r#type: "checkbox",
                            checked: smtp_use_tls,
                            disabled: busy,
                            onchange: move |e| set_smtp_use_tls.call(e.checked()),
                        }
                        " Use TLS (implicit on port 465, STARTTLS on port 587)"
                    }
                }
                if warn_starttls {
                    p {
                        class: "onboarding-notice",
                        role: "note",
                        "STARTTLS (port 587) sends the server greeting, EHLO, and STARTTLS \
                         in the clear, including through the proxy. AUTH and the message \
                         are encrypted after the upgrade. Prefer implicit TLS on port 465 \
                         when the server supports it."
                    }
                }
                if warn_plain {
                    p {
                        class: "onboarding-notice",
                        role: "alert",
                        "Plaintext SMTP sends AUTH and the message in the clear, including \
                         through the proxy. Prefer implicit TLS on port 465 or STARTTLS \
                         on port 587."
                    }
                }
            }
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
