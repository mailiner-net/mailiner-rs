use std::collections::HashSet;
use std::rc::Rc;

use dioxus::html::Key;
use dioxus::prelude::*;
use mailiner_core::MailboxRole;

use super::super::icons::{Icon, IconButton, IconKind};

use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::mailbox::{
    MailboxId, can_manage_folder, can_toggle_subscription, mailbox_tree_filter_ids,
    mailbox_visible_in_tree,
};

/// Prompt for a single folder path segment. Non-web builds fail closed.
pub(crate) fn prompt_folder_name(message: &str, default: &str) -> Option<String> {
    #[cfg(feature = "web")]
    {
        web_sys::window()
            .and_then(|window| {
                window
                    .prompt_with_message_and_default(message, default)
                    .ok()
                    .flatten()
            })
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
    #[cfg(not(feature = "web"))]
    {
        let _ = (message, default);
        None
    }
}

fn confirm_delete_folder(title: &str, has_children: bool) -> bool {
    let message = if has_children {
        format!("Delete folder \"{title}\" and its subfolders?")
    } else {
        format!("Delete folder \"{title}\"?")
    };
    #[cfg(feature = "web")]
    {
        web_sys::window()
            .and_then(|window| window.confirm_with_message(&message).ok())
            .unwrap_or(false)
    }
    #[cfg(not(feature = "web"))]
    {
        let _ = message;
        false
    }
}

fn is_valid_drop_target(
    selectable: bool,
    dest: &MailboxId,
    source: Option<&MailboxId>,
    dragging: bool,
) -> bool {
    dragging && selectable && source.is_some_and(|id| id != dest)
}

fn role_icon(role: MailboxRole) -> IconKind {
    match role {
        MailboxRole::Inbox => IconKind::Inbox,
        MailboxRole::Archive => IconKind::ArchiveBox,
        MailboxRole::Drafts => IconKind::PencilSquare,
        MailboxRole::Sent => IconKind::PaperAirplane,
        MailboxRole::Outbox => IconKind::InboxStack,
        MailboxRole::Trash => IconKind::Trash,
        MailboxRole::Junk => IconKind::NoSymbol,
        MailboxRole::Other => IconKind::Folder,
    }
}

fn folder_shown(
    id: &MailboxId,
    nodes: &std::collections::HashMap<MailboxId, crate::mailbox::MailboxNode>,
    show_all: bool,
    visible: &Option<Rc<HashSet<MailboxId>>>,
) -> bool {
    mailbox_visible_in_tree(id, nodes, show_all)
        && visible.as_ref().is_none_or(|ids| ids.contains(id))
}

#[derive(Clone, PartialEq)]
pub(crate) struct FolderMenu {
    mailbox_id: MailboxId,
    x: f64,
    y: f64,
}

#[component]
pub fn MailboxTreeView() -> Element {
    let ctx = use_context::<AppContext>();
    let mut query = use_signal(String::new);
    let mut menu = use_signal(|| None::<FolderMenu>);
    let roots = (ctx.mailbox_roots)();
    let nodes = ctx.mailbox_nodes.read();
    let show_all = *ctx.show_all_folders.read();
    let visible = mailbox_tree_filter_ids(&roots, &nodes, &query.read()).map(Rc::new);
    let drop_active = ctx.message_drag.read().is_some();
    let shown_roots: Vec<MailboxId> = roots
        .iter()
        .filter(|id| folder_shown(id, &nodes, show_all, &visible))
        .cloned()
        .collect();
    let no_matches = visible.as_ref().is_some_and(|ids| ids.is_empty())
        || (visible.is_some() && shown_roots.is_empty());
    rsx! {
        div {
            class: "mailbox-tree-filter",
            input {
                class: "ui-input",
                r#type: "text",
                value: "{query}",
                placeholder: "Filter folders",
                aria_label: "Filter folders",
                autocomplete: "off",
                spellcheck: false,
                oninput: move |evt| query.set(evt.value()),
                onkeydown: move |evt: KeyboardEvent| {
                    if evt.key() == Key::Escape && !query.read().is_empty() {
                        evt.prevent_default();
                        query.set(String::new());
                    }
                },
            }
        }
        div {
            id: "mailboxtreeview",
            class: if drop_active { "drop-active" },

            if no_matches {
                p {
                    class: "mailbox-tree-empty",
                    "No matching folders"
                }
            } else {
                for mailbox_id in shown_roots {
                    MailboxTreeViewItem {
                        key: "{mailbox_id.as_str()}",
                        mailbox_id: mailbox_id.clone(),
                        visible: visible.clone(),
                        on_menu: move |next| menu.set(Some(next)),
                    }
                }
            }
        }

        if let Some(open) = menu() {
            FolderContextMenu {
                menu: open,
                onclose: move |_| menu.set(None),
            }
        }
    }
}

#[derive(PartialEq, Clone, Props)]
pub struct MailboxTreeViewItemProps {
    pub mailbox_id: MailboxId,
    visible: Option<Rc<HashSet<MailboxId>>>,
    pub on_menu: EventHandler<FolderMenu>,
}

#[component]
fn MailboxTreeViewItem(props: MailboxTreeViewItemProps) -> Element {
    let ctx = use_context::<AppContext>();
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let mut message_drag = ctx.message_drag;
    let mailboxes = ctx.mailbox_nodes.read();
    let mailbox = mailboxes.get(&props.mailbox_id).unwrap();
    let selectable = mailbox.selectable;
    let subscribed = mailbox.subscribed;
    let show_all = *ctx.show_all_folders.read();
    let visible_children: Vec<MailboxId> = mailbox
        .children
        .iter()
        .filter(|id| folder_shown(id, &mailboxes, show_all, &props.visible))
        .cloned()
        .collect();
    let is_selected = ctx
        .selected_mailbox
        .read()
        .as_ref()
        .map(|id| *id == props.mailbox_id)
        .unwrap_or(false);
    let mut children_visible = use_signal(|| false);
    let filtering = props.visible.is_some();
    let has_visible_children = !visible_children.is_empty();
    let show_children = has_visible_children && (filtering || children_visible());
    let is_drop_target = message_drag
        .read()
        .as_ref()
        .is_some_and(|d| d.over.as_ref() == Some(&props.mailbox_id));
    let mailbox_id = props.mailbox_id.clone();
    // Reveal a nested restored/jumped folder so the selected row is visible.
    {
        let mailbox_id = mailbox_id.clone();
        use_effect(move || {
            let selected = ctx.selected_mailbox.read().clone();
            let nodes = ctx.mailbox_nodes.read();
            if selected
                .as_ref()
                .is_some_and(|sel| crate::mailbox::mailbox_is_ancestor(&mailbox_id, sel, &nodes))
            {
                children_visible.set(true);
            }
        });
    }
    let on_menu = props.on_menu;
    let mailbox_id_menu = mailbox_id.clone();
    rsx! {
        div {
            class: "mailbox-tree-view-item",

            div {
                class: "mailbox-row",
                class: if is_selected { "selected" },
                class: if is_drop_target { "drop-target" },
                class: if !subscribed { "unsubscribed" },

                onclick: {
                    let mailbox_id = mailbox_id.clone();
                    move |_| {
                        if !selectable {
                            return;
                        }
                        let _ = core_tx.send(CoreEvent::SelectMailbox(mailbox_id.clone()));
                    }
                },

                ondragover: {
                    let mailbox_id = mailbox_id.clone();
                    move |evt: DragEvent| {
                        let source = message_drag
                            .peek()
                            .as_ref()
                            .map(|d| d.source_mailbox.clone());
                        if !is_valid_drop_target(
                            selectable,
                            &mailbox_id,
                            source.as_ref(),
                            source.is_some(),
                        ) {
                            return;
                        }
                        evt.prevent_default();
                        evt.data_transfer().set_drop_effect("move");
                        if let Some(mut d) = message_drag()
                            && d.over.as_ref() != Some(&mailbox_id)
                        {
                            d.over = Some(mailbox_id.clone());
                            message_drag.set(Some(d));
                        }
                    }
                },

                ondragenter: {
                    let mailbox_id = mailbox_id.clone();
                    move |evt: DragEvent| {
                        let source = message_drag
                            .peek()
                            .as_ref()
                            .map(|d| d.source_mailbox.clone());
                        if !is_valid_drop_target(
                            selectable,
                            &mailbox_id,
                            source.as_ref(),
                            source.is_some(),
                        ) {
                            return;
                        }
                        evt.prevent_default();
                    }
                },

                ondragleave: {
                    let mailbox_id = mailbox_id.clone();
                    move |_| {
                        if let Some(mut d) = message_drag()
                            && d.over.as_ref() == Some(&mailbox_id)
                        {
                            d.over = None;
                            message_drag.set(Some(d));
                        }
                    }
                },

                ondrop: {
                    let dest_mailbox_id = mailbox_id.clone();
                    move |evt: DragEvent| {
                        evt.prevent_default();
                        evt.stop_propagation();
                        let Some(drag) = message_drag.take() else {
                            return;
                        };
                        if ctx.selected_mailbox.peek().as_ref() != Some(&drag.source_mailbox) {
                            return;
                        }
                        if !is_valid_drop_target(
                            selectable,
                            &dest_mailbox_id,
                            Some(&drag.source_mailbox),
                            true,
                        ) {
                            return;
                        }
                        if drag.message_ids.is_empty() {
                            return;
                        }
                        let _ = core_tx.send(CoreEvent::MoveMessages {
                            mailbox_id: drag.source_mailbox,
                            message_ids: drag.message_ids,
                            dest_mailbox_id: dest_mailbox_id.clone(),
                        });
                    }
                },
                oncontextmenu: move |evt: MouseEvent| {
                    evt.prevent_default();
                    evt.stop_propagation();
                    let coords = evt.client_coordinates();
                    on_menu.call(FolderMenu {
                        mailbox_id: mailbox_id_menu.clone(),
                        x: coords.x,
                        y: coords.y,
                    });
                },

                if has_visible_children && !filtering {
                    IconButton {
                        class: "flat mailbox-chevron",
                        icon: if show_children { IconKind::ChevronDown } else { IconKind::ChevronRight },
                        size: 16,
                        onclick: move |e: MouseEvent| {
                            children_visible.set(!children_visible());
                            e.stop_propagation();
                        }
                    }
                }

                span {
                    class: "mailbox-icon",
                    Icon {
                        size: 18,
                        icon: role_icon(mailbox.role),
                    }
                }

                div {
                    class: "mailbox-name",
                    span { class: "mailbox-title", "{mailbox.title()}" }
                    if mailbox.unread_count > 0 {
                        span {
                            class: "mailbox-unread",
                            class: if mailbox.has_new { "is-new" },
                            " ({mailbox.unread_count})"
                        }
                    }
                }
            }

            div {
                display: if show_children { "block" } else { "none" },

                for child_id in visible_children {
                    MailboxTreeViewItem {
                        key: "{child_id.as_str()}",
                        mailbox_id: child_id.clone(),
                        visible: props.visible.clone(),
                        on_menu,
                    }
                }
            }
        }
    }
}

#[component]
fn FolderContextMenu(menu: FolderMenu, onclose: EventHandler<MouseEvent>) -> Element {
    let mut ctx = use_context::<AppContext>();
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let nodes = ctx.mailbox_nodes.read();
    let Some(node) = nodes.get(&menu.mailbox_id) else {
        return rsx! {};
    };
    let title = node.title().to_string();
    let name = node.name.clone();
    let can_manage = can_manage_folder(node);
    let can_toggle = can_toggle_subscription(node);
    let has_children = !node.children.is_empty();
    let subscribed = node.subscribed;
    let toggle_label = if subscribed {
        "Unsubscribe"
    } else {
        "Subscribe"
    };
    let mailbox_id = menu.mailbox_id.clone();
    let account_id = ctx.selected_account.read().clone();
    let create_account = account_id.clone();
    let rename_account = account_id.clone();
    let delete_account = account_id.clone();
    let create_mailbox = mailbox_id.clone();
    let rename_mailbox = mailbox_id.clone();
    let delete_mailbox = mailbox_id.clone();
    let rename_current = name.clone();
    rsx! {
        div {
            class: "folder-menu-backdrop",
            onclick: move |evt| onclose.call(evt),
            oncontextmenu: move |evt: MouseEvent| {
                evt.prevent_default();
                onclose.call(evt);
            },
            div {
                class: "folder-menu",
                role: "menu",
                style: "left: {menu.x}px; top: {menu.y}px;",
                onclick: move |evt| evt.stop_propagation(),

                button {
                    class: "folder-menu-item",
                    r#type: "button",
                    role: "menuitem",
                    onclick: move |evt| {
                        onclose.call(evt);
                        let Some(account_id) = create_account.clone() else {
                            return;
                        };
                        let Some(name) = prompt_folder_name("New folder name", "") else {
                            return;
                        };
                        let _ = core_tx.send(CoreEvent::CreateFolder {
                            account_id,
                            parent_id: Some(create_mailbox.clone()),
                            name,
                        });
                    },
                    "New folder"
                }

                if can_manage {
                    button {
                        class: "folder-menu-item",
                        r#type: "button",
                        role: "menuitem",
                        onclick: move |evt| {
                            onclose.call(evt);
                            let Some(account_id) = rename_account.clone() else {
                                return;
                            };
                            let Some(new_name) =
                                prompt_folder_name("Rename folder", &rename_current)
                            else {
                                return;
                            };
                            if new_name == rename_current {
                                return;
                            }
                            let _ = core_tx.send(CoreEvent::RenameFolder {
                                account_id,
                                mailbox_id: rename_mailbox.clone(),
                                new_name,
                            });
                        },
                        "Rename"
                    }
                    button {
                        class: "folder-menu-item is-danger",
                        r#type: "button",
                        role: "menuitem",
                        onclick: move |evt| {
                            onclose.call(evt);
                            let Some(account_id) = delete_account.clone() else {
                                return;
                            };
                            if !confirm_delete_folder(&title, has_children) {
                                return;
                            }
                            let _ = core_tx.send(CoreEvent::DeleteFolder {
                                account_id,
                                mailbox_id: delete_mailbox.clone(),
                            });
                        },
                        "Delete"
                    }
                }

                if can_toggle {
                    button {
                        class: "folder-menu-item",
                        role: "menuitem",
                        onclick: move |evt| {
                            if let Some(account_id) = account_id.clone() {
                                let _ = core_tx.send(CoreEvent::SetFolderSubscribed {
                                    account_id,
                                    mailbox_id: mailbox_id.clone(),
                                    subscribed: !subscribed,
                                });
                            }
                            onclose.call(evt);
                        },
                        "{toggle_label}"
                    }
                }
                button {
                    class: "folder-menu-item",
                    role: "menuitem",
                    onclick: move |evt| {
                        ctx.folder_subscribe_open.set(true);
                        onclose.call(evt);
                    },
                    "Manage subscriptions…"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> MailboxId {
        MailboxId::from(s.to_string())
    }

    #[test]
    fn drop_requires_drag_selectable_and_other_folder() {
        let inbox = id("INBOX");
        let sent = id("Sent");
        assert!(is_valid_drop_target(true, &sent, Some(&inbox), true));
        assert!(!is_valid_drop_target(true, &inbox, Some(&inbox), true));
        assert!(!is_valid_drop_target(false, &sent, Some(&inbox), true));
        assert!(!is_valid_drop_target(true, &sent, Some(&inbox), false));
        assert!(!is_valid_drop_target(true, &sent, None, true));
    }
}
