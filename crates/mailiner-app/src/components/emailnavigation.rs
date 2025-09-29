use dioxus::prelude::*;

mod navigationheader;
mod mailboxtreeview;
mod messagelist;
mod messagelist_virtual;

pub use navigationheader::NavigationHeader;
pub use mailboxtreeview::MailboxTreeView;
pub use messagelist::MessageList;
pub use messagelist_virtual::{VirtualMessageList, MessageListWithRealData};

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

                // Use VirtualMessageList for demo, or MessageListWithRealData for production
                VirtualMessageList {
                }
            }
        }
    }
}