use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;

#[component]
pub fn MessageList() -> Element {
    rsx! {
        div {
            class: class!(h_full w_full flex flex_col),
            "MessageList"
        }
    }
}