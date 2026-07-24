use std::collections::HashMap;
use std::rc::Rc;

use dioxus::prelude::*;

use crate::account_config::dev_default_config;
use crate::account_store::InMemoryAccountStore;
use crate::components::virtual_scroll::SparseList;
use crate::components::{EmailNavigation, MessageList, MessageView};
use crate::context::AppContext;
use crate::core_event::{CoreEvent, core_loop};

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

#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
enum Route {
    #[layout(MainLayout)]
    #[route("/")]
    MainView {},
}

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/main.css");

fn main() {
    dioxus::launch(App);
}

#[component]
fn MainLayout() -> Element {
    rsx! {
        Outlet::<Route> {}
    }
}

#[component]
fn App() -> Element {
    // Interim (PR2–PR4): empty InMemory store + optional dev_default synthetic account.
    // No store write for dev_default; BrowserAccountStore lands in PR3.
    let store: Rc<dyn crate::account_store::AccountStore> = Rc::new(InMemoryAccountStore::new());

    let (initial_accounts, initial_selected) = if let Some(cfg) = dev_default_config() {
        let id = cfg.id.clone();
        let ui = cfg.to_ui_account();
        (HashMap::from([(id.clone(), ui)]), Some(id))
    } else {
        (HashMap::new(), None)
    };

    let selected_account = use_signal(|| initial_selected.clone());
    let accounts = use_signal(|| initial_accounts);
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
    let store_for_core = store.clone();

    use_context_provider(|| ctx);
    let tx = use_coroutine(move |core_rx| {
        let ctx = ctx_clone.clone();
        let store = store_for_core.clone();
        async move { core_loop(core_rx, ctx, store).await }
    });

    // Bootstrap: connect active (dev_default or later store-backed) with soft-fail.
    tx.send(CoreEvent::Bootstrap {
        active: initial_selected,
    });

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }

        Router::<Route> {}
    }
}

#[component]
fn MainView() -> Element {
    rsx! {
        div {
            id: "app",

            EmailNavigation {
            }

            div {
                id: "content",

                MessageList {
                }

                MessageView {
                }
            }
        }
    }
}
