use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;
use crate::components::{Toolbar, Sidebar};

/// Main layout for the email application with a toolbar, sidebar, message list and message view.
pub fn MainLayout() -> Element {
    rsx! {
        div {
            class: class!(flex flex_col h_screen w_full max_h_screen),
            Toolbar {
                items: vec![],
            },
            div {
                class: class!(flex flex_1),
                Sidebar {
                    items: vec![],
                },
                div {
                    class: class!(flex flex_1),
                    //MessageList {},
                    //MessageView {},
                }
            }
        }
    }
}