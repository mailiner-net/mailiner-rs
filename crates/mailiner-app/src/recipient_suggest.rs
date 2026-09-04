//! Recipient autocomplete: address book + recently used From/To addresses.

use mailiner_composer::shell::recipient_field::{chip_is_valid, parse_recipient};
use mailiner_composer::{ComposerAddress, emails_equal, flatten_addresses};
use mailiner_core::{EmailAddress, Envelope};
use serde::{Deserialize, Serialize};

use crate::account_store::{MemoryKvStore, StringKvStore, WebLocalStorage};
use crate::address_book::{
    AddressBookError, Contact, DEFAULT_SUGGESTION_LIMIT, address_suggest_rank, parse_contact,
};

/// `localStorage` key for the recent-recipients blob (independent of the address book).
pub const RECENT_RECIPIENTS_LOCAL_STORAGE_KEY: &str = "mailiner.recent-recipients.v1";
/// Recent-recipients blob schema.
pub const RECENT_RECIPIENTS_SCHEMA_VERSION: u32 = 1;
/// Cap on persisted recents. Newest first; overflow is dropped.
pub const MAX_RECENT_RECIPIENTS: usize = 50;
/// Cap on addresses harvested from the open mailbox list.
pub const MAX_HARVESTED_ADDRESSES: usize = 200;

/// Where a suggestion came from (contacts win ties).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SuggestionSource {
    /// Local address book.
    Contact = 0,
    /// Sent-mail recents or From/To on loaded envelopes.
    Recent = 1,
}

/// One autocomplete row for the compose chip field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipientSuggestion {
    /// Name + mailbox. Formatting and chip conversion come from [`Contact`].
    pub contact: Contact,
    /// Address book vs recent / harvested.
    pub source: SuggestionSource,
}

impl RecipientSuggestion {
    /// Visible primary line: name, or the mailbox when unnamed.
    pub fn display_label(&self) -> &str {
        self.contact.display_label()
    }

    /// Quoted `Name <email>` when a name exists; otherwise the mailbox.
    pub fn formatted(&self) -> String {
        self.contact.formatted()
    }

    /// Chip to commit when the row is accepted.
    pub fn to_composer_address(&self) -> ComposerAddress {
        self.contact.to_composer_address()
    }

    /// Short source label for the suggestion list.
    pub fn source_label(&self) -> &'static str {
        match self.source {
            SuggestionSource::Contact => "Contact",
            SuggestionSource::Recent => "Recent",
        }
    }
}

/// Prefix/substring matches from the address book, then recents.
///
/// Empty query returns no suggestions. Already-chipped mailboxes are omitted.
/// Ranking matches [`crate::address_book::suggest_contacts`]; contacts beat recents
/// on a tie. Results stay unique and are capped at `limit`.
pub fn suggest_recipients(
    contacts: &[Contact],
    recents: &[Contact],
    query: &str,
    exclude: &[ComposerAddress],
    limit: usize,
) -> Vec<RecipientSuggestion> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() || limit == 0 {
        return Vec::new();
    }
    let mut seen: Vec<String> = Vec::new();
    let mut ranked: Vec<(crate::address_book::SuggestRank, RecipientSuggestion)> = Vec::new();
    for (source, list) in [
        (SuggestionSource::Contact, contacts),
        (SuggestionSource::Recent, recents),
    ] {
        for contact in list {
            if exclude
                .iter()
                .any(|chip| emails_equal(&chip.email, &contact.email))
            {
                continue;
            }
            if seen.iter().any(|email| emails_equal(email, &contact.email)) {
                continue;
            }
            let Some(rank) = address_suggest_rank(&contact.name, &contact.email, &needle) else {
                continue;
            };
            seen.push(contact.email.clone());
            ranked.push((
                rank,
                RecipientSuggestion {
                    contact: contact.clone(),
                    source,
                },
            ));
        }
    }
    ranked.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.source.cmp(&b.1.source))
            .then_with(|| {
                a.1.contact
                    .name
                    .to_ascii_lowercase()
                    .cmp(&b.1.contact.name.to_ascii_lowercase())
            })
            .then_with(|| {
                a.1.contact
                    .email
                    .to_ascii_lowercase()
                    .cmp(&b.1.contact.email.to_ascii_lowercase())
            })
    });
    ranked.into_iter().take(limit).map(|(_, s)| s).collect()
}

