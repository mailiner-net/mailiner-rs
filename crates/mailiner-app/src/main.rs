use dioxus::prelude::*;

use crate::components::{EmailNavigation, MessageView, Sidebar};

mod components;
mod context;

#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
enum Route {
    #[layout(MainLayout)]
    #[route("/")]
    MainView {},
}

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/main.css");

fn main() {
    dioxus::launch(App);
}

#[component]
fn MainLayout() -> Element {
    rsx! {
        div {
            id: "app",

            Outlet::<Route> {}
        }
    }
}

#[component]
fn App() -> Element {

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }

        Router::<Route> {}

        div {
            id: "app",
        }
    }
}

#[component]
fn MainView() -> Element {
    rsx! {
        div {
            id: "app",

            Sidebar {
            }

            EmailNavigation {
            }

            MessageView {
            }
        }
    }
}
