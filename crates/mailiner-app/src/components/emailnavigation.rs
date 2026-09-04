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
        nav {
            id: "emailnavigation",
            aria_label: crate::i18n::t("folder.folders_nav"),

            NavigationHeader {
                mode: Mode::MailboxTreeView,
            }

            MailboxTreeView {
            }
        }
    }
}
