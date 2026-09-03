use std::collections::HashSet;
use std::rc::Rc;

use dioxus::html::Key;
use dioxus::prelude::*;
use mailiner_core::MailboxRole;

use super::super::icons::{Icon, IconButton, IconKind};

use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::mailbox::{MailboxId, mailbox_tree_filter_ids};

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

#[component]
pub fn MailboxTreeView() -> Element {
    let ctx = use_context::<AppContext>();
    let mut query = use_signal(String::new);
    let roots = (ctx.mailbox_roots)();
    let nodes = ctx.mailbox_nodes.read();
    let visible = mailbox_tree_filter_ids(&roots, &nodes, &query.read()).map(Rc::new);
    let no_matches = visible.as_ref().is_some_and(|ids| ids.is_empty());
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

            if no_matches {
                p {
                    class: "mailbox-tree-empty",
                    "No matching folders"
                }
            } else {
                for mailbox_id in roots.iter().cloned() {
                    if visible.as_ref().is_none_or(|ids| ids.contains(&mailbox_id)) {
                        MailboxTreeViewItem {
                            key: "{mailbox_id.as_str()}",
                            mailbox_id: mailbox_id.clone(),
                            visible: visible.clone(),
                        }
                    }
                }
            }
        }
    }
}

#[derive(PartialEq, Clone, Props)]
pub struct MailboxTreeViewItemProps {
    pub mailbox_id: MailboxId,
    visible: Option<Rc<HashSet<MailboxId>>>,
}

#[component]
pub fn MailboxTreeViewItem(props: MailboxTreeViewItemProps) -> Element {
    let ctx = use_context::<AppContext>();
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let mailboxes = ctx.mailbox_nodes.read();
    let mailbox = mailboxes.get(&props.mailbox_id).unwrap();
    let selectable = mailbox.selectable;
    let is_selected = ctx
        .selected_mailbox
        .read()
        .as_ref()
        .map(|id| *id == props.mailbox_id)
        .unwrap_or(false);
    let mut children_visible = use_signal(|| false);
    let filtering = props.visible.is_some();
    let has_visible_children = mailbox
        .children
        .iter()
        .any(|id| props.visible.as_ref().is_none_or(|ids| ids.contains(id)));
    let show_children = has_visible_children && (filtering || children_visible());
    // Reveal a nested restored/jumped folder so the selected row is visible.
    {
        let mailbox_id = props.mailbox_id.clone();
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
    rsx! {
        div {
            class: "mailbox-tree-view-item",

            div {
                class: "mailbox-row",
                class: if is_selected { "selected" },

                onclick: move |_| {
                    if !selectable {
                        return;
                    }
                    let _ = core_tx.send(CoreEvent::SelectMailbox(props.mailbox_id.clone()));
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

                for child_id in mailbox.children.iter().cloned() {
                    if props.visible.as_ref().is_none_or(|ids| ids.contains(&child_id)) {
                        MailboxTreeViewItem {
                            key: "{child_id.as_str()}",
                            mailbox_id: child_id.clone(),
                            visible: props.visible.clone(),
                        }
                    }
                }
            }
        }
    }
}
