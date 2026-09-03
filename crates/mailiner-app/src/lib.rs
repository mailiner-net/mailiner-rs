//! Library surface for unit tests (formatters, loaders, download, account config).
pub mod account;
pub mod account_config;
pub mod account_store;
pub mod download;
pub mod formatter;
pub mod layout;
pub mod mail_cache;
pub mod mailbox;
pub mod message;
pub mod message_loader;
pub mod outbox_store;
pub mod print;
pub mod selection;
pub mod send;
pub mod shortcuts;
pub mod toast;
pub mod ui_prefs;

// connection / core_loop are binary-only (Dioxus signals + WASM WebSocket).
