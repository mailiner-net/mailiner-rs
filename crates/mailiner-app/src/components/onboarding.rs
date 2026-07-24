//! First-run onboarding form: connect-before-persist via `CommitNewAccount`.

use chrono::Utc;
use dioxus::prelude::*;
use uuid::Uuid;

use crate::AppBootstrapState;
use crate::account::AccountId;
use crate::account_config::{AccountConfig, ImapSettings, ProxySettings, dev_form_prefill};
use crate::connection::{ConnectErrorKind, ConnectionState};
use crate::context::AppContext;
use crate::core_event::CoreEvent;

#[derive(Clone, Copy, PartialEq, Eq)]
enum FormPhase {
    Idle,
    Testing,
    Saving,
}

/// Full first-run account form (identity, IMAP, proxy). No SMTP fields.
#[component]
pub fn OnboardingForm() -> Element {
    let mut bootstrap = use_context::<Signal<AppBootstrapState>>();
    let mut ctx = use_context::<AppContext>();
    let core_tx = use_coroutine_handle::<CoreEvent>();

    let prefill = use_hook(dev_form_prefill);

    // Stable account id for this form mount (commit path).
    let account_id = use_hook(|| AccountId::new(Uuid::new_v4().to_string()));
    let account_id_for_effect = account_id.clone();
    let account_id_for_test = account_id.clone();
    let account_id_for_save = account_id.clone();

    let mut display_name = use_signal(|| prefill.display_name.clone());
    let mut email = use_signal(|| prefill.email.clone());
    let mut imap_host = use_signal(|| prefill.imap_host.clone());
    let mut imap_port = use_signal(|| prefill.imap_port.to_string());
    let mut imap_username = use_signal(|| prefill.imap_username.clone());
    let mut imap_password = use_signal(|| prefill.imap_password.clone());
    let mut proxy_base_url = use_signal(|| prefill.proxy_base_url.clone());
    let mut proxy_token = use_signal(|| prefill.proxy_token.clone());
    let mut remote_host = use_signal(|| prefill.remote_host.clone());
    let mut remote_port = use_signal(|| prefill.remote_port.clone());

    let mut phase = use_signal(|| FormPhase::Idle);
    let mut status_message = use_signal(|| None::<StatusMessage>);
    let mut test_request_id = use_signal(|| None::<AccountId>);
    // Ignore Ready/Error until we have seen Connecting/Authenticating for this attempt.
    // Prevents a stale Error from a previous Save from aborting a retry (and missing success).
    let mut save_seen_progress = use_signal(|| false);
    let mut test_seen_progress = use_signal(|| false);

    // Watch connection_states for Save / Test outcomes.
    // Success: set Ready; AppShell replaces `/onboarding` → `/`.
    use_effect(move || {
        let states = ctx.connection_states.read().clone();
        let current_phase = phase();
        match current_phase {
            FormPhase::Saving => {
                if let Some(state) = states.get(&account_id_for_effect) {
                    match state {
                        ConnectionState::Connecting | ConnectionState::Authenticating => {
                            save_seen_progress.set(true);
                        }
                        ConnectionState::Ready => {
                            if !save_seen_progress() {
                                return;
                            }
                            // Wait until core refreshed UI accounts (upsert succeeded).
                            if ctx.accounts.read().contains_key(&account_id_for_effect) {
                                phase.set(FormPhase::Idle);
                                save_seen_progress.set(false);
                                status_message.set(None);
                                bootstrap.set(AppBootstrapState::Ready);
                            }
                        }
                        ConnectionState::Error { message, kind, .. } => {
                            if !save_seen_progress() {
                                return;
                            }
                            phase.set(FormPhase::Idle);
                            save_seen_progress.set(false);
                            status_message
                                .set(Some(StatusMessage::error(kind_label(*kind), message)));
                        }
                        _ => {}
                    }
                }
            }
            FormPhase::Testing => {
                if let Some(rid) = test_request_id()
                    && let Some(state) = states.get(&rid)
                {
                    match state {
                        ConnectionState::Connecting | ConnectionState::Authenticating => {
                            test_seen_progress.set(true);
                        }
                        ConnectionState::Ready => {
                            if !test_seen_progress() {
                                return;
                            }
                            phase.set(FormPhase::Idle);
                            test_seen_progress.set(false);
                            // Drop ephemeral test key (UI-owned cleanup).
                            ctx.connection_states.write().remove(&rid);
                            test_request_id.set(None);
                            status_message.set(Some(StatusMessage::success(
                                "Connection successful. You can save and continue.",
                            )));
                        }
                        ConnectionState::Error { message, kind, .. } => {
                            if !test_seen_progress() {
                                return;
                            }
                            phase.set(FormPhase::Idle);
                            test_seen_progress.set(false);
                            ctx.connection_states.write().remove(&rid);
                            test_request_id.set(None);
                            status_message
                                .set(Some(StatusMessage::error(kind_label(*kind), message)));
                        }
                        _ => {}
                    }
                }
            }
            FormPhase::Idle => {}
        }
    });

    let busy = !matches!(phase(), FormPhase::Idle);
    let insecure_proxy = {
        let base = proxy_base_url();
        ProxySettings {
            base_url: base,
            token: String::new(),
            remote_host: None,
            remote_port: None,
        }
        .is_insecure_remote_ws()
    };

    let host_placeholder = email_to_imap_host_hint(&email());

    let on_test = move |_| {
        if !matches!(phase(), FormPhase::Idle) {
            return;
        }
        status_message.set(None);
        match build_config_from_form(
            &account_id_for_test,
            &display_name(),
            &email(),
            &imap_host(),
            &imap_port(),
            &imap_username(),
            &imap_password(),
            &proxy_base_url(),
            &proxy_token(),
            &remote_host(),
            &remote_port(),
        ) {
            Ok(config) => {
                // Clear previous ephemeral test state if any.
                if let Some(prev) = test_request_id() {
                    ctx.connection_states.write().remove(&prev);
                }
                let request_id = AccountId::new(Uuid::new_v4().to_string());
                // Optimistic Connecting so the watcher ignores nothing stale and
                // is armed before core handles the event.
                ctx.connection_states
                    .write()
                    .insert(request_id.clone(), ConnectionState::Connecting);
                test_request_id.set(Some(request_id.clone()));
                test_seen_progress.set(true);
                phase.set(FormPhase::Testing);
                status_message.set(Some(StatusMessage::info("Testing connection…")));
                core_tx.send(CoreEvent::TestConnection { request_id, config });
            }
            Err(msg) => {
                status_message.set(Some(StatusMessage::error("Validation", &msg)));
            }
        }
    };

    let on_save = move |_| {
        if !matches!(phase(), FormPhase::Idle) {
            return;
        }
        status_message.set(None);
        match build_config_from_form(
            &account_id_for_save,
            &display_name(),
            &email(),
            &imap_host(),
            &imap_port(),
            &imap_username(),
            &imap_password(),
            &proxy_base_url(),
            &proxy_token(),
            &remote_host(),
            &remote_port(),
        ) {
            Ok(config) => {
                // Replace any stale Error/Ready with Connecting for this form id
                // so a retry is not terminated by the previous attempt's state.
                ctx.connection_states
                    .write()
                    .insert(account_id_for_save.clone(), ConnectionState::Connecting);
                save_seen_progress.set(true);
                phase.set(FormPhase::Saving);
                status_message.set(Some(StatusMessage::info("Connecting…")));
                core_tx.send(CoreEvent::CommitNewAccount { config });
            }
            Err(msg) => {
                status_message.set(Some(StatusMessage::error("Validation", &msg)));
            }
        }
    };

    rsx! {
        div {
            class: "bootstrap-shell onboarding-shell",
            div {
                class: "bootstrap-card onboarding-card",
                h1 { class: "bootstrap-title", "Welcome to Mailiner" }
                p {
                    class: "bootstrap-muted",
                    "Add your first email account. Use your IMAP username and password \
                     (or provider app password). OAuth sign-in is not supported yet."
                }

                form {
                    class: "onboarding-form",
                    onsubmit: move |evt| {
                        evt.prevent_default();
                    },

                    fieldset {
                        class: "onboarding-section",
                        legend { "Identity" }
                        FormField {
                            label: "Display name",
                            id: "onboarding-display-name",
                            value: display_name(),
                            oninput: move |v| display_name.set(v),
                            autocomplete: "name",
                            disabled: busy,
                        }
                        FormField {
                            label: "Email",
                            id: "onboarding-email",
                            value: email(),
                            oninput: move |v| email.set(v),
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
                            id: "onboarding-imap-host",
                            value: imap_host(),
                            oninput: move |v| imap_host.set(v),
                            placeholder: host_placeholder.clone(),
                            autocomplete: "off",
                            disabled: busy,
                        }
                        FormField {
                            label: "Port",
                            id: "onboarding-imap-port",
                            value: imap_port(),
                            oninput: move |v| imap_port.set(v),
                            input_type: "number",
                            autocomplete: "off",
                            disabled: busy,
                        }
                        FormField {
                            label: "Username",
                            id: "onboarding-imap-user",
                            value: imap_username(),
                            oninput: move |v| imap_username.set(v),
                            autocomplete: "username",
                            disabled: busy,
                        }
                        FormField {
                            label: "Password",
                            id: "onboarding-imap-password",
                            value: imap_password(),
                            oninput: move |v| imap_password.set(v),
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
                            id: "onboarding-proxy-url",
                            value: proxy_base_url(),
                            oninput: move |v| proxy_base_url.set(v),
                            placeholder: "ws://localhost:9400/proxy",
                            autocomplete: "off",
                            disabled: busy,
                        }
                        FormField {
                            label: "Proxy token",
                            id: "onboarding-proxy-token",
                            value: proxy_token(),
                            oninput: move |v| proxy_token.set(v),
                            input_type: "password",
                            autocomplete: "off",
                            disabled: busy,
                        }
                        details {
                            class: "onboarding-advanced",
                            open: !prefill.remote_host.is_empty() || !prefill.remote_port.is_empty(),
                            summary { "Advanced: remote override" }
                            FormField {
                                label: "Remote host (optional)",
                                id: "onboarding-remote-host",
                                value: remote_host(),
                                oninput: move |v| remote_host.set(v),
                                placeholder: "Defaults to IMAP host",
                                autocomplete: "off",
                                disabled: busy,
                            }
                            FormField {
                                label: "Remote port (optional)",
                                id: "onboarding-remote-port",
                                value: remote_port(),
                                oninput: move |v| remote_port.set(v),
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

                    if let Some(msg) = status_message() {
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

                    div {
                        class: "onboarding-actions",
                        button {
                            r#type: "button",
                            class: "onboarding-btn onboarding-btn-secondary",
                            disabled: busy,
                            onclick: on_test,
                            if matches!(phase(), FormPhase::Testing) {
                                "Testing…"
                            } else {
                                "Test connection"
                            }
                        }
                        button {
                            r#type: "button",
                            class: "onboarding-btn onboarding-btn-primary",
                            disabled: busy,
                            onclick: on_save,
                            if matches!(phase(), FormPhase::Saving) {
                                "Connecting…"
                            } else {
                                "Save & continue"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StatusLevel {
    Info,
    Success,
    Error,
}

#[derive(Clone, PartialEq, Eq)]
struct StatusMessage {
    level: StatusLevel,
    title: Option<String>,
    body: String,
}

impl StatusMessage {
    fn info(body: impl Into<String>) -> Self {
        Self {
            level: StatusLevel::Info,
            title: None,
            body: body.into(),
        }
    }
    fn success(body: impl Into<String>) -> Self {
        Self {
            level: StatusLevel::Success,
            title: None,
            body: body.into(),
        }
    }
    fn error(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            level: StatusLevel::Error,
            title: Some(title.into()),
            body: body.into(),
        }
    }
}

fn kind_label(kind: ConnectErrorKind) -> &'static str {
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
fn email_to_imap_host_hint(email: &str) -> String {
    email
        .rsplit_once('@')
        .map(|(_, domain)| domain.trim())
        .filter(|d| !d.is_empty() && d.contains('.'))
        .map(|d| format!("imap.{d}"))
        .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
fn build_config_from_form(
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
        created_at: now,
        updated_at: now,
    };

    config
        .validate()
        .map_err(|e| format!("Invalid settings: {e}"))?;

    Ok(config)
}

#[component]
fn FormField(
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
