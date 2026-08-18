//! Composer send / Test SMTP UI state (no secrets).

use mailiner_composer::DraftDocument;
use mailiner_core::submit::SendErrorKind;

use crate::account::AccountId;

/// Open compose session (owned by [`crate::context::AppContext::compose_draft`]).
#[derive(Clone, Debug)]
pub struct ComposeSession {
    /// Dialog title (`New message`, `Reply`, `Forward`).
    pub title: String,
    /// Prefill document.
    pub draft: DraftDocument,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendPhase {
    Connecting,
    Transmitting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendState {
    Idle,
    Sending {
        account_id: AccountId,
        phase: SendPhase,
    },
    Sent {
        account_id: AccountId,
    },
    Failed {
        account_id: AccountId,
        kind: SendErrorKind,
        message: String,
        retryable: bool,
    },
}

/// Subject + To preview carried with `SendMessage` for the outbox row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxDisplay {
    pub subject: String,
    pub to_preview: String,
}

pub fn send_kind_label(kind: SendErrorKind) -> &'static str {
    match kind {
        SendErrorKind::NetworkOrProxy => "Network / proxy",
        SendErrorKind::TlsOrSni => "TLS / certificate",
        SendErrorKind::Auth => "Sign-in failed",
        SendErrorKind::Timeout => "Timed out",
        SendErrorKind::Cancelled => "Cancelled",
        SendErrorKind::NotConfigured => "SMTP not configured",
        SendErrorKind::TlsModeUnsupported => "TLS mode not supported",
        SendErrorKind::RecipientRejected => "Recipient rejected",
        SendErrorKind::MessageTooLarge => "Message too large",
        SendErrorKind::Transient => "Temporarily unavailable",
        SendErrorKind::Permanent => "Permanently rejected",
        SendErrorKind::Internal => "Error",
    }
}

pub fn send_kind_user_message(kind: SendErrorKind) -> &'static str {
    match kind {
        SendErrorKind::NetworkOrProxy => {
            "Could not reach the proxy or mail server. Check the proxy URL and network."
        }
        SendErrorKind::TlsOrSni => {
            "Secure connection failed. Check the SMTP hostname (certificate / SNI)."
        }
        SendErrorKind::Auth => "SMTP sign-in failed. Check the username and password.",
        SendErrorKind::Timeout => "Sending timed out. Try again or check the proxy and SMTP host.",
        SendErrorKind::Cancelled => "Sending was cancelled.",
        SendErrorKind::NotConfigured => {
            "This account has no SMTP settings. Add them in account settings to send."
        }
        SendErrorKind::TlsModeUnsupported => {
            "This SMTP TLS mode is not supported."
        }
        SendErrorKind::RecipientRejected => "The server rejected a recipient.",
        SendErrorKind::MessageTooLarge => "The server rejected the message as too large.",
        SendErrorKind::Transient => "The server is temporarily unavailable (4xx). Try again.",
        SendErrorKind::Permanent => "The server permanently rejected the message.",
        SendErrorKind::Internal => "Something went wrong while sending.",
    }
}
