use dioxus::prelude::*;

mod mailboxtreeview;
mod messagelist;
mod navigationheader;

pub use mailboxtreeview::MailboxTreeView;
pub use messagelist::MessageList;
pub use navigationheader::NavigationHeader;

use crate::{components::emailnavigation::navigationheader::Mode, context::AppContext};

#[component]
pub fn EmailNavigation() -> Element {
    let ctx = use_context::<AppContext>();
    rsx! {
        section {
            id: "emailnavigation",

           div {
                display: if ctx.selected_mailbox.read().is_none() { "block" } else { "none" },

                NavigationHeader {
                    mode: Mode::MailboxTreeView,
                }

                MailboxTreeView {
                }
            }

            div {
                display: if ctx.selected_mailbox.read().is_some() { "block" } else { "none" },

                NavigationHeader {
                    mode: Mode::MessageList,
                }

                MessageList {
                }
            }
        }
    }
}
