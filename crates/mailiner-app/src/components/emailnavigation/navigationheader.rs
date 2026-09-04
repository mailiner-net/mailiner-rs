use dioxus::prelude::*;
use mailiner_core::MessageSort;

use super::super::icons::{Icon, IconKind};
use super::super::theme::ThemeSelect;

use super::mailboxtreeview::prompt_folder_name;

use crate::Route;
use crate::account::AccountId;
use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::download::{DownloadStatus, MAIL_IMPORT_KEY, MAX_DOWNLOAD_BYTES};
use crate::mail_file::parse_import_files;
use crate::mailbox::{MailboxId, can_empty_trash};
use crate::toast::ToastAction;
use crate::ui_prefs::{MessageListDensity, MessageListView};

async fn import_selected_files(
    ctx: AppContext,
    core_tx: Coroutine<CoreEvent>,
    account_id: AccountId,
    mailbox_id: MailboxId,
    files: Vec<dioxus::html::FileData>,
) {
    let mut raw = Vec::new();
    for file in files {
        let filename = file.name();
        if file.size() > MAX_DOWNLOAD_BYTES as u64 {
            ctx.show_toast(ToastAction::error(format!(
                "\"{filename}\" is too large to import (max {} bytes)",
                MAX_DOWNLOAD_BYTES
            )));
            return;
        }
        match file.read_bytes().await {
            Ok(bytes) => raw.push((filename, bytes.to_vec())),
            Err(_) => {
                ctx.show_toast(ToastAction::error(format!("Could not read \"{filename}\"")));
                return;
            }
        }
    }
    match parse_import_files(raw) {
        Ok(messages) => {
            let _ = core_tx.send(CoreEvent::ImportMessages {
                account_id,
                mailbox_id,
                messages,
            });
        }
        Err(e) => ctx.show_toast(ToastAction::error(e)),
    }
}

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

/// Narrow-viewport back control. Hidden on desktop by CSS.
#[component]
pub fn MobileBackButton() -> Element {
    let ctx = use_context::<AppContext>();
    rsx! {
        button {
            r#type: "button",
            class: "mobile-back",
            title: "Back",
            aria_label: "Back",
            onclick: move |_| ctx.mobile_back(),
            Icon {
                size: 20,
                icon: IconKind::ChevronLeft,
            }
            "Back"
        }
    }
}

