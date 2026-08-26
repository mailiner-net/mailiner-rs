//! SMTP submission DTOs (no transport).

use serde::{Deserialize, Serialize};

/// Outbound message for `SmtpConnector::submit`.
#[derive(Debug, Clone)]
pub struct SubmitRequest {
    /// SMTP MAIL FROM mailbox.
    pub mail_from: String,
    /// SMTP RCPT TO mailboxes.
    pub rcpt_to: Vec<String>,
    /// Complete RFC 5322 message (headers + body). Not DATA-dot-stuffed.
    pub rfc822: Vec<u8>,
    /// Message-ID already written into `rfc822`.
    pub message_id: String,
}

/// Result of a successful SMTP DATA.
#[derive(Debug, Clone)]
pub struct SubmitReceipt {
    /// Copied from [`SubmitRequest::message_id`].
    pub message_id: String,
    /// SMTP reply text, truncated, no secrets.
    pub server_reply: Option<String>,
}

/// Classified send/test failure. Persisted on outbox items (no secrets).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendErrorKind {
    /// Proxy or TCP/WebSocket failure.
    NetworkOrProxy,
    /// rustls / SNI / certificate.
    TlsOrSni,
    /// AUTH rejected or no usable mechanism.
    Auth,
    /// Connect or DATA budget exceeded.
    Timeout,
    /// User cancelled (disconnect / account delete).
    Cancelled,
    /// Account has no SMTP settings.
    NotConfigured,
    /// A RCPT TO was rejected (5xx).
    RecipientRejected,
    /// Server rejected the message as too large.
    MessageTooLarge,
    /// SMTP 4xx.
    Transient,
    /// Programmer / unexpected error.
    Internal,
    /// SMTP 5xx not otherwise classified. Unknown persisted variants land here.
    #[serde(other)]
    Permanent,
}

impl SendErrorKind {
    /// Auto-retry / keep `Queued` after write-ahead persist.
    pub fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::NetworkOrProxy | Self::TlsOrSni | Self::Timeout | Self::Transient
        )
    }
}

#[cfg(test)]
mod tests {
    use super::SendErrorKind;

    #[test]
    fn unknown_kind_deserializes_as_permanent() {
        let kind: SendErrorKind = serde_json::from_str("\"future_kind\"").unwrap();
        assert_eq!(kind, SendErrorKind::Permanent);
    }
}