/// True when Enter should commit the typed token instead of the highlighted row.
///
/// A complete, valid mailbox that is not the highlight is treated as a new address
/// (so `ada@other.com` is not replaced by a prefix-matching contact).
pub fn typed_overrides_suggestion(draft: &str, highlighted_email: &str) -> bool {
    let Some(addr) = parse_recipient(draft) else {
        return false;
    };
    chip_is_valid(&addr) && !emails_equal(&addr.email, highlighted_email)
}

/// From/To mailboxes on loaded envelopes (inbox senders, sent-mail recipients).
///
/// Skips `skip_emails` (typically the user's own accounts). Invalid rows are
/// dropped. Order is first-seen; the list is capped at [`MAX_HARVESTED_ADDRESSES`].
pub fn contacts_from_envelopes<'a>(
    envelopes: impl IntoIterator<Item = &'a Envelope>,
    skip_emails: &[&str],
) -> Vec<Contact> {
    let mut out = Vec::new();
    for env in envelopes {
        push_envelope_addresses(env.from.as_ref(), skip_emails, &mut out);
        push_envelope_addresses(env.to.as_ref(), skip_emails, &mut out);
        if out.len() >= MAX_HARVESTED_ADDRESSES {
            break;
        }
    }
    out
}

fn push_envelope_addresses(
    addr: Option<&EmailAddress>,
    skip_emails: &[&str],
    out: &mut Vec<Contact>,
) {
    let Some(addr) = addr else {
        return;
    };
    for chip in flatten_addresses(addr) {
        if out.len() >= MAX_HARVESTED_ADDRESSES {
            return;
        }
        if skip_emails
            .iter()
            .any(|skip| emails_equal(skip, &chip.email))
        {
            continue;
        }
        let Some(contact) = contact_from_address(chip.name.as_deref().unwrap_or(""), &chip.email)
        else {
            continue;
        };
        if out
            .iter()
            .any(|existing: &Contact| emails_equal(&existing.email, &contact.email))
        {
            continue;
        }
        out.push(contact);
    }
}

/// Parse a name+email pair, keeping a valid mailbox when the display name is over the cap.
pub fn contact_from_address(name: &str, email: &str) -> Option<Contact> {
    match parse_contact(name, email) {
        Ok(contact) => Some(contact),
        Err(AddressBookError::NameTooLong) => parse_contact("", email).ok(),
        Err(_) => None,
    }
}

/// Merge persisted recents (newest first) with harvested envelope addresses.
pub fn merge_recent_candidates(persisted: &[Contact], harvested: &[Contact]) -> Vec<Contact> {
    let mut out = persisted.to_vec();
    for contact in harvested {
        if out
            .iter()
            .any(|existing| emails_equal(&existing.email, &contact.email))
        {
            continue;
        }
        out.push(contact.clone());
    }
    out
}

/// Single JSON document stored under [`RECENT_RECIPIENTS_LOCAL_STORAGE_KEY`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecentRecipientsBlob {
    pub schema_version: u32,
    #[serde(default)]
    pub recipients: Vec<Contact>,
}

impl RecentRecipientsBlob {
    pub fn empty() -> Self {
        Self {
            schema_version: RECENT_RECIPIENTS_SCHEMA_VERSION,
            recipients: Vec::new(),
        }
    }

    pub fn encode(&self) -> Result<String, AddressBookError> {
        serde_json::to_string(self).map_err(|e| AddressBookError::Serialization(e.to_string()))
    }

    /// Rejects blobs whose `schema_version` is greater than
    /// [`RECENT_RECIPIENTS_SCHEMA_VERSION`] so a future format is not rewritten as v1.
    pub fn decode(json: &str) -> Result<Self, AddressBookError> {
        let raw: RawRecentRecipientsBlob = serde_json::from_str(json)
            .map_err(|e| AddressBookError::Serialization(e.to_string()))?;
        if raw.schema_version > RECENT_RECIPIENTS_SCHEMA_VERSION {
            return Err(AddressBookError::Serialization(format!(
                "unsupported recent recipients schema_version {} (max supported {})",
                raw.schema_version, RECENT_RECIPIENTS_SCHEMA_VERSION
            )));
        }
        let mut recipients = match raw.recipients {
            serde_json::Value::Null => Vec::new(),
            serde_json::Value::Array(rows) => rows
                .into_iter()
                .filter_map(|row| serde_json::from_value::<Contact>(row).ok())
                .collect(),
            _ => Vec::new(),
        };
        sanitize_recents(&mut recipients);
        Ok(Self {
            schema_version: RECENT_RECIPIENTS_SCHEMA_VERSION,
            recipients,
        })
    }