#[component]
pub fn NavigationHeader(props: EmailNavigationHeaderProps) -> Element {
    let ctx = use_context::<AppContext>();
    let mut folder_subscribe_open = ctx.folder_subscribe_open;
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let mailboxes = ctx.mailbox_nodes.read();
    let accounts = ctx.accounts.read();
    let current_mailbox_id = ctx.selected_mailbox.read();
    let current_account_id = ctx.selected_account.read();
    let sort = *ctx.message_sort.read();
    let mut density = ctx.message_list_density;
    let current_density = *density.read();
    let mut list_view = ctx.message_list_view;
    let current_view = *list_view.read();
    let mut expanded_conversations = ctx.expanded_conversations;
    let filter = *ctx.message_list_filter.read();
    let supports_size_sender = *ctx.sort_supports_size_sender.read();
    let message_total = ctx.messages.read().total_count();
    let quota = *ctx.account_quota.read();

    let current_mailbox = current_mailbox_id.as_ref().and_then(|id| mailboxes.get(id));
    let current_account = current_account_id.as_ref().and_then(|id| accounts.get(id));
    let show_empty_trash = props.mode == Mode::MessageList
        && current_mailbox.is_some_and(can_empty_trash)
        && message_total > 0;
    let import_busy = matches!(
        ctx.download_status.read().get(MAIL_IMPORT_KEY),
        Some(DownloadStatus::Queued | DownloadStatus::InProgress { .. })
    );
    let can_import = props.mode == Mode::MessageList
        && current_account_id.is_some()
        && current_mailbox.is_some_and(|n| n.selectable)
        && !import_busy;
    let mut import_input_gen = use_signal(|| 0u32);
    rsx! {
        header {
            class: "pane-header",

            if props.mode == Mode::MessageList {
                MobileBackButton {}
            }

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
                div {
                    class: "message-list-filters",
                    role: "group",
                    aria_label: "Filter messages",

                    FilterChip {
                        label: "Unread",
                        title: "Show unread messages",
                        active: filter.unread,
                        on_toggle: move |_| {
                            let _ = core_tx.send(CoreEvent::ToggleMessageListFilter {
                                unread: true,
                                flagged: false,
                                has_attachment: false,
                            });
                        },
                    }
                    FilterChip {
                        label: "Flagged",
                        title: "Show flagged messages",
                        active: filter.flagged,
                        on_toggle: move |_| {
                            let _ = core_tx.send(CoreEvent::ToggleMessageListFilter {
                                unread: false,
                                flagged: true,
                                has_attachment: false,
                            });
                        },
                    }
                    FilterChip {
                        label: "Attachment",
                        title: "Show messages with attachments (among loaded messages)",
                        active: filter.has_attachment,
                        on_toggle: move |_| {
                            let _ = core_tx.send(CoreEvent::ToggleMessageListFilter {
                                unread: false,
                                flagged: false,
                                has_attachment: true,
                            });
                        },
                    }
                }

                select {
                    class: "message-view-mode",
                    aria_label: "Message list view",
                    title: "Group messages into conversations",
                    value: "{current_view.as_key()}",
                    onchange: move |evt| {
                        if let Some(next) = MessageListView::from_key(&evt.value()) {
                            crate::ui_prefs::save_message_list_view(next);
                            list_view.set(next);
                            if next == MessageListView::Flat {
                                expanded_conversations.write().clear();
                            }
                        }
                    },
                    for option in MessageListView::ALL {
                        option {
                            value: "{option.as_key()}",
                            selected: option == current_view,
                            "{option.label()}"
                        }
                    }
                }

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

            if props.mode == Mode::MessageList {
                if let (Some(mailbox_id), Some(account_id)) =
                    (current_mailbox_id.clone(), current_account_id.clone())
                {
                    label {
                        class: if can_import {
                            "folder-new mail-import"
                        } else {
                            "folder-new mail-import is-disabled"
                        },
                        title: "Import .eml or mbox into this folder",
                        input {
                            key: "{import_input_gen()}",
                            class: "compose-attach-input",
                            r#type: "file",
                            multiple: true,
                            accept: ".eml,.mbox,message/rfc822,application/mbox",
                            disabled: !can_import,
                            aria_label: "Import .eml or mbox",
                            onchange: {
                                let ctx = ctx.clone();
                                let mailbox_id = mailbox_id.clone();
                                let account_id = account_id.clone();
                                move |evt: FormEvent| {
                                    if !can_import {
                                        return;
                                    }
                                    let files = evt.files();
                                    import_input_gen.set(import_input_gen() + 1);
                                    if files.is_empty() {
                                        return;
                                    }
                                    spawn(import_selected_files(
                                        ctx.clone(),
                                        core_tx,
                                        account_id.clone(),
                                        mailbox_id.clone(),
                                        files,
                                    ));
                                }
                            },
                        }
                        if import_busy { "Importing…" } else { "Import" }
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
                if let Some(account_id) = current_account_id.clone() {
                    button {
                        class: "folder-new",
                        title: "Create a new folder",
                        aria_label: "New folder",
                        onclick: move |_| {
                            let Some(name) = prompt_folder_name("New folder name", "") else {
                                return;
                            };
                            let _ = core_tx.send(CoreEvent::CreateFolder {
                                account_id: account_id.clone(),
                                parent_id: None,
                                name,
                            });
                        },
                        "New folder"
                    }
                }
                ThemeSelect { class: "theme-select pane-header-theme", }
                button {
                    class: "folder-subscribe-btn",
                    title: "Choose which folders appear in the tree",
                    aria_label: "Folder subscriptions",
                    onclick: move |_| {
                        folder_subscribe_open.set(true);
                    },
                    "Folders"
                }
                Link {
                    to: Route::SettingsView {},
                    class: "pane-header-settings",
                    title: "Settings",
                    aria_label: "Settings",
                    Icon {
                        size: 22,
                        icon: IconKind::Cog6Tooth,
                    }
                }
            }
        }
    }
}

#[component]
fn FilterChip(
    label: &'static str,
    title: &'static str,
    active: bool,
    on_toggle: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        button {
            r#type: "button",
            class: "message-filter-chip",
            class: if active { "is-active" },
            title,
            aria_pressed: if active { "true" } else { "false" },
            aria_label: title,
            onclick: move |evt| on_toggle.call(evt),
            "{label}"
        }
    }
}
