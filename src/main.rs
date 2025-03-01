#![allow(non_snake_case)]
use console_error_panic_hook;
use dioxus::prelude::*;
use std::panic;

mod components;
mod layouts;
mod pages;
mod utils;
mod mailiner_core;

use components::{ComponentGallery, ComponentGalleryLayout};
use pages::accountwizard::{EditAccount, NewAccount};
use layouts::MailLayout;
use pages::MailView;
use mailiner_core::mail_controller::MailController;

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
    #[layout(MailLayout)]
    MailView {},
    #[end_layout]

    #[layout(MailLayout)]
    #[route("/gallery")]
    ComponentGallery {},
}


fn App() -> Element {
    // Create the mail controller
    let mail_controller = MailController::new();
    
    // Provide the mail controller to the context
    use_context_provider(move || mail_controller);

    rsx! {
        document::Stylesheet {
            href: asset!("assets/css/tailwind.css")
        }

        Router::<Route> {}
    }
}
