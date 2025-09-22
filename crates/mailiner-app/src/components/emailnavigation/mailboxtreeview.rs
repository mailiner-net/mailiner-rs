use dioxus::logger::tracing::info;
use dioxus::prelude::*;
use dioxus_heroicons::IconButton;
use dioxus_heroicons::solid::Shape;

use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::mailbox::MailboxId;

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
    let mut children_visible = use_signal(|| false);
    rsx! {
        div {
            class: "mailbox-tree-view-item",

            div {
                onclick: move |_| {
                    let _ = core_tx.send(CoreEvent::SelectMailbox(props.mailbox_id.clone()));
                },

                if mailbox.children.len() > 0 {
                    IconButton {
                        class: "flat",
                        icon: if children_visible() { Shape::ChevronDown } else { Shape::ChevronRight },
                        size: 24,

                        onclick: move |e: MouseEvent| {
                            children_visible.set(!children_visible());
                            e.stop_propagation();
                        }
                    }
                } else {
                    span {
                        style: "width: 24px",
                    }
                }

                div {
                    "{mailbox.name}"
                    if mailbox.unread_count > 0 {
                        " ({mailbox.unread_count})"
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
