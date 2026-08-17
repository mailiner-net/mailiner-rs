//! Library surface for unit tests (formatters, loaders, download, account config).
pub mod account;
pub mod account_config;
pub mod account_store;
pub mod download;
pub mod outbox_store;
pub mod send;
pub mod formatter;
pub mod message_loader;

// connection / core_loop are binary-only (Dioxus signals + WASM WebSocket).
