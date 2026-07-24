use std::collections::HashMap;
use std::rc::Rc;

use dioxus::logger::tracing::{info, warn};
use dioxus::prelude::*;
use mailiner_core::ids::AccountId;

use crate::account_config::dev_default_config;
use crate::account_store::{
    AccountStore, AccountStoreError, BrowserAccountStore, InMemoryAccountStore,
};
use crate::components::virtual_scroll::SparseList;
use crate::components::{EmailNavigation, MessageList, MessageView};
use crate::context::AppContext;
use crate::core_event::core_loop;

mod account;
mod account_config;
mod account_store;
mod components;
mod connection;
mod context;
mod core_event;
mod download;
mod formatter;
mod mailbox;
mod message;
mod message_loader;
mod websocket_stream;

/// UI bootstrap state machine (store open → onboarding vs main).
#[derive(Clone, Debug, PartialEq)]
pub enum AppBootstrapState {
    /// Store open + list in flight. Full-page spinner; no mail chrome.
    LoadingStore,
    /// Zero accounts (and no interim `dev_default`). Only onboarding is valid.
    NeedsOnboarding,
    /// Accounts loaded (store or memory-only dev_default); main app allowed.
    Ready,
    /// localStorage unavailable or unreadable.
    StoreError { message: String },
}

/// Shared handle to the process-lifetime account store (secrets). Opened once.
#[derive(Clone)]
pub struct AccountStoreContext(pub Rc<dyn AccountStore>);

#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
#[allow(clippy::enum_variant_names)] // Route component names end in View by convention.
enum Route {
    #[layout(AppShell)]
    #[route("/")]
    MainView {},
    #[route("/onboarding")]
    OnboardingView {},
    #[route("/settings/accounts")]
    AccountsSettingsView {},
    #[route("/settings/accounts/new")]
    AccountNewView {},
    #[route("/settings/accounts/:id")]
    AccountEditView { id: String },
}

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/main.css");

fn main() {
    dioxus::launch(App);
}

/// Result of opening the store and applying the bootstrap resolution algorithm.
struct BootstrapOutcome {
    store: Rc<dyn AccountStore>,
    /// `Some` → run `CoreEvent::Bootstrap` with this active id (possibly `None`).
    /// `None` → store failed; skip bootstrap connect.
    initial_bootstrap: Option<Option<AccountId>>,
}

/// Open `BrowserAccountStore`, resolve bootstrap state, populate UI accounts (no secrets).
///
/// Algorithm (design doc):
/// - open failure → StoreError
/// - empty + `dev_default_config()` → Ready + memory-only UI account + Bootstrap { active }
///   (no localStorage write)
/// - empty otherwise → NeedsOnboarding
/// - non-empty → Ready, UI from `to_ui_account`, resolve active, Bootstrap { active }
async fn run_bootstrap(
    ctx: &mut AppContext,
    mut bootstrap: Signal<AppBootstrapState>,
    mut store_ctx: Signal<Option<AccountStoreContext>>,
) -> BootstrapOutcome {
    let store: Rc<dyn AccountStore> = match BrowserAccountStore::open().await {
        Ok(s) => Rc::new(s),
        Err(e) => {
            let message = match e {
                AccountStoreError::Unavailable => {
                    "Account storage is unavailable in this browser (blocked or private mode). \
                     Accounts cannot be saved."
                        .to_string()
                }
                other => format!("Failed to open account storage: {other}"),
            };
            warn!("BrowserAccountStore open failed: {message}");
            bootstrap.set(AppBootstrapState::StoreError { message });
            return BootstrapOutcome {
                store: Rc::new(InMemoryAccountStore::new()),
                initial_bootstrap: None,
            };
        }
    };

    store_ctx.set(Some(AccountStoreContext(store.clone())));

    let list = match store.list().await {
        Ok(list) => list,
        Err(e) => {
            let message = format!("Failed to read accounts from storage: {e}");
            warn!("{message}");
            bootstrap.set(AppBootstrapState::StoreError { message });
            return BootstrapOutcome {
                store,
                initial_bootstrap: None,
            };
        }
    };

    if list.is_empty() {
        // Interim (PR2–PR4): empty store + dev_default ⇒ Ready/main, memory-only, no write.
        if let Some(cfg) = dev_default_config() {
            info!(
                "Bootstrap: empty store + dev_default → Ready (memory-only account {})",
                cfg.id
            );
            let id = cfg.id.clone();
            let ui = cfg.to_ui_account();
            ctx.accounts.set(HashMap::from([(id.clone(), ui)]));
            ctx.selected_account.set(Some(id.clone()));
            bootstrap.set(AppBootstrapState::Ready);
            return BootstrapOutcome {
                store,
                initial_bootstrap: Some(Some(id)),
            };
        }

        info!("Bootstrap: empty store → NeedsOnboarding");
        ctx.accounts.set(HashMap::new());
        ctx.selected_account.set(None);
        bootstrap.set(AppBootstrapState::NeedsOnboarding);
        return BootstrapOutcome {
            store,
            initial_bootstrap: Some(None),
        };
    }

    // Non-empty: UI accounts from store (to_ui_account only — no secrets).
    let mut map = HashMap::new();
    for cfg in &list {
        map.insert(cfg.id.clone(), cfg.to_ui_account());
    }
    ctx.accounts.set(map);

    let active = resolve_active_id(store.as_ref(), &list).await;
    ctx.selected_account.set(active.clone());
    info!(
        "Bootstrap: {} account(s) from store → Ready (active={:?})",
        list.len(),
        active.as_ref().map(|a| a.as_str())
    );
    bootstrap.set(AppBootstrapState::Ready);

    BootstrapOutcome {
        store,
        initial_bootstrap: Some(active),
    }
}

