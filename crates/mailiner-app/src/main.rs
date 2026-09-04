use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use dioxus::logger::tracing::{info, warn};
use dioxus::prelude::*;
use mailiner_core::ids::AccountId;

use crate::account_store::{
    AccountStore, AccountStoreError, BrowserAccountStore, InMemoryAccountStore,
};
use crate::account_vault::VaultState;
use crate::components::virtual_scroll::SparseList;
use crate::components::{
    AccountEditPage, AccountNewPage, AccountsSettingsPage, ComposeOverlay, ConnectionStatusBanner,
    EmailNavigation, FolderSubscribeHost, MailboxPickerHost, MessageHeadersHost, MessageList,
    MessageSourceHost, MessageView, OauthCallbackPage, OnboardingForm, OutboxPanel, SettingsPage,
    ShortcutsHost, SplitAxis, SplitHandle, ToastHost,
};
use crate::context::AppContext;
use crate::core_event::{CoreEvent, InitialBootstrap, core_loop};
use crate::mail_cache::{BrowserMailCache, InMemoryMailCache, MailCache};
use crate::message_loader::LoadedMessageCache;
use crate::outbox_store::{BrowserOutboxStore, InMemoryOutboxStore, OutboxStore};

mod a11y;
mod account;
mod account_config;
mod account_store;
mod account_vault;
mod address_book;
mod autocrypt;
mod autodiscover;
mod background_sync;
mod components;
mod connection;
mod context;
mod conversation;
mod core_event;
mod download;
mod draft_store;
mod formatter;
mod headers;
mod i18n;
#[cfg(target_arch = "wasm32")]
mod idb;
mod keywords;
mod layout;
mod local_data;
mod mail_cache;
mod mail_file;
mod mail_rules;
mod mailbox;
mod message;
mod message_list_filter;
mod message_loader;
mod notifications;
mod oauth;
mod object_cache;
mod offline_cache;
mod outbox_store;
mod phishing;
mod pin;
mod print;
mod provider_preset;
mod recipient_suggest;
mod reconnect;
mod selection;
mod send;
mod shortcuts;
mod smime;
mod smtp_inflight;
mod smtp_session;
mod snippet;
mod snooze;
mod source;
mod toast;
mod ui_prefs;
mod unified_inbox;
mod vacation;
mod websocket_stream;

