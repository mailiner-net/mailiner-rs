use dioxus::prelude::*;
use dioxus_heroicons::{Icon, solid::Shape};

use crate::Route;
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
    let ctx = use_context::<AppContext>();
    let mailboxes = ctx.mailbox_nodes.read();
    let accounts = ctx.accounts.read();
    let current_mailbox_id = ctx.selected_mailbox.read();
    let current_account_id = ctx.selected_account.read();

    let current_mailbox = current_mailbox_id.as_ref().and_then(|id| mailboxes.get(id));
    let current_account = current_account_id.as_ref().and_then(|id| accounts.get(id));
    rsx! {
        header {
            class: "pane-header",

            Icon {
                size: 24,
                icon: if props.mode == Mode::MailboxTreeView { Shape::User } else { Shape::Folder },
            }

            div {
                class: "pane-header-title",
                if props.mode == Mode::MessageList {
                    if let Some(mailbox) = current_mailbox {
                        "{mailbox.title()}"
                    } else {
                        "Messages"
                    }
                } else if let Some(account) = current_account {
                    // Account name → settings list (switch / manage accounts).
                    Link {
                        to: Route::AccountsSettingsView {},
                        class: "pane-header-account-link",
                        title: "Manage accounts",
                        "{account.name}"
                    }
                } else {
                    Link {
                        to: Route::AccountsSettingsView {},
                        class: "pane-header-account-link",
                        title: "Manage accounts",
                        "Accounts"
                    }
                }
            }

            if props.mode == Mode::MailboxTreeView {
                Link {
                    to: Route::AccountsSettingsView {},
                    class: "pane-header-settings",
                    title: "Account settings",
                    aria_label: "Account settings",
                    Icon {
                        size: 22,
                        icon: Shape::Cog6Tooth,
                    }
                }
            }
        }
    }
}
