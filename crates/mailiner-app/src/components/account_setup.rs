//! Multi-step account setup used by first-run onboarding and add-account.

use chrono::Utc;
use dioxus::logger::tracing::warn;
use dioxus::prelude::*;
use uuid::Uuid;

use crate::AccountStoreContext;
use crate::AppBootstrapState;
use crate::Route;
use crate::account::AccountId;
use crate::account_config::DEFAULT_SMTP_PORT;
use crate::account_config::{
    AuthKind, Oauth2Provider, Oauth2Tokens, SmtpTlsMode, dev_form_prefill,
    imap_tls_mode_from_legacy, tls_mode_from_legacy,
};
use crate::account_vault::{MIN_PASSPHRASE_CHARS, VaultState};
use crate::components::account_form::{
    AccountIdentityFields, AccountImapFields, AccountOauthFields, AccountProxyFields,
    AccountSmtpFields, FormAuth, FormField, FormPhase, FormStatusBanner, LookupEditGuard,
    StatusMessage, apply_form_auth, build_config_from_form, kind_label, provide_lookup_edit_guard,
    start_server_lookup,
};
use crate::components::wizard::WizardShell;
use crate::connection::ConnectionState;
use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::provider_preset::PresetFormFields;
use crate::setup_wizard::{
    SetupMode, SetupStep, include_proxy_step, last_used_proxy_source, setup_steps,
};

