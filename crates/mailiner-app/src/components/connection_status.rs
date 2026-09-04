//! Connection status strip for the main mail chrome (PR7 polish).
//!
//! Shows Connecting / Authenticating / Connected / Error / Disconnected with a
//! clear status indicator. Retry sends [`CoreEvent::Reconnect`] for the selected
//! account when the state is retryable (or disconnected).

use dioxus::prelude::*;

use crate::connection::{ConnectErrorKind, ConnectionState};
use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::reconnect::MAX_AUTO_RECONNECT_ATTEMPTS;

/// Polished connection status strip for the selected account.
///
/// Placed at the top of the main content column (status-bar style “toolbar”
/// chrome). Full message is also available via the `title` tooltip.
#[component]
pub fn ConnectionStatusBanner() -> Element {
    let ctx = use_context::<AppContext>();
    let core_tx = use_coroutine_handle::<CoreEvent>();
    let mut retry_pending = use_signal(|| false);

    // Re-arm Retry once core has left the Error/Disconnected surface (or finished
    // Ready). Prevents double-clicks from enqueueing multiple Reconnects while
    // the UI is still showing the previous Error/Disconnected state.
    use_effect(move || {
        let selected = ctx.selected_account.read().clone();
        let states = ctx.connection_states.read();
        let Some(id) = selected else {
            return;
        };
        let Some(state) = states.get(&id) else {
            return;
        };
        if matches!(
            state,
            ConnectionState::Connecting | ConnectionState::Authenticating | ConnectionState::Ready
        ) {
            if retry_pending() {
                retry_pending.set(false);
            }
        }
    });

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

    let account_id_for_retry = account_id.clone();
    let view = StatusView::from_state(state);

    let show_retry = view.show_retry;
    let class = view.banner_class;
    let dot_class = view.dot_class;
    let label = view.label;
    let detail = view.detail;
    let tooltip = view.tooltip;
    let retry_disabled = retry_pending();

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
                    disabled: retry_disabled,
                    title: if retry_disabled {
                        crate::i18n::t("connection.reconnecting")
                    } else {
                        crate::i18n::t("connection.reconnect_tip")
                    },
                    onclick: move |_| {
                        if retry_pending() {
                            return;
                        }
                        retry_pending.set(true);
                        let _ = core_tx.send(CoreEvent::Reconnect {
                            account_id: account_id_for_retry.clone(),
                        });
                    },
                    if retry_disabled { {crate::i18n::t("connection.retrying")} } else { {crate::i18n::t("connection.retry")} }
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
                label: crate::i18n::t("connection.connecting"),
                detail: None,
                tooltip: crate::i18n::t("connection.connecting_tip"),
                show_retry: false,
            },
            ConnectionState::Authenticating => Self {
                banner_class: "connection-banner connection-banner-info",
                dot_class: "connection-banner-dot connection-banner-dot-info connection-banner-dot-pulse",
                label: crate::i18n::t("connection.signing_in"),
                detail: None,
                tooltip: crate::i18n::t("connection.signing_in_tip"),
                show_retry: false,
            },
            ConnectionState::Ready => Self {
                banner_class: "connection-banner connection-banner-ready",
                dot_class: "connection-banner-dot connection-banner-dot-ready",
                label: crate::i18n::t("connection.connected"),
                detail: None,
                tooltip: crate::i18n::t("connection.connected_tip"),
                show_retry: false,
            },
            ConnectionState::Reconnecting {
                failed_attempts,
                delay_ms,
            } => {
                let detail = if *delay_ms >= 1_000 {
                    Some(format!("Retrying in {}s", delay_ms / 1_000))
                } else {
                    None
                };
                Self {
                    banner_class: "connection-banner connection-banner-info",
                    dot_class: "connection-banner-dot connection-banner-dot-info connection-banner-dot-pulse",
                    label: crate::i18n::t("connection.reconnecting"),
                    tooltip: format!(
                        "IMAP session dropped; automatic reconnect (attempt {} of {})",
                        failed_attempts.saturating_add(1),
                        MAX_AUTO_RECONNECT_ATTEMPTS
                    ),
                    detail,
                    show_retry: true,
                }
            }
            ConnectionState::Error {
                message,
                kind,
                retryable,
            } => {
                let kind_label = error_kind_label(*kind);
                Self {
                    banner_class: "connection-banner connection-banner-error",
                    dot_class: "connection-banner-dot connection-banner-dot-error",
                    label: kind_label.clone(),
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

fn error_kind_label(kind: ConnectErrorKind) -> String {
    match kind {
        ConnectErrorKind::NetworkOrProxy => crate::i18n::t("account_form.error_network"),
        ConnectErrorKind::TlsOrSni => crate::i18n::t("account_form.error_tls"),
        ConnectErrorKind::Auth => crate::i18n::t("account_form.error_auth"),
        ConnectErrorKind::Timeout => crate::i18n::t("account_form.error_timeout"),
        ConnectErrorKind::Cancelled => crate::i18n::t("account_form.error_cancelled"),
        ConnectErrorKind::Internal => crate::i18n::t("account_form.error_internal"),
    }
}
