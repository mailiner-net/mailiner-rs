//! Library surface for unit tests (formatters, loaders, download, account config).
pub mod account;
pub mod account_config;
pub mod account_store;
pub mod address_book;
pub mod autodiscover;
pub mod download;
pub mod draft_store;
pub mod formatter;
pub mod headers;
pub mod layout;
pub mod local_data;
pub mod mail_cache;
pub mod mailbox;
pub mod message;
pub mod message_list_filter;
pub mod message_loader;
pub mod notifications;
pub mod outbox_store;
pub mod phishing;
pub mod print;
pub mod provider_preset;
pub mod recipient_suggest;
pub mod reconnect;
pub mod selection;
pub mod send;
pub mod shortcuts;
pub mod smtp_inflight;
pub mod snippet;
pub mod source;
pub mod toast;
pub mod ui_prefs;

// connection / core_loop are binary-only (Dioxus signals + WASM WebSocket).
