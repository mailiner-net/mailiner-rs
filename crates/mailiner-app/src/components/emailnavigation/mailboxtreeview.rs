use dioxus::prelude::*;
use mailiner_core::MailboxRole;

use super::super::icons::{Icon, IconButton, IconKind};

use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::mailbox::MailboxId;

fn role_icon(role: MailboxRole) -> IconKind {
    match role {
        MailboxRole::Inbox => IconKind::Inbox,
        MailboxRole::Drafts => IconKind::PencilSquare,
        MailboxRole::Sent => IconKind::PaperAirplane,
        MailboxRole::Outbox => IconKind::InboxStack,
        MailboxRole::Trash => IconKind::Trash,
        MailboxRole::Other => IconKind::Folder,
    }
}

#[component]
pub fn MailboxTreeView() -> Element {
    let ctx = use_context::<AppContext>();
    let roots = (ctx.mailbox_roots)();
    rsx! {
        div {
            id: "mailboxtreeview",

            for mailbox_id in roots.iter().cloned() {
                MailboxTreeViewItem {
                    mailbox_id: mailbox_id.clone(),
                }
            }

        }
    }
}

#[derive(PartialEq, Clone, Props)]
pub struct MailboxTreeViewItemProps {
    pub mailbox_id: MailboxId,
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

                if mailbox.children.len() > 0 {
                    IconButton {
                        class: "flat mailbox-chevron",
                        icon: if children_visible() { IconKind::ChevronDown } else { IconKind::ChevronRight },
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
                display: if children_visible() { "block" } else { "none" },

                for child_id in mailbox.children.iter().cloned() {
                    MailboxTreeViewItem {
                        mailbox_id: child_id.clone(),
                    }
                }
            }
        }
    }
}
