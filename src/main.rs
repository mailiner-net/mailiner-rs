#![allow(non_snake_case)]

use console_error_panic_hook;
use corelib::imap_account_manager::ImapAccountManager;
use dioxus::prelude::*;
use dioxus_logger::tracing::{error, info, Level};
use std::collections::HashMap;
use std::panic;
use uuid::Uuid;

mod corelib;
mod pages;

use corelib::hooks::use_persistent;
use corelib::settings::{AppSettings, MailAccount};
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
