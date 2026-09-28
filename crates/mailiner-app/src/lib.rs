//! Mailiner application: mail session, UI, and on-device stores.
//!
//! Host builds do not compile the `wasm32` call sites, so `dead_code` is
//! enforced on the wasm target (CI) rather than on every host artifact.
//! Shell modules stay `pub(crate)` so that lint still sees their `pub` items;
//! the binary only needs [`launch`].
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
pub(crate) mod components;
pub(crate) mod connection;
pub(crate) mod context;
pub mod conversation;
pub(crate) mod core_event;
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
pub(crate) mod shell;
pub mod shortcuts;
pub mod smime;
pub mod smtp_inflight;
pub(crate) mod smtp_session;
pub mod snippet;
pub mod snooze;
pub mod source;
pub mod toast;
pub mod ui_prefs;
pub mod unified_inbox;
pub mod vacation;
pub(crate) mod websocket_stream;

pub use shell::launch;
pub(crate) use shell::{AccountStoreContext, AppBootstrapState, Route, resolve_active_id};
