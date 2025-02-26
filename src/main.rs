#![allow(non_snake_case)]
use console_error_panic_hook;
use mailiner_core::imap_account_manager::ImapAccountManager;
use dioxus::prelude::*;
use std::panic;

mod components;
mod pages;
mod utils;

use components::ComponentGallery;
use pages::accountwizard::{EditAccount, NewAccount};
use pages::MainView;

fn main() {
    panic::set_hook(Box::new(console_error_panic_hook::hook));

    tracing::info!("starting app");
    dioxus::launch(App);
}

#[derive(PartialEq, Clone, Debug, Routable)]
enum Route {
    #[nest("/accountwizard")]
    #[route("/")]
    NewAccount {},
    #[route("/:account_id")]
    EditAccount { account_id: String },
    #[end_nest]

    #[route("/")]
    MainView {},

    #[route("/gallery")]
    ComponentGallery {},
}

fn App() -> Element {
    let imap_account_manager =
        use_signal(|| ImapAccountManager::new().expect("Failed to load ImapAccountManager"));
    use_context_provider(move || imap_account_manager);

    rsx! {
        document::Stylesheet {
            href: asset!("assets/css/tailwind.css")
        }

        Router::<Route> {}
    }
}