    /// Move `incoming` to the front (first item = newest). Keeps an older display
    /// name when the new row is email-only. Truncates to [`MAX_RECENT_RECIPIENTS`].
    pub fn remember(&mut self, incoming: &[Contact]) {
        for contact in incoming.iter().rev() {
            let old_name = self
                .recipients
                .iter()
                .find(|c| emails_equal(&c.email, &contact.email))
                .map(|c| c.name.clone())
                .filter(|n| !n.is_empty());
            self.recipients
                .retain(|c| !emails_equal(&c.email, &contact.email));
            let mut stored = contact.clone();
            if stored.name.is_empty()
                && let Some(name) = old_name
            {
                stored.name = name;
            }
            self.recipients.insert(0, stored);
        }
        if self.recipients.len() > MAX_RECENT_RECIPIENTS {
            self.recipients.truncate(MAX_RECENT_RECIPIENTS);
        }
        self.schema_version = RECENT_RECIPIENTS_SCHEMA_VERSION;
    }
}

#[derive(Debug, Deserialize)]
struct RawRecentRecipientsBlob {
    schema_version: u32,
    #[serde(default)]
    recipients: serde_json::Value,
}

fn sanitize_recents(recipients: &mut Vec<Contact>) {
    let mut kept = Vec::with_capacity(recipients.len());
    for raw in recipients.drain(..) {
        let Ok(contact) = parse_contact(&raw.name, &raw.email) else {
            continue;
        };
        if kept
            .iter()
            .any(|c: &Contact| emails_equal(&c.email, &contact.email))
        {
            continue;
        }
        if kept.len() >= MAX_RECENT_RECIPIENTS {
            break;
        }
        kept.push(contact);
    }
    *recipients = kept;
}

/// Recents over a string key-value store (`localStorage` in the browser).
pub struct RecentRecipientStore<K: StringKvStore> {
    kv: K,
}

impl RecentRecipientStore<WebLocalStorage> {
    pub fn open() -> Result<Self, AddressBookError> {
        Ok(Self {
            kv: WebLocalStorage::try_open()?,
        })
    }
}

impl RecentRecipientStore<MemoryKvStore> {
    pub fn open_memory() -> Self {
        Self {
            kv: MemoryKvStore::new(),
        }
    }
}

impl<K: StringKvStore> RecentRecipientStore<K> {
    pub fn with_kv(kv: K) -> Self {
        Self { kv }
    }

    fn load(&self) -> Result<RecentRecipientsBlob, AddressBookError> {
        match self.kv.get_item(RECENT_RECIPIENTS_LOCAL_STORAGE_KEY)? {
            None => Ok(RecentRecipientsBlob::empty()),
            Some(s) if s.trim().is_empty() => Ok(RecentRecipientsBlob::empty()),
            Some(s) => RecentRecipientsBlob::decode(&s),
        }
    }

    fn save(&self, blob: &RecentRecipientsBlob) -> Result<(), AddressBookError> {
        self.kv
            .set_item(RECENT_RECIPIENTS_LOCAL_STORAGE_KEY, &blob.encode()?)
            .map_err(AddressBookError::from)
    }

    pub fn list(&self) -> Result<Vec<Contact>, AddressBookError> {
        Ok(self.load()?.recipients)
    }

    pub fn remember(&self, incoming: &[Contact]) -> Result<(), AddressBookError> {
        if incoming.is_empty() {
            return Ok(());
        }
        let mut blob = self.load()?;
        blob.remember(incoming);
        self.save(&blob)
    }
}

/// Load persisted recents. Empty when storage is missing or unreadable.
pub fn load_recent_recipients() -> Vec<Contact> {
    RecentRecipientStore::<WebLocalStorage>::open()
        .and_then(|store| store.list())
        .unwrap_or_default()
}

/// Persist To/Cc/Bcc (and similar) after the user sends. No-op on storage failure.
pub fn remember_recipients<'a>(addrs: impl IntoIterator<Item = &'a ComposerAddress>) {
    let incoming: Vec<Contact> = addrs
        .into_iter()
        .filter_map(|addr| contact_from_address(addr.name.as_deref().unwrap_or(""), &addr.email))
        .collect();
    if incoming.is_empty() {
        return;
    }
    let _ =
        RecentRecipientStore::<WebLocalStorage>::open().and_then(|store| store.remember(&incoming));
}

