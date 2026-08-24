//! Library surface for unit tests (formatters, loaders, download, account config).
pub mod account;
pub mod account_config;
pub mod account_store;
pub mod download;
pub mod outbox_store;
pub mod send;
pub mod mailbox;
pub mod message;
pub mod formatter;
pub mod layout;
pub mod ui_prefs;
pub mod message_loader;
pub mod toast;
pub mod shortcuts;

// connection / core_loop are binary-only (Dioxus signals + WASM WebSocket).
