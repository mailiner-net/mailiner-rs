use dioxus::prelude::*;
use dioxus_heroicons::{Icon, IconButton};
use dioxus_heroicons::solid::Shape;
use mailiner_core::MailboxRole;

use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::mailbox::MailboxId;

fn role_icon(role: MailboxRole) -> Shape {
    match role {
        MailboxRole::Inbox => Shape::Inbox,
        MailboxRole::Drafts => Shape::PencilSquare,
        MailboxRole::Sent => Shape::PaperAirplane,
        MailboxRole::Outbox => Shape::InboxStack,
        MailboxRole::Trash => Shape::Trash,
        MailboxRole::Other => Shape::Folder,
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
    let is_selected = ctx
        .selected_mailbox
        .read()
        .as_ref()
        .map(|id| *id == props.mailbox_id)
        .unwrap_or(false);
    let mut children_visible = use_signal(|| false);
    rsx! {
        div {
            class: "mailbox-tree-view-item",

            div {
                class: "mailbox-row",
                class: if is_selected { "selected" },

                onclick: move |_| {
                    let _ = core_tx.send(CoreEvent::SelectMailbox(props.mailbox_id.clone()));
                },

                if mailbox.children.len() > 0 {
                    IconButton {
                        class: "flat mailbox-chevron",
                        icon: if children_visible() { Shape::ChevronDown } else { Shape::ChevronRight },
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
                    "{mailbox.title()}"
                    if mailbox.unread_count > 0 {
                        span { class: "mailbox-unread", " {mailbox.unread_count}" }
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