/// Default cap used by the compose chip field.
pub fn default_suggestion_limit() -> usize {
    DEFAULT_SUGGESTION_LIMIT
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailiner_core::{EmailAddr, EmailAddress};

    fn sample(name: &str, email: &str) -> Contact {
        parse_contact(name, email).expect("valid sample")
    }

    fn env_with(from: Option<EmailAddress>, to: Option<EmailAddress>) -> Envelope {
        let folder = mailiner_core::FolderId::new("INBOX");
        Envelope {
            id: mailiner_core::MessageId::new(folder.clone(), "1"),
            account_id: mailiner_core::ids::AccountId::new("acc"),
            folder_id: folder,
            subject: None,
            from,
            to,
            cc: None,
            bcc: None,
            reply_to: None,
            rfc_message_id: None,
            in_reply_to: None,
            references: Vec::new(),
            date: chrono::Utc::now(),
            is_read: true,
            is_answered: false,
            is_starred: false,
            is_flagged: false,
            is_draft: false,
            is_deleted: false,
            keywords: Vec::new(),
            has_attachments: false,
            snippet: None,
            size: None,
            auth_results: Default::default(),
        }
    }

    fn list(name: &str, email: &str) -> EmailAddress {
        EmailAddress::List(vec![EmailAddr {
            name: if name.is_empty() {
                None
            } else {
                Some(name.into())
            },
            email: Some(email.into()),
        }])
    }

    #[test]
    fn storage_key_is_versioned() {
        assert_eq!(
            RECENT_RECIPIENTS_LOCAL_STORAGE_KEY,
            "mailiner.recent-recipients.v1"
        );
        assert_eq!(RECENT_RECIPIENTS_SCHEMA_VERSION, 1);
        assert!(RECENT_RECIPIENTS_LOCAL_STORAGE_KEY.starts_with("mailiner."));
    }

    #[test]
    fn suggest_prefers_contacts_then_recents() {
        let contacts = vec![sample("Ada Lovelace", "ada@example.com")];
        let recents = vec![
            sample("Ada Copy", "ADA@example.com"),
            sample("Bob", "bob@example.com"),
            sample("Carol", "ada.help@team.example.com"),
        ];
        let hits = suggest_recipients(&contacts, &recents, "ada", &[], 8);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].contact.email, "ada@example.com");
        assert_eq!(hits[0].source, SuggestionSource::Contact);
        assert_eq!(hits[0].contact.name, "Ada Lovelace");
        assert_eq!(hits[1].contact.email, "ada.help@team.example.com");
        assert_eq!(hits[1].source, SuggestionSource::Recent);

        let excluded = vec![ComposerAddress::email_only("ada@example.com")];
        let rest = suggest_recipients(&contacts, &recents, "ada", &excluded, 8);
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].contact.email, "ada.help@team.example.com");

        assert!(suggest_recipients(&contacts, &recents, "  ", &[], 8).is_empty());
        assert_eq!(
            suggest_recipients(&contacts, &recents, "b", &[], 1)[0]
                .contact
                .email,
            "bob@example.com"
        );
    }

    #[test]
    fn suggest_ranks_email_prefix_over_name_substring() {
        let contacts = vec![sample("Help Desk", "support@example.com")];
        let recents = vec![sample("", "help@example.com")];
        let hits = suggest_recipients(&contacts, &recents, "help", &[], 8);
        assert_eq!(hits[0].contact.email, "help@example.com");
        assert_eq!(hits[0].source, SuggestionSource::Recent);
        assert_eq!(hits[1].contact.email, "support@example.com");
    }

    #[test]
    fn typed_complete_email_is_not_replaced() {
        assert!(typed_overrides_suggestion(
            "ada@other.com",
            "ada@example.com"
        ));
        assert!(!typed_overrides_suggestion("ada", "ada@example.com"));
        assert!(!typed_overrides_suggestion(
            "Ada Lovelace <ada@example.com>",
            "ada@example.com"
        ));
        assert!(!typed_overrides_suggestion("   ", "ada@example.com"));
    }

    #[test]
    fn harvest_from_to_skips_self_and_invalid() {
        let envelopes = [
            env_with(
                Some(list("Ada", "ada@example.com")),
                Some(list("", "me@example.com")),
            ),
            env_with(
                Some(list("Bad", "not-an-email")),
                Some(list("Bob", "bob@ex.com")),
            ),
            env_with(Some(list("Ada Dup", "ADA@example.com")), None),
        ];
        let hits = contacts_from_envelopes(&envelopes, &["me@example.com"]);
        let emails: Vec<_> = hits.iter().map(|c| c.email.as_str()).collect();
        assert_eq!(emails, ["ada@example.com", "bob@ex.com"]);
        assert_eq!(hits[0].name, "Ada");
    }

    #[test]
    fn remember_moves_to_front_and_keeps_name() {
        let mut blob = RecentRecipientsBlob::empty();
        blob.remember(&[
            sample("Ada", "ada@example.com"),
            sample("Bob", "bob@ex.com"),
        ]);
        assert_eq!(blob.recipients[0].email, "ada@example.com");
        assert_eq!(blob.recipients[1].email, "bob@ex.com");

        blob.remember(&[sample("", "ADA@example.com")]);
        assert_eq!(blob.recipients[0].email, "ADA@example.com");
        assert_eq!(blob.recipients[0].name, "Ada");
        assert_eq!(blob.recipients.len(), 2);
    }

    #[test]
    fn remember_truncates_and_store_roundtrips() {
        let mut blob = RecentRecipientsBlob::empty();
        let many: Vec<_> = (0..MAX_RECENT_RECIPIENTS + 3)
            .map(|i| sample("", &format!("u{i}@example.com")))
            .collect();
        blob.remember(&many);
        assert_eq!(blob.recipients.len(), MAX_RECENT_RECIPIENTS);
        assert_eq!(blob.recipients[0].email, "u0@example.com");

        let store = RecentRecipientStore::open_memory();
        store.remember(&[sample("Ada", "ada@example.com")]).unwrap();
        store.remember(&[sample("Bob", "bob@ex.com")]).unwrap();
        let listed = store.list().unwrap();
        assert_eq!(listed[0].email, "bob@ex.com");
        assert_eq!(listed[1].name, "Ada");

        let json = store.load().unwrap().encode().unwrap();
        assert!(json.contains("\"schema_version\":1"), "json={json}");
        let err =
            RecentRecipientsBlob::decode(r#"{"schema_version":99,"recipients":[]}"#).unwrap_err();
        match err {
            AddressBookError::Serialization(msg) => {
                assert!(
                    msg.contains("unsupported") && msg.contains("99"),
                    "msg={msg}"
                );
            }
            other => panic!("expected Serialization, got {other:?}"),
        }
    }

    #[test]
    fn merge_recent_candidates_dedupes() {
        let persisted = vec![sample("Ada", "ada@example.com")];
        let harvested = vec![
            sample("Ada Copy", "ADA@example.com"),
            sample("Bob", "bob@ex.com"),
        ];
        let merged = merge_recent_candidates(&persisted, &harvested);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].name, "Ada");
        assert_eq!(merged[1].email, "bob@ex.com");
    }

    #[test]
    fn suggestion_formats_chip() {
        let named = RecipientSuggestion {
            contact: sample("Ada", "ada@example.com"),
            source: SuggestionSource::Contact,
        };
        assert_eq!(named.display_label(), "Ada");
        assert_eq!(named.formatted(), "Ada <ada@example.com>");
        assert_eq!(named.to_composer_address().name.as_deref(), Some("Ada"));
        assert_eq!(named.source_label(), "Contact");

        let comma = RecipientSuggestion {
            contact: sample("Smith, Alice", "alice@example.com"),
            source: SuggestionSource::Contact,
        };
        assert_eq!(comma.formatted(), r#""Smith, Alice" <alice@example.com>"#);
        assert_eq!(
            comma.to_composer_address().name.as_deref(),
            Some("Smith, Alice")
        );
        assert_eq!(comma.to_composer_address().email, "alice@example.com");

        let bare = RecipientSuggestion {
            contact: sample("", "bob@ex.com"),
            source: SuggestionSource::Recent,
        };
        assert_eq!(bare.display_label(), "bob@ex.com");
        assert_eq!(bare.formatted(), "bob@ex.com");
        assert_eq!(bare.source_label(), "Recent");
    }

    #[test]
    fn long_display_name_falls_back_to_email_only() {
        let long = "n".repeat(crate::address_book::MAX_CONTACT_NAME_CHARS + 1);
        let kept = contact_from_address(&long, "ada@example.com").expect("email kept");
        assert_eq!(kept.name, "");
        assert_eq!(kept.email, "ada@example.com");
        assert!(contact_from_address("Ada", "not-an-email").is_none());

        let envelopes = [env_with(Some(list(&long, "ada@example.com")), None)];
        let hits = contacts_from_envelopes(&envelopes, &[]);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].email, "ada@example.com");
        assert_eq!(hits[0].name, "");
    }
}