/// First-run or add-account wizard.
#[component]
pub fn AccountSetupWizard(mode: SetupMode) -> Element {
    let mut bootstrap = use_context::<Signal<AppBootstrapState>>();
    let mut ctx = use_context::<AppContext>();
    let store_ctx = use_context::<Signal<Option<AccountStoreContext>>>();
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let nav = use_navigator();

    let prefill = use_hook(dev_form_prefill);
    provide_lookup_edit_guard();
    let guard = use_context::<LookupEditGuard>();

    let account_id = use_hook(|| AccountId::new(Uuid::new_v4().to_string()));
    let account_id_effect = account_id.clone();
    let account_id_save = account_id.clone();

    let mut model = use_account_form_model(&prefill);

    let mut unlock_passphrase = use_signal(String::new);
    let mut unlock_passphrase_confirm = use_signal(String::new);
    let mut force_proxy = use_signal(|| false);
    let mut current = use_signal(|| match mode {
        SetupMode::FirstRun => SetupStep::Welcome,
        SetupMode::AddAccount => SetupStep::Email,
    });
    let mut phase = use_signal(|| FormPhase::Idle);
    let mut status_message = use_signal(|| None::<StatusMessage>);
    let mut save_seen_progress = use_signal(|| false);

    use_future(move || async move {
        if mode != SetupMode::AddAccount {
            return;
        }
        if !model.proxy_base_url.peek().trim().is_empty() {
            return;
        }
        let Some(store) = store_ctx() else {
            return;
        };
        let Ok(list) = store.0.list().await else {
            return;
        };
        let preferred = ctx.selected_account.peek().clone();
        let Some(src) = last_used_proxy_source(&list, preferred.as_ref()) else {
            return;
        };
        model.proxy_base_url.set(src.proxy.base_url.clone());
        model.proxy_token.set(src.proxy.token.clone());
        model
            .remote_host
            .set(src.proxy.remote_host.clone().unwrap_or_default());
        model.remote_port.set(
            src.proxy
                .remote_port
                .map(|p| p.to_string())
                .unwrap_or_default(),
        );
        if let Some(smtp) = src.smtp.as_ref() {
            model
                .smtp_remote_host
                .set(smtp.remote_host.clone().unwrap_or_default());
            model
                .smtp_remote_port
                .set(smtp.remote_port.map(|p| p.to_string()).unwrap_or_default());
        }
    });

    use_future(move || async move {
        match crate::oauth::complete_same_tab_signin().await {
            Ok(Some(tokens)) => {
                model.oauth_tokens.set(Some(tokens));
                model.auth_kind.set(AuthKind::Oauth2);
                current.set(SetupStep::SignIn);
            }
            Ok(None) => {}
            Err(e) => {
                current.set(SetupStep::SignIn);
                status_message.set(Some(StatusMessage::error("OAuth", e.user_message())));
            }
        }
    });

    let ctx_watch = ctx.clone();
    use_effect(move || {
        if phase() != FormPhase::Saving {
            return;
        }
        let states = ctx_watch.connection_states.read().clone();
        let Some(state) = states.get(&account_id_effect) else {
            return;
        };
        match state {
            ConnectionState::Connecting | ConnectionState::Authenticating => {
                save_seen_progress.set(true);
            }
            ConnectionState::Ready => {
                if !save_seen_progress() {
                    return;
                }
                if !ctx_watch.accounts.read().contains_key(&account_id_effect) {
                    return;
                }
                let passphrase = unlock_passphrase();
                let store = store_ctx();
                phase.set(FormPhase::Idle);
                save_seen_progress.set(false);
                status_message.set(None);
                match mode {
                    SetupMode::FirstRun => {
                        if !passphrase.is_empty()
                            && let Some(store) = store
                            && store.0.vault_state() == VaultState::Plaintext
                        {
                            spawn(async move {
                                if let Err(e) = store.0.set_passphrase(&passphrase).await {
                                    warn!("post-commit set_passphrase failed: {e}");
                                }
                                bootstrap.set(AppBootstrapState::Ready);
                            });
                        } else {
                            bootstrap.set(AppBootstrapState::Ready);
                        }
                    }
                    SetupMode::AddAccount => {
                        nav.replace(Route::AccountsSettingsView {});
                    }
                }
            }
            ConnectionState::Error { message, kind, .. } => {
                if !save_seen_progress() {
                    return;
                }
                phase.set(FormPhase::Idle);
                save_seen_progress.set(false);
                status_message.set(Some(StatusMessage::error(kind_label(*kind), message)));
            }
            _ => {}
        }
    });

    let busy = !matches!(phase(), FormPhase::Idle);
    let steps = setup_steps(
        mode,
        include_proxy_step(&model.proxy_base_url(), force_proxy()),
    );
    let step = if steps.contains(&current()) {
        current()
    } else {
        *steps.last().unwrap_or(&SetupStep::Email)
    };
    let step_index = steps.iter().position(|s| *s == step).unwrap_or(0);
    let step_count = steps.len();
    let show_back = step_index > 0;
    let id_prefix = match mode {
        SetupMode::FirstRun => "onboarding",
        SetupMode::AddAccount => "account-new",
    };
    let eyebrow = match mode {
        SetupMode::FirstRun => crate::i18n::t("wizard.eyebrow_setup"),
        SetupMode::AddAccount => crate::i18n::t("wizard.eyebrow_add"),
    };
    let (title, subtitle) = step_copy(step);
    let primary_label = if matches!(phase(), FormPhase::Saving) {
        crate::i18n::t("wizard.connecting")
    } else {
        match step {
            SetupStep::Welcome => crate::i18n::t("wizard.get_started"),
            SetupStep::Review => crate::i18n::t("wizard.connect"),
            _ => crate::i18n::t("wizard.continue"),
        }
    };

    let on_back = {
        let steps = steps.clone();
        move |_| {
            if busy {
                return;
            }
            status_message.set(None);
            if let Some(prev) = step_index.checked_sub(1) {
                current.set(steps[prev]);
            }
        }
    };

    let on_primary = {
        let steps = steps.clone();
        move |_| {
            if busy {
                return;
            }
            status_message.set(None);
            if let Err(msg) = validate_step(
                step,
                &model,
                unlock_passphrase(),
                unlock_passphrase_confirm(),
            ) {
                status_message.set(Some(StatusMessage::error(
                    crate::i18n::t("onboarding.validation"),
                    &msg,
                )));
                return;
            }
            if step == SetupStep::Email {
                trigger_lookup(model, guard, busy);
            }
            if step == SetupStep::Review {
                match model.build(&account_id_save) {
                    Ok(config) => {
                        ctx.connection_states
                            .write()
                            .insert(account_id_save.clone(), ConnectionState::Connecting);
                        save_seen_progress.set(true);
                        phase.set(FormPhase::Saving);
                        status_message.set(Some(StatusMessage::info(crate::i18n::t(
                            "wizard.connecting",
                        ))));
                        core_tx.send(CoreEvent::CommitNewAccount { config });
                    }
                    Err(msg) => {
                        status_message.set(Some(StatusMessage::error(
                            crate::i18n::t("onboarding.validation"),
                            &msg,
                        )));
                    }
                }
                return;
            }
            if let Some(next) = steps.get(step_index + 1) {
                current.set(*next);
            }
        }
    };

    let go_proxy = EventHandler::new(move |_| {
        force_proxy.set(true);
        current.set(SetupStep::Proxy);
        status_message.set(None);
    });

    rsx! {
        main {
            class: "bootstrap-shell onboarding-shell",
            div {
                class: "bootstrap-card onboarding-card",
                WizardShell {
                    title: title,
                    subtitle: subtitle,
                    step_index: step_index,
                    step_count: step_count,
                    primary_label: primary_label,
                    show_back: show_back,
                    busy: busy,
                    eyebrow: eyebrow,
                    on_back: on_back,
                    on_primary: on_primary,
                    {step_body(
                        step,
                        id_prefix,
                        model,
                        busy,
                        unlock_passphrase,
                        unlock_passphrase_confirm,
                        go_proxy,
                    )}
                    FormStatusBanner { message: status_message() }
                }
                if mode == SetupMode::AddAccount {
                    nav {
                        class: "bootstrap-nav",
                        Link { to: Route::AccountsSettingsView {}, "Back to accounts" }
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct AccountFormModel {
    display_name: Signal<String>,
    email: Signal<String>,
    imap_host: Signal<String>,
    imap_port: Signal<String>,
    imap_username: Signal<String>,
    imap_password: Signal<String>,
    imap_tls_mode: Signal<crate::account_config::ImapTlsMode>,
    proxy_base_url: Signal<String>,
    proxy_token: Signal<String>,
    remote_host: Signal<String>,
    remote_port: Signal<String>,
    smtp_host: Signal<String>,
    smtp_port: Signal<String>,
    smtp_username: Signal<String>,
    smtp_password: Signal<String>,
    smtp_tls_mode: Signal<SmtpTlsMode>,
    smtp_remote_host: Signal<String>,
    smtp_remote_port: Signal<String>,
    smtp_open: Signal<bool>,
    extra_ca_pems: Signal<String>,
    signature: Signal<String>,
    auth_kind: Signal<AuthKind>,
    oauth_provider: Signal<Oauth2Provider>,
    oauth_client_id: Signal<String>,
    oauth_tenant: Signal<String>,
    oauth_tokens: Signal<Option<Oauth2Tokens>>,
}

fn use_account_form_model(prefill: &crate::account_config::DevFormPrefill) -> AccountFormModel {
    AccountFormModel {
        display_name: use_signal(|| prefill.display_name.clone()),
        email: use_signal(|| prefill.email.clone()),
        imap_host: use_signal(|| prefill.imap_host.clone()),
        imap_port: use_signal(|| prefill.imap_port.to_string()),
        imap_username: use_signal(|| prefill.imap_username.clone()),
        imap_password: use_signal(|| prefill.imap_password.clone()),
        imap_tls_mode: use_signal(|| imap_tls_mode_from_legacy(true, prefill.imap_port)),
        proxy_base_url: use_signal(|| prefill.proxy_base_url.clone()),
        proxy_token: use_signal(|| prefill.proxy_token.clone()),
        remote_host: use_signal(|| prefill.remote_host.clone()),
        remote_port: use_signal(|| prefill.remote_port.clone()),
        smtp_host: use_signal(String::new),
        smtp_port: use_signal(|| DEFAULT_SMTP_PORT.to_string()),
        smtp_username: use_signal(String::new),
        smtp_password: use_signal(String::new),
        smtp_tls_mode: use_signal(|| SmtpTlsMode::Implicit),
        smtp_remote_host: use_signal(String::new),
        smtp_remote_port: use_signal(String::new),
        smtp_open: use_signal(|| false),
        extra_ca_pems: use_signal(String::new),
        signature: use_signal(String::new),
        auth_kind: use_signal(|| AuthKind::Password),
        oauth_provider: use_signal(|| Oauth2Provider::Google),
        oauth_client_id: use_signal(String::new),
        oauth_tenant: use_signal(String::new),
        oauth_tokens: use_signal(|| None::<Oauth2Tokens>),
    }
}

impl AccountFormModel {
    fn display_name(self) -> String {
        (self.display_name)()
    }
    fn email(self) -> String {
        (self.email)()
    }
    fn imap_host(self) -> String {
        (self.imap_host)()
    }
    fn imap_port(self) -> String {
        (self.imap_port)()
    }
    fn imap_username(self) -> String {
        (self.imap_username)()
    }
    fn imap_password(self) -> String {
        (self.imap_password)()
    }
    fn imap_tls_mode(self) -> crate::account_config::ImapTlsMode {
        (self.imap_tls_mode)()
    }
    fn proxy_base_url(self) -> String {
        (self.proxy_base_url)()
    }
    fn proxy_token(self) -> String {
        (self.proxy_token)()
    }
    fn remote_host(self) -> String {
        (self.remote_host)()
    }
    fn remote_port(self) -> String {
        (self.remote_port)()
    }
    fn smtp_host(self) -> String {
        (self.smtp_host)()
    }
    fn smtp_port(self) -> String {
        (self.smtp_port)()
    }
    fn smtp_username(self) -> String {
        (self.smtp_username)()
    }
    fn smtp_password(self) -> String {
        (self.smtp_password)()
    }
    fn smtp_tls_mode(self) -> SmtpTlsMode {
        (self.smtp_tls_mode)()
    }
    fn smtp_remote_host(self) -> String {
        (self.smtp_remote_host)()
    }
    fn smtp_remote_port(self) -> String {
        (self.smtp_remote_port)()
    }
    fn smtp_open(self) -> bool {
        (self.smtp_open)()
    }
    fn extra_ca_pems(self) -> String {
        (self.extra_ca_pems)()
    }
    fn signature(self) -> String {
        (self.signature)()
    }
    fn auth_kind(self) -> AuthKind {
        (self.auth_kind)()
    }
    fn oauth_provider(self) -> Oauth2Provider {
        (self.oauth_provider)()
    }
    fn oauth_client_id(self) -> String {
        (self.oauth_client_id)()
    }
    fn oauth_tenant(self) -> String {
        (self.oauth_tenant)()
    }
    fn oauth_tokens(self) -> Option<Oauth2Tokens> {
        (self.oauth_tokens)()
    }

    fn preset_fields(self) -> PresetFormFields {
        PresetFormFields {
            imap_host: self.imap_host(),
            imap_port: self.imap_port(),
            imap_username: self.imap_username(),
            smtp_host: self.smtp_host(),
            smtp_port: self.smtp_port(),
            smtp_username: self.smtp_username(),
            smtp_use_tls: self.smtp_tls_mode() != SmtpTlsMode::None,
        }
    }

    fn current_auth(self) -> FormAuth {
        FormAuth {
            kind: self.auth_kind(),
            provider: self.oauth_provider(),
            client_id: self.oauth_client_id(),
            tenant: self.oauth_tenant(),
            tokens: self.oauth_tokens(),
        }
    }

    fn build(self, account_id: &AccountId) -> Result<crate::account_config::AccountConfig, String> {
        build_config_from_form(
            account_id,
            &self.display_name(),
            &self.email(),
            &self.imap_host(),
            &self.imap_port(),
            &self.imap_username(),
            &self.imap_password(),
            self.imap_tls_mode(),
            &self.proxy_base_url(),
            &self.proxy_token(),
            &self.remote_host(),
            &self.remote_port(),
            &self.smtp_host(),
            &self.smtp_port(),
            &self.smtp_username(),
            &self.smtp_password(),
            self.smtp_tls_mode(),
            &self.smtp_remote_host(),
            &self.smtp_remote_port(),
            &self.signature(),
            &self.extra_ca_pems(),
            Utc::now(),
        )
        .and_then(|c| apply_form_auth(c, &self.current_auth()))
    }
}

fn trigger_lookup(mut model: AccountFormModel, guard: LookupEditGuard, busy: bool) {
    start_server_lookup(
        model.email(),
        model.preset_fields(),
        true,
        busy,
        guard.hosts_dirty,
        guard.lookup_gen,
        guard.lookup_status,
        guard.last_discovered,
        EventHandler::new(move |v| model.imap_host.set(v)),
        EventHandler::new(move |v| model.imap_port.set(v)),
        EventHandler::new(move |v| model.imap_username.set(v)),
        EventHandler::new(move |v| model.smtp_host.set(v)),
        EventHandler::new(move |v| model.smtp_port.set(v)),
        EventHandler::new(move |v| model.smtp_username.set(v)),
        EventHandler::new(move |v| {
            let port = model.smtp_port().parse().unwrap_or(465);
            model.smtp_tls_mode.set(tls_mode_from_legacy(v, port));
        }),
        EventHandler::new(move |v| model.smtp_open.set(v)),
    );
}

fn validate_step(
    step: SetupStep,
    model: &AccountFormModel,
    passphrase: String,
    confirm: String,
) -> Result<(), String> {
    match step {
        SetupStep::Welcome => Ok(()),
        SetupStep::Email => {
            if model.display_name.peek().trim().is_empty() {
                return Err(crate::i18n::t("wizard.validation_name"));
            }
            let email = model.email.peek();
            if email.trim().is_empty() || !email.contains('@') {
                return Err(crate::i18n::t("wizard.validation_email"));
            }
            Ok(())
        }
        SetupStep::SignIn => match model.auth_kind() {
            AuthKind::Password => {
                if model.imap_password.peek().is_empty() {
                    Err(crate::i18n::t("wizard.validation_password"))
                } else {
                    Ok(())
                }
            }
            AuthKind::Oauth2 => {
                if model
                    .oauth_tokens
                    .peek()
                    .as_ref()
                    .is_some_and(Oauth2Tokens::access_token_nonempty)
                {
                    Ok(())
                } else {
                    Err(crate::i18n::t("wizard.validation_oauth"))
                }
            }
        },
        SetupStep::Servers => {
            if model.imap_host.peek().trim().is_empty() {
                return Err(crate::i18n::t("wizard.validation_imap_host"));
            }
            let port: Result<u16, _> = model.imap_port.peek().trim().parse();
            if !matches!(port, Ok(p) if p > 0) {
                return Err(crate::i18n::t("wizard.validation_imap_port"));
            }
            if model.imap_username.peek().trim().is_empty() {
                return Err(crate::i18n::t("wizard.validation_imap_user"));
            }
            Ok(())
        }
        SetupStep::Proxy => {
            if model.proxy_base_url.peek().trim().is_empty() {
                Err(crate::i18n::t("wizard.validation_proxy"))
            } else {
                Ok(())
            }
        }
        SetupStep::Unlock => {
            if passphrase.is_empty() && confirm.is_empty() {
                return Ok(());
            }
            if passphrase != confirm {
                return Err(crate::i18n::t("onboarding.passphrase_mismatch"));
            }
            if passphrase.chars().count() < MIN_PASSPHRASE_CHARS {
                return Err(crate::i18n::t_args(
                    "onboarding.passphrase_too_short",
                    &[("n", &MIN_PASSPHRASE_CHARS.to_string())],
                ));
            }
            Ok(())
        }
        SetupStep::Review => Ok(()),
    }
}

fn step_copy(step: SetupStep) -> (String, String) {
    match step {
        SetupStep::Welcome => (
            crate::i18n::t("wizard.welcome_title"),
            crate::i18n::t("wizard.welcome_body"),
        ),
        SetupStep::Email => (
            crate::i18n::t("wizard.email_title"),
            crate::i18n::t("wizard.email_subtitle"),
        ),
        SetupStep::SignIn => (
            crate::i18n::t("wizard.signin_title"),
            crate::i18n::t("wizard.signin_subtitle"),
        ),
        SetupStep::Servers => (
            crate::i18n::t("wizard.servers_title"),
            crate::i18n::t("wizard.servers_subtitle"),
        ),
        SetupStep::Proxy => (
            crate::i18n::t("wizard.proxy_title"),
            crate::i18n::t("wizard.proxy_subtitle"),
        ),
        SetupStep::Unlock => (
            crate::i18n::t("wizard.unlock_title"),
            crate::i18n::t("wizard.unlock_subtitle"),
        ),
        SetupStep::Review => (
            crate::i18n::t("wizard.review_title"),
            crate::i18n::t("wizard.review_subtitle"),
        ),
    }
}

fn step_body(
    step: SetupStep,
    id_prefix: &'static str,
    mut model: AccountFormModel,
    busy: bool,
    mut unlock_passphrase: Signal<String>,
    mut unlock_passphrase_confirm: Signal<String>,
    go_proxy: EventHandler<()>,
) -> Element {
    match step {
        SetupStep::Welcome => rsx! {},
        SetupStep::Email => rsx! {
            AccountIdentityFields {
                id_prefix: id_prefix,
                display_name: model.display_name(),
                email: model.email(),
                imap_host: model.imap_host(),
                imap_port: model.imap_port(),
                imap_username: model.imap_username(),
                smtp_host: model.smtp_host(),
                smtp_port: model.smtp_port(),
                smtp_username: model.smtp_username(),
                smtp_use_tls: model.smtp_tls_mode() != SmtpTlsMode::None,
                set_display_name: move |v| model.display_name.set(v),
                set_email: move |v| model.email.set(v),
                set_imap_host: move |v| model.imap_host.set(v),
                set_imap_port: move |v| model.imap_port.set(v),
                set_imap_username: move |v| model.imap_username.set(v),
                set_smtp_host: move |v| model.smtp_host.set(v),
                set_smtp_port: move |v| model.smtp_port.set(v),
                set_smtp_username: move |v| model.smtp_username.set(v),
                set_smtp_use_tls: move |v| {
                    let port = model.smtp_port().parse().unwrap_or(465);
                    model.smtp_tls_mode.set(tls_mode_from_legacy(v, port));
                },
                set_smtp_open: move |v| model.smtp_open.set(v),
                busy: busy,
                show_lookup_button: false,
            }
        },
        SetupStep::SignIn => rsx! {
            if model.auth_kind() == AuthKind::Password {
                FormField {
                    label: crate::i18n::t("account_form.password"),
                    id: "{id_prefix}-imap-password",
                    value: model.imap_password(),
                    oninput: move |v| model.imap_password.set(v),
                    input_type: "password",
                    autocomplete: "current-password",
                    disabled: busy,
                }
            }
            AccountOauthFields {
                id_prefix: id_prefix,
                auth_kind: model.auth_kind(),
                set_auth_kind: move |v| model.auth_kind.set(v),
                provider: model.oauth_provider(),
                set_provider: move |v| model.oauth_provider.set(v),
                client_id: model.oauth_client_id(),
                set_client_id: move |v| model.oauth_client_id.set(v),
                tenant: model.oauth_tenant(),
                set_tenant: move |v| model.oauth_tenant.set(v),
                tokens: model.oauth_tokens(),
                set_tokens: move |v| model.oauth_tokens.set(v),
                imap_host: model.imap_host(),
                busy: busy,
            }
        },
        SetupStep::Servers => rsx! {
            AccountImapFields {
                id_prefix: id_prefix,
                email: model.email(),
                imap_host: model.imap_host(),
                imap_port: model.imap_port(),
                imap_username: model.imap_username(),
                imap_password: model.imap_password(),
                imap_tls_mode: model.imap_tls_mode(),
                set_imap_host: move |v| model.imap_host.set(v),
                set_imap_port: move |v| model.imap_port.set(v),
                set_imap_username: move |v| model.imap_username.set(v),
                set_imap_password: move |v| model.imap_password.set(v),
                set_imap_tls_mode: move |v| model.imap_tls_mode.set(v),
                busy: busy,
                hide_imap_password: true,
                compact_tls: true,
            }
            AccountSmtpFields {
                id_prefix: id_prefix,
                smtp_host: model.smtp_host(),
                smtp_port: model.smtp_port(),
                smtp_username: model.smtp_username(),
                smtp_password: model.smtp_password(),
                smtp_tls_mode: model.smtp_tls_mode(),
                set_smtp_host: move |v| model.smtp_host.set(v),
                set_smtp_port: move |v| model.smtp_port.set(v),
                set_smtp_username: move |v| model.smtp_username.set(v),
                set_smtp_password: move |v| model.smtp_password.set(v),
                set_smtp_tls_mode: move |v| model.smtp_tls_mode.set(v),
                busy: busy,
                open: model.smtp_open() || !model.smtp_host().trim().is_empty(),
                hide_password: true,
            }
        },
        SetupStep::Proxy => rsx! {
            AccountProxyFields {
                id_prefix: id_prefix,
                proxy_base_url: model.proxy_base_url(),
                proxy_token: model.proxy_token(),
                remote_host: model.remote_host(),
                remote_port: model.remote_port(),
                smtp_remote_host: model.smtp_remote_host(),
                smtp_remote_port: model.smtp_remote_port(),
                set_proxy_base_url: move |v| model.proxy_base_url.set(v),
                set_proxy_token: move |v| model.proxy_token.set(v),
                set_remote_host: move |v| model.remote_host.set(v),
                set_remote_port: move |v| model.remote_port.set(v),
                set_smtp_remote_host: move |v| model.smtp_remote_host.set(v),
                set_smtp_remote_port: move |v| model.smtp_remote_port.set(v),
                busy: busy,
                open_advanced: !model.remote_host().is_empty() || !model.remote_port().is_empty(),
                show_intro: false,
                show_disclosure: false,
            }
        },
        SetupStep::Unlock => rsx! {
            p { class: "bootstrap-muted", {crate::i18n::t("wizard.unlock_skip")} }
            FormField {
                label: crate::i18n::t("onboarding.unlock_passphrase"),
                id: "{id_prefix}-unlock-passphrase",
                value: unlock_passphrase(),
                oninput: move |v| unlock_passphrase.set(v),
                input_type: "password",
                autocomplete: "new-password",
                disabled: busy,
            }
            FormField {
                label: crate::i18n::t("onboarding.confirm_passphrase"),
                id: "{id_prefix}-unlock-passphrase-confirm",
                value: unlock_passphrase_confirm(),
                oninput: move |v| unlock_passphrase_confirm.set(v),
                input_type: "password",
                autocomplete: "new-password",
                disabled: busy,
            }
        },
        SetupStep::Review => {
            let smtp = model.smtp_host();
            let smtp_line = if smtp.trim().is_empty() {
                crate::i18n::t("wizard.review_smtp_none")
            } else {
                format!("{}:{}", smtp.trim(), model.smtp_port())
            };
            let imap_line = format!("{}:{}", model.imap_host(), model.imap_port());
            rsx! {
                ul {
                    class: "wizard-review",
                    li {
                        span { class: "wizard-review-label", {crate::i18n::t("wizard.review_name")} }
                        span { class: "wizard-review-value", "{model.display_name()}" }
                    }
                    li {
                        span { class: "wizard-review-label", {crate::i18n::t("wizard.review_email")} }
                        span { class: "wizard-review-value", "{model.email()}" }
                    }
                    li {
                        span { class: "wizard-review-label", {crate::i18n::t("wizard.review_imap")} }
                        span { class: "wizard-review-value", "{imap_line}" }
                    }
                    li {
                        span { class: "wizard-review-label", {crate::i18n::t("wizard.review_smtp")} }
                        span { class: "wizard-review-value", "{smtp_line}" }
                    }
                    li {
                        span { class: "wizard-review-label", {crate::i18n::t("wizard.review_proxy")} }
                        span { class: "wizard-review-value", "{model.proxy_base_url()}" }
                    }
                }
                button {
                    r#type: "button",
                    class: "onboarding-btn onboarding-btn-secondary",
                    disabled: busy,
                    onclick: move |_| go_proxy.call(()),
                    {crate::i18n::t("wizard.change_connection")}
                }
            }
        }
    }
}
