use std::collections::HashMap;

use dioxus::prelude::*;

use crate::account::{Account, AccountId};
use crate::components::virtual_scroll::SparseList;
use crate::components::{EmailNavigation, MessageList, MessageView};
use crate::context::AppContext;
use crate::core_event::{core_loop, CoreEvent};

mod account;
// Wired for later PRs; unused by the binary runtime until connection/bootstrap land.
#[allow(dead_code)]
mod account_config;
#[allow(dead_code)]
mod account_store;
mod components;
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
    let dummy_account_id = AccountId::new("1");

    let selected_account = use_signal(|| Some(dummy_account_id.clone()));
    let accounts = use_signal(|| {
        HashMap::from([(
            dummy_account_id.clone(),
            Account {
                id: dummy_account_id.clone(),
                name: "Valhalla".to_string(),
                email: "me@dvratil.cz".to_string(),
            },
        )])
    });

    let mailbox_nodes = use_signal(|| HashMap::new());
    let mailbox_roots = use_signal(|| { Vec::new() });
    let selected_mailbox = use_signal(|| None);

    let messages = use_signal(|| SparseList::new(0));
    let messages_loading = use_signal(|| false);
    let selected_message = use_signal(|| None);
    let message_view = use_signal(|| crate::context::MessageViewState::Empty);
    let download_status = use_signal(std::collections::HashMap::new);

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
    };
    let ctx_clone = ctx.clone();

    use_context_provider(|| ctx);
    let tx = use_coroutine(move |core_rx| {
        let ctx = ctx_clone.clone();
        async move { core_loop(core_rx, ctx).await }
    });
    tx.send(CoreEvent::SelectAccount(dummy_account_id.clone()));

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
