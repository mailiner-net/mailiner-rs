use dioxus::prelude::*;

#[component]
pub fn Sidebar() -> Element {
    rsx! {
        section {
            id: "sidebar",
            "Sidebar"
        }
    }
}