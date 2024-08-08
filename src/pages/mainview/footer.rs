use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;

#[component]
pub fn Footer() -> Element {
    rsx! {
        div {
            class: class!(flex items_center justify_around p_4 bg_gray_800 text_white),
            "Made with ❤️ in Prague",
        }
    }
}