/// UI bootstrap state machine (store open → onboarding vs main).
#[derive(Clone, Debug, PartialEq)]
pub enum AppBootstrapState {
    /// Store open + list in flight. Full-page spinner; no mail chrome.
    LoadingStore,
    /// Zero accounts. Only onboarding is valid.
    NeedsOnboarding,
    /// Vault present; session key not yet derived.
    NeedsUnlock,
    /// Accounts loaded from store; main app allowed.
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
pub(crate) enum Route {
    #[layout(AppShell)]
    #[route("/")]
    MainView {},
    #[route("/onboarding")]
    OnboardingView {},
    #[route("/settings")]
    SettingsView {},
    #[route("/settings/accounts")]
    AccountsSettingsView {},
    #[route("/settings/accounts/new")]
    AccountNewView {},
    #[route("/settings/accounts/:id")]
    AccountEditView { id: String },
    #[route("/oauth/callback")]
    OauthCallbackView {},
}

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/main.css");
const MANIFEST_HREF: &str = "/manifest.webmanifest";
const APPLE_TOUCH_ICON_HREF: &str = "/icons/apple-touch-icon.png";
const THEME_COLOR: &str = "#121212";

/// Baseline Content-Security-Policy for the Mailiner origin (PR7).
///
/// Goals: reduce XSS impact (script injection → local secrets) while keeping
/// the Dioxus/WASM runtime, user-defined mail proxies, and intentional remote
/// message images working.
///
/// Tradeoffs:
/// - `script-src 'self' 'unsafe-eval' 'wasm-unsafe-eval'`: WASM instantiation
///   needs `wasm-unsafe-eval`. wasm-bindgen also builds closures with
///   `new Function(...)`, which requires `unsafe-eval`. No third-party scripts.
/// - `style-src 'self' 'unsafe-inline'`: Dioxus components (e.g. virtual list)
///   use inline `style=` attributes; strict style-src breaks layout. Remote
///   stylesheets are not allowed (formatter/sanitizer strips them).
/// - `img-src 'self' data: blob: http: https:`: cid→`data:` inlines and
///   `blob:` downloads / image previews, plus remote `http(s)` images when the
///   user clicks **Allow remote resources** in the message viewer. Privacy is
///   gated in the HTML formatter first; CSP must not veto that path. CSS
///   `url(...)` image loads are also constrained by `img-src` in most browsers.
/// - `frame-src 'self' blob:`: PDF attachment preview uses an `<iframe>` with a
///   `blob:` URL (`object-src` stays `'none'` so we never `<embed>` untrusted
///   documents).
/// - `connect-src 'self' ws: wss: http: https:`: user-entered proxy hosts make
///   a strict host allowlist impossible without dynamic CSP (limited in browsers).
///   Schemes stay open so self-hosted proxies work; IMAP traffic is still
///   TLS-wrapped client-side. This mitigates script XSS, not proxy diversity.
/// - Meta tag is injected after WASM/`App` mounts, so the initial document +
///   script load are not covered under `dx serve`. Cloudflare Pages deploy
///   sends this same policy as an **HTTP CSP header** (`_headers` in the built
///   public dir) for first-paint coverage. HMR/`dx serve` may inject scripts
///   after mount and does not send the header.
const CONTENT_SECURITY_POLICY: &str = "\
default-src 'self'; \
script-src 'self' 'unsafe-eval' 'wasm-unsafe-eval'; \
style-src 'self' 'unsafe-inline'; \
img-src 'self' data: blob: http: https:; \
font-src 'self'; \
connect-src 'self' ws: wss: http: https:; \
frame-src 'self' blob:; \
object-src 'none'; \
base-uri 'self'; \
form-action 'self'\
";

fn main() {
    register_service_worker();
    dioxus::launch(App);
}

/// Register the app-shell service worker so the origin is installable.
///
/// No-op off wasm32 or when `navigator.serviceWorker` is missing (insecure
/// context, unsupported browser). Mail bodies are not cached by the worker.
fn register_service_worker() {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(window) = web_sys::window() else {
            return;
        };
        let navigator = window.navigator();
        if !js_sys::Reflect::has(
            &navigator,
            &wasm_bindgen::JsValue::from_str("serviceWorker"),
        )
        .unwrap_or(false)
        {
            return;
        }
        let _ = navigator.service_worker().register("/sw.js");
    }
}

/// Result of opening the store and applying the bootstrap resolution algorithm.
struct BootstrapOutcome {
    store: Rc<dyn AccountStore>,
    outbox: Rc<dyn OutboxStore>,
    cache: Rc<dyn MailCache>,
    initial_bootstrap: InitialBootstrap,
}

