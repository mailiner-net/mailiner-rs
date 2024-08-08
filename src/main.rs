#![allow(non_snake_case)]

use console_error_panic_hook;
use std::panic;
use dioxus::prelude::*;
use dioxus_logger::tracing::{info, Level};

mod corelib;
mod pages;

use pages::MainView;
use pages::accountwizard::{NewAccount, EditAccount};
use corelib::settings::AppSettings;
use corelib::hooks::use_persistent;

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
        EditAccount{
            account_id: String
        },

        #[route("/")]
        NewAccount {},
    #[end_nest]

    #[route("/")]
    MainView {},
}

fn App() -> Element {
    let settings = use_persistent("app_settings", || AppSettings::default());
    use_context_provider(|| settings);

    rsx! {
        Router::<Route> {}
    }
}
