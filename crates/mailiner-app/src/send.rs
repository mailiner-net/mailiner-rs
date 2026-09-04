//! Composer send / Test SMTP UI state (no secrets).

use mailiner_composer::FileAttachment;
use mailiner_composer::flatten_addresses;
use mailiner_composer::identity::FromIdentity;
use mailiner_composer::model::draft::{ComposerAddress, DraftDocument};
use mailiner_core::EmailAddress;
use mailiner_core::MessageId;
use mailiner_core::submit::SendErrorKind;

use crate::account::{Account, AccountId};
use crate::account_config::AccountIdentity;

/// Open compose session (owned by [`crate::context::AppContext::compose_draft`]).
#[derive(Clone, Debug)]
pub struct ComposeSession {
    /// Account this session was opened for (From / SMTP; independent of the open folder).
    pub account_id: AccountId,
    /// Dialog title (`New message`, `Reply`, `Forward`).
    pub title: String,
    /// Prefill document.
    pub draft: DraftDocument,
    /// Source message to mark `\Answered` after a successful Reply / Reply All.
    pub reply_source: Option<MessageId>,
    /// Original forwarded files removed via the include toggle.
    pub stashed_originals: Vec<FileAttachment>,
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

/// One selectable From row (account + identity index).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FromChoice {
    pub account_id: AccountId,
    pub identity: AccountIdentity,
    /// `0` is the account primary; extras follow.
    pub index: usize,
}

impl FromChoice {
    /// `<select>` value: `account_id` + unit separator + index.
    pub fn key(&self) -> String {
        encode_from_choice_key(&self.account_id, self.index)
    }
}

const FROM_KEY_SEP: char = '\u{1f}';

/// Encode a From picker value.
pub fn encode_from_choice_key(account_id: &AccountId, index: usize) -> String {
    format!("{}{FROM_KEY_SEP}{index}", account_id.as_str())
}

/// Parse a From picker value.
pub fn parse_from_choice_key(key: &str) -> Option<(AccountId, usize)> {
    let (id, idx) = key.rsplit_once(FROM_KEY_SEP)?;
    let index = idx.parse().ok()?;
    if id.is_empty() {
        return None;
    }
    Some((AccountId::new(id), index))
}

/// Flatten stored accounts into From picker rows (primary then extras).
pub fn list_from_choices(accounts: &[Account]) -> Vec<FromChoice> {
    let mut out = Vec::new();
    for account in accounts {
        for (index, identity) in account.all_identities().into_iter().enumerate() {
            out.push(FromChoice {
                account_id: account.id.clone(),
                identity,
                index,
            });
        }
    }
    out
}

fn identity_matches(identity: &AccountIdentity, from: &ComposerAddress) -> bool {
    from.email
        .trim()
        .eq_ignore_ascii_case(identity.email.trim())
        && match from
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            Some(name) => name.eq_ignore_ascii_case(identity.display_name.trim()),
            None => identity.display_name.trim().is_empty(),
        }
}

/// Identity that matches `from` on `account`, else the primary.
pub fn resolve_account_identity(
    account: &Account,
    from: Option<&ComposerAddress>,
) -> AccountIdentity {
    let identities = account.all_identities();
    if let Some(from) = from {
        if let Some(id) = identities.iter().find(|id| identity_matches(id, from)) {
            return id.clone();
        }
        if let Some(id) = identities
            .iter()
            .find(|id| from.email.trim().eq_ignore_ascii_case(id.email.trim()))
        {
            return id.clone();
        }
    }
    account.primary_identity()
}

/// Extra identity whose email appears in `emails`, else the primary if it does.
///
/// Aliases win over the primary when both are present so Reply/Forward send
/// from the address that received the mail, not the account default.
pub fn identity_matching_emails<'a>(
    account: &Account,
    emails: impl IntoIterator<Item = &'a str>,
) -> AccountIdentity {
    let listed: Vec<&str> = emails.into_iter().collect();
    for extra in &account.identities {
        if listed
            .iter()
            .any(|email| email.trim().eq_ignore_ascii_case(extra.email.trim()))
        {
            return extra.clone();
        }
    }
    if listed
        .iter()
        .any(|email| email.trim().eq_ignore_ascii_case(account.email.trim()))
    {
        return account.primary_identity();
    }
    account.primary_identity()
}