/// Open `BrowserAccountStore`, resolve bootstrap state, populate UI accounts (no secrets).
///
/// Algorithm:
/// - open failure → StoreError
/// - empty → NeedsOnboarding (form may be prefilled via `dev-defaults`; no auto-connect)
/// - non-empty → Ready, UI from `to_ui_account`, resolve active, Bootstrap { active }
async fn run_bootstrap(
    ctx: &mut AppContext,
    mut bootstrap: Signal<AppBootstrapState>,
    mut store_ctx: Signal<Option<AccountStoreContext>>,
) -> BootstrapOutcome {
    let outbox: Rc<dyn OutboxStore> = match BrowserOutboxStore::open().await {
        Ok(s) => Rc::new(s),
        Err(e) => {
            warn!("BrowserOutboxStore open failed ({e}); using in-memory outbox");
            Rc::new(InMemoryOutboxStore::new())
        }
    };

    let cache: Rc<dyn MailCache> = open_mail_cache().await;

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
                outbox,
                cache,
                initial_bootstrap: InitialBootstrap::Skip,
            };
        }
    };

    store_ctx.set(Some(AccountStoreContext(store.clone())));

    if store.vault_state() == VaultState::Locked {
        info!("Bootstrap: encrypted store → NeedsUnlock");
        ctx.accounts.set(HashMap::new());
        ctx.selected_account.set(None);
        bootstrap.set(AppBootstrapState::NeedsUnlock);
        return BootstrapOutcome {
            store,
            outbox,
            cache,
            initial_bootstrap: InitialBootstrap::Skip,
        };
    }

    let list = match store.list().await {
        Ok(list) => list,
        Err(e) => {
            let message = format!("Failed to read accounts from storage: {e}");
            warn!("{message}");
            bootstrap.set(AppBootstrapState::StoreError { message });
            return BootstrapOutcome {
                store,
                outbox,
                cache,
                initial_bootstrap: InitialBootstrap::Skip,
            };
        }
    };

    if list.is_empty() {
        info!("Bootstrap: empty store → NeedsOnboarding");
        ctx.accounts.set(HashMap::new());
        ctx.selected_account.set(None);
        bootstrap.set(AppBootstrapState::NeedsOnboarding);
        return BootstrapOutcome {
            store,
            outbox,
            cache,
            initial_bootstrap: InitialBootstrap::Run { active: None },
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
    if let Some(account_id) = active.as_ref() {
        crate::core_event::hydrate_account_into(cache.as_ref(), ctx, account_id).await;
    }
    info!(
        "Bootstrap: {} account(s) from store → Ready (active={:?})",
        list.len(),
        active.as_ref().map(|a| a.as_str())
    );
    bootstrap.set(AppBootstrapState::Ready);

    BootstrapOutcome {
        store,
        outbox,
        cache,
        initial_bootstrap: InitialBootstrap::Run { active },
    }
}

/// IndexedDB first (envelopes + parts); localStorage prefixes if IDB is blocked.
async fn open_mail_cache() -> Rc<dyn MailCache> {
    #[cfg(target_arch = "wasm32")]
    {
        match crate::idb::open_indexed_db_mail_cache().await {
            Ok(cache) => return Rc::new(cache),
            Err(e) => {
                warn!("IndexedDB mail cache unavailable ({e}); falling back to localStorage");
            }
        }
    }
    match BrowserMailCache::open().await {
        Ok(s) => Rc::new(s),
        Err(e) => {
            warn!("BrowserMailCache open failed ({e}); using in-memory mail cache");
            Rc::new(InMemoryMailCache::new())
        }
    }
}

/// Resolve active account: stored id if valid, else first by `created_at` ascending.
pub(crate) async fn resolve_active_id(
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
    let list_text_filter = use_signal(String::new);
    let list_search_query = use_signal(String::new);
    let saved_searches = use_signal(crate::ui_prefs::load_saved_searches);
    let active_saved_search = use_signal(|| None::<String>);
    let messages_loading = use_signal(|| false);
    let message_sort = use_signal(crate::ui_prefs::load_message_sort);
    let message_list_density = use_signal(crate::ui_prefs::load_message_list_density);
    let message_list_view = use_signal(crate::ui_prefs::load_message_list_view);
    let expanded_conversations = use_signal(HashSet::new);
    let message_list_filter = use_signal(crate::ui_prefs::load_message_list_filter);
    let sort_supports_size_sender = use_signal(|| false);
    let account_quota = use_signal(|| None);
    let selection = use_signal(crate::selection::MessageSelection::default);
    let message_view = use_signal(|| crate::context::MessageViewState::Empty);
    let message_headers = use_signal(|| crate::context::MessageHeadersState::Closed);
    let message_source = use_signal(|| crate::context::MessageSourceState::Closed);
    let download_status = use_signal(HashMap::new);
    let attachment_blobs = use_signal(HashMap::new);
    let attachment_preview = use_signal(|| None);
    let nested_rfc822 = use_signal(Vec::new);
    let send_status = use_signal(HashMap::new);
    let smtp_test_status = use_signal(HashMap::new);
    let smtp_test_abandoned = use_signal(HashSet::new);
    let outbox = use_signal(Vec::new);
    let toast = use_signal(|| None);
    let compose_draft = use_signal(|| None);
    let compose_placement = use_signal(crate::ui_prefs::load_compose_placement);
    let mail_layout = use_signal(crate::ui_prefs::load_mail_layout);
    let mobile_pane = use_signal(crate::layout::MobilePane::default);
    let mailbox_picker = use_signal(|| None);
    let theme = use_signal(|| {
        let pref = crate::ui_prefs::load_theme();
        crate::ui_prefs::apply_theme(pref);
        pref
    });
    let sign_out_epoch = use_signal(|| 0u64);
    let sign_out_pending = use_signal(|| false);
    let sign_out_started = use_signal(|| 0u64);
    let sign_out_error = use_signal(|| None::<String>);
    let message_drag = use_signal(|| None);
    let notify_inbox = use_signal(crate::ui_prefs::load_notify_inbox);
    let folder_subscribe_open = use_signal(|| false);
    let show_all_folders = use_signal(crate::ui_prefs::load_show_all_folders);
    let pinned_uids = use_signal(Vec::new);
    let sr_status = use_signal(String::new);
    let snoozed_messages = use_signal(Vec::new);
    let snooze_picker_open = use_signal(|| false);
    let account_inbox_unread = use_signal(HashMap::new);
    let unified_inbox_notes = use_signal(Vec::new);
    let locale = use_signal(|| {
        let locale = crate::ui_prefs::load_locale();
        crate::i18n::set_locale(locale);
        locale
    });

    let ctx = AppContext {
        accounts,
        mailbox_nodes,
        mailbox_roots,
        messages,
        list_text_filter,
        list_search_query,
        saved_searches,
        active_saved_search,
        messages_loading,
        message_sort,
        message_list_density,
        message_list_view,
        expanded_conversations,
        message_list_filter,
        sort_supports_size_sender,
        account_quota,
        selected_mailbox,
        selected_account,
        selection,
        message_view,
        message_bodies: Rc::new(std::cell::RefCell::new(LoadedMessageCache::new())),
        message_headers,
        message_source,
        download_status,
        attachment_blobs,
        attachment_preview,
        nested_rfc822,
        connection_states,
        send_status,
        smtp_test_status,
        smtp_test_abandoned,
        outbox,
        toast,
        compose_draft,
        compose_placement,
        mail_layout,
        mobile_pane,
        mailbox_picker,
        sign_out_epoch,
        sign_out_pending,
        sign_out_started,
        sign_out_error,
        message_drag,
        notify_inbox,
        folder_subscribe_open,
        show_all_folders,
        pinned_uids,
        sr_status,
        snoozed_messages,
        snooze_picker_open,
        account_inbox_unread,
        unified_inbox_notes,
        locale,
    };
    let ctx_clone = ctx.clone();

    use_context_provider(|| ctx);
    use_context_provider(|| bootstrap_state);
    use_context_provider(|| store_ctx);
    use_context_provider(|| theme);

    // Open BrowserAccountStore once; pass clone into core_loop; provide via context.
    // core_loop stays idle until bootstrap resolves, then runs initial Bootstrap if Ready.
    let _tx = use_coroutine(move |core_rx| {
        let mut ctx = ctx_clone.clone();
        let bootstrap_state = bootstrap_state;
        let store_ctx = store_ctx;
        async move {
            let outcome = run_bootstrap(&mut ctx, bootstrap_state, store_ctx).await;
            let (smtp_tx, smtp_rx) = futures_channel::mpsc::unbounded();
            core_loop(
                core_rx,
                smtp_rx,
                smtp_tx,
                ctx,
                outcome.store,
                outcome.outbox,
                outcome.cache,
                outcome.initial_bootstrap,
            )
            .await;
        }
    });

    let snooze_tx = _tx;
    use_hook(move || {
        spawn(async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(15_000).await;
                let _ = snooze_tx.send(CoreEvent::SweepSnooze);
            }
        });
    });

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        document::Link { rel: "manifest", href: MANIFEST_HREF }
        document::Link { rel: "apple-touch-icon", href: APPLE_TOUCH_ICON_HREF }
        document::Meta {
            name: "viewport",
            content: "width=device-width, initial-scale=1",
        }
        document::Meta {
            name: "theme-color",
            content: THEME_COLOR,
        }
        document::Meta {
            http_equiv: "Content-Security-Policy",
            content: CONTENT_SECURITY_POLICY,
        }
        TabTitle {}

        Router::<Route> {}
    }
}

