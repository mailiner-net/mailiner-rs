use dioxus::prelude::*;
use dioxus_heroicons::{solid::Shape, Icon, IconButton};

use crate::context::AppContext;

#[derive(PartialEq, Clone, Copy)]
pub enum Mode {
    MailboxTreeView,
    MessageList,
}

#[derive(PartialEq, Clone, Props)]
pub struct EmailNavigationHeaderProps {
    pub mode: Mode,
}

#[component]
pub fn NavigationHeader(props: EmailNavigationHeaderProps) -> Element {
    let mut ctx = use_context::<AppContext>();
    let mailboxes = ctx.mailbox_nodes.read();
    let accounts = ctx.accounts.read();
    let current_mailbox_id = ctx.selected_mailbox.read();
    let current_account_id = ctx.selected_account.read();

    let current_mailbox = current_mailbox_id
        .as_ref()
        .and_then(|id| mailboxes.get(&id));
    let current_account = current_account_id.as_ref().and_then(|id| accounts.get(&id));
    rsx! {
        header {
            id: "navigation-header",

            IconButton {
                class: "back-button",

                onclick: move |_| {
                    ctx.selected_mailbox.set(None);
                },

                size: 24,
                icon: Shape::ChevronLeft,
            }

            Icon {
                size: 24,
                icon: if props.mode == Mode::MailboxTreeView { Shape::User } else { Shape::Folder },
            }

            div {
                if let Some(mailbox) = current_mailbox {
                    "{mailbox.name}"
                } else if let Some(account) = current_account {
                    "{account.name}"
                } else {
                    "Accounts"
                }
            }
        }
    }
}