/// From identity for Reply/Forward from the original To/Cc.
///
/// Aliases win over the primary even when the primary is listed first (To)
/// and the alias is only on Cc — walking recipients in header order would
/// otherwise lock onto the primary and ignore the alias.
pub fn identity_for_reply(
    account: &Account,
    to: Option<&EmailAddress>,
    cc: Option<&EmailAddress>,
) -> AccountIdentity {
    let recipient_emails: Vec<String> = to
        .into_iter()
        .chain(cc)
        .flat_map(flatten_addresses)
        .map(|a| a.email)
        .collect();
    identity_matching_emails(account, recipient_emails.iter().map(String::as_str))
}

/// Whether `email` is this account's primary mailbox or an extra identity.
pub fn is_account_identity_email(account: &Account, email: &str) -> bool {
    account
        .all_identities()
        .iter()
        .any(|id| email.trim().eq_ignore_ascii_case(id.email.trim()))
}

/// Drop To/Cc/Bcc addresses that belong to `account` (all identities).
pub fn strip_account_identities(draft: &mut DraftDocument, account: &Account) {
    draft
        .to
        .retain(|a| !is_account_identity_email(account, &a.email));
    draft
        .cc
        .retain(|a| !is_account_identity_email(account, &a.email));
    draft
        .bcc
        .retain(|a| !is_account_identity_email(account, &a.email));
}

/// Picker row for the current session From, falling back to the account primary.
pub fn selected_from_choice<'a>(
    choices: &'a [FromChoice],
    account_id: Option<&AccountId>,
    from: Option<&ComposerAddress>,
) -> Option<&'a FromChoice> {
    let account_id = account_id?;
    let on_account: Vec<&FromChoice> = choices
        .iter()
        .filter(|c| &c.account_id == account_id)
        .collect();
    if let Some(from) = from {
        if let Some(c) = on_account
            .iter()
            .copied()
            .find(|c| identity_matches(&c.identity, from))
        {
            return Some(c);
        }
        if let Some(c) = on_account.iter().copied().find(|c| {
            from.email
                .trim()
                .eq_ignore_ascii_case(c.identity.email.trim())
        }) {
            return Some(c);
        }
    }
    on_account.into_iter().find(|c| c.index == 0)
}

/// Sender identity for an account's display name + mailbox.
pub fn identity_from_account(name: impl Into<String>, email: impl Into<String>) -> FromIdentity {
    FromIdentity::new(name, email)
}

