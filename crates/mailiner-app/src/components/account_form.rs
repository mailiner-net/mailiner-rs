//! Shared account settings form fields and validation (onboarding + accounts UI).

use chrono::Utc;
use dioxus::prelude::*;
use uuid::Uuid;

use crate::account::AccountId;
use crate::account_config::{
    AccountConfig, ImapSettings, ImapTlsMode, ProxySettings, SmtpTlsMode,
    default_port_for_tls_mode, normalize_signature, optional_smtp_from_tls_mode,
    port_for_imap_tls_mode_change, port_for_tls_mode_change,
};
use crate::autodiscover::{
    DiscoverSource, DiscoveredConfig, apply_discovered, domain_from_email, lookup_servers,
    should_autofill_hosts,
};
use crate::connection::ConnectErrorKind;
use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::provider_preset::{
    PresetFormFields, ProviderPreset, apply_email_change, apply_preset, matching_preset,
};
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

fn parse_optional_override_host(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn parse_optional_override_port(raw: &str, field: &str) -> Result<Option<u16>, String> {
    let t = raw.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let p: u16 = t
        .parse()
        .map_err(|_| format!("{field} must be a number between 1 and 65535."))?;
    if p == 0 {
        return Err(format!("{field} must be a number between 1 and 65535."));
    }
    Ok(Some(p))
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

#[derive(Clone, PartialEq, Eq)]
enum LookupStatus {
    Idle,
    Looking,
    NeedEmail,
    Filled {
        source: DiscoverSource,
        domain: String,
    },
    AlreadySet,
    Failed,
}

impl LookupStatus {
    fn message(&self) -> String {
        match self {
            Self::Idle => String::new(),
            Self::Looking => "Looking up IMAP and SMTP over HTTPS…".into(),
            Self::NeedEmail => "Enter a full email address (user@example.com) first.".into(),
            Self::Filled { source, domain } => source.filled_message(domain),
            Self::AlreadySet => {
                "IMAP and SMTP are already set. Clear a host to fill from lookup.".into()
            }
            Self::Failed => "Could not look up servers. Enter IMAP and SMTP manually.".into(),
        }
    }
}

#[derive(Clone, Copy)]
struct LookupEditGuard {
    hosts_dirty: Signal<bool>,
    lookup_gen: Signal<u64>,
    lookup_status: Signal<LookupStatus>,
}

impl LookupEditGuard {
    fn mark_dirty(mut self) {
        self.hosts_dirty.set(true);
        invalidate_lookup(self.lookup_gen, self.lookup_status);
    }
}

/// Shared by IMAP + SMTP fieldsets so an in-flight lookup cannot overwrite edits.
pub fn provide_lookup_edit_guard() {
    let guard = LookupEditGuard {
        hosts_dirty: use_signal(|| false),
        lookup_gen: use_signal(|| 0u64),
        lookup_status: use_signal(|| LookupStatus::Idle),
    };
    use_context_provider(|| guard);
}

fn invalidate_lookup(mut lookup_gen: Signal<u64>, mut lookup_status: Signal<LookupStatus>) {
    let next = lookup_gen.peek().saturating_add(1);
    lookup_gen.set(next);
    lookup_status.set(LookupStatus::Idle);
}

#[allow(clippy::too_many_arguments)]
fn start_server_lookup(
    email: String,
    fields: PresetFormFields,
    force: bool,
    busy: bool,
    mut hosts_dirty: Signal<bool>,
    mut lookup_gen: Signal<u64>,
    mut lookup_status: Signal<LookupStatus>,
    mut last_discovered: Signal<Option<DiscoveredConfig>>,
    set_imap_host: EventHandler<String>,
    set_imap_port: EventHandler<String>,
    set_imap_username: EventHandler<String>,
    set_smtp_host: EventHandler<String>,
    set_smtp_port: EventHandler<String>,
    set_smtp_username: EventHandler<String>,
    set_smtp_use_tls: EventHandler<bool>,
    set_smtp_open: EventHandler<bool>,
) {
    if busy {
        return;
    }
    let Some(domain) = domain_from_email(&email) else {
        if force {
            lookup_status.set(LookupStatus::NeedEmail);
        }
        return;
    };
    if !should_autofill_hosts(&fields, last_discovered.peek().as_ref()) {
        if force {
            lookup_status.set(LookupStatus::AlreadySet);
        }
        return;
    }
    // Clearing a host sets dirty; that edit is the reason we are looking up.
    // A later edit during the request sets dirty again and aborts apply.
    hosts_dirty.set(false);
    let generation = lookup_gen.peek().saturating_add(1);
    lookup_gen.set(generation);
    lookup_status.set(LookupStatus::Looking);
    spawn(async move {
        let result = lookup_servers(&email).await;
        if lookup_gen() != generation {
            return;
        }
        if hosts_dirty() {
            lookup_status.set(LookupStatus::Idle);
            return;
        }
        let Some(cfg) = result else {
            lookup_status.set(LookupStatus::Failed);
            return;
        };
        let last = last_discovered();
        let mut next = fields;
        apply_discovered(&cfg, &email, &mut next, last.as_ref());
        hosts_dirty.set(false);
        last_discovered.set(Some(cfg.clone()));
        lookup_status.set(LookupStatus::Filled {
            source: cfg.source,
            domain,
        });
        let open_smtp = !next.smtp_host.trim().is_empty();
        set_imap_host.call(next.imap_host);
        set_imap_port.call(next.imap_port);
        set_imap_username.call(next.imap_username);
        set_smtp_host.call(next.smtp_host);
        set_smtp_port.call(next.smtp_port);
        set_smtp_username.call(next.smtp_username);
        set_smtp_use_tls.call(next.smtp_use_tls);
        if open_smtp {
            set_smtp_open.call(true);
        }
    });
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
    imap_tls_mode: ImapTlsMode,
    proxy_base_url: &str,
    proxy_token: &str,
    remote_host: &str,
    remote_port: &str,
    smtp_host: &str,
    smtp_port: &str,
    smtp_username: &str,
    smtp_password: &str,
    smtp_tls_mode: SmtpTlsMode,
    smtp_remote_host: &str,
    smtp_remote_port: &str,
    signature: &str,
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

    let remote_host = parse_optional_override_host(remote_host);
    let remote_port = parse_optional_override_port(remote_port, "Remote port")?;
    let smtp_remote_host = parse_optional_override_host(smtp_remote_host);
    let smtp_remote_port = parse_optional_override_port(smtp_remote_port, "SMTP remote port")?;

    let mut smtp = optional_smtp_from_tls_mode(
        smtp_host,
        smtp_port,
        smtp_username,
        smtp_password,
        smtp_tls_mode,
    )?;
    match smtp.as_mut() {
        Some(smtp) => {
            smtp.remote_host = smtp_remote_host;
            smtp.remote_port = smtp_remote_port;
        }
        None if smtp_remote_host.is_some() || smtp_remote_port.is_some() => {
            return Err("SMTP host is required when other SMTP fields are filled. \
                 Clear SMTP fields to skip outbound settings."
                .into());
        }
        None => {}
    }

    let now = Utc::now();
    let config = AccountConfig {
        id: account_id.clone(),
        display_name: display_name.to_string(),
        email: email.to_string(),
        signature: normalize_signature(signature),
        imap: ImapSettings::new(
            host,
            port,
            username.to_string(),
            password.to_string(),
            imap_tls_mode,
        ),
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
        || old.imap.tls_mode != new.imap.tls_mode
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
    phase: Signal<FormPhase>,
) {
    use_drop(move || {
        if let Some(rid) = test_request_id.peek().clone() {
            if *phase.peek() == FormPhase::TestingSmtp {
                ctx.smtp_test_abandoned.write().insert(rid.clone());
            }
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
    #[props(default)] onblur: EventHandler<String>,
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
                onblur: {
                    let value = value.clone();
                    move |_| onblur.call(value.clone())
                },
            }
        }
    }
}

/// Identity + IMAP + proxy fieldsets. Shared by onboarding and accounts.
/// SMTP is a separate [`AccountSmtpFields`] section; host/port/TLS setters
/// are accepted here so a provider preset can fill both sides.
#[component]
pub fn AccountConnectionFields(
    id_prefix: String,
    display_name: String,
    email: String,
    imap_host: String,
    imap_port: String,
    imap_username: String,
    imap_password: String,
    imap_tls_mode: ImapTlsMode,
    proxy_base_url: String,
    proxy_token: String,
    remote_host: String,
    remote_port: String,
    smtp_remote_host: String,
    smtp_remote_port: String,
    smtp_host: String,
    smtp_port: String,
    smtp_username: String,
    smtp_use_tls: bool,
    set_display_name: EventHandler<String>,
    set_email: EventHandler<String>,
    set_imap_host: EventHandler<String>,
    set_imap_port: EventHandler<String>,
    set_imap_username: EventHandler<String>,
    set_imap_password: EventHandler<String>,
    set_imap_tls_mode: EventHandler<ImapTlsMode>,
    set_proxy_base_url: EventHandler<String>,
    set_proxy_token: EventHandler<String>,
    set_remote_host: EventHandler<String>,
    set_remote_port: EventHandler<String>,
    set_smtp_remote_host: EventHandler<String>,
    set_smtp_remote_port: EventHandler<String>,
    set_smtp_host: EventHandler<String>,
    set_smtp_port: EventHandler<String>,
    set_smtp_username: EventHandler<String>,
    set_smtp_use_tls: EventHandler<bool>,
    set_smtp_open: EventHandler<bool>,
    busy: bool,
    #[props(default)] open_advanced: bool,
) -> Element {
    let host_placeholder = email_to_imap_host_hint(&email);
    let warn_starttls = imap_tls_mode == ImapTlsMode::StartTls;
    let warn_plain = imap_tls_mode == ImapTlsMode::None;
    let imap_port_for_tls = imap_port.clone();
    let guard = use_context::<LookupEditGuard>();
    let mut hosts_dirty = guard.hosts_dirty;
    let lookup_gen = guard.lookup_gen;
    let lookup_status = guard.lookup_status;
    let last_discovered = use_signal(|| None::<DiscoveredConfig>);
    let looking = matches!(lookup_status(), LookupStatus::Looking);
    let current_fields = PresetFormFields {
        imap_host: imap_host.clone(),
        imap_port: imap_port.clone(),
        imap_username: imap_username.clone(),
        smtp_host: smtp_host.clone(),
        smtp_port: smtp_port.clone(),
        smtp_username: smtp_username.clone(),
        smtp_use_tls,
    };
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
                value: email.clone(),
                oninput: {
                    let previous_email = email.clone();
                    let imap_username = imap_username.clone();
                    let smtp_username = smtp_username.clone();
                    let smtp_host = smtp_host.clone();
                    move |v: String| {
                        invalidate_lookup(lookup_gen, lookup_status);
                        let mut next = PresetFormFields {
                            imap_host: String::new(),
                            imap_port: String::new(),
                            imap_username: imap_username.clone(),
                            smtp_host: smtp_host.clone(),
                            smtp_port: String::new(),
                            smtp_username: smtp_username.clone(),
                            smtp_use_tls: true,
                        };
                        apply_email_change(&previous_email, &v, &mut next);
                        set_email.call(v);
                        if next.imap_username != imap_username {
                            set_imap_username.call(next.imap_username);
                        }
                        if next.smtp_username != smtp_username {
                            set_smtp_username.call(next.smtp_username);
                        }
                        if next.smtp_host != smtp_host {
                            set_smtp_host.call(next.smtp_host);
                        }
                    }
                },
                onblur: {
                    let fields = current_fields.clone();
                    move |v: String| {
                        start_server_lookup(
                            v,
                            fields.clone(),
                            false,
                            busy,
                            hosts_dirty,
                            lookup_gen,
                            lookup_status,
                            last_discovered,
                            set_imap_host,
                            set_imap_port,
                            set_imap_username,
                            set_smtp_host,
                            set_smtp_port,
                            set_smtp_username,
                            set_smtp_use_tls,
                            set_smtp_open,
                        );
                    }
                },
                input_type: "email",
                autocomplete: "email",
                disabled: busy,
            }
            div {
                class: "onboarding-lookup-row",
                button {
                    r#type: "button",
                    class: "onboarding-btn onboarding-btn-secondary",
                    disabled: busy || looking,
                    onclick: {
                        let email = email.clone();
                        let fields = current_fields.clone();
                        move |_| {
                            start_server_lookup(
                                email.clone(),
                                fields.clone(),
                                true,
                                busy,
                                hosts_dirty,
                                lookup_gen,
                                lookup_status,
                                last_discovered,
                                set_imap_host,
                                set_imap_port,
                                set_imap_username,
                                set_smtp_host,
                                set_smtp_port,
                                set_smtp_username,
                                set_smtp_use_tls,
                                set_smtp_open,
                            );
                        }
                    },
                    if looking {
                        "Looking up…"
                    } else {
                        "Look up servers"
                    }
                }
                p {
                    class: "bootstrap-muted onboarding-lookup-status",
                    role: "status",
                    "{lookup_status().message()}"
                }
            }
            p {
                class: "bootstrap-muted onboarding-preset-hint",
                "Looks up Mozilla ISPDB and common imap./smtp. hosts over HTTPS. \
                 Only the domain is sent. You can edit the result."
            }
        }

        fieldset {
            class: "onboarding-section",
            legend { "IMAP" }
            ProviderPresetSelect {
                id: "{id_prefix}-provider",
                email: email.clone(),
                fields: PresetFormFields {
                    imap_host: imap_host.clone(),
                    imap_port: imap_port.clone(),
                    imap_username: imap_username.clone(),
                    smtp_host: smtp_host.clone(),
                    smtp_port: smtp_port.clone(),
                    smtp_username: smtp_username.clone(),
                    smtp_use_tls,
                },
                on_apply: move |next: PresetFormFields| {
                    hosts_dirty.set(true);
                    let open_smtp = !next.smtp_host.trim().is_empty();
                    set_imap_host.call(next.imap_host);
                    set_imap_port.call(next.imap_port);
                    set_imap_username.call(next.imap_username);
                    set_smtp_host.call(next.smtp_host);
                    set_smtp_port.call(next.smtp_port);
                    set_smtp_username.call(next.smtp_username);
                    set_smtp_use_tls.call(next.smtp_use_tls);
                    if open_smtp {
                        set_smtp_open.call(true);
                    }
                },
                busy: busy,
            }
            FormField {
                label: "Host",
                id: "{id_prefix}-imap-host",
                value: imap_host,
                oninput: move |v| {
                    hosts_dirty.set(true);
                    set_imap_host.call(v);
                },
                placeholder: host_placeholder.clone(),
                autocomplete: "off",
                disabled: busy,
            }
            FormField {
                label: "Port",
                id: "{id_prefix}-imap-port",
                value: imap_port,
                oninput: move |v| {
                    hosts_dirty.set(true);
                    set_imap_port.call(v);
                },
                input_type: "number",
                autocomplete: "off",
                disabled: busy,
            }
            div {
                class: "onboarding-field",
                label {
                    r#for: "{id_prefix}-imap-tls",
                    "TLS mode"
                }
                select {
                    id: "{id_prefix}-imap-tls",
                    name: "{id_prefix}-imap-tls",
                    value: imap_tls_mode.as_form_value(),
                    disabled: busy,
                    onchange: move |e| {
                        let new_mode = ImapTlsMode::from_form_value(&e.value());
                        let next_port = port_for_imap_tls_mode_change(
                            &imap_port_for_tls,
                            imap_tls_mode,
                            new_mode,
                        );
                        if next_port != imap_port_for_tls {
                            set_imap_port.call(next_port);
                        }
                        set_imap_tls_mode.call(new_mode);
                    },
                    option {
                        value: "implicit",
                        selected: imap_tls_mode == ImapTlsMode::Implicit,
                        "Implicit TLS (port 993)"
                    }
                    option {
                        value: "start_tls",
                        selected: imap_tls_mode == ImapTlsMode::StartTls,
                        "STARTTLS (port 143)"
                    }
                    option {
                        value: "none",
                        selected: imap_tls_mode == ImapTlsMode::None,
                        "None (plaintext)"
                    }
                }
            }
            if warn_starttls {
                p {
                    class: "onboarding-notice",
                    role: "note",
                    "STARTTLS (port 143) sends the server greeting and STARTTLS \
                     in the clear, including through the proxy. LOGIN is encrypted \
                     after the upgrade. Prefer implicit TLS on port 993 when the \
                     server supports it."
                }
            }
            if warn_plain {
                p {
                    class: "onboarding-notice",
                    role: "alert",
                    "Plaintext IMAP sends LOGIN and mail in the clear, including \
                     through the proxy. Prefer implicit TLS on port 993 or STARTTLS \
                     on port 143."
                }
            }
            FormField {
                label: "Username",
                id: "{id_prefix}-imap-user",
                value: imap_username,
                oninput: move |v| {
                    invalidate_lookup(lookup_gen, lookup_status);
                    set_imap_username.call(v);
                },
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
                FormField {
                    label: "SMTP remote host (optional)",
                    id: "{id_prefix}-smtp-remote-host",
                    value: smtp_remote_host,
                    oninput: move |v| set_smtp_remote_host.call(v),
                    placeholder: "Defaults to SMTP host",
                    autocomplete: "off",
                    disabled: busy,
                }
                FormField {
                    label: "SMTP remote port (optional)",
                    id: "{id_prefix}-smtp-remote-port",
                    value: smtp_remote_port,
                    oninput: move |v| set_smtp_remote_port.call(v),
                    placeholder: "Defaults to SMTP port",
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

/// Optional plain-text signature (new / reply / forward drafts).
#[component]
pub fn AccountSignatureFields(
    id_prefix: String,
    signature: String,
    set_signature: EventHandler<String>,
    busy: bool,
) -> Element {
    rsx! {
        fieldset {
            class: "onboarding-section",
            legend { "Signature" }
            p {
                class: "bootstrap-muted",
                "Optional plain text appended to new, reply, and forward messages."
            }
            div {
                class: "onboarding-field",
                label {
                    r#for: "{id_prefix}-signature",
                    "Signature"
                }
                textarea {
                    id: "{id_prefix}-signature",
                    name: "{id_prefix}-signature",
                    value: "{signature}",
                    rows: 4,
                    disabled: busy,
                    placeholder: "Jane Doe",
                    oninput: move |e| set_signature.call(e.value()),
                }
            }
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
    smtp_tls_mode: SmtpTlsMode,
    set_smtp_host: EventHandler<String>,
    set_smtp_port: EventHandler<String>,
    set_smtp_username: EventHandler<String>,
    set_smtp_password: EventHandler<String>,
    set_smtp_tls_mode: EventHandler<SmtpTlsMode>,
    busy: bool,
    #[props(default)] open: bool,
) -> Element {
    let port_placeholder = default_port_for_tls_mode(smtp_tls_mode).to_string();
    let warn_starttls = smtp_tls_mode == SmtpTlsMode::StartTls;
    let warn_plain = smtp_tls_mode == SmtpTlsMode::None;
    let smtp_port_for_tls = smtp_port.clone();
    let guard = use_context::<LookupEditGuard>();
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
                    oninput: move |v| {
                        guard.mark_dirty();
                        set_smtp_host.call(v);
                    },
                    placeholder: "smtp.example.com",
                    autocomplete: "off",
                    disabled: busy,
                }
                FormField {
                    label: "SMTP port",
                    id: "{id_prefix}-smtp-port",
                    value: smtp_port,
                    oninput: move |v| {
                        guard.mark_dirty();
                        set_smtp_port.call(v);
                    },
                    placeholder: port_placeholder,
                    input_type: "number",
                    autocomplete: "off",
                    disabled: busy,
                }
                FormField {
                    label: "SMTP username",
                    id: "{id_prefix}-smtp-user",
                    value: smtp_username,
                    oninput: move |v| {
                        invalidate_lookup(guard.lookup_gen, guard.lookup_status);
                        set_smtp_username.call(v);
                    },
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
                    class: "onboarding-field",
                    label {
                        r#for: "{id_prefix}-smtp-tls",
                        "TLS mode"
                    }
                    select {
                        id: "{id_prefix}-smtp-tls",
                        name: "{id_prefix}-smtp-tls",
                        value: smtp_tls_mode.as_form_value(),
                        disabled: busy,
                        onchange: move |e| {
                            guard.mark_dirty();
                            let new_mode = SmtpTlsMode::from_form_value(&e.value());
                            let next_port = port_for_tls_mode_change(
                                &smtp_port_for_tls,
                                smtp_tls_mode,
                                new_mode,
                            );
                            if next_port != smtp_port_for_tls {
                                set_smtp_port.call(next_port);
                            }
                            set_smtp_tls_mode.call(new_mode);
                        },
                        option {
                            value: "implicit",
                            selected: smtp_tls_mode == SmtpTlsMode::Implicit,
                            "Implicit TLS (port 465)"
                        }
                        option {
                            value: "start_tls",
                            selected: smtp_tls_mode == SmtpTlsMode::StartTls,
                            "STARTTLS (port 587)"
                        }
                        option {
                            value: "none",
                            selected: smtp_tls_mode == SmtpTlsMode::None,
                            "None (plaintext)"
                        }
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

/// Provider dropdown. Selecting a named preset fills IMAP/SMTP host/port/TLS
/// and usernames-from-email when empty; email and passwords stay untouched.
#[component]
fn ProviderPresetSelect(
    id: String,
    email: String,
    fields: PresetFormFields,
    on_apply: EventHandler<PresetFormFields>,
    busy: bool,
) -> Element {
    // Independent of field matching so Custom stays selected after a named fill.
    let mut chosen = use_signal(|| matching_preset(&fields));
    let matched = matching_preset(&fields);
    let selected = if chosen() != ProviderPreset::Custom && matched != chosen() {
        ProviderPreset::Custom
    } else {
        chosen()
    };
    rsx! {
        div {
            class: "onboarding-field",
            label {
                r#for: "{id}",
                "Provider"
            }
            select {
                id: "{id}",
                name: "{id}",
                value: "{selected.as_key()}",
                disabled: busy,
                onchange: move |e| {
                    let preset = ProviderPreset::from_key(&e.value());
                    chosen.set(preset);
                    let mut next = fields.clone();
                    apply_preset(preset, &email, &mut next);
                    on_apply.call(next);
                },
                for option in ProviderPreset::ALL {
                    option {
                        value: "{option.as_key()}",
                        selected: *option == selected,
                        "{option.label()}"
                    }
                }
            }
            p {
                class: "bootstrap-muted onboarding-preset-hint",
                "Fills IMAP and SMTP host, port, and TLS. Email and password are not changed."
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

#[cfg(test)]
mod tests {
    use super::*;

    fn form_config(
        imap_port: &str,
        imap_tls_mode: ImapTlsMode,
        smtp_host: &str,
        smtp_port: &str,
        smtp_tls_mode: SmtpTlsMode,
    ) -> Result<AccountConfig, String> {
        form_config_with_remotes(
            imap_port,
            imap_tls_mode,
            smtp_host,
            smtp_port,
            smtp_tls_mode,
            "",
            "",
        )
    }

    fn form_config_with_remotes(
        imap_port: &str,
        imap_tls_mode: ImapTlsMode,
        smtp_host: &str,
        smtp_port: &str,
        smtp_tls_mode: SmtpTlsMode,
        smtp_remote_host: &str,
        smtp_remote_port: &str,
    ) -> Result<AccountConfig, String> {
        build_config_from_form(
            &AccountId::new("550e8400-e29b-41d4-a716-446655440000"),
            "Work",
            "user@example.com",
            "imap.example.com",
            imap_port,
            "user@example.com",
            "secret",
            imap_tls_mode,
            "ws://localhost:9400/proxy",
            "token",
            "",
            "",
            smtp_host,
            smtp_port,
            "",
            "",
            smtp_tls_mode,
            smtp_remote_host,
            smtp_remote_port,
            "",
            Utc::now(),
        )
    }

    fn form(
        smtp_host: &str,
        smtp_remote_host: &str,
        smtp_remote_port: &str,
    ) -> Result<AccountConfig, String> {
        form_config_with_remotes(
            "993",
            ImapTlsMode::Implicit,
            smtp_host,
            "465",
            SmtpTlsMode::Implicit,
            smtp_remote_host,
            smtp_remote_port,
        )
    }

    fn smtp_form_config(
        smtp_host: &str,
        smtp_port: &str,
        smtp_tls_mode: SmtpTlsMode,
    ) -> Result<AccountConfig, String> {
        form_config(
            "993",
            ImapTlsMode::Implicit,
            smtp_host,
            smtp_port,
            smtp_tls_mode,
        )
    }

    fn imap_form_config(
        imap_port: &str,
        imap_tls_mode: ImapTlsMode,
    ) -> Result<AccountConfig, String> {
        form_config(imap_port, imap_tls_mode, "", "", SmtpTlsMode::Implicit)
    }

    #[test]
    fn form_implicit_tls_writes_tls_mode_and_use_tls() {
        let smtp = smtp_form_config("smtp.example.com", "465", SmtpTlsMode::Implicit)
            .unwrap()
            .smtp
            .expect("Some");
        assert_eq!(smtp.tls_mode, SmtpTlsMode::Implicit);
        assert!(smtp.use_tls);
        assert_eq!(smtp.port, 465);
        let imap = imap_form_config("993", ImapTlsMode::Implicit).unwrap().imap;
        assert_eq!(imap.tls_mode, ImapTlsMode::Implicit);
        assert!(imap.use_tls);
        assert_eq!(imap.port, 993);
    }

    #[test]
    fn form_starttls_writes_mode_directly_not_from_port() {
        let smtp = smtp_form_config("smtp.example.com", "465", SmtpTlsMode::StartTls)
            .unwrap()
            .smtp
            .expect("Some");
        assert_eq!(smtp.tls_mode, SmtpTlsMode::StartTls);
        assert!(smtp.use_tls);
        assert_eq!(smtp.port, 465);
        let imap = imap_form_config("993", ImapTlsMode::StartTls).unwrap().imap;
        assert_eq!(imap.tls_mode, ImapTlsMode::StartTls);
        assert!(imap.use_tls);
        assert_eq!(imap.port, 993);
    }

    #[test]
    fn form_none_writes_plaintext() {
        let smtp = smtp_form_config("smtp.example.com", "25", SmtpTlsMode::None)
            .unwrap()
            .smtp
            .expect("Some");
        assert_eq!(smtp.tls_mode, SmtpTlsMode::None);
        assert!(!smtp.use_tls);
        assert_eq!(smtp.port, 25);
        let imap = imap_form_config("143", ImapTlsMode::None).unwrap().imap;
        assert_eq!(imap.tls_mode, ImapTlsMode::None);
        assert!(!imap.use_tls);
        assert_eq!(imap.port, 143);
    }

    #[test]
    fn form_empty_smtp_section_is_none() {
        let config = smtp_form_config("", "465", SmtpTlsMode::Implicit).unwrap();
        assert!(config.smtp.is_none());
    }

    #[test]
    fn smtp_remotes_persist_on_settings() {
        let cfg = form("smtp.example.com", "smtp-backend.internal", "2525").unwrap();
        let smtp = cfg.smtp.expect("smtp");
        assert_eq!(smtp.host, "smtp.example.com");
        assert_eq!(smtp.port, 465);
        assert_eq!(smtp.remote_host.as_deref(), Some("smtp-backend.internal"));
        assert_eq!(smtp.remote_port, Some(2525));
        let url = cfg.proxy.websocket_url_for_smtp(&smtp).unwrap();
        assert!(url.contains("remote=smtp-backend.internal:2525"), "{url}");
        assert!(!url.contains("smtp.example.com"));
    }

    #[test]
    fn smtp_remotes_empty_stay_none() {
        let cfg = form("smtp.example.com", "  ", "").unwrap();
        let smtp = cfg.smtp.expect("smtp");
        assert!(smtp.remote_host.is_none());
        assert!(smtp.remote_port.is_none());
        assert_eq!(smtp.dial_host(), "smtp.example.com");
        assert_eq!(smtp.dial_port(), 465);
    }

    #[test]
    fn smtp_remote_without_host_is_error() {
        let err = form("", "smtp-backend.internal", "").unwrap_err();
        assert!(err.contains("SMTP host"), "{err}");
    }

    #[test]
    fn smtp_remote_port_must_be_valid() {
        let err = form("smtp.example.com", "", "0").unwrap_err();
        assert!(err.contains("SMTP remote port"), "{err}");
        let err = form("smtp.example.com", "", "nope").unwrap_err();
        assert!(err.contains("SMTP remote port"), "{err}");
    }
}
