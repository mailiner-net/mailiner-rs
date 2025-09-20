use std::collections::HashMap;

use dioxus::prelude::*;

use crate::{
    components::{EmailNavigation, MessageView, Sidebar},
    context::AppContext,
    core_event::core_loop,
    mailbox::{MailboxId, MailboxNode},
};

mod components;
mod context;
mod core_event;
mod mailbox;

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

    let ctx = AppContext {
        mailbox_nodes,
        mailbox_roots,
        selected_mailbox,
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
