//! First-run onboarding form: connect-before-persist via `CommitNewAccount`.

use chrono::Utc;
use dioxus::logger::tracing::warn;
use dioxus::prelude::*;
use uuid::Uuid;

use crate::AccountStoreContext;
use crate::AppBootstrapState;
use crate::account::AccountId;
use crate::account_config::DEFAULT_SMTP_PORT;
use crate::account_config::{SmtpTlsMode, dev_form_prefill, imap_tls_mode_from_legacy};
use crate::account_vault::{MIN_PASSPHRASE_CHARS, VaultState};
use crate::components::account_form::{
    AccountConnectionFields, AccountSignatureFields, AccountSmtpFields, FormField, FormPhase,
    FormStatusBanner, StatusMessage, apply_smtp_test_outcome, build_config_from_form, kind_label,
    provide_lookup_edit_guard, start_smtp_test, use_form_test_status_cleanup,
};
use crate::connection::ConnectionState;
use crate::context::AppContext;
use crate::core_event::CoreEvent;

/// Full first-run account form (identity, IMAP, proxy, optional SMTP).
#[component]
pub fn OnboardingForm() -> Element {
    let mut bootstrap = use_context::<Signal<AppBootstrapState>>();
    let mut ctx = use_context::<AppContext>();
    let store_ctx = use_context::<Signal<Option<AccountStoreContext>>>();
    let core_tx = use_coroutine_handle::<CoreEvent>();

    let prefill = use_hook(dev_form_prefill);
    provide_lookup_edit_guard();

    // Stable account id for this form mount (commit path).
    let account_id = use_hook(|| AccountId::new(Uuid::new_v4().to_string()));
    let account_id_for_effect = account_id.clone();
    let account_id_for_test = account_id.clone();
    let account_id_for_smtp_test = account_id.clone();
    let account_id_for_save = account_id.clone();

    let mut display_name = use_signal(|| prefill.display_name.clone());
    let mut email = use_signal(|| prefill.email.clone());
    let mut imap_host = use_signal(|| prefill.imap_host.clone());
    let mut imap_port = use_signal(|| prefill.imap_port.to_string());
    let mut imap_username = use_signal(|| prefill.imap_username.clone());
    let mut imap_password = use_signal(|| prefill.imap_password.clone());
    let mut imap_tls_mode = use_signal(|| imap_tls_mode_from_legacy(true, prefill.imap_port));
    let mut proxy_base_url = use_signal(|| prefill.proxy_base_url.clone());
    let mut proxy_token = use_signal(|| prefill.proxy_token.clone());
    let mut remote_host = use_signal(|| prefill.remote_host.clone());
    let mut remote_port = use_signal(|| prefill.remote_port.clone());
    let mut smtp_host = use_signal(String::new);
    let mut smtp_port = use_signal(|| DEFAULT_SMTP_PORT.to_string());
    let mut smtp_username = use_signal(String::new);
    let mut smtp_password = use_signal(String::new);
    let mut smtp_tls_mode = use_signal(|| SmtpTlsMode::Implicit);
    let mut smtp_remote_host = use_signal(String::new);
    let mut smtp_remote_port = use_signal(String::new);
    let mut smtp_open = use_signal(|| false);
    let mut signature = use_signal(String::new);
    let mut unlock_passphrase = use_signal(String::new);
    let mut unlock_passphrase_confirm = use_signal(String::new);

    let mut phase = use_signal(|| FormPhase::Idle);
    let mut status_message = use_signal(|| None::<StatusMessage>);
    let mut test_request_id = use_signal(|| None::<AccountId>);
    // Ignore Ready/Error until we have seen Connecting/Authenticating for this attempt.
    // Prevents a stale Error from a previous Save from aborting a retry (and missing success).
    let mut save_seen_progress = use_signal(|| false);
    let mut test_seen_progress = use_signal(|| false);

    use_form_test_status_cleanup(ctx.clone(), test_request_id, phase);

    // Watch connection_states for Save / Test outcomes.
    // Success: set Ready; AppShell replaces `/onboarding` → `/`.
    let ctx_smtp = ctx.clone();
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
                                let passphrase = unlock_passphrase();
                                let store = store_ctx();
                                phase.set(FormPhase::Idle);
                                save_seen_progress.set(false);
                                status_message.set(None);
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
            FormPhase::TestingSmtp => {
                if let Some(rid) = test_request_id() {
                    apply_smtp_test_outcome(
                        ctx_smtp.clone(),
                        &rid,
                        phase,
                        test_request_id,
                        status_message,
                    );
                }
            }
            FormPhase::Idle => {}
        }
    });

    let busy = !matches!(phase(), FormPhase::Idle);

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
            imap_tls_mode(),
            &proxy_base_url(),
            &proxy_token(),
            &remote_host(),
            &remote_port(),
            &smtp_host(),
            &smtp_port(),
            &smtp_username(),
            &smtp_password(),
            smtp_tls_mode(),
            &smtp_remote_host(),
            &smtp_remote_port(),
            &signature(),
            Utc::now(),
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

    let on_test_smtp = move |_| {
        start_smtp_test(
            build_config_from_form(
                &account_id_for_smtp_test,
                &display_name(),
                &email(),
                &imap_host(),
                &imap_port(),
                &imap_username(),
                &imap_password(),
                imap_tls_mode(),
                &proxy_base_url(),
                &proxy_token(),
                &remote_host(),
                &remote_port(),
                &smtp_host(),
                &smtp_port(),
                &smtp_username(),
                &smtp_password(),
                smtp_tls_mode(),
                &smtp_remote_host(),
                &smtp_remote_port(),
                &signature(),
                Utc::now(),
            ),
            phase,
            test_request_id,
            status_message,
            core_tx,
        );
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
            imap_tls_mode(),
            &proxy_base_url(),
            &proxy_token(),
            &remote_host(),
            &remote_port(),
            &smtp_host(),
            &smtp_port(),
            &smtp_username(),
            &smtp_password(),
            smtp_tls_mode(),
            &smtp_remote_host(),
            &smtp_remote_port(),
            &signature(),
            Utc::now(),
        ) {
            Ok(config) => {
                let passphrase = unlock_passphrase();
                let confirm = unlock_passphrase_confirm();
                if !passphrase.is_empty() || !confirm.is_empty() {
                    if passphrase != confirm {
                        status_message.set(Some(StatusMessage::error(
                            "Validation",
                            "Unlock passphrase and confirmation do not match.",
                        )));
                        return;
                    }
                    if passphrase.chars().count() < MIN_PASSPHRASE_CHARS {
                        status_message.set(Some(StatusMessage::error(
                            "Validation",
                            format!(
                                "Unlock passphrase must be at least {MIN_PASSPHRASE_CHARS} characters."
                            ),
                        )));
                        return;
                    }
                }
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

                    AccountConnectionFields {
                        id_prefix: "onboarding",
                        display_name: display_name(),
                        email: email(),
                        imap_host: imap_host(),
                        imap_port: imap_port(),
                        imap_username: imap_username(),
                        imap_password: imap_password(),
                        imap_tls_mode: imap_tls_mode(),
                        proxy_base_url: proxy_base_url(),
                        proxy_token: proxy_token(),
                        remote_host: remote_host(),
                        remote_port: remote_port(),
                        smtp_remote_host: smtp_remote_host(),
                        smtp_remote_port: smtp_remote_port(),
                        set_display_name: move |v| display_name.set(v),
                        set_email: move |v| email.set(v),
                        set_imap_host: move |v| imap_host.set(v),
                        set_imap_port: move |v| imap_port.set(v),
                        set_imap_username: move |v| imap_username.set(v),
                        set_imap_password: move |v| imap_password.set(v),
                        set_imap_tls_mode: move |v| imap_tls_mode.set(v),
                        set_proxy_base_url: move |v| proxy_base_url.set(v),
                        set_proxy_token: move |v| proxy_token.set(v),
                        set_remote_host: move |v| remote_host.set(v),
                        set_remote_port: move |v| remote_port.set(v),
                        set_smtp_remote_host: move |v| smtp_remote_host.set(v),
                        set_smtp_remote_port: move |v| smtp_remote_port.set(v),
                        smtp_host: smtp_host(),
                        smtp_port: smtp_port(),
                        smtp_username: smtp_username(),
                        smtp_use_tls: smtp_tls_mode() != SmtpTlsMode::None,
                        set_smtp_host: move |v| smtp_host.set(v),
                        set_smtp_port: move |v| smtp_port.set(v),
                        set_smtp_username: move |v| smtp_username.set(v),
                        set_smtp_use_tls: move |v| {
                            let port = smtp_port().parse().unwrap_or(465);
                            smtp_tls_mode.set(crate::account_config::tls_mode_from_legacy(v, port));
                        },
                        set_smtp_open: move |v| smtp_open.set(v),
                        busy: busy,
                        open_advanced: !prefill.remote_host.is_empty() || !prefill.remote_port.is_empty(),
                    }

                    AccountSignatureFields {
                        id_prefix: "onboarding",
                        signature: signature(),
                        set_signature: move |v| signature.set(v),
                        busy: busy,
                    }

                    AccountSmtpFields {
                        id_prefix: "onboarding",
                        smtp_host: smtp_host(),
                        smtp_port: smtp_port(),
                        smtp_username: smtp_username(),
                        smtp_password: smtp_password(),
                        smtp_tls_mode: smtp_tls_mode(),
                        set_smtp_host: move |v| smtp_host.set(v),
                        set_smtp_port: move |v| smtp_port.set(v),
                        set_smtp_username: move |v| smtp_username.set(v),
                        set_smtp_password: move |v| smtp_password.set(v),
                        set_smtp_tls_mode: move |v| smtp_tls_mode.set(v),
                        busy: busy,
                        open: smtp_open(),
                    }

                    fieldset {
                        class: "onboarding-section",
                        legend { "Unlock passphrase (optional)" }
                        p {
                            class: "bootstrap-muted",
                            "Encrypt IMAP/SMTP passwords and the proxy token in this browser. \
                             You will enter it once per session. Mailiner cannot recover it."
                        }
                        FormField {
                            label: "Unlock passphrase",
                            id: "onboarding-unlock-passphrase",
                            value: unlock_passphrase(),
                            oninput: move |v| unlock_passphrase.set(v),
                            input_type: "password",
                            autocomplete: "new-password",
                            disabled: busy,
                        }
                        FormField {
                            label: "Confirm passphrase",
                            id: "onboarding-unlock-passphrase-confirm",
                            value: unlock_passphrase_confirm(),
                            oninput: move |v| unlock_passphrase_confirm.set(v),
                            input_type: "password",
                            autocomplete: "new-password",
                            disabled: busy,
                        }
                    }

                    FormStatusBanner { message: status_message() }

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
                            class: "onboarding-btn onboarding-btn-secondary",
                            disabled: busy,
                            onclick: on_test_smtp,
                            if matches!(phase(), FormPhase::TestingSmtp) {
                                "Testing SMTP…"
                            } else {
                                "Test SMTP"
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