/// Map a stored [`AccountIdentity`] to the composer type.
pub fn identity_from_stored(identity: &AccountIdentity) -> FromIdentity {
    identity_from_account(identity.display_name.clone(), identity.email.clone())
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
pub fn set_session_from_identity(
    session: &mut ComposeSession,
    account_id: AccountId,
    identity: &AccountIdentity,
) {
    session.account_id = account_id;
    session.draft.from = Some(composer_address_from_identity(&identity_from_stored(
        identity,
    )));
    session.draft.touch();
}

/// Switch the session's send account and draft From header.
pub fn set_session_from_account(
    session: &mut ComposeSession,
    account_id: AccountId,
    name: &str,
    email: &str,
) {
    set_session_from_identity(session, account_id, &AccountIdentity::new(name, email));
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
            stashed_originals: Vec::new(),
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
            stashed_originals: Vec::new(),
        };
        set_session_from_account(&mut session, id("new"), "", "new@example.com");
        let from = session.draft.from.expect("from");
        assert_eq!(from.email, "new@example.com");
        assert_eq!(from.name, None);
    }

    fn account(id: &str, name: &str, email: &str, extras: Vec<AccountIdentity>) -> Account {
        Account {
            id: AccountId::new(id),
            name: name.into(),
            email: email.into(),
            host: "imap.example.com".into(),
            signature: None,
            identities: extras,
        }
    }

    #[test]
    fn list_from_choices_includes_primary_and_extras() {
        let work = account(
            "w",
            "Work",
            "work@example.com",
            vec![AccountIdentity::new("Support", "support@example.com")],
        );
        let home = account("h", "Home", "home@example.com", Vec::new());
        let choices = list_from_choices(&[work, home]);
        assert_eq!(choices.len(), 3);
        assert_eq!(choices[0].account_id.as_str(), "w");
        assert_eq!(choices[0].index, 0);
        assert_eq!(choices[0].identity.email, "work@example.com");
        assert_eq!(choices[1].index, 1);
        assert_eq!(choices[1].identity.email, "support@example.com");
        assert_eq!(choices[2].account_id.as_str(), "h");
        assert_eq!(parse_from_choice_key(&choices[1].key()), Some((id("w"), 1)));
    }

    #[test]
    fn resolve_account_identity_prefers_name_and_email() {
        let acc = account(
            "w",
            "Work",
            "work@example.com",
            vec![
                AccountIdentity::new("Support", "support@example.com"),
                AccountIdentity::new("Help", "support@example.com"),
            ],
        );
        let from = ComposerAddress {
            name: Some("Help".into()),
            email: "support@example.com".into(),
        };
        let id = resolve_account_identity(&acc, Some(&from));
        assert_eq!(id.display_name, "Help");
        assert_eq!(id.email, "support@example.com");

        let unknown = ComposerAddress {
            name: Some("X".into()),
            email: "other@example.com".into(),
        };
        let fallback = resolve_account_identity(&acc, Some(&unknown));
        assert_eq!(fallback.email, "work@example.com");
    }

    #[test]
    fn identity_matching_emails_picks_alias() {
        let acc = account(
            "w",
            "Work",
            "work@example.com",
            vec![AccountIdentity::new("Support", "support@example.com")],
        );
        let id = identity_matching_emails(&acc, ["boss@example.com", "support@example.com"]);
        assert_eq!(id.email, "support@example.com");
        let primary = identity_matching_emails(&acc, ["nobody@example.com"]);
        assert_eq!(primary.email, "work@example.com");
    }

    #[test]
    fn identity_matching_emails_prefers_alias_over_primary() {
        let acc = account(
            "w",
            "Work",
            "work@example.com",
            vec![AccountIdentity::new("Support", "support@example.com")],
        );
        let id = identity_matching_emails(&acc, ["work@example.com", "support@example.com"]);
        assert_eq!(id.email, "support@example.com");
        assert_eq!(id.display_name, "Support");
    }

    fn addr(email: &str) -> mailiner_core::EmailAddress {
        mailiner_core::EmailAddress::List(vec![mailiner_core::EmailAddr {
            name: None,
            email: Some(email.into()),
        }])
    }

    #[test]
    fn identity_for_reply_prefers_alias_in_cc_over_primary_in_to() {
        let acc = account(
            "w",
            "Work",
            "work@example.com",
            vec![AccountIdentity::new("Support", "support@example.com")],
        );
        let to = addr("work@example.com");
        let cc = addr("support@example.com");
        let id = identity_for_reply(&acc, Some(&to), Some(&cc));
        assert_eq!(id.email, "support@example.com");
        assert_eq!(id.display_name, "Support");
    }

    #[test]
    fn strip_account_identities_removes_primary_and_aliases() {
        let acc = account(
            "w",
            "Work",
            "work@example.com",
            vec![AccountIdentity::new("Support", "support@example.com")],
        );
        let identity = identity_from_account("Support", "support@example.com");
        let mut draft = DraftDocument::new_empty(&identity);
        draft.to = vec![
            ComposerAddress::email_only("boss@example.com"),
            ComposerAddress::email_only("work@example.com"),
        ];
        draft.cc = vec![
            ComposerAddress::email_only("support@example.com"),
            ComposerAddress::email_only("cc@example.com"),
        ];
        strip_account_identities(&mut draft, &acc);
        assert_eq!(
            draft
                .to
                .iter()
                .map(|a| a.email.as_str())
                .collect::<Vec<_>>(),
            vec!["boss@example.com"]
        );
        assert_eq!(
            draft
                .cc
                .iter()
                .map(|a| a.email.as_str())
                .collect::<Vec<_>>(),
            vec!["cc@example.com"]
        );
    }

    #[test]
    fn selected_from_choice_matches_draft_from() {
        let acc = account(
            "w",
            "Work",
            "work@example.com",
            vec![AccountIdentity::new("Support", "support@example.com")],
        );
        let choices = list_from_choices(&[acc]);
        let from = ComposerAddress {
            name: Some("Support".into()),
            email: "support@example.com".into(),
        };
        let selected = selected_from_choice(&choices, Some(&id("w")), Some(&from)).unwrap();
        assert_eq!(selected.index, 1);
        let fallback = selected_from_choice(&choices, Some(&id("w")), None).unwrap();
        assert_eq!(fallback.index, 0);
    }

    #[test]
    fn set_session_from_identity_updates_from() {
        let identity = identity_from_account("Old", "old@example.com");
        let mut session = ComposeSession {
            title: "New message".into(),
            draft: DraftDocument::new_empty(&identity),
            account_id: id("old"),
            reply_source: None,
            stashed_originals: Vec::new(),
        };
        set_session_from_identity(
            &mut session,
            id("w"),
            &AccountIdentity::new("Support", "support@example.com"),
        );
        assert_eq!(session.account_id.as_str(), "w");
        let from = session.draft.from.expect("from");
        assert_eq!(from.email, "support@example.com");
        assert_eq!(from.name.as_deref(), Some("Support"));
    }
}
