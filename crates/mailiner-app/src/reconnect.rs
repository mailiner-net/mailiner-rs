//! Auto-reconnect backoff and transport-death classification.
//!
//! Pure helpers so the schedule can be unit-tested without WASM / Dioxus.

use std::io::ErrorKind;

use mailiner_core::MailinerError;

/// Delay (ms) before the next auto-reconnect after `failed_attempts` failures.
///
/// Attempt 0 is immediate. After that: 1s, 2s, 5s, 15s, 30s. `None` means give up.
pub const RECONNECT_BACKOFF_MS: &[u32] = &[0, 1_000, 2_000, 5_000, 15_000, 30_000];

/// Maximum automatic connect attempts (including the immediate first try).
pub const MAX_AUTO_RECONNECT_ATTEMPTS: u32 = RECONNECT_BACKOFF_MS.len() as u32;

/// Delay before trying again after `failed_attempts` unsuccessful auto-reconnects.
pub fn reconnect_backoff_ms(failed_attempts: u32) -> Option<u32> {
    RECONNECT_BACKOFF_MS.get(failed_attempts as usize).copied()
}

/// True when `err` means the IMAP/WebSocket transport is dead (not a mailbox/protocol error).
pub fn is_session_death(err: &MailinerError) -> bool {
    match err {
        MailinerError::Io(io) => is_transport_io(io),
        MailinerError::Tls(_) => true,
        MailinerError::Auth(_)
        | MailinerError::InvalidData(_)
        | MailinerError::NotFound(_)
        | MailinerError::PartialMove { .. }
        | MailinerError::Serialization(_) => false,
        MailinerError::Connector(msg) => is_session_death_message(msg),
    }
}

/// Classify an I/O error as a dropped socket / proxy.
pub fn is_transport_io(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        ErrorKind::ConnectionReset
            | ErrorKind::ConnectionAborted
            | ErrorKind::BrokenPipe
            | ErrorKind::UnexpectedEof
            | ErrorKind::NotConnected
            | ErrorKind::ConnectionRefused
    ) || is_session_death_message(&err.to_string())
}

/// Heuristic on a connector / IMAP error string (async-imap wraps I/O as text).
pub fn is_session_death_message(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    const NEEDLES: &[&str] = &[
        "websocket closed",
        "websocket error",
        "failed to send websocket",
        "broken pipe",
        "connection aborted",
        "connection reset",
        "connection refused",
        "connection lost",
        "connection closed",
        "not connected",
        "unexpected eof",
        "reset by peer",
        "network is unreachable",
        "i/o error",
        "io error",
    ];
    NEEDLES.iter().any(|n| lower.contains(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_starts_immediate_then_caps() {
        assert_eq!(reconnect_backoff_ms(0), Some(0));
        assert_eq!(reconnect_backoff_ms(1), Some(1_000));
        assert_eq!(reconnect_backoff_ms(2), Some(2_000));
        assert_eq!(reconnect_backoff_ms(3), Some(5_000));
        assert_eq!(reconnect_backoff_ms(4), Some(15_000));
        assert_eq!(reconnect_backoff_ms(5), Some(30_000));
        assert_eq!(reconnect_backoff_ms(6), None);
        assert_eq!(reconnect_backoff_ms(99), None);
    }

    #[test]
    fn backoff_table_len_matches_max_attempts() {
        assert_eq!(
            RECONNECT_BACKOFF_MS.len() as u32,
            MAX_AUTO_RECONNECT_ATTEMPTS
        );
        assert!(RECONNECT_BACKOFF_MS.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn session_death_from_io_kinds() {
        for kind in [
            ErrorKind::BrokenPipe,
            ErrorKind::ConnectionAborted,
            ErrorKind::ConnectionReset,
            ErrorKind::UnexpectedEof,
            ErrorKind::NotConnected,
            ErrorKind::ConnectionRefused,
        ] {
            let err = MailinerError::Io(std::io::Error::new(kind, "socket"));
            assert!(is_session_death(&err), "{kind:?} should be session death");
        }
        let timed_out = MailinerError::Io(std::io::Error::new(
            ErrorKind::TimedOut,
            "operation timed out",
        ));
        assert!(!is_session_death(&timed_out));
        let other = MailinerError::Io(std::io::Error::new(
            ErrorKind::InvalidData,
            "malformed frame",
        ));
        assert!(!is_session_death(&other));
    }

    #[test]
    fn session_death_from_wrapped_connector_text() {
        assert!(is_session_death(&MailinerError::Connector(
            "Failed to LIST folders: WebSocket closed".into()
        )));
        assert!(is_session_death(&MailinerError::Connector(
            "Failed to FETCH: broken pipe".into()
        )));
        assert!(is_session_death(&MailinerError::Tls(
            "unexpected eof".into()
        )));
        assert!(is_session_death_message(
            "io error: connection reset by peer"
        ));
        assert!(!is_session_death(&MailinerError::Connector(
            "NO [NONEXISTENT] no such mailbox".into()
        )));
        assert!(!is_session_death(&MailinerError::Auth(
            "LOGIN failed".into()
        )));
        assert!(!is_session_death(&MailinerError::NotFound(
            "message gone".into()
        )));
        assert!(!is_session_death(&MailinerError::InvalidData(
            "bad envelope".into()
        )));
        assert!(!is_session_death(&MailinerError::Connector(
            "FETCH timed out".into()
        )));
        assert!(!is_session_death(&MailinerError::PartialMove {
            message: "expunge failed".into(),
            dest_ids: vec![],
        }));
    }
}
