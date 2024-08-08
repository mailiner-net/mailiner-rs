use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;

#[component]
pub fn Header() -> Element {
    rsx! {
        header {
            class: class!(flex items_center justify_between p_4 bg_gray_800 text_white),
            h1 {
                class: class!(text_2xl),
                "Mailiner"
            }
        }
    }
}