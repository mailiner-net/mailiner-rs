use dioxus::prelude::*;

mod mailboxtreeview;
mod messagelist;
mod navigationheader;

pub use mailboxtreeview::MailboxTreeView;
pub use messagelist::MessageList;
pub use navigationheader::{MobileBackButton, NavigationHeader};

use crate::components::emailnavigation::navigationheader::Mode;

#[component]
pub fn EmailNavigation() -> Element {
    rsx! {
        section {
            id: "emailnavigation",

            NavigationHeader {
                mode: Mode::MailboxTreeView,
            }

            MailboxTreeView {
            }
        }
    }
}