/// Inbox unread on `document.title`. Updates whenever folder counts change.
#[component]
fn TabTitle() -> Element {
    let ctx = use_context::<AppContext>();
    let unread_map = ctx.account_inbox_unread.read();
    let unread = if unread_map.is_empty() {
        crate::notifications::inbox_unread(&ctx.mailbox_nodes.read())
            .map(|(_, n)| n)
            .unwrap_or(0)
    } else {
        crate::unified_inbox::sum_inbox_unread(unread_map.values().copied()) as usize
    };
    let title = crate::notifications::tab_title(unread);
    rsx! { document::Title { "{title}" } }
}

/// Root layout: no mail chrome on loading / store error / onboarding; outlet otherwise.
#[component]
fn AppShell() -> Element {
    let route = router().current::<Route>();
    if matches!(route, Route::OauthCallbackView {}) {
        return rsx! { Outlet::<Route> {} };
    }

    let mut bootstrap = use_context::<Signal<AppBootstrapState>>();
    let nav = use_navigator();
    let ctx = use_context::<AppContext>();
    let epoch = ctx.sign_out_epoch;
    let mut pending = ctx.sign_out_pending;
    let started = ctx.sign_out_started;
    let mut sign_out_error = ctx.sign_out_error;

    // Survives leaving the accounts confirm dialog so a finished wipe still
    // reaches onboarding even if the user navigated away mid-delete.
    use_effect(move || {
        let current = epoch();
        if let Some(_err) = sign_out_error() {
            pending.set(false);
            sign_out_error.set(None);
            return;
        }
        if pending() && current != started() {
            pending.set(false);
            info!("Signed out → NeedsOnboarding");
            bootstrap.set(AppBootstrapState::NeedsOnboarding);
        }
    });

    // Deep-link guards (prefer replace to avoid back-stack traps).
    // Reads bootstrap signal + current route (subscribes via router) so both updates re-run.
    use_effect(move || {
        let state = bootstrap();
        let route = router().current::<Route>();
        match state {
            AppBootstrapState::NeedsOnboarding => {
                // Zero accounts + any non-onboarding route (incl. /settings/*) → /onboarding
                if !matches!(
                    route,
                    Route::OnboardingView {} | Route::OauthCallbackView {}
                ) {
                    nav.replace(Route::OnboardingView {});
                }
            }
            AppBootstrapState::Ready => {
                // Non-empty store (or post-commit Ready) + /onboarding → /
                if matches!(route, Route::OnboardingView {}) {
                    nav.replace(Route::MainView {});
                }
            }
            AppBootstrapState::LoadingStore
            | AppBootstrapState::NeedsUnlock
            | AppBootstrapState::StoreError { .. } => {}
        }
    });

    match bootstrap() {
        AppBootstrapState::LoadingStore => rsx! {
            main {
                class: "bootstrap-shell",
                div {
                    class: "bootstrap-card",
                    p { class: "bootstrap-title", "Loading accounts…" }
                    p { class: "bootstrap-muted", "Opening local account storage." }
                }
            }
        },
        AppBootstrapState::StoreError { message } => rsx! {
            main {
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
        AppBootstrapState::NeedsUnlock => rsx! {
            crate::components::UnlockForm {}
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
    let ctx = use_context::<AppContext>();
    let mail_layout = *ctx.mail_layout.read();
    let mobile_pane = *ctx.mobile_pane.read();
    let list_axis = match mail_layout {
        crate::ui_prefs::MailLayout::Stacked => SplitAxis::List,
        crate::ui_prefs::MailLayout::Classic => SplitAxis::ListWidth,
    };

    // While NeedsOnboarding, guard redirects away; avoid mounting mail chrome.
    if !matches!(bootstrap(), AppBootstrapState::Ready) {
        return redirecting_shell();
    }

    rsx! {
        div {
            class: "mail-shell",
            a {
                class: "skip-link",
                href: "#{crate::a11y::SKIP_TO_MESSAGE_ID}",
                onclick: move |evt| {
                    evt.prevent_default();
                    crate::a11y::focus_element_by_id(crate::a11y::SKIP_TO_MESSAGE_ID);
                },
                {crate::i18n::t("nav.skip_to_message")}
            }
            div {
                id: "app",
                class: "{mail_layout.css_class()} {mobile_pane.css_class()}",
                onmounted: move |_| {
                    crate::layout::apply_saved_layout();
                },

                EmailNavigation {}
                SplitHandle { axis: SplitAxis::Folder }

                div {
                    id: "content",

                    ConnectionStatusBanner {}

                    MessageList {}
                    SplitHandle { axis: list_axis }
                    MessageView {}

                    OutboxPanel {}
                }
            }

            ComposeOverlay {}
        }

        LiveStatus {}
        ToastHost {}
        MailboxPickerHost {}
        MessageHeadersHost {}
        MessageSourceHost {}
        FolderSubscribeHost {}
        ShortcutsHost {}
    }
}

/// First-run onboarding form (connect-before-persist). No mail chrome.
#[component]
fn OnboardingView() -> Element {
    rsx! {
        OnboardingForm {}
    }
}

/// OAuth popup / same-tab redirect target. No account store required.
#[component]
fn OauthCallbackView() -> Element {
    rsx! { OauthCallbackPage {} }
}

/// Minimal shell while deep-link guards redirect away from settings/main.
fn redirecting_shell() -> Element {
    rsx! {
        main {
            class: "bootstrap-shell",
            p { class: "bootstrap-muted", "Redirecting…" }
        }
    }
}

/// Polite live region for new-mail announcements (toasts have their own status).
#[component]
fn LiveStatus() -> Element {
    let ctx = use_context::<AppContext>();
    let text = ctx.sr_status.read().clone();
    rsx! {
        div {
            class: "sr-only",
            role: "status",
            aria_live: "polite",
            aria_atomic: "true",
            "{text}"
        }
    }
}

/// General settings (`/settings`).
#[component]
fn SettingsView() -> Element {
    let bootstrap = use_context::<Signal<AppBootstrapState>>();
    if !matches!(bootstrap(), AppBootstrapState::Ready) {
        return redirecting_shell();
    }
    rsx! { SettingsPage {} }
}

/// Account list settings (`/settings/accounts`).
#[component]
fn AccountsSettingsView() -> Element {
    let bootstrap = use_context::<Signal<AppBootstrapState>>();
    // Settings require Ready. Under NeedsOnboarding the deep-link guard replace()s
    // to /onboarding; avoid flashing this view for a frame before the effect runs.
    if !matches!(bootstrap(), AppBootstrapState::Ready) {
        return redirecting_shell();
    }
    rsx! { AccountsSettingsPage {} }
}

/// Add account (`/settings/accounts/new`) — CommitNewAccount path.
#[component]
fn AccountNewView() -> Element {
    let bootstrap = use_context::<Signal<AppBootstrapState>>();
    if !matches!(bootstrap(), AppBootstrapState::Ready) {
        return redirecting_shell();
    }
    rsx! { AccountNewPage {} }
}

/// Edit account (`/settings/accounts/:id`).
#[component]
fn AccountEditView(id: String) -> Element {
    let bootstrap = use_context::<Signal<AppBootstrapState>>();
    if !matches!(bootstrap(), AppBootstrapState::Ready) {
        return redirecting_shell();
    }
    rsx! { AccountEditPage { id } }
}
