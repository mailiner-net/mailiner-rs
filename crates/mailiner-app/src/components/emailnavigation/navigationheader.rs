use dioxus::prelude::*;
use mailiner_core::MessageSort;

use super::super::icons::{Icon, IconKind};

use crate::Route;
use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::mailbox::can_empty_trash;

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
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let mailboxes = ctx.mailbox_nodes.read();
    let accounts = ctx.accounts.read();
    let current_mailbox_id = ctx.selected_mailbox.read();
    let current_account_id = ctx.selected_account.read();
    let sort = *ctx.message_sort.read();
    let supports_size_sender = *ctx.sort_supports_size_sender.read();
    let message_total = ctx.messages.read().total_count();

    let current_mailbox = current_mailbox_id.as_ref().and_then(|id| mailboxes.get(id));
    let current_account = current_account_id.as_ref().and_then(|id| accounts.get(id));
    let show_empty_trash = props.mode == Mode::MessageList
        && current_mailbox.is_some_and(can_empty_trash)
        && message_total > 0;
    rsx! {
        header {
            class: "pane-header",

            Icon {
                size: 24,
                icon: if props.mode == Mode::MailboxTreeView { IconKind::User } else { IconKind::Folder },
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

            if props.mode == Mode::MessageList {
                select {
                    class: "message-sort",
                    aria_label: "Sort messages",
                    title: "Sort messages",
                    value: "{sort.as_key()}",
                    onchange: move |evt| {
                        if let Some(next) = MessageSort::from_key(&evt.value()) {
                            let _ = core_tx.send(CoreEvent::SetMessageSort(next));
                        }
                    },
                    for option in MessageSort::ALL {
                        option {
                            value: "{option.as_key()}",
                            selected: option == sort,
                            disabled: option.needs_sort_capability() && !supports_size_sender,
                            title: if option.needs_sort_capability() && !supports_size_sender {
                                "This server does not support IMAP SORT"
                            } else {
                                ""
                            },
                            "{option.label()}"
                        }
                    }
                }
            }

            if show_empty_trash {
                if let Some(mailbox_id) = current_mailbox_id.clone() {
                    button {
                        class: "empty-trash",
                        title: "Permanently delete all messages in Trash",
                        aria_label: "Empty Trash",
                        onclick: move |_| {
                            #[cfg(feature = "web")]
                            {
                                let Some(window) = web_sys::window() else {
                                    return;
                                };
                                match window.confirm_with_message(
                                    "Permanently delete all messages in Trash?",
                                ) {
                                    Ok(true) => {}
                                    _ => return,
                                }
                            }
                            let _ = core_tx.send(CoreEvent::EmptyTrash {
                                mailbox_id: mailbox_id.clone(),
                            });
                        },
                        "Empty Trash"
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
                        icon: IconKind::Cog6Tooth,
                    }
                }
            }
        }
    }
}
