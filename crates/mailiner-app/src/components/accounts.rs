//! Account management settings: list / add / edit / delete / switch.

use chrono::Utc;
use dioxus::logger::tracing::{info, warn};
use dioxus::prelude::*;
use uuid::Uuid;

use crate::AccountStoreContext;
use crate::AppBootstrapState;
use crate::Route;
use crate::account::AccountId;
use crate::account_config::{
    AccountConfig, DEFAULT_SMTP_PORT, ImapTlsMode, SmtpTlsMode, dev_form_prefill,
    imap_tls_mode_from_legacy,
};
use crate::components::account_form::{
    AccountConnectionFields, AccountSignatureFields, AccountSmtpFields, FormPhase,
    FormStatusBanner, StatusMessage, apply_smtp_test_outcome, build_config_from_form,
    credentials_changed, kind_label, start_smtp_test, use_form_test_status_cleanup,
};
use crate::components::theme::ThemeSelect;
use crate::connection::ConnectionState;
use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::download::save_text_download;
use crate::local_data::{AccountsExport, accounts_export_filename};

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
    let quota = *ctx.account_quota.read();
    let wiping = *ctx.sign_out_pending.read();

    let mut confirm_delete_id = use_signal(|| None::<AccountId>);
    let mut confirm_data = use_signal(|| None::<DataConfirm>);
    let mut action_error = use_signal(|| None::<String>);
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

                fieldset {
                    class: "onboarding-section appearance-section",
                    legend { "Appearance" }
                    div {
                        class: "onboarding-field",
                        label {
                            r#for: "theme-pref",
                            "Theme"
                        }
                        ThemeSelect {
                            id: "theme-pref",
                            class: "theme-select appearance-theme-select",
                        }
                    }
                    p {
                        class: "bootstrap-muted",
                        "System follows this device's light or dark setting."
                    }
                }

                PrivacyPrefsSection {}

                NotificationsSettings {}

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
                                            if is_active {
                                                if let Some(quota) = quota {
                                                    " · {quota.display()}"
                                                }
                                            }
                                        }
                                    }
                                    div {
                                        class: "accounts-list-actions",
                                        if !is_active {
                                            button {
                                                r#type: "button",
                                                class: "onboarding-btn onboarding-btn-secondary accounts-btn-sm",
                                                disabled: switching || wiping,
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
                                        if !wiping {
                                            Link {
                                                to: Route::AccountEditView {
                                                    id: id.as_str().to_string(),
                                                },
                                                class: "onboarding-btn onboarding-btn-secondary accounts-btn-sm accounts-link-btn",
                                                "Edit"
                                            }
                                        }
                                        button {
                                            r#type: "button",
                                            class: "onboarding-btn onboarding-btn-secondary accounts-btn-sm accounts-btn-danger",
                                            disabled: wiping,
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

                section {
                    class: "accounts-data onboarding-section",
                    h2 { class: "accounts-data-title", "Data" }
                    p {
                        class: "bootstrap-muted",
                        "Export connection settings, or remove everything Mailiner stored in \
                         this browser. Mail on the server is not affected."
                    }
                    div {
                        class: "accounts-data-actions",
                        button {
                            r#type: "button",
                            class: "onboarding-btn onboarding-btn-secondary accounts-btn-sm",
                            onclick: move |_| {
                                action_error.set(None);
                                spawn(download_accounts_export(store_ctx, action_error, false));
                            },
                            "Export accounts"
                        }
                        button {
                            r#type: "button",
                            class: "onboarding-btn onboarding-btn-secondary accounts-btn-sm",
                            onclick: move |_| {
                                action_error.set(None);
                                confirm_data.set(Some(DataConfirm::FullBackup));
                            },
                            "Export full backup…"
                        }
                        button {
                            r#type: "button",
                            class: "onboarding-btn onboarding-btn-secondary accounts-btn-sm accounts-btn-danger",
                            onclick: move |_| {
                                action_error.set(None);
                                confirm_data.set(Some(DataConfirm::SignOut));
                            },
                            "Sign out / delete local data…"
                        }
                    }
                    if let Some(kind) = confirm_data() {
                        DataActionConfirm {
                            kind,
                            confirm_data,
                            action_error,
                            bootstrap,
                            store_ctx,
                        }
                    }
                }

                nav {
                    class: "bootstrap-nav accounts-nav",
                    if wiping {
                        span {
                            class: "onboarding-btn onboarding-btn-primary accounts-link-btn",
                            "Add account"
                        }
                        span {
                            class: "onboarding-btn onboarding-btn-secondary accounts-link-btn",
                            "Settings"
                        }
                        span {
                            class: "onboarding-btn onboarding-btn-secondary accounts-link-btn",
                            "Back to mail"
                        }
                    } else {
                        Link {
                            to: Route::AccountNewView {},
                            class: "onboarding-btn onboarding-btn-primary accounts-link-btn",
                            "Add account"
                        }
                        Link {
                            to: Route::SettingsView {},
                            class: "onboarding-btn onboarding-btn-secondary accounts-link-btn",
                            "Settings"
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
}

#[component]
fn PrivacyPrefsSection() -> Element {
    use crate::ui_prefs;

    let mut allow_remote = use_signal(ui_prefs::load_allow_remote_images);
    let mut senders_tick = use_signal(|| 0u32);
    let _ = senders_tick();
    let entries = ui_prefs::load_remote_image_senders().entries();

    rsx! {
        fieldset {
            class: "onboarding-section privacy-section",
            legend { "Privacy" }
            p {
                class: "bootstrap-muted",
                "Remote images can tell the sender that you opened a message. \
                 Blocked by default; you can still allow them on a single message \
                 or remember a sender."
            }
            div {
                class: "onboarding-field",
                label {
                    r#for: "privacy-remote-images",
                    "Remote images"
                }
                select {
                    id: "privacy-remote-images",
                    class: "theme-select appearance-theme-select",
                    value: if allow_remote() { "allow" } else { "block" },
                    onchange: move |evt| {
                        let next = evt.value() == "allow";
                        ui_prefs::save_allow_remote_images(next);
                        allow_remote.set(next);
                    },
                    option {
                        value: "block",
                        selected: !allow_remote(),
                        "Block by default"
                    }
                    option {
                        value: "allow",
                        selected: allow_remote(),
                        "Allow by default"
                    }
                }
            }
            if !entries.is_empty() {
                p {
                    class: "bootstrap-muted privacy-sender-heading",
                    "Remembered senders"
                }
                ul {
                    class: "privacy-sender-list",
                    for entry in entries {
                        {
                            let kind = entry.kind;
                            let key = entry.key.clone();
                            let display = entry.display_key();
                            let pref = entry.pref.label();
                            let kind_key = match kind {
                                crate::ui_prefs::RemoteImageSenderKind::Address => "addr",
                                crate::ui_prefs::RemoteImageSenderKind::Domain => "domain",
                            };
                            rsx! {
                                li {
                                    class: "privacy-sender-row",
                                    key: "{kind_key}:{key}",
                                    span {
                                        class: "privacy-sender-key",
                                        title: "{display}",
                                        "{display}"
                                    }
                                    span {
                                        class: "privacy-sender-pref",
                                        "{pref}"
                                    }
                                    button {
                                        r#type: "button",
                                        class: "onboarding-btn onboarding-btn-secondary accounts-btn-sm",
                                        onclick: move |_| {
                                            ui_prefs::clear_remote_image_entry(kind, &key);
                                            senders_tick.set(senders_tick() + 1);
                                        },
                                        "Remove"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DataConfirm {
    FullBackup,
    SignOut,
}

async fn download_accounts_export(
    store_ctx: Signal<Option<AccountStoreContext>>,
    mut action_error: Signal<Option<String>>,
    includes_secrets: bool,
) {
    let Some(store) = store_ctx() else {
        action_error.set(Some("Account storage is not available.".into()));
        return;
    };
    let configs = match store.0.list().await {
        Ok(list) => list,
        Err(e) => {
            action_error.set(Some(format!("Failed to read accounts: {e}")));
            return;
        }
    };
    let active = match store.0.get_active_id().await {
        Ok(id) => id,
        Err(e) => {
            action_error.set(Some(format!("Failed to read active account: {e}")));
            return;
        }
    };
    let export = AccountsExport::new(configs, active, includes_secrets, Utc::now());
    let json = match export.to_pretty_json() {
        Ok(s) => s,
        Err(e) => {
            action_error.set(Some(format!("Failed to encode export: {e}")));
            return;
        }
    };
    let name = accounts_export_filename(includes_secrets, export.exported_at);
    if let Err(e) = save_text_download(&name, "application/json", &json) {
        action_error.set(Some(format!("Failed to download export: {e}")));
        return;
    }
    action_error.set(None);
}

#[component]
fn DataActionConfirm(
    kind: DataConfirm,
    mut confirm_data: Signal<Option<DataConfirm>>,
    mut action_error: Signal<Option<String>>,
    bootstrap: Signal<AppBootstrapState>,
    store_ctx: Signal<Option<AccountStoreContext>>,
) -> Element {
    let mut ctx = use_context::<AppContext>();
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let wiping = *ctx.sign_out_pending.read();
    let mut sign_out_error = ctx.sign_out_error;
    use_effect(move || {
        if let Some(err) = sign_out_error() {
            action_error.set(Some(format!(
                "Some local Mailiner data could not be removed: {err}"
            )));
        }
    });

    let (body, confirm_label, danger) = match kind {
        DataConfirm::FullBackup => (
            "This file contains IMAP/SMTP passwords and proxy tokens. Anyone with the file can \
             access your mail. Continue?",
            "Download backup",
            false,
        ),
        DataConfirm::SignOut => (
            "Delete all Mailiner data stored in this browser? This removes accounts, passwords, \
             cached mail, the outbox, the address book, and preferences. Mail on the server is \
             not affected.",
            "Delete local data",
            true,
        ),
    };

    rsx! {
        div {
            class: "accounts-confirm",
            role: "alertdialog",
            p { "{body}" }
            div {
                class: "onboarding-actions",
                button {
                    r#type: "button",
                    class: "onboarding-btn onboarding-btn-secondary",
                    disabled: wiping,
                    onclick: move |_| confirm_data.set(None),
                    "Cancel"
                }
                button {
                    r#type: "button",
                    class: if danger {
                        "onboarding-btn onboarding-btn-primary accounts-btn-danger-solid"
                    } else {
                        "onboarding-btn onboarding-btn-primary"
                    },
                    disabled: wiping,
                    onclick: move |_| {
                        match kind {
                            DataConfirm::FullBackup => {
                                confirm_data.set(None);
                                spawn(download_accounts_export(store_ctx, action_error, true));
                            }
                            DataConfirm::SignOut => {
                                // Wipe runs in core_loop after the current IMAP/SMTP
                                // handler finishes. AppShell navigates after the ack.
                                ctx.sign_out_error.set(None);
                                ctx.sign_out_started.set(*ctx.sign_out_epoch.peek());
                                ctx.sign_out_pending.set(true);
                                core_tx.send(CoreEvent::ClearLocalData);
                            }
                        }
                    },
                    if wiping { "Deleting…" } else { "{confirm_label}" }
                }
            }
        }
    }
}

/// Inbox desktop-notification toggle. Off until the browser grants permission.
#[component]
fn NotificationsSettings() -> Element {
    let ctx = use_context::<AppContext>();
    let mut notify_inbox = ctx.notify_inbox;
    let mut permission = use_signal(crate::notifications::current_permission);
    let mut hint = use_signal(|| None::<String>);
    let pref_on = notify_inbox();
    let perm = permission();
    let checked = pref_on && perm == crate::notifications::NotifyPermission::Granted;
    // Denied stays clickable so the user can retry after changing site settings.
    let blocked = perm == crate::notifications::NotifyPermission::Unsupported;

    rsx! {
        fieldset {
            class: "onboarding-section accounts-notifications",
            legend { "Notifications" }
            p {
                class: "bootstrap-muted",
                "The tab title always shows Inbox unread. Desktop alerts stay off until \
                 you allow them in this browser."
            }
            div {
                class: "onboarding-checkbox-field",
                label {
                    class: "onboarding-checkbox-label",
                    input {
                        r#type: "checkbox",
                        checked: checked,
                        disabled: blocked,
                        onchange: move |evt| {
                            let want = evt.checked();
                            spawn(async move {
                                if want {
                                    // Re-read first: the user may have granted
                                    // the site in browser settings while Denied.
                                    permission.set(
                                        crate::notifications::current_permission(),
                                    );
                                    let next =
                                        crate::notifications::request_permission().await;
                                    permission.set(next);
                                    if next == crate::notifications::NotifyPermission::Granted
                                    {
                                        crate::ui_prefs::save_notify_inbox(true);
                                        notify_inbox.set(true);
                                        hint.set(None);
                                    } else {
                                        crate::ui_prefs::save_notify_inbox(false);
                                        notify_inbox.set(false);
                                        hint.set(Some(permission_hint(next).to_string()));
                                    }
                                } else {
                                    crate::ui_prefs::save_notify_inbox(false);
                                    notify_inbox.set(false);
                                    hint.set(None);
                                }
                            });
                        },
                    }
                    "Notify me about new Inbox mail"
                }
            }
            if let Some(text) = hint() {
                p { class: "bootstrap-muted", "{text}" }
            } else if let Some(text) = permission_status_text(perm) {
                p { class: "bootstrap-muted", "{text}" }
            }
        }
    }
}

fn permission_hint(perm: crate::notifications::NotifyPermission) -> &'static str {
    match perm {
        crate::notifications::NotifyPermission::Denied => {
            "Notifications are blocked for this site. Allow them in the browser, then try again."
        }
        crate::notifications::NotifyPermission::Unsupported => {
            "This browser does not support notifications."
        }
        crate::notifications::NotifyPermission::Prompt => {
            "Permission was not granted. Enable the toggle again to retry."
        }
        crate::notifications::NotifyPermission::Granted => "",
    }
}

fn permission_status_text(perm: crate::notifications::NotifyPermission) -> Option<&'static str> {
    match perm {
        crate::notifications::NotifyPermission::Denied => Some(permission_hint(perm)),
        crate::notifications::NotifyPermission::Unsupported => Some(permission_hint(perm)),
        _ => None,
    }
}

fn connection_label(state: &ConnectionState) -> String {
    match state {
        ConnectionState::Idle => "Idle".into(),
        ConnectionState::Connecting => "Connecting…".into(),
        ConnectionState::Authenticating => "Signing in…".into(),
        ConnectionState::Ready => "Connected".into(),
        ConnectionState::Reconnecting { .. } => "Reconnecting…".into(),
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
                        id_prefix: "account-new",
                        signature: signature(),
                        set_signature: move |v| signature.set(v),
                        busy: busy,
                    }

                    AccountSmtpFields {
                        id_prefix: "account-new",
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
    let mut imap_tls_mode = use_signal(|| ImapTlsMode::Implicit);
    let mut proxy_base_url = use_signal(String::new);
    let mut proxy_token = use_signal(String::new);
    let mut remote_host = use_signal(String::new);
    let mut remote_port = use_signal(String::new);
    let mut smtp_host = use_signal(String::new);
    let mut smtp_port = use_signal(|| DEFAULT_SMTP_PORT.to_string());
    let mut smtp_username = use_signal(String::new);
    let mut smtp_password = use_signal(String::new);
    let mut smtp_tls_mode = use_signal(|| SmtpTlsMode::Implicit);
    let mut smtp_remote_host = use_signal(String::new);
    let mut smtp_remote_port = use_signal(String::new);
    let mut signature = use_signal(String::new);
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
                    imap_tls_mode.set(cfg.imap.tls_mode);
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
                        smtp_tls_mode.set(smtp.tls_mode);
                        smtp_remote_host.set(smtp.remote_host.clone().unwrap_or_default());
                        smtp_remote_port
                            .set(smtp.remote_port.map(|p| p.to_string()).unwrap_or_default());
                        open_smtp.set(true);
                    } else {
                        smtp_host.set(String::new());
                        smtp_port.set(DEFAULT_SMTP_PORT.to_string());
                        smtp_username.set(String::new());
                        smtp_password.set(String::new());
                        smtp_tls_mode.set(SmtpTlsMode::Implicit);
                        smtp_remote_host.set(String::new());
                        smtp_remote_port.set(String::new());
                        open_smtp.set(false);
                    }
                    signature.set(cfg.signature.clone().unwrap_or_default());
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
        let srh = smtp_remote_host();
        let srp = smtp_remote_port();
        !rh.is_empty() || !rp.is_empty() || !srh.is_empty() || !srp.is_empty()
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
                    ui.signature = config.signature.clone();
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
                        set_smtp_open: move |v| open_smtp.set(v),
                        busy: busy,
                        open_advanced: open_advanced,
                    }

                    AccountSignatureFields {
                        id_prefix: "account-edit",
                        signature: signature(),
                        set_signature: move |v| signature.set(v),
                        busy: busy,
                    }

                    AccountSmtpFields {
                        id_prefix: "account-edit",
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
