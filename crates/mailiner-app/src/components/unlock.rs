//! Session unlock for a passphrase-wrapped account store.

use std::collections::HashMap;

use dioxus::logger::tracing::{info, warn};
use dioxus::prelude::*;

use crate::AccountStoreContext;
use crate::AppBootstrapState;
use crate::account_store::AccountStoreError;
use crate::account_vault::MIN_PASSPHRASE_CHARS;
use crate::components::account_form::{FormField, FormStatusBanner, StatusMessage};
use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::resolve_active_id;

/// Full-page unlock form shown when the account blob has a vault.
#[component]
pub fn UnlockForm() -> Element {
    let mut bootstrap = use_context::<Signal<AppBootstrapState>>();
    let mut ctx = use_context::<AppContext>();
    let store_ctx = use_context::<Signal<Option<AccountStoreContext>>>();
    let core_tx = use_coroutine_handle::<CoreEvent>();

    let mut passphrase = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut confirm_wipe = use_signal(|| false);
    let mut status_message = use_signal(|| None::<StatusMessage>);

    let wiping = *ctx.sign_out_pending.read();

    rsx! {
        main {
            class: "bootstrap-shell onboarding-shell",
            div {
                class: "bootstrap-card onboarding-card",
                h1 { class: "bootstrap-title", "Unlock accounts" }
                p {
                    class: "bootstrap-muted",
                    "Account passwords and proxy tokens are encrypted in this browser. \
                     Enter your unlock passphrase. Mailiner cannot recover it."
                }

                form {
                    class: "onboarding-form",
                    onsubmit: move |evt| {
                        evt.prevent_default();
                        start_unlock(
                            bootstrap,
                            ctx.clone(),
                            store_ctx,
                            core_tx,
                            passphrase,
                            busy,
                            status_message,
                            wiping,
                        );
                    },

                    FormField {
                        label: "Unlock passphrase",
                        id: "unlock-passphrase",
                        value: passphrase(),
                        oninput: move |v| passphrase.set(v),
                        input_type: "password",
                        autocomplete: "current-password",
                        disabled: busy() || wiping,
                    }

                    FormStatusBanner { message: status_message() }

                    div {
                        class: "onboarding-actions",
                        button {
                            r#type: "submit",
                            class: "onboarding-btn onboarding-btn-primary",
                            disabled: busy() || wiping,
                            if busy() { "Unlocking…" } else { "Unlock" }
                        }
                    }
                }

                p {
                    class: "bootstrap-muted accounts-security-note",
                    "Forgot the passphrase? You can delete all Mailiner data stored in this \
                     browser and start over. Mail on the server is not affected. \
                     Passphrases must be at least {MIN_PASSPHRASE_CHARS} characters."
                }

                if confirm_wipe() {
                    div {
                        class: "accounts-confirm",
                        role: "alertdialog",
                        p {
                            "Delete all Mailiner data stored in this browser? This removes \
                             accounts, the encrypted vault, cached mail, and preferences."
                        }
                        div {
                            class: "onboarding-actions",
                            button {
                                r#type: "button",
                                class: "onboarding-btn onboarding-btn-secondary",
                                disabled: wiping,
                                onclick: move |_| confirm_wipe.set(false),
                                "Cancel"
                            }
                            button {
                                r#type: "button",
                                class: "onboarding-btn onboarding-btn-primary accounts-btn-danger-solid",
                                disabled: wiping,
                                onclick: move |_| {
                                    ctx.sign_out_error.set(None);
                                    ctx.sign_out_started.set(*ctx.sign_out_epoch.peek());
                                    ctx.sign_out_pending.set(true);
                                    core_tx.send(CoreEvent::ClearLocalData);
                                },
                                if wiping { "Deleting…" } else { "Delete local data" }
                            }
                        }
                    }
                } else {
                    div {
                        class: "onboarding-actions",
                        button {
                            r#type: "button",
                            class: "onboarding-btn onboarding-btn-secondary accounts-btn-danger",
                            disabled: busy() || wiping,
                            onclick: move |_| confirm_wipe.set(true),
                            "Forgot passphrase…"
                        }
                    }
                }
            }
        }
    }
}

fn start_unlock(
    mut bootstrap: Signal<AppBootstrapState>,
    mut ctx: AppContext,
    store_ctx: Signal<Option<AccountStoreContext>>,
    core_tx: Coroutine<CoreEvent>,
    mut passphrase: Signal<String>,
    mut busy: Signal<bool>,
    mut status_message: Signal<Option<StatusMessage>>,
    wiping: bool,
) {
    if busy() || wiping {
        return;
    }
    let Some(store) = store_ctx() else {
        status_message.set(Some(StatusMessage::error(
            "Storage",
            "Account storage is not available.",
        )));
        return;
    };
    let entered = passphrase();
    busy.set(true);
    status_message.set(None);
    spawn(async move {
        match store.0.unlock(&entered).await {
            Ok(()) => match store.0.list().await {
                Ok(list) if list.is_empty() => {
                    info!("Unlock: empty store → NeedsOnboarding");
                    ctx.accounts.set(HashMap::new());
                    ctx.selected_account.set(None);
                    passphrase.set(String::new());
                    bootstrap.set(AppBootstrapState::NeedsOnboarding);
                    core_tx.send(CoreEvent::Bootstrap { active: None });
                }
                Ok(list) => {
                    let mut map = HashMap::new();
                    for cfg in &list {
                        map.insert(cfg.id.clone(), cfg.to_ui_account());
                    }
                    ctx.accounts.set(map);
                    let active = resolve_active_id(store.0.as_ref(), &list).await;
                    ctx.selected_account.set(active.clone());
                    passphrase.set(String::new());
                    info!("Unlock: {} account(s) → Ready", list.len());
                    bootstrap.set(AppBootstrapState::Ready);
                    core_tx.send(CoreEvent::Bootstrap { active });
                }
                Err(e) => {
                    warn!("Unlock list failed: {e}");
                    status_message.set(Some(StatusMessage::error(
                        "Storage",
                        format!("Unlocked, but failed to read accounts: {e}"),
                    )));
                }
            },
            Err(AccountStoreError::WrongPassphrase) => {
                status_message.set(Some(StatusMessage::error(
                    "Unlock failed",
                    "That passphrase is incorrect.",
                )));
            }
            Err(e) => {
                warn!("Unlock failed: {e}");
                status_message.set(Some(StatusMessage::error("Unlock failed", e.to_string())));
            }
        }
        busy.set(false);
    });
}
