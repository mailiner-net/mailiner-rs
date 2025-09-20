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
                "Inbox"
            }
        }
    }
}
