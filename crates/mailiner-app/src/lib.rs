//! Mailiner application: mail session, UI, and on-device stores.
//!
//! Host builds do not compile the `wasm32` call sites, so `dead_code` is
//! enforced on the wasm target (CI) rather than on every host artifact.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

pub mod a11y;
pub mod account;
pub mod account_config;
pub mod account_store;
pub mod account_vault;
pub mod address_book;
pub mod autocrypt;
pub mod autodiscover;
pub mod background_sync;
pub mod components;
pub mod connection;
pub mod context;
pub mod conversation;
pub mod core_event;
pub mod download;
pub mod draft_store;
pub mod formatter;
pub mod headers;
pub mod i18n;
#[cfg(target_arch = "wasm32")]
pub mod idb;
pub mod keywords;
pub mod layout;
pub mod local_data;
pub mod mail_cache;
pub mod mail_file;
pub mod mail_rules;
pub mod mailbox;
pub mod message;
pub mod message_list_filter;
pub mod message_loader;
pub mod notifications;
pub mod oauth;
pub mod object_cache;
pub mod offline_cache;
pub mod outbox_store;
pub mod phishing;
pub mod pin;
pub mod print;
pub mod provider_preset;
pub mod recipient_suggest;
pub mod reconnect;
pub mod selection;
pub mod send;
pub mod setup_wizard;
pub mod shell;
pub mod shortcuts;
pub mod smime;
pub mod smtp_inflight;
pub mod smtp_session;
pub mod snippet;
pub mod snooze;
pub mod source;
pub mod toast;
pub mod ui_prefs;
pub mod unified_inbox;
pub mod vacation;
pub mod websocket_stream;

pub use shell::{AccountStoreContext, AppBootstrapState, launch};
pub(crate) use shell::{Route, resolve_active_id};
