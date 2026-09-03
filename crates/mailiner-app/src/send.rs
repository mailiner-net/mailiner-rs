//! Composer send / Test SMTP UI state (no secrets).

use mailiner_composer::identity::FromIdentity;
use mailiner_composer::model::draft::{ComposerAddress, DraftDocument};
use mailiner_core::MessageId;
use mailiner_core::submit::SendErrorKind;

use crate::account::AccountId;

/// Open compose session (owned by [`crate::context::AppContext::compose_draft`]).
#[derive(Clone, Debug)]
pub struct ComposeSession {
    /// Account used for From identity and SMTP/IMAP send (independent of the open folder).
    pub account_id: AccountId,
    /// Dialog title (`New message`, `Reply`, `Forward`).
    pub title: String,
    /// Prefill document.
    pub draft: DraftDocument,
    /// Source message to mark `\Answered` after a successful Reply / Reply All.
    pub reply_source: Option<MessageId>,
}

/// Choose which stored account a compose session should send from.
///
/// `preferred` wins when it is still in `stored` (selected account for new
/// compose; message owner for reply/forward). Otherwise `fallback` (typically
/// the currently selected account), then the first stored id.
pub fn resolve_compose_account_id(
    preferred: Option<&AccountId>,
    fallback: Option<&AccountId>,
    stored: &[AccountId],
) -> Option<AccountId> {
    for candidate in [preferred, fallback].into_iter().flatten() {
        if stored.iter().any(|id| id == candidate) {
            return Some(candidate.clone());
        }
    }
    stored.first().cloned()
}

/// From picker label: `Name <email>` or just the address.
pub fn from_account_label(name: &str, email: &str) -> String {
    let name = name.trim();
    let email = email.trim();
    if name.is_empty() || name.eq_ignore_ascii_case(email) {
        email.to_string()
    } else {
        format!("{name} <{email}>")
    }
}

/// Sender identity for an account's display name + mailbox.
pub fn identity_from_account(name: impl Into<String>, email: impl Into<String>) -> FromIdentity {
    FromIdentity::new(name, email)
}

/// Draft From address for an identity (empty display name is omitted).
pub fn composer_address_from_identity(identity: &FromIdentity) -> ComposerAddress {
    ComposerAddress {
        name: if identity.display_name.is_empty() {
            None
        } else {
            Some(identity.display_name.clone())
        },
        email: identity.email.clone(),
    }
}

/// Switch the session's send account and draft From header.
pub fn set_session_from_account(
    session: &mut ComposeSession,
    account_id: AccountId,
    name: &str,
    email: &str,
) {
    session.account_id = account_id;
    session.draft.from = Some(composer_address_from_identity(&identity_from_account(
        name, email,
    )));
    session.draft.touch();
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
        SendErrorKind::RecipientRejected => "The server rejected a recipient.",
        SendErrorKind::MessageTooLarge => "The server rejected the message as too large.",
        SendErrorKind::Transient => "The server is temporarily unavailable (4xx). Try again.",
        SendErrorKind::Permanent => "The server permanently rejected the message.",
        SendErrorKind::Internal => "Something went wrong while sending.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> AccountId {
        AccountId::new(s)
    }

    #[test]
    fn resolve_prefers_known_preferred() {
        let a = id("a");
        let b = id("b");
        let stored = [a.clone(), b.clone()];
        assert_eq!(
            resolve_compose_account_id(Some(&b), Some(&a), &stored),
            Some(b)
        );
    }

    #[test]
    fn resolve_falls_back_when_preferred_missing() {
        let a = id("a");
        let b = id("b");
        let ghost = id("ghost");
        let stored = [a.clone(), b.clone()];
        assert_eq!(
            resolve_compose_account_id(Some(&ghost), Some(&b), &stored),
            Some(b)
        );
        assert_eq!(
            resolve_compose_account_id(Some(&ghost), None, &stored),
            Some(a.clone())
        );
        assert_eq!(
            resolve_compose_account_id(None, None, &stored),
            Some(a.clone())
        );
        assert_eq!(resolve_compose_account_id(None, None, &[]), None);
    }

    #[test]
    fn resolve_ignores_fallback_not_in_store() {
        let a = id("a");
        let ghost = id("ghost");
        let stored = [a.clone()];
        assert_eq!(
            resolve_compose_account_id(None, Some(&ghost), &stored),
            Some(a)
        );
    }

    #[test]
    fn from_account_label_formats_name_and_email() {
        assert_eq!(
            from_account_label("Alice", "alice@example.com"),
            "Alice <alice@example.com>"
        );
        assert_eq!(
            from_account_label("", "alice@example.com"),
            "alice@example.com"
        );
        assert_eq!(
            from_account_label("  ", "alice@example.com"),
            "alice@example.com"
        );
        assert_eq!(
            from_account_label("alice@example.com", "alice@example.com"),
            "alice@example.com"
        );
        assert_eq!(
            from_account_label(" Alice ", " alice@example.com "),
            "Alice <alice@example.com>"
        );
    }

    #[test]
    fn set_session_from_account_updates_id_and_draft_from() {
        let identity = identity_from_account("Old", "old@example.com");
        let mut session = ComposeSession {
            title: "New message".into(),
            draft: DraftDocument::new_empty(&identity),
            account_id: id("old"),
            reply_source: None,
        };
        set_session_from_account(&mut session, id("new"), "New", "new@example.com");
        assert_eq!(session.account_id.as_str(), "new");
        let from = session.draft.from.expect("from");
        assert_eq!(from.email, "new@example.com");
        assert_eq!(from.name.as_deref(), Some("New"));
    }

    #[test]
    fn composer_address_from_identity_omits_empty_name() {
        let with_name = identity_from_account("Me", "me@example.com");
        let addr = composer_address_from_identity(&with_name);
        assert_eq!(addr.name.as_deref(), Some("Me"));
        assert_eq!(addr.email, "me@example.com");

        let no_name = identity_from_account("", "me@example.com");
        let addr = composer_address_from_identity(&no_name);
        assert_eq!(addr.name, None);
        assert_eq!(addr.email, "me@example.com");
    }

    #[test]
    fn set_session_from_account_omits_empty_display_name() {
        let identity = identity_from_account("Old", "old@example.com");
        let mut session = ComposeSession {
            title: "Reply".into(),
            draft: DraftDocument::new_empty(&identity),
            account_id: id("old"),
            reply_source: None,
        };
        set_session_from_account(&mut session, id("new"), "", "new@example.com");
        let from = session.draft.from.expect("from");
        assert_eq!(from.email, "new@example.com");
        assert_eq!(from.name, None);
    }
}
