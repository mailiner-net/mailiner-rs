//! SMTP submission DTOs (no transport).

use serde::{Deserialize, Serialize};

/// RFC 3461 `RET` parameter: how much of the original to return in a DSN.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DsnReturn {
    /// `RET=HDRS` — headers only (default; less of the message is bounced).
    #[default]
    Hdrs,
    /// `RET=FULL` — entire original message.
    Full,
}

impl DsnReturn {
    /// SMTP token (`HDRS` / `FULL`).
    pub fn as_smtp(self) -> &'static str {
        match self {
            Self::Hdrs => "HDRS",
            Self::Full => "FULL",
        }
    }
}

/// Optional RFC 3461 DSN request for one submission.
///
/// The SMTP connector emits `NOTIFY` / `RET` / `ENVID` only when the server
/// advertised `DSN` on EHLO. Unchecked success and failure means no DSN.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DsnRequest {
    /// `NOTIFY=SUCCESS`.
    pub notify_success: bool,
    /// `NOTIFY=FAILURE`.
    pub notify_failure: bool,
    /// `RET=HDRS` or `RET=FULL`.
    #[serde(default)]
    pub ret: DsnReturn,
    /// `ENVID` token. Generated from Message-ID when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub envid: Option<String>,
}

impl DsnRequest {
    /// Request success and/or failure notifications. `None` when both are off.
    pub fn new(notify_success: bool, notify_failure: bool) -> Option<Self> {
        if !notify_success && !notify_failure {
            return None;
        }
        Some(Self {
            notify_success,
            notify_failure,
            ret: DsnReturn::Hdrs,
            envid: None,
        })
    }

    /// Whether any DSN notification was requested.
    pub fn is_requested(&self) -> bool {
        self.notify_success || self.notify_failure
    }

    /// SMTP `NOTIFY=` value, or `None` when no notification is requested.
    pub fn notify_value(&self) -> Option<&'static str> {
        match (self.notify_success, self.notify_failure) {
            (false, false) => None,
            (true, false) => Some("SUCCESS"),
            (false, true) => Some("FAILURE"),
            (true, true) => Some("SUCCESS,FAILURE"),
        }
    }

    /// `ENVID` token: explicit value, else a sanitized Message-ID.
    pub fn envid_value(&self, message_id: &str) -> String {
        if let Some(id) = self
            .envid
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return sanitize_envid(id);
        }
        sanitize_envid(message_id)
    }
}

/// Keep ENVID as a short xtext-safe ASCII token (RFC 3461).
pub fn sanitize_envid(raw: &str) -> String {
    const MAX: usize = 80;
    let stripped = raw.trim().trim_matches(|c| c == '<' || c == '>');
    let mut out = String::new();
    for c in stripped.chars() {
        if out.len() >= MAX {
            break;
        }
        if c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '-' | '_') {
            out.push(c);
        } else if !c.is_ascii_whitespace() {
            out.push('_');
        }
    }
    if out.is_empty() {
        "mailiner".into()
    } else {
        out
    }
}

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
    /// Optional DSN (NOTIFY/RET/ENVID). Applied only if the server advertises DSN.
    pub dsn: Option<DsnRequest>,
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
    use super::{sanitize_envid, DsnRequest, DsnReturn, SendErrorKind};

    #[test]
    fn unknown_kind_deserializes_as_permanent() {
        let kind: SendErrorKind = serde_json::from_str("\"future_kind\"").unwrap();
        assert_eq!(kind, SendErrorKind::Permanent);
    }

    #[test]
    fn dsn_new_none_when_both_off() {
        assert!(DsnRequest::new(false, false).is_none());
        let both = DsnRequest::new(true, true).unwrap();
        assert_eq!(both.notify_value(), Some("SUCCESS,FAILURE"));
        assert_eq!(both.ret, DsnReturn::Hdrs);
        assert_eq!(
            DsnRequest::new(true, false).unwrap().notify_value(),
            Some("SUCCESS")
        );
        assert_eq!(
            DsnRequest::new(false, true).unwrap().notify_value(),
            Some("FAILURE")
        );
    }

    #[test]
    fn sanitize_envid_strips_brackets_and_limits() {
        assert_eq!(sanitize_envid("<id@example.com>"), "id@example.com");
        assert_eq!(sanitize_envid("  "), "mailiner");
        assert_eq!(sanitize_envid("a b/c"), "ab_c");
        let long = "x".repeat(120);
        assert_eq!(sanitize_envid(&long).len(), 80);
    }

    #[test]
    fn envid_value_prefers_explicit() {
        let mut dsn = DsnRequest::new(true, false).unwrap();
        dsn.envid = Some("<custom+id>".into());
        assert_eq!(dsn.envid_value("<msg@example.com>"), "custom_id");
        dsn.envid = None;
        assert_eq!(dsn.envid_value("<msg@example.com>"), "msg@example.com");
    }
}