/// Resolve active account: stored id if valid, else first by `created_at` ascending.
async fn resolve_active_id(
    store: &dyn AccountStore,
    accounts: &[crate::account_config::AccountConfig],
) -> Option<AccountId> {
    if accounts.is_empty() {
        return None;
    }

    match store.get_active_id().await {
        Ok(Some(id)) if accounts.iter().any(|a| a.id == id) => return Some(id),
        Ok(Some(id)) => {
            warn!(
                "Bootstrap: active_account_id {} missing from store; picking first by created_at",
                id
            );
        }
        Ok(None) => {
            info!("Bootstrap: no active_account_id; picking first by created_at");
        }
        Err(e) => {
            warn!("Bootstrap: get_active_id failed ({e}); picking first by created_at");
        }
    }

    let mut ordered: Vec<&crate::account_config::AccountConfig> = accounts.iter().collect();
    ordered.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id.as_str().cmp(b.id.as_str()))
    });
    let id = ordered[0].id.clone();
    if let Err(e) = store.set_active_id(Some(&id)).await {
        warn!("Bootstrap: set_active_id({id}) failed: {e}");
    }
    Some(id)
}

#[component]
fn App() -> Element {
    let bootstrap_state = use_signal(|| AppBootstrapState::LoadingStore);
    let store_ctx = use_signal(|| None::<AccountStoreContext>);

    let selected_account = use_signal(|| None);
    let accounts = use_signal(HashMap::new);
    let connection_states = use_signal(HashMap::new);

    let mailbox_nodes = use_signal(HashMap::new);
    let mailbox_roots = use_signal(Vec::new);
    let selected_mailbox = use_signal(|| None);

    let messages = use_signal(|| SparseList::new(0));
    let messages_loading = use_signal(|| false);
    let selected_message = use_signal(|| None);
    let message_view = use_signal(|| crate::context::MessageViewState::Empty);
    let download_status = use_signal(HashMap::new);

    let ctx = AppContext {
        accounts,
        mailbox_nodes,
        mailbox_roots,
        messages,
        messages_loading,
        selected_mailbox,
        selected_account,
        selected_message,
        message_view,
        download_status,
        connection_states,
    };
    let ctx_clone = ctx.clone();

    use_context_provider(|| ctx);
    use_context_provider(|| bootstrap_state);
    use_context_provider(|| store_ctx);

    // Open BrowserAccountStore once; pass clone into core_loop; provide via context.
    // core_loop stays idle until bootstrap resolves, then runs initial Bootstrap if Ready.
    let _tx = use_coroutine(move |core_rx| {
        let mut ctx = ctx_clone.clone();
        let bootstrap_state = bootstrap_state;
        let store_ctx = store_ctx;
        async move {
            let outcome = run_bootstrap(&mut ctx, bootstrap_state, store_ctx).await;
            core_loop(core_rx, ctx, outcome.store, outcome.initial_bootstrap).await;
        }
    });

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }

        Router::<Route> {}
    }
}

