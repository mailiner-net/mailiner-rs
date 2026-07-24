//! Connection status strip for the main mail chrome (PR7 polish).
//!
//! Shows Connecting / Authenticating / Connected / Error / Disconnected with a
//! clear status indicator. Retry sends [`CoreEvent::Reconnect`] for the selected
//! account when the state is retryable (or disconnected).

use dioxus::prelude::*;

use crate::connection::{ConnectErrorKind, ConnectionState};
use crate::context::AppContext;
use crate::core_event::CoreEvent;

/// Polished connection status strip for the selected account.
///
/// Placed at the top of the main content column (status-bar style “toolbar”
/// chrome). Full message is also available via the `title` tooltip.
#[component]
pub fn ConnectionStatusBanner() -> Element {
    let ctx = use_context::<AppContext>();
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let selected = ctx.selected_account.read().clone();
    let states = ctx.connection_states.read();

    let Some(account_id) = selected else {
        return rsx! {};
    };

    let Some(state) = states.get(&account_id) else {
        return rsx! {};
    };

    if matches!(state, ConnectionState::Idle) {
        return rsx! {};
    }

    // Snapshot fields we need after the read-guard is dropped (Retry move).
    let account_id_for_retry = account_id.clone();
    let view = StatusView::from_state(state);

    let show_retry = view.show_retry;
    let class = view.banner_class;
    let dot_class = view.dot_class;
    let label = view.label;
    let detail = view.detail;
    let tooltip = view.tooltip;

    rsx! {
        div {
            class: "{class}",
            role: "status",
            "aria-live": "polite",
            title: "{tooltip}",

            span {
                class: "{dot_class}",
                "aria-hidden": "true",
            }

            div {
                class: "connection-banner-text",
                span {
                    class: "connection-banner-label",
                    "{label}"
                }
                if let Some(detail) = detail {
                    span {
                        class: "connection-banner-detail",
                        "{detail}"
                    }
                }
            }

            if show_retry {
                button {
                    class: "connection-banner-retry",
                    r#type: "button",
                    title: "Reconnect to the mail server",
                    onclick: move |_| {
                        let _ = core_tx.send(CoreEvent::Reconnect {
                            account_id: account_id_for_retry.clone(),
                        });
                    },
                    "Retry"
                }
            }
        }
    }
}

/// Rendered fields derived from [`ConnectionState`].
struct StatusView {
    banner_class: &'static str,
    dot_class: &'static str,
    label: String,
    detail: Option<String>,
    tooltip: String,
    show_retry: bool,
}

impl StatusView {
    fn from_state(state: &ConnectionState) -> Self {
        match state {
            ConnectionState::Connecting => Self {
                banner_class: "connection-banner connection-banner-info",
                dot_class: "connection-banner-dot connection-banner-dot-info connection-banner-dot-pulse",
                label: "Connecting…".into(),
                detail: None,
                tooltip: "Opening WebSocket proxy and TLS to the IMAP server".into(),
                show_retry: false,
            },
            ConnectionState::Authenticating => Self {
                banner_class: "connection-banner connection-banner-info",
                dot_class: "connection-banner-dot connection-banner-dot-info connection-banner-dot-pulse",
                label: "Signing in…".into(),
                detail: None,
                tooltip: "Authenticating with the IMAP server".into(),
                show_retry: false,
            },
            ConnectionState::Ready => Self {
                banner_class: "connection-banner connection-banner-ready",
                dot_class: "connection-banner-dot connection-banner-dot-ready",
                label: "Connected".into(),
                detail: None,
                tooltip: "IMAP session ready".into(),
                show_retry: false,
            },
            ConnectionState::Error {
                message,
                kind,
                retryable,
            } => {
                let kind_label = error_kind_label(*kind);
                Self {
                    banner_class: "connection-banner connection-banner-error",
                    dot_class: "connection-banner-dot connection-banner-dot-error",
                    label: kind_label.into(),
                    detail: Some(message.clone()),
                    tooltip: format!("{kind_label}: {message}"),
                    show_retry: *retryable,
                }
            }
            ConnectionState::Disconnected => Self {
                banner_class: "connection-banner connection-banner-muted",
                dot_class: "connection-banner-dot connection-banner-dot-muted",
                label: "Disconnected".into(),
                detail: None,
                tooltip: "Not connected to the mail server".into(),
                show_retry: true,
            },
            ConnectionState::Idle => unreachable!("Idle filtered before StatusView::from_state"),
        }
    }
}

fn error_kind_label(kind: ConnectErrorKind) -> &'static str {
    match kind {
        ConnectErrorKind::NetworkOrProxy => "Connection failed",
        ConnectErrorKind::TlsOrSni => "Secure connection failed",
        ConnectErrorKind::Auth => "Sign-in failed",
        ConnectErrorKind::Timeout => "Timed out",
        ConnectErrorKind::Cancelled => "Cancelled",
        ConnectErrorKind::Internal => "Connection error",
    }
}
