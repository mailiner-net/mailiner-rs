use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;

use super::message_detail::MessageDetail;
use super::message_list::MessageList;

/// Main view for the mail client
#[component]
pub fn MailView() -> Element {
    rsx! {
        div {
            class: class!(flex_1 flex overflow_hidden),
            
            // Message list
            MessageList {}
            
            // Message detail
            MessageDetail {}
        }
    }
} 