/// Root layout: no mail chrome on loading / store error / onboarding; outlet otherwise.
#[component]
fn AppShell() -> Element {
    let bootstrap = use_context::<Signal<AppBootstrapState>>();
    let nav = use_navigator();

    // Deep-link guards (prefer replace to avoid back-stack traps).
    // Reads bootstrap signal + current route (subscribes via router) so both updates re-run.
    use_effect(move || {
        let state = bootstrap();
        let route = router().current::<Route>();
        match state {
            AppBootstrapState::NeedsOnboarding => {
                // Zero accounts + any non-onboarding route (incl. /settings/*) → /onboarding
                if !matches!(route, Route::OnboardingView {}) {
                    nav.replace(Route::OnboardingView {});
                }
            }
            AppBootstrapState::Ready => {
                // Non-empty (or dev_default) + /onboarding → /
                if matches!(route, Route::OnboardingView {}) {
                    nav.replace(Route::MainView {});
                }
            }
            AppBootstrapState::LoadingStore | AppBootstrapState::StoreError { .. } => {}
        }
    });

    match bootstrap() {
        AppBootstrapState::LoadingStore => rsx! {
            div {
                class: "bootstrap-shell",
                div {
                    class: "bootstrap-card",
                    p { class: "bootstrap-title", "Loading accounts…" }
                    p { class: "bootstrap-muted", "Opening local account storage." }
                }
            }
        },
        AppBootstrapState::StoreError { message } => rsx! {
            div {
                class: "bootstrap-shell",
                div {
                    class: "bootstrap-card bootstrap-error",
                    h1 { class: "bootstrap-title", "Account storage unavailable" }
                    p { "{message}" }
                    p {
                        class: "bootstrap-muted",
                        "Mailiner stores account settings in this browser only. \
                         Enable storage or try a different browser profile."
                    }
                }
            }
        },
        // Onboarding and settings share the outlet; mail chrome is only in MainView.
        AppBootstrapState::NeedsOnboarding | AppBootstrapState::Ready => rsx! {
            Outlet::<Route> {}
        },
    }
}

#[component]
fn MainView() -> Element {
    let bootstrap = use_context::<Signal<AppBootstrapState>>();

    // While NeedsOnboarding, guard redirects away; avoid mounting mail chrome.
    if !matches!(bootstrap(), AppBootstrapState::Ready) {
        return rsx! {
            div {
                class: "bootstrap-shell",
                p { class: "bootstrap-muted", "Redirecting…" }
            }
        };
    }

    rsx! {
        div {
            id: "app",

            EmailNavigation {}

            div {
                id: "content",

                MessageList {}

                MessageView {}
            }
        }
    }
}

/// First-run placeholder (full form is PR5). No mail chrome.
#[component]
fn OnboardingView() -> Element {
    rsx! {
        div {
            class: "bootstrap-shell",
            div {
                class: "bootstrap-card",
                h1 { class: "bootstrap-title", "Welcome to Mailiner" }
                p {
                    "Add your first email account to get started. \
                     Full onboarding form arrives in a follow-up."
                }
                p {
                    class: "bootstrap-muted",
                    "Your IMAP password will be stored only in this browser on this device. \
                     Mailiner has no server account."
                }
            }
        }
    }
}

/// Account list settings placeholder (full UI is PR6).
#[component]
fn AccountsSettingsView() -> Element {
    let ctx = use_context::<AppContext>();
    let accounts = ctx.accounts;

    rsx! {
        div {
            class: "bootstrap-shell",
            div {
                class: "bootstrap-card",
                h1 { class: "bootstrap-title", "Accounts" }
                p { class: "bootstrap-muted", "Account management UI (placeholder)." }

                ul {
                    class: "bootstrap-account-list",
                    for (_id, account) in accounts.read().iter() {
                        li {
                            Link {
                                to: Route::AccountEditView { id: account.id.as_str().to_string() },
                                "{account.name} — {account.email}"
                            }
                        }
                    }
                }

                nav {
                    class: "bootstrap-nav",
                    Link { to: Route::AccountNewView {}, "Add account" }
                    " · "
                    Link { to: Route::MainView {}, "Back to mail" }
                }
            }
        }
    }
}

/// Add-account placeholder (full form is PR5/PR6).
#[component]
fn AccountNewView() -> Element {
    rsx! {
        div {
            class: "bootstrap-shell",
            div {
                class: "bootstrap-card",
                h1 { class: "bootstrap-title", "Add account" }
                p { class: "bootstrap-muted", "New account form (placeholder)." }
                nav {
                    class: "bootstrap-nav",
                    Link { to: Route::AccountsSettingsView {}, "Back to accounts" }
                }
            }
        }
    }
}

/// Edit-account placeholder (full form is PR6).
#[component]
fn AccountEditView(id: String) -> Element {
    rsx! {
        div {
            class: "bootstrap-shell",
            div {
                class: "bootstrap-card",
                h1 { class: "bootstrap-title", "Edit account" }
                p { "Account id: {id}" }
                p { class: "bootstrap-muted", "Edit account form (placeholder)." }
                nav {
                    class: "bootstrap-nav",
                    Link { to: Route::AccountsSettingsView {}, "Back to accounts" }
                }
            }
        }
    }
}
