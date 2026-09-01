//! Account management settings: list / add / edit / delete / switch.

use chrono::Utc;
use dioxus::logger::tracing::{info, warn};
use dioxus::prelude::*;
use uuid::Uuid;

use crate::AccountStoreContext;
use crate::AppBootstrapState;
use crate::Route;
use crate::account::AccountId;
use crate::account_config::{AccountConfig, DEFAULT_SMTP_PORT, dev_form_prefill};
use crate::components::account_form::{
    AccountConnectionFields, AccountSmtpFields, FormPhase, FormStatusBanner, StatusMessage,
    apply_smtp_test_outcome, build_config_from_form, credentials_changed, kind_label,
    start_smtp_test, use_form_test_status_cleanup,
};
use crate::connection::ConnectionState;
use crate::context::AppContext;
use crate::core_event::CoreEvent;

/// Debounce for rapid account-switch clicks (ms).
const SWITCH_DEBOUNCE_MS: u32 = 200;

// —— List ——

/// Full `/settings/accounts` list UI.
#[component]
pub fn AccountsSettingsPage() -> Element {
    let bootstrap = use_context::<Signal<AppBootstrapState>>();
    let ctx = use_context::<AppContext>();
    let store_ctx = use_context::<Signal<Option<AccountStoreContext>>>();
    let core_tx = use_coroutine_handle::<CoreEvent>();

    let accounts = ctx.accounts;
    let selected = ctx.selected_account;
    let connection_states = ctx.connection_states;

    let mut confirm_delete_id = use_signal(|| None::<AccountId>);
    let action_error = use_signal(|| None::<String>);
    let pending_switch = use_signal(|| None::<AccountId>);
    let switch_gen = use_signal(|| 0u64);

    let mut listed: Vec<_> = accounts.read().values().cloned().collect();
    listed.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.id.as_str().cmp(b.id.as_str()))
    });

    rsx! {
        div {
            class: "bootstrap-shell onboarding-shell",
            div {
                class: "bootstrap-card onboarding-card accounts-card",
                h1 { class: "bootstrap-title", "Accounts" }
                p {
                    class: "bootstrap-muted",
                    "Manage email accounts stored in this browser. Only the active account \
                     stays connected."
                }

                if let Some(err) = action_error() {
                    p {
                        class: "onboarding-status onboarding-status-error",
                        role: "alert",
                        "{err}"
                    }
                }

                ul {
                    class: "accounts-list",
                    for account in listed.iter() {
                        {
                            let id = account.id.clone();
                            let id_switch = account.id.clone();
                            let id_delete = account.id.clone();
                            let is_active = selected.read().as_ref() == Some(&account.id);
                            let conn = connection_states
                                .read()
                                .get(&account.id)
                                .cloned()
                                .unwrap_or(ConnectionState::Idle);
                            let conn_label = connection_label(&conn);
                            let switching = pending_switch.read().as_ref() == Some(&account.id);

                            let mut ctx_switch = ctx.clone();
                            let store_switch = store_ctx;
                            let core_switch = core_tx;
                            let mut pending_switch_btn = pending_switch;
                            let mut action_error_btn = action_error;
                            let mut switch_gen_btn = switch_gen;
                            let selected_switch = selected;

                            rsx! {
                                li {
                                    class: if is_active { "accounts-list-item active" } else { "accounts-list-item" },
                                    key: "{account.id}",
                                    div {
                                        class: "accounts-list-main",
                                        div {
                                            class: "accounts-list-title",
                                            span { class: "accounts-list-name", "{account.name}" }
                                            if is_active {
                                                span { class: "accounts-badge", "Active" }
                                            }
                                        }
                                        div {
                                            class: "accounts-list-meta",
                                            span { "{account.email}" }
                                            span { class: "bootstrap-muted", " · {account.host}" }
                                        }
                                        div {
                                            class: "accounts-list-conn bootstrap-muted",
                                            "{conn_label}"
                                        }
                                    }
                                    div {
                                        class: "accounts-list-actions",
                                        if !is_active {
                                            button {
                                                r#type: "button",
                                                class: "onboarding-btn onboarding-btn-secondary accounts-btn-sm",
                                                disabled: switching,
                                                onclick: move |_| {
                                                    if selected_switch.read().as_ref() == Some(&id_switch) {
                                                        return;
                                                    }
                                                    action_error_btn.set(None);
                                                    pending_switch_btn.set(Some(id_switch.clone()));
                                                    let generation = {
                                                        let next = switch_gen_btn() + 1;
                                                        switch_gen_btn.set(next);
                                                        next
                                                    };
                                                    let id = id_switch.clone();
                                                    spawn(async move {
                                                        sleep_ms(SWITCH_DEBOUNCE_MS).await;
                                                        if switch_gen_btn() != generation {
                                                            return;
                                                        }
                                                        if pending_switch_btn.read().as_ref() != Some(&id) {
                                                            return;
                                                        }
                                                        pending_switch_btn.set(None);
                                                        let Some(store) = store_switch() else {
                                                            action_error_btn.set(Some(
                                                                "Account storage is not available."
                                                                    .into(),
                                                            ));
                                                            return;
                                                        };
                                                        if let Err(e) =
                                                            store.0.set_active_id(Some(&id)).await
                                                        {
                                                            warn!("set_active_id failed on switch: {e}");
                                                            action_error_btn.set(Some(format!(
                                                                "Failed to set active account: {e}"
                                                            )));
                                                            return;
                                                        }
                                                        ctx_switch.selected_account.set(Some(id.clone()));
                                                        core_switch.send(CoreEvent::SelectAccount(id));
                                                    });
                                                },
                                                if switching { "Switching…" } else { "Switch" }
                                            }
                                        }
                                        Link {
                                            to: Route::AccountEditView {
                                                id: id.as_str().to_string(),
                                            },
                                            class: "onboarding-btn onboarding-btn-secondary accounts-btn-sm accounts-link-btn",
                                            "Edit"
                                        }
                                        button {
                                            r#type: "button",
                                            class: "onboarding-btn onboarding-btn-secondary accounts-btn-sm accounts-btn-danger",
                                            onclick: move |_| confirm_delete_id.set(Some(id_delete.clone())),
                                            "Delete"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if listed.is_empty() {
                    p { class: "bootstrap-muted", "No accounts yet." }
                }

                if let Some(del_id) = confirm_delete_id() {
                    DeleteAccountConfirm {
                        account_id: del_id,
                        confirm_delete_id,
                        action_error,
                        bootstrap,
                        store_ctx,
                    }
                }

                p {
                    class: "onboarding-disclosure bootstrap-muted accounts-security-note",
                    "Security: IMAP passwords and proxy tokens are stored only in this browser \
                     (localStorage). Mailiner has no server-side account. Anyone with access to \
                     this browser profile can read stored credentials. On a shared device, use a \
                     locked profile or clear site data when finished. Multi-tab edits are \
                     last-write-wins."
                }

                nav {
                    class: "bootstrap-nav accounts-nav",
                    Link {
                        to: Route::AccountNewView {},
                        class: "onboarding-btn onboarding-btn-primary accounts-link-btn",
                        "Add account"
                    }
                    Link {
                        to: Route::MainView {},
                        class: "onboarding-btn onboarding-btn-secondary accounts-link-btn",
                        "Back to mail"
                    }
                }
            }
        }
    }
}

fn connection_label(state: &ConnectionState) -> String {
    match state {
        ConnectionState::Idle => "Idle".into(),
        ConnectionState::Connecting => "Connecting…".into(),
        ConnectionState::Authenticating => "Signing in…".into(),
        ConnectionState::Ready => "Connected".into(),
        ConnectionState::Disconnected => "Disconnected".into(),
        ConnectionState::Error { message, .. } => format!("Error: {message}"),
    }
}

#[component]
fn DeleteAccountConfirm(
    account_id: AccountId,
    mut confirm_delete_id: Signal<Option<AccountId>>,
    mut action_error: Signal<Option<String>>,
    mut bootstrap: Signal<AppBootstrapState>,
    store_ctx: Signal<Option<AccountStoreContext>>,
) -> Element {
    let mut ctx = use_context::<AppContext>();
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let nav = use_navigator();
    let selected = ctx.selected_account;
    let id_for_delete = account_id.clone();
    let account_name = ctx
        .accounts
        .read()
        .get(&account_id)
        .map(|a| a.name.clone())
        .unwrap_or_else(|| "this account".into());

    rsx! {
        div {
            class: "accounts-confirm",
            role: "alertdialog",
            p {
                "Delete \"{account_name}\" from this browser? Mail on the server is not affected."
            }
            div {
                class: "onboarding-actions",
                button {
                    r#type: "button",
                    class: "onboarding-btn onboarding-btn-secondary",
                    onclick: move |_| confirm_delete_id.set(None),
                    "Cancel"
                }
                button {
                    r#type: "button",
                    class: "onboarding-btn onboarding-btn-primary accounts-btn-danger-solid",
                    onclick: move |_| {
                        let id = id_for_delete.clone();
                        spawn(async move {
                            let Some(store) = store_ctx() else {
                                action_error
                                    .set(Some("Account storage is not available.".into()));
                                confirm_delete_id.set(None);
                                return;
                            };
                            let store = store.0;

                            let remaining_before: Vec<AccountId> = match store.list().await {
                                Ok(list) => list.into_iter().map(|c| c.id).collect(),
                                Err(e) => {
                                    action_error
                                        .set(Some(format!("Failed to list accounts: {e}")));
                                    confirm_delete_id.set(None);
                                    return;
                                }
                            };
                            let was_active = selected.read().as_ref() == Some(&id);
                            let is_last = remaining_before.len() <= 1;

                            if let Err(e) = store.delete(&id).await {
                                action_error
                                    .set(Some(format!("Failed to delete account: {e}")));
                                confirm_delete_id.set(None);
                                return;
                            }

                            core_tx.send(CoreEvent::DisconnectAccount(id.clone()));
                            confirm_delete_id.set(None);
                            action_error.set(None);

                            if is_last {
                                info!("Deleted last account → NeedsOnboarding");
                                if let Err(e) = store.set_active_id(None).await {
                                    warn!(
                                        "set_active_id(None) after last delete failed: {e}"
                                    );
                                }
                                ctx.selected_account.set(None);
                                ctx.accounts.write().clear();
                                core_tx.send(CoreEvent::AccountsChanged);
                                bootstrap.set(AppBootstrapState::NeedsOnboarding);
                                nav.replace(Route::OnboardingView {});
                                return;
                            }

                            if was_active {
                                let next = remaining_before.into_iter().find(|x| x != &id);
                                if let Some(next_id) = next {
                                    if let Err(e) = store.set_active_id(Some(&next_id)).await {
                                        warn!("set_active_id after delete failed: {e}");
                                    }
                                    ctx.selected_account.set(Some(next_id.clone()));
                                    core_tx.send(CoreEvent::AccountsChanged);
                                    core_tx.send(CoreEvent::SelectAccount(next_id));
                                } else {
                                    core_tx.send(CoreEvent::AccountsChanged);
                                }
                            } else {
                                core_tx.send(CoreEvent::AccountsChanged);
                            }
                        });
                    },
                    "Delete account"
                }
            }
        }
    }
}

// —— New account ——

/// `/settings/accounts/new` — same fields / CommitNewAccount as onboarding.
#[component]
pub fn AccountNewPage() -> Element {
    let mut ctx = use_context::<AppContext>();
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let nav = use_navigator();

    let prefill = use_hook(dev_form_prefill);
    let account_id = use_hook(|| AccountId::new(Uuid::new_v4().to_string()));
    let account_id_effect = account_id.clone();
    let account_id_test = account_id.clone();
    let account_id_smtp_test = account_id.clone();
    let account_id_save = account_id.clone();

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
    let mut smtp_host = use_signal(String::new);
    let mut smtp_port = use_signal(|| DEFAULT_SMTP_PORT.to_string());
    let mut smtp_username = use_signal(String::new);
    let mut smtp_password = use_signal(String::new);
    let mut smtp_use_tls = use_signal(|| true);

    let mut phase = use_signal(|| FormPhase::Idle);
    let mut status_message = use_signal(|| None::<StatusMessage>);
    let mut test_request_id = use_signal(|| None::<AccountId>);
    let mut save_seen_progress = use_signal(|| false);
    let mut test_seen_progress = use_signal(|| false);

    use_form_test_status_cleanup(ctx.clone(), test_request_id, phase);

    let ctx_smtp = ctx.clone();
    use_effect(move || {
        let states = ctx.connection_states.read().clone();
        match phase() {
            FormPhase::Saving => {
                if let Some(state) = states.get(&account_id_effect) {
                    match state {
                        ConnectionState::Connecting | ConnectionState::Authenticating => {
                            save_seen_progress.set(true);
                        }
                        ConnectionState::Ready => {
                            if !save_seen_progress() {
                                return;
                            }
                            if ctx.accounts.read().contains_key(&account_id_effect) {
                                phase.set(FormPhase::Idle);
                                save_seen_progress.set(false);
                                status_message.set(None);
                                nav.replace(Route::AccountsSettingsView {});
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
                            ctx.connection_states.write().remove(&rid);
                            test_request_id.set(None);
                            status_message.set(Some(StatusMessage::success(
                                "Connection successful. You can save the account.",
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
        if busy {
            return;
        }
        status_message.set(None);
        match build_config_from_form(
            &account_id_test,
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
            &smtp_host(),
            &smtp_port(),
            &smtp_username(),
            &smtp_password(),
            smtp_use_tls(),
            Utc::now(),
        ) {
            Ok(config) => {
                if let Some(prev) = test_request_id() {
                    ctx.connection_states.write().remove(&prev);
                }
                let request_id = AccountId::new(Uuid::new_v4().to_string());
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
                &account_id_smtp_test,
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
                &smtp_host(),
                &smtp_port(),
                &smtp_username(),
                &smtp_password(),
                smtp_use_tls(),
                Utc::now(),
            ),
            phase,
            test_request_id,
            status_message,
            core_tx,
        );
    };

    let on_save = move |_| {
        if busy {
            return;
        }
        status_message.set(None);
        match build_config_from_form(
            &account_id_save,
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
            &smtp_host(),
            &smtp_port(),
            &smtp_username(),
            &smtp_password(),
            smtp_use_tls(),
            Utc::now(),
        ) {
            Ok(config) => {
                ctx.connection_states
                    .write()
                    .insert(account_id_save.clone(), ConnectionState::Connecting);
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
                h1 { class: "bootstrap-title", "Add account" }
                p {
                    class: "bootstrap-muted",
                    "Use your IMAP username and password (or provider app password). \
                     OAuth sign-in is not supported yet."
                }

                form {
                    class: "onboarding-form",
                    onsubmit: move |evt| evt.prevent_default(),

                    AccountConnectionFields {
                        id_prefix: "account-new",
                        display_name: display_name(),
                        email: email(),
                        imap_host: imap_host(),
                        imap_port: imap_port(),
                        imap_username: imap_username(),
                        imap_password: imap_password(),
                        proxy_base_url: proxy_base_url(),
                        proxy_token: proxy_token(),
                        remote_host: remote_host(),
                        remote_port: remote_port(),
                        set_display_name: move |v| display_name.set(v),
                        set_email: move |v| email.set(v),
                        set_imap_host: move |v| imap_host.set(v),
                        set_imap_port: move |v| imap_port.set(v),
                        set_imap_username: move |v| imap_username.set(v),
                        set_imap_password: move |v| imap_password.set(v),
                        set_proxy_base_url: move |v| proxy_base_url.set(v),
                        set_proxy_token: move |v| proxy_token.set(v),
                        set_remote_host: move |v| remote_host.set(v),
                        set_remote_port: move |v| remote_port.set(v),
                        busy: busy,
                        open_advanced: !prefill.remote_host.is_empty() || !prefill.remote_port.is_empty(),
                    }

                    AccountSmtpFields {
                        id_prefix: "account-new",
                        smtp_host: smtp_host(),
                        smtp_port: smtp_port(),
                        smtp_username: smtp_username(),
                        smtp_password: smtp_password(),
                        smtp_use_tls: smtp_use_tls(),
                        set_smtp_host: move |v| smtp_host.set(v),
                        set_smtp_port: move |v| smtp_port.set(v),
                        set_smtp_username: move |v| smtp_username.set(v),
                        set_smtp_password: move |v| smtp_password.set(v),
                        set_smtp_use_tls: move |v| smtp_use_tls.set(v),
                        busy: busy,
                    }

                    FormStatusBanner { message: status_message() }

                    div {
                        class: "onboarding-actions",
                        button {
                            r#type: "button",
                            class: "onboarding-btn onboarding-btn-secondary",
                            disabled: busy,
                            onclick: on_test,
                            if matches!(phase(), FormPhase::Testing) { "Testing…" } else { "Test connection" }
                        }
                        button {
                            r#type: "button",
                            class: "onboarding-btn onboarding-btn-secondary",
                            disabled: busy,
                            onclick: on_test_smtp,
                            if matches!(phase(), FormPhase::TestingSmtp) { "Testing SMTP…" } else { "Test SMTP" }
                        }
                        button {
                            r#type: "button",
                            class: "onboarding-btn onboarding-btn-primary",
                            disabled: busy,
                            onclick: on_save,
                            if matches!(phase(), FormPhase::Saving) { "Connecting…" } else { "Save account" }
                        }
                    }
                }

                nav {
                    class: "bootstrap-nav",
                    Link { to: Route::AccountsSettingsView {}, "Back to accounts" }
                }
            }
        }
    }
}

// —— Edit account ——

#[derive(Clone, PartialEq, Eq)]
enum EditLoadState {
    Loading,
    Missing,
    Error(String),
    Ready,
}

/// `/settings/accounts/:id` — edit form; credentials use connect-before-persist.
#[component]
pub fn AccountEditPage(id: String) -> Element {
    let mut ctx = use_context::<AppContext>();
    let store_ctx = use_context::<Signal<Option<AccountStoreContext>>>();
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let nav = use_navigator();

    let account_id = AccountId::new(id.clone());
    let account_id_effect = account_id.clone();
    let account_id_test = account_id.clone();
    let account_id_smtp_test = account_id.clone();
    let account_id_save = account_id.clone();

    let mut load_state = use_signal(|| EditLoadState::Loading);
    let mut original = use_signal(|| None::<AccountConfig>);

    let mut display_name = use_signal(String::new);
    let mut email = use_signal(String::new);
    let mut imap_host = use_signal(String::new);
    let mut imap_port = use_signal(|| "993".to_string());
    let mut imap_username = use_signal(String::new);
    let mut imap_password = use_signal(String::new);
    let mut proxy_base_url = use_signal(String::new);
    let mut proxy_token = use_signal(String::new);
    let mut remote_host = use_signal(String::new);
    let mut remote_port = use_signal(String::new);
    let mut smtp_host = use_signal(String::new);
    let mut smtp_port = use_signal(|| DEFAULT_SMTP_PORT.to_string());
    let mut smtp_username = use_signal(String::new);
    let mut smtp_password = use_signal(String::new);
    let mut smtp_use_tls = use_signal(|| true);
    let mut open_smtp = use_signal(|| false);

    let mut phase = use_signal(|| FormPhase::Idle);
    let mut status_message = use_signal(|| None::<StatusMessage>);
    let mut test_request_id = use_signal(|| None::<AccountId>);
    let mut save_seen_progress = use_signal(|| false);
    let mut test_seen_progress = use_signal(|| false);
    let mut save_via_commit = use_signal(|| false);
    // Prior store `updated_at` when credential save starts; success requires a newer value.
    let mut save_baseline_updated_at = use_signal(|| None::<chrono::DateTime<Utc>>);

    use_form_test_status_cleanup(ctx.clone(), test_request_id, phase);

    // Load secrets only into component-local state.
    use_future(move || {
        let account_id = account_id.clone();
        let store_ctx = store_ctx;
        async move {
            let Some(store) = store_ctx() else {
                load_state.set(EditLoadState::Error(
                    "Account storage is not available.".into(),
                ));
                return;
            };
            match store.0.get(&account_id).await {
                Ok(Some(cfg)) => {
                    display_name.set(cfg.display_name.clone());
                    email.set(cfg.email.clone());
                    imap_host.set(cfg.imap.host.clone());
                    imap_port.set(cfg.imap.port.to_string());
                    imap_username.set(cfg.imap.username.clone());
                    imap_password.set(cfg.imap.password.clone());
                    proxy_base_url.set(cfg.proxy.base_url.clone());
                    proxy_token.set(cfg.proxy.token.clone());
                    remote_host.set(cfg.proxy.remote_host.clone().unwrap_or_default());
                    remote_port.set(
                        cfg.proxy
                            .remote_port
                            .map(|p| p.to_string())
                            .unwrap_or_default(),
                    );
                    if let Some(ref smtp) = cfg.smtp {
                        smtp_host.set(smtp.host.clone());
                        smtp_port.set(smtp.port.to_string());
                        smtp_username.set(smtp.username.clone());
                        smtp_password.set(smtp.password.clone().unwrap_or_default());
                        smtp_use_tls.set(smtp.use_tls);
                        open_smtp.set(true);
                    } else {
                        smtp_host.set(String::new());
                        smtp_port.set(DEFAULT_SMTP_PORT.to_string());
                        smtp_username.set(String::new());
                        smtp_password.set(String::new());
                        smtp_use_tls.set(true);
                        open_smtp.set(false);
                    }
                    original.set(Some(cfg));
                    load_state.set(EditLoadState::Ready);
                }
                Ok(None) => load_state.set(EditLoadState::Missing),
                Err(e) => load_state.set(EditLoadState::Error(format!("Failed to load: {e}"))),
            }
        }
    });

    // Watch connection_states for Save (credential commit) and Test independently.
    // Test must not require save_via_commit (BUG-1). Save waits for upsert via store
    // updated_at (and/or final Ready after demoted Connecting) — not connect-Ready alone.
    let ctx_smtp = ctx.clone();
    use_effect(move || {
        let states = ctx.connection_states.read().clone();
        match phase() {
            FormPhase::Saving if save_via_commit() => {
                if let Some(state) = states.get(&account_id_effect) {
                    match state {
                        ConnectionState::Connecting | ConnectionState::Authenticating => {
                            save_seen_progress.set(true);
                        }
                        ConnectionState::Ready | ConnectionState::Disconnected => {
                            if !save_seen_progress() {
                                return;
                            }
                            // Confirm store upsert finished (background edits end Disconnected;
                            // active edits end Ready after upsert — not mid-connect Ready).
                            let baseline = save_baseline_updated_at();
                            let id = account_id_effect.clone();
                            let store_ctx = store_ctx;
                            let nav = nav;
                            spawn(async move {
                                if phase() != FormPhase::Saving || !save_via_commit() {
                                    return;
                                }
                                let Some(baseline) = baseline else {
                                    return;
                                };
                                let Some(store) = store_ctx() else {
                                    return;
                                };
                                match store.0.get(&id).await {
                                    Ok(Some(cfg)) if cfg.updated_at > baseline => {
                                        if phase() != FormPhase::Saving {
                                            return;
                                        }
                                        phase.set(FormPhase::Idle);
                                        save_seen_progress.set(false);
                                        save_via_commit.set(false);
                                        save_baseline_updated_at.set(None);
                                        status_message.set(None);
                                        original.set(Some(cfg));
                                        nav.replace(Route::AccountsSettingsView {});
                                    }
                                    _ => {
                                        // Not persisted yet (still connecting) or failed
                                        // restore without upsert — keep watching.
                                    }
                                }
                            });
                        }
                        ConnectionState::Error { message, kind, .. } => {
                            if !save_seen_progress() {
                                return;
                            }
                            phase.set(FormPhase::Idle);
                            save_seen_progress.set(false);
                            save_via_commit.set(false);
                            save_baseline_updated_at.set(None);
                            status_message
                                .set(Some(StatusMessage::error(kind_label(*kind), message)));
                        }
                        _ => {}
                    }
                }
            }
            FormPhase::Testing => {
                // Independent of save_via_commit (BUG-1).
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
                            ctx.connection_states.write().remove(&rid);
                            test_request_id.set(None);
                            status_message.set(Some(StatusMessage::success(
                                "Connection successful. You can save the account.",
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
            FormPhase::Saving | FormPhase::Idle => {}
        }
    });

    match load_state() {
        EditLoadState::Loading => {
            return rsx! {
                div {
                    class: "bootstrap-shell",
                    p { class: "bootstrap-muted", "Loading account…" }
                }
            };
        }
        EditLoadState::Missing => {
            return rsx! {
                div {
                    class: "bootstrap-shell",
                    div {
                        class: "bootstrap-card",
                        h1 { class: "bootstrap-title", "Account not found" }
                        p { class: "bootstrap-muted", "This account is no longer in storage." }
                        nav {
                            class: "bootstrap-nav",
                            Link { to: Route::AccountsSettingsView {}, "Back to accounts" }
                        }
                    }
                }
            };
        }
        EditLoadState::Error(msg) => {
            return rsx! {
                div {
                    class: "bootstrap-shell",
                    div {
                        class: "bootstrap-card bootstrap-error",
                        h1 { class: "bootstrap-title", "Could not load account" }
                        p { "{msg}" }
                        nav {
                            class: "bootstrap-nav",
                            Link { to: Route::AccountsSettingsView {}, "Back to accounts" }
                        }
                    }
                }
            };
        }
        EditLoadState::Ready => {}
    }

    let busy = !matches!(phase(), FormPhase::Idle);
    let open_advanced = {
        let rh = remote_host();
        let rp = remote_port();
        !rh.is_empty() || !rp.is_empty()
    };

    let on_test = move |_| {
        if busy {
            return;
        }
        let Some(orig) = original() else {
            return;
        };
        status_message.set(None);
        match build_config_from_form(
            &account_id_test,
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
            &smtp_host(),
            &smtp_port(),
            &smtp_username(),
            &smtp_password(),
            smtp_use_tls(),
            orig.created_at,
        ) {
            Ok(config) => {
                if let Some(prev) = test_request_id() {
                    ctx.connection_states.write().remove(&prev);
                }
                let request_id = AccountId::new(Uuid::new_v4().to_string());
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
        let Some(orig) = original() else {
            return;
        };
        start_smtp_test(
            build_config_from_form(
                &account_id_smtp_test,
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
                &smtp_host(),
                &smtp_port(),
                &smtp_username(),
                &smtp_password(),
                smtp_use_tls(),
                orig.created_at,
            ),
            phase,
            test_request_id,
            status_message,
            core_tx,
        );
    };

    let on_save = move |_| {
        if busy {
            return;
        }
        let Some(orig) = original() else {
            return;
        };
        status_message.set(None);
        let config = match build_config_from_form(
            &account_id_save,
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
            &smtp_host(),
            &smtp_port(),
            &smtp_username(),
            &smtp_password(),
            smtp_use_tls(),
            orig.created_at,
        ) {
            Ok(c) => c,
            Err(msg) => {
                status_message.set(Some(StatusMessage::error("Validation", &msg)));
                return;
            }
        };

        if credentials_changed(&orig, &config) {
            // Connect-before-persist (reuse CommitNewAccount; core force-reconnects).
            ctx.connection_states
                .write()
                .insert(account_id_save.clone(), ConnectionState::Connecting);
            save_seen_progress.set(true);
            save_via_commit.set(true);
            save_baseline_updated_at.set(Some(orig.updated_at));
            phase.set(FormPhase::Saving);
            status_message.set(Some(StatusMessage::info("Connecting…")));
            core_tx.send(CoreEvent::CommitNewAccount { config });
        } else {
            // Non-secret fields only: upsert immediately.
            let store_ctx = store_ctx;
            let core_tx = core_tx;
            let nav = nav;
            let mut original = original;
            spawn(async move {
                phase.set(FormPhase::Saving);
                status_message.set(Some(StatusMessage::info("Saving…")));
                let Some(store) = store_ctx() else {
                    phase.set(FormPhase::Idle);
                    status_message.set(Some(StatusMessage::error(
                        "Storage",
                        "Account storage is not available.",
                    )));
                    return;
                };
                if let Err(e) = store.0.upsert(&config).await {
                    phase.set(FormPhase::Idle);
                    status_message.set(Some(StatusMessage::error(
                        "Storage",
                        format!("Failed to save: {e}"),
                    )));
                    return;
                }
                original.set(Some(config.clone()));
                // Refresh UI account display fields without reconnect.
                if let Some(ui) = ctx.accounts.write().get_mut(&config.id) {
                    ui.name = config.display_name.clone();
                    ui.email = config.email.clone();
                    ui.host = config.imap.host.clone();
                }
                core_tx.send(CoreEvent::AccountsChanged);
                phase.set(FormPhase::Idle);
                status_message.set(None);
                nav.replace(Route::AccountsSettingsView {});
            });
        }
    };

    rsx! {
        div {
            class: "bootstrap-shell onboarding-shell",
            div {
                class: "bootstrap-card onboarding-card",
                h1 { class: "bootstrap-title", "Edit account" }
                p {
                    class: "bootstrap-muted",
                    "Changes to password, host, or proxy are verified with a live connection \
                     before they are saved."
                }

                form {
                    class: "onboarding-form",
                    onsubmit: move |evt| evt.prevent_default(),

                    AccountConnectionFields {
                        id_prefix: "account-edit",
                        display_name: display_name(),
                        email: email(),
                        imap_host: imap_host(),
                        imap_port: imap_port(),
                        imap_username: imap_username(),
                        imap_password: imap_password(),
                        proxy_base_url: proxy_base_url(),
                        proxy_token: proxy_token(),
                        remote_host: remote_host(),
                        remote_port: remote_port(),
                        set_display_name: move |v| display_name.set(v),
                        set_email: move |v| email.set(v),
                        set_imap_host: move |v| imap_host.set(v),
                        set_imap_port: move |v| imap_port.set(v),
                        set_imap_username: move |v| imap_username.set(v),
                        set_imap_password: move |v| imap_password.set(v),
                        set_proxy_base_url: move |v| proxy_base_url.set(v),
                        set_proxy_token: move |v| proxy_token.set(v),
                        set_remote_host: move |v| remote_host.set(v),
                        set_remote_port: move |v| remote_port.set(v),
                        busy: busy,
                        open_advanced: open_advanced,
                    }

                    AccountSmtpFields {
                        id_prefix: "account-edit",
                        smtp_host: smtp_host(),
                        smtp_port: smtp_port(),
                        smtp_username: smtp_username(),
                        smtp_password: smtp_password(),
                        smtp_use_tls: smtp_use_tls(),
                        set_smtp_host: move |v| smtp_host.set(v),
                        set_smtp_port: move |v| smtp_port.set(v),
                        set_smtp_username: move |v| smtp_username.set(v),
                        set_smtp_password: move |v| smtp_password.set(v),
                        set_smtp_use_tls: move |v| smtp_use_tls.set(v),
                        busy: busy,
                        open: open_smtp(),
                    }

                    FormStatusBanner { message: status_message() }

                    div {
                        class: "onboarding-actions",
                        button {
                            r#type: "button",
                            class: "onboarding-btn onboarding-btn-secondary",
                            disabled: busy,
                            onclick: on_test,
                            if matches!(phase(), FormPhase::Testing) { "Testing…" } else { "Test connection" }
                        }
                        button {
                            r#type: "button",
                            class: "onboarding-btn onboarding-btn-secondary",
                            disabled: busy,
                            onclick: on_test_smtp,
                            if matches!(phase(), FormPhase::TestingSmtp) { "Testing SMTP…" } else { "Test SMTP" }
                        }
                        button {
                            r#type: "button",
                            class: "onboarding-btn onboarding-btn-primary",
                            disabled: busy,
                            onclick: on_save,
                            if matches!(phase(), FormPhase::Saving) { "Saving…" } else { "Save changes" }
                        }
                    }
                }

                nav {
                    class: "bootstrap-nav",
                    Link { to: Route::AccountsSettingsView {}, "Back to accounts" }
                }
            }
        }
    }
}

async fn sleep_ms(ms: u32) {
    #[cfg(target_arch = "wasm32")]
    {
        gloo_timers::future::TimeoutFuture::new(ms).await;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        tokio::time::sleep(std::time::Duration::from_millis(ms as u64)).await;
    }
}
