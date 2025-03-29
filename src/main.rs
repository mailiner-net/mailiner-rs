#![allow(non_snake_case)]
use console_error_panic_hook;
use dioxus::prelude::*;
use std::panic;

use mailiner_rs::{AppState, EmailApp, EmailRoute, ComponentGallery};
use mailiner_rs::kernel::service::EmailServiceFactory;
use mailiner_rs::kernel::backend::BackendType;

fn main() {
    panic::set_hook(Box::new(console_error_panic_hook::hook));

    tracing::info!("starting app");
    dioxus::launch(App);
}

#[derive(PartialEq, Clone, Debug, Routable)]
enum Route {
    #[route("/gallery")]
    ComponentGallery,
    
    #[route("/email")]
    EmailApp,
    
    #[route("/")]
    Index,
}

#[component]
fn Index() -> Element {
    rsx! {
        div {
            class: "flex h-screen items-center justify-center flex-col gap-4",
            h1 {
                class: "text-3xl font-semibold",
                "Welcome to Mailiner"
            }
            Link {
                to: Route::EmailApp,
                class: "px-4 py-2 bg-blue-500 text-white rounded hover:bg-blue-600 transition",
                "Launch Email App"
            }
            Link {
                to: Route::ComponentGallery,
                class: "text-blue-500 hover:underline",
                "View Component Gallery"
            }
        }
    }
}

fn App() -> Element {
    // Create the mail controller
    use_context_provider(move || {
        let email_service = EmailServiceFactory::create_service(BackendType::Mock, "")
            .expect("Failed to create email service");
        AppState::new(email_service)
    });

    rsx! {
        document::Stylesheet {
            href: asset!("assets/css/tailwind.css")
        }

        Router::<Route> {}
    }
}
