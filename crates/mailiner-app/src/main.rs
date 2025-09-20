use std::collections::HashMap;
use std::sync::Arc;

use dioxus::prelude::*;

use crate::account::{Account, AccountId};
use crate::components::{EmailNavigation, MessageView, Sidebar};
use crate::context::AppContext;
use crate::core_event::core_loop;
use crate::mailbox::{MailboxId, MailboxNode};
use crate::message::{Message, MessageId};

mod account;
mod components;
mod context;
mod core_event;
mod mailbox;
mod message;

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
    let dummy_account_id = AccountId::new();

    let selected_account = use_signal(|| Some(dummy_account_id.clone()));
    let accounts = use_signal(|| {
        HashMap::from([(
            dummy_account_id.clone(),
            Account {
                id: dummy_account_id,
                name: "Valhalla".to_string(),
                email: "me@dvratil.cz".to_string(),
            },
        )])
    });

    let mailbox_nodes = use_signal(|| {
        HashMap::from([
            (
                MailboxId::from("INBOX".to_string()),
                MailboxNode {
                    id: MailboxId::from("INBOX".to_string()),
                    name: "INBOX".to_string(),
                    parent: None,
                    children: vec![],
                    unread_count: 0,
                    total_count: 0,
                },
            ),
            (
                MailboxId::from("Sent".to_string()),
                MailboxNode {
                    id: MailboxId::from("Sent".to_string()),
                    name: "Sent".to_string(),
                    parent: None,
                    children: vec![],
                    unread_count: 0,
                    total_count: 0,
                },
            ),
            (
                MailboxId::from("Drafts".to_string()),
                MailboxNode {
                    id: MailboxId::from("Drafts".to_string()),
                    name: "Drafts".to_string(),
                    parent: None,
                    children: vec![],
                    unread_count: 0,
                    total_count: 0,
                },
            ),
        ])
    });
    let mailbox_roots = use_signal(|| {
        Vec::from([
            MailboxId::from("INBOX".to_string()),
            MailboxId::from("Sent".to_string()),
            MailboxId::from("Drafts".to_string()),
        ])
    });
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
    use_coroutine(move |core_rx| {
        let ctx = ctx_clone.clone();
        async move { core_loop(core_rx, ctx).await }
    });

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }

        Router::<Route> {}

        div {
            id: "app",
        }
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
