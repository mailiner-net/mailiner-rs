use std::collections::HashMap;
use std::sync::Arc;

use dioxus::logger::tracing::Level;
use dioxus::prelude::*;

use crate::account::{Account, AccountId};
use crate::components::{EmailNavigation, MessageView, Sidebar};
use crate::context::AppContext;
use crate::core_event::{core_loop, CoreEvent};
use crate::mailbox::{MailboxId, MailboxNode};
use crate::message::{Message, MessageId};

mod account;
mod components;
mod context;
mod core_event;
mod mailbox;
mod message;
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
        div {
            id: "app",

            Outlet::<Route> {}
        }
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

    let messages = use_signal(|| {
        vec![
            Arc::new(Message {
                id: MessageId::from("1".to_string()),
                subject: "Hello".to_string(),
                from: "John Doe".to_string(),
                to: "Jane Doe".to_string(),
                cc: "".to_string(),
                bcc: "".to_string(),
            }),
            Arc::new(Message {
                id: MessageId::from("2".to_string()),
                subject: "Hello".to_string(),
                from: "John Doe".to_string(),
                to: "Jane Doe".to_string(),
                cc: "".to_string(),
                bcc: "".to_string(),
            }),
        ]
    });
    let selected_message = use_signal(|| None);

    let ctx = AppContext {
        accounts,
        mailbox_nodes,
        mailbox_roots,
        messages,

        selected_mailbox,
        selected_account,
        selected_message,
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

            Sidebar {
            }

            EmailNavigation {
            }

            MessageView {
            }
        }
    }
}
