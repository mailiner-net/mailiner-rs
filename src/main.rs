#![allow(non_snake_case)]
use console_error_panic_hook;
use mailiner_core::imap_account_manager::ImapAccountManager;
use dioxus::prelude::*;
use dioxus_logger::tracing::{info, Level};
use std::panic;

mod pages;
mod utils;

use pages::accountwizard::{EditAccount, NewAccount};
use pages::MainView;

fn main() {
    panic::set_hook(Box::new(console_error_panic_hook::hook));
    dioxus_logger::init(Level::INFO).expect("failed to init logger");

    info!("starting app");
    launch(App);
}

#[derive(PartialEq, Clone, Debug, Routable)]
enum Route {
    #[nest("/accountwizard")]
    #[route("/:account_id")]
    EditAccount { account_id: String },

    #[route("/")]
    NewAccount {},
    #[end_nest]
    #[route("/")]
    MainView {},
}

fn App() -> Element {
    let imap_account_manager =
        use_signal(|| ImapAccountManager::new().expect("Failed to load ImapAccountManager"));
    use_context_provider(move || imap_account_manager);

    rsx! {
        Router::<Route> {}
    }
}
