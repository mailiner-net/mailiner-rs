use dioxus::prelude::*;
use mailiner_core::MessageSort;

use super::super::icons::{Icon, IconKind};
use super::super::theme::ThemeSelect;

use crate::Route;
use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::mailbox::can_empty_trash;
use crate::ui_prefs::MessageListDensity;

/// Confirm before permanently emptying Trash. Non-web builds fail closed.
fn confirm_empty_trash() -> bool {
    #[cfg(feature = "web")]
    {
        web_sys::window()
            .and_then(|window| {
                window
                    .confirm_with_message("Permanently delete all messages in Trash?")
                    .ok()
            })
            .unwrap_or(false)
    }
    #[cfg(not(feature = "web"))]
    {
        false
    }
}

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
    let mut density = ctx.message_list_density;
    let current_density = *density.read();
    let supports_size_sender = *ctx.sort_supports_size_sender.read();
    let message_total = ctx.messages.read().total_count();
    let quota = *ctx.account_quota.read();

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
                        span { "{mailbox.title()}" }
                    } else {
                        span { "Messages" }
                    }
                } else if let Some(account) = current_account {
                    // Account name → settings list (switch / manage accounts).
                    Link {
                        to: Route::AccountsSettingsView {},
                        class: "pane-header-account-link",
                        title: "Manage accounts",
                        "{account.name}"
                    }
                    if let Some(quota) = quota {
                        span {
                            class: "pane-header-quota",
                            title: "{quota.used_percent()}% used",
                            "{quota.display()}"
                        }
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
                    class: "message-density",
                    aria_label: "Message list density",
                    title: "List density",
                    value: "{current_density.as_key()}",
                    onchange: move |evt| {
                        if let Some(next) = MessageListDensity::from_key(&evt.value()) {
                            crate::ui_prefs::save_message_list_density(next);
                            density.set(next);
                        }
                    },
                    for option in MessageListDensity::ALL {
                        option {
                            value: "{option.as_key()}",
                            selected: option == current_density,
                            "{option.label()}"
                        }
                    }
                }

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
                            title: if option == MessageSort::Date && !supports_size_sender {
                                "IMAP SORT unavailable; using arrival order (not the Date header)"
                            } else if option.needs_sort_capability() && !supports_size_sender {
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
                if let (Some(mailbox_id), Some(account_id)) =
                    (current_mailbox_id.clone(), current_account_id.clone())
                {
                    button {
                        class: "empty-trash",
                        title: "Permanently delete all messages in Trash",
                        aria_label: "Empty Trash",
                        onclick: move |_| {
                            if !confirm_empty_trash() {
                                return;
                            }
                            let _ = core_tx.send(CoreEvent::EmptyTrash {
                                account_id: account_id.clone(),
                                mailbox_id: mailbox_id.clone(),
                            });
                        },
                        "Empty Trash"
                    }
                }
            }

            if props.mode == Mode::MailboxTreeView {
                ThemeSelect { class: "theme-select pane-header-theme", }
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
