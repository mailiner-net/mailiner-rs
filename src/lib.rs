#[allow(non_snake_case)]

use dioxus::prelude::*;

pub mod components;
pub mod kernel;
pub mod email_app;

pub use components::ComponentGallery;
pub use email_app::{EmailApp, EmailRoute};
use kernel::model::{AccountId, FolderId, MessageId};
use kernel::service::EmailService;
use std::sync::Arc;


#[derive(Clone)]
pub struct AppState {
    pub email_service: Arc<EmailService>,
    pub selected_account: Signal<Option<AccountId>>,
    pub selected_folder: Signal<Option<FolderId>>,
    pub selected_message: Signal<Option<MessageId>>,
    pub is_loading: Signal<bool>,
    pub error: Signal<Option<String>>,
}

impl AppState {
    pub fn new(email_service: Arc<EmailService>) -> Self {
        Self {
            email_service,
            selected_account: Signal::new(None),
            selected_folder: Signal::new(None),
            selected_message: Signal::new(None),
            is_loading: Signal::new(false),
            error: Signal::new(None),
        }
    }
}

