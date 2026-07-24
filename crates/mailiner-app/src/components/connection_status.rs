//! Minimal connection status line for the main mail chrome (PR5).

use dioxus::prelude::*;

use crate::connection::ConnectionState;
use crate::context::AppContext;

/// Compact status for the selected account: Connecting / Ready / Error.
///
/// PR5 minimum — toolbar polish lands in PR7.
#[component]
pub fn ConnectionStatusBanner() -> Element {
    let ctx = use_context::<AppContext>();
    let selected = ctx.selected_account.read().clone();
    let states = ctx.connection_states.read();

    let Some(account_id) = selected else {
        return rsx! {};
    };

    let Some(state) = states.get(&account_id) else {
        return rsx! {};
    };

    let (class, text) = match state {
        ConnectionState::Connecting => (
            "connection-banner connection-banner-info",
            "Connecting…".to_string(),
        ),
        ConnectionState::Authenticating => (
            "connection-banner connection-banner-info",
            "Signing in…".to_string(),
        ),
        ConnectionState::Ready => (
            "connection-banner connection-banner-ready",
            "Connected".to_string(),
        ),
        ConnectionState::Error { message, .. } => {
            ("connection-banner connection-banner-error", message.clone())
        }
        ConnectionState::Disconnected => (
            "connection-banner connection-banner-muted",
            "Disconnected".to_string(),
        ),
        ConnectionState::Idle => return rsx! {},
    };

    rsx! {
        div {
            class: "{class}",
            role: "status",
            "{text}"
        }
    }
}
