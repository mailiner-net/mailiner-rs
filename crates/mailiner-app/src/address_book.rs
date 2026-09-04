//! Local address book (`mailiner.addressbook.v1`).
//!
//! Simple name+email contacts in origin storage. Settings adds and removes
//! rows; [`suggest_contacts`] is the lookup hook for recipient autocomplete (#89).

use mailiner_composer::{ComposerAddress, emails_equal, is_valid_email_v1};
use serde::{Deserialize, Serialize};

use crate::account_store::{AccountStoreError, MemoryKvStore, StringKvStore, WebLocalStorage};

/// `localStorage` key for the address-book blob (account schema is untouched).
pub const ADDRESS_BOOK_LOCAL_STORAGE_KEY: &str = "mailiner.addressbook.v1";
/// Address-book blob schema (independent of the account store).
pub const ADDRESS_BOOK_SCHEMA_VERSION: u32 = 1;
/// Cap on stored contacts. Each row is tiny; refuse rather than grow without bound.
pub const MAX_CONTACTS: usize = 500;
/// Display-name character cap (Unicode scalar values).
pub const MAX_CONTACT_NAME_CHARS: usize = 200;
/// Mailbox character cap (RFC 5321 path limit).
pub const MAX_CONTACT_EMAIL_CHARS: usize = 254;
/// Default suggestion cap for [`suggest_contacts`].
pub const DEFAULT_SUGGESTION_LIMIT: usize = 8;

/// One local contact. Email is the identity (ASCII case-insensitive).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Contact {
    /// Display name. Empty when the contact is email-only.
    #[serde(default)]
    pub name: String,
    /// Mailbox address as entered (trimmed, not lowercased).
    pub email: String,
}

impl Contact {
    /// Visible name, or the mailbox when no name was stored.
    pub fn display_label(&self) -> &str {
        let name = self.name.trim();
        if name.is_empty() {
            self.email.as_str()
        } else {
            name
        }
    }

    /// `Name <email>` when a name exists; otherwise the mailbox.
    ///
    /// Names that contain `,` / quotes / angle brackets are quoted so
    /// [`mailiner_composer::shell::recipient_field::parse_recipient`] keeps one token.
    pub fn formatted(&self) -> String {
        format_named_mailbox(&self.name, &self.email)
    }

    /// Composer chip for autocomplete / prefill (#89).
    pub fn to_composer_address(&self) -> ComposerAddress {
        let name = self.name.trim();
        ComposerAddress {
            name: if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            },
            email: self.email.clone(),
        }
    }
}

/// Persistence / validation failure for the address book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressBookError {
    /// `localStorage` blocked or unavailable.
    Unavailable,
    /// JSON or schema failure.
    Serialization(String),
    /// Email field was empty after trim.
    EmptyEmail,
    /// Email failed [`is_valid_email_v1`].
    InvalidEmail,
    /// Same mailbox already exists (ASCII case-insensitive).
    Duplicate,
    /// Display name longer than [`MAX_CONTACT_NAME_CHARS`].
    NameTooLong,
    /// Email longer than [`MAX_CONTACT_EMAIL_CHARS`].
    EmailTooLong,
    /// [`MAX_CONTACTS`] already stored.
    Full,
}

impl std::fmt::Display for AddressBookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => write!(f, "Contact storage is unavailable in this browser."),
            Self::Serialization(msg) => write!(f, "Could not read the address book: {msg}"),
            Self::EmptyEmail => write!(f, "Enter an email address."),
            Self::InvalidEmail => write!(f, "That email address does not look valid."),
            Self::Duplicate => write!(f, "That email is already in the address book."),
            Self::NameTooLong => write!(
                f,
                "Name is too long (max {MAX_CONTACT_NAME_CHARS} characters)."
            ),
            Self::EmailTooLong => write!(
                f,
                "Email is too long (max {MAX_CONTACT_EMAIL_CHARS} characters)."
            ),
            Self::Full => write!(
                f,
                "The address book is full ({MAX_CONTACTS} contacts). Remove one and try again."
            ),
        }
    }
}

impl std::error::Error for AddressBookError {}

impl From<AccountStoreError> for AddressBookError {
    fn from(e: AccountStoreError) -> Self {
        match e {
            AccountStoreError::Unavailable => Self::Unavailable,
            AccountStoreError::Serialization(msg) | AccountStoreError::Other(msg) => {
                Self::Serialization(msg)
            }
        }
    }
}

/// Format `Name <email>`, quoting the name when it would break comma-splitting.
pub fn format_named_mailbox(name: &str, email: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        return email.to_string();
    }
    if name.contains([',', '<', '>', '"', '\\']) || name.contains('@') {
        let quoted = format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""));
        format!("{quoted} <{email}>")
    } else {
        format!("{name} <{email}>")
    }
}

/// Trim and validate a name+email pair. Does not touch storage.
pub fn parse_contact(name: &str, email: &str) -> Result<Contact, AddressBookError> {
    let name = name.trim();
    let email = email.trim();
    if name.chars().count() > MAX_CONTACT_NAME_CHARS {
        return Err(AddressBookError::NameTooLong);
    }
    if email.is_empty() {
        return Err(AddressBookError::EmptyEmail);
    }
    if email.chars().count() > MAX_CONTACT_EMAIL_CHARS {
        return Err(AddressBookError::EmailTooLong);
    }
    if !is_valid_email_v1(email) {
        return Err(AddressBookError::InvalidEmail);
    }
    Ok(Contact {
        name: name.to_string(),
        email: email.to_string(),
    })
}

/// Single JSON document stored under [`ADDRESS_BOOK_LOCAL_STORAGE_KEY`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AddressBookBlob {
    pub schema_version: u32,
    #[serde(default)]
    pub contacts: Vec<Contact>,
}

impl AddressBookBlob {
    pub fn empty() -> Self {
        Self {
            schema_version: ADDRESS_BOOK_SCHEMA_VERSION,
            contacts: Vec::new(),
        }
    }

    pub fn encode(&self) -> Result<String, AddressBookError> {
        serde_json::to_string(self).map_err(|e| AddressBookError::Serialization(e.to_string()))
    }

    /// Rejects blobs whose `schema_version` is greater than
    /// [`ADDRESS_BOOK_SCHEMA_VERSION`] so a future format is not rewritten as v1.
    ///
    /// The envelope is decoded first; each contact row is parsed on its own so
    /// one malformed row cannot hide the rest of the book.
    pub fn decode(json: &str) -> Result<Self, AddressBookError> {
        let raw: RawAddressBookBlob = serde_json::from_str(json)
            .map_err(|e| AddressBookError::Serialization(e.to_string()))?;
        if raw.schema_version > ADDRESS_BOOK_SCHEMA_VERSION {
            return Err(AddressBookError::Serialization(format!(
                "unsupported address book schema_version {} (max supported {})",
                raw.schema_version, ADDRESS_BOOK_SCHEMA_VERSION
            )));
        }
        let mut contacts = match raw.contacts {
            serde_json::Value::Null => Vec::new(),
            serde_json::Value::Array(rows) => rows
                .into_iter()
                .filter_map(|row| serde_json::from_value::<Contact>(row).ok())
                .collect(),
            _ => Vec::new(),
        };
        sanitize_contacts(&mut contacts);
        Ok(Self {
            schema_version: ADDRESS_BOOK_SCHEMA_VERSION,
            contacts,
        })
    }

    pub fn find_email(&self, email: &str) -> Option<&Contact> {
        self.contacts.iter().find(|c| emails_equal(&c.email, email))
    }

    /// Insert a validated contact. Rejects a duplicate mailbox.
    pub fn add(&mut self, contact: Contact) -> Result<(), AddressBookError> {
        if self.find_email(&contact.email).is_some() {
            return Err(AddressBookError::Duplicate);
        }
        if self.contacts.len() >= MAX_CONTACTS {
            return Err(AddressBookError::Full);
        }
        self.contacts.push(contact);
        sort_contacts(&mut self.contacts);
        self.schema_version = ADDRESS_BOOK_SCHEMA_VERSION;
        Ok(())
    }

    /// Remove by mailbox (ASCII case-insensitive). Returns whether a row was dropped.
    pub fn remove_email(&mut self, email: &str) -> bool {
        let before = self.contacts.len();
        self.contacts.retain(|c| !emails_equal(&c.email, email));
        if self.contacts.len() != before {
            self.schema_version = ADDRESS_BOOK_SCHEMA_VERSION;
            true
        } else {
            false
        }
    }
}

/// Envelope only. Contact rows are parsed one-by-one in [`AddressBookBlob::decode`].
#[derive(Debug, Deserialize)]
struct RawAddressBookBlob {
    schema_version: u32,
    #[serde(default)]
    contacts: serde_json::Value,
}

fn sanitize_contacts(contacts: &mut Vec<Contact>) {
    let mut kept = Vec::with_capacity(contacts.len());
    for raw in contacts.drain(..) {
        let Ok(contact) = parse_contact(&raw.name, &raw.email) else {
            continue;
        };
        if kept
            .iter()
            .any(|c: &Contact| emails_equal(&c.email, &contact.email))
        {
            continue;
        }
        if kept.len() >= MAX_CONTACTS {
            break;
        }
        kept.push(contact);
    }
    sort_contacts(&mut kept);
    *contacts = kept;
}

fn sort_contacts(contacts: &mut [Contact]) {
    contacts.sort_by(|a, b| {
        a.display_label()
            .to_ascii_lowercase()
            .cmp(&b.display_label().to_ascii_lowercase())
            .then_with(|| {
                a.email
                    .to_ascii_lowercase()
                    .cmp(&b.email.to_ascii_lowercase())
            })
    });
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SuggestRank {
    EmailPrefix = 0,
    NamePrefix = 1,
    EmailContains = 2,
    NameContains = 3,
}

pub(crate) fn address_suggest_rank(name: &str, email: &str, needle: &str) -> Option<SuggestRank> {
    let email = email.to_ascii_lowercase();
    if email.starts_with(needle) {
        return Some(SuggestRank::EmailPrefix);
    }
    let name = name.to_ascii_lowercase();
    if !name.is_empty() && name.starts_with(needle) {
        return Some(SuggestRank::NamePrefix);
    }
    if email.contains(needle) {
        return Some(SuggestRank::EmailContains);
    }
    if !name.is_empty() && name.contains(needle) {
        return Some(SuggestRank::NameContains);
    }
    None
}

fn suggest_rank(contact: &Contact, needle: &str) -> Option<SuggestRank> {
    address_suggest_rank(&contact.name, &contact.email, needle)
}

/// Prefix/substring matches for recipient autocomplete (#89).
///
/// Empty query returns no suggestions. Ranking prefers email prefix, then name
/// prefix, then substring matches. Results stay unique and are capped at `limit`.
pub fn suggest_contacts<'a>(
    contacts: &'a [Contact],
    query: &str,
    limit: usize,
) -> Vec<&'a Contact> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() || limit == 0 {
        return Vec::new();
    }
    let mut seen: Vec<&str> = Vec::new();
    let mut ranked: Vec<(SuggestRank, &Contact)> = Vec::new();
    for contact in contacts {
        if seen.iter().any(|email| emails_equal(email, &contact.email)) {
            continue;
        }
        let Some(rank) = suggest_rank(contact, &needle) else {
            continue;
        };
        seen.push(contact.email.as_str());
        ranked.push((rank, contact));
    }
    ranked.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| {
                a.1.name
                    .to_ascii_lowercase()
                    .cmp(&b.1.name.to_ascii_lowercase())
            })
            .then_with(|| {
                a.1.email
                    .to_ascii_lowercase()
                    .cmp(&b.1.email.to_ascii_lowercase())
            })
    });
    ranked.into_iter().take(limit).map(|(_, c)| c).collect()
}

/// Address book over a string key-value store (`localStorage` in the browser).
pub struct AddressBookStore<K: StringKvStore> {
    kv: K,
}

impl AddressBookStore<WebLocalStorage> {
    /// Open `window.localStorage`, or [`AddressBookError::Unavailable`].
    pub fn open() -> Result<Self, AddressBookError> {
        Ok(Self {
            kv: WebLocalStorage::try_open()?,
        })
    }
}

impl AddressBookStore<MemoryKvStore> {
    pub fn open_memory() -> Self {
        Self {
            kv: MemoryKvStore::new(),
        }
    }
}

impl<K: StringKvStore> AddressBookStore<K> {
    pub fn with_kv(kv: K) -> Self {
        Self { kv }
    }

    fn load(&self) -> Result<AddressBookBlob, AddressBookError> {
        match self.kv.get_item(ADDRESS_BOOK_LOCAL_STORAGE_KEY)? {
            None => Ok(AddressBookBlob::empty()),
            Some(s) if s.trim().is_empty() => Ok(AddressBookBlob::empty()),
            Some(s) => AddressBookBlob::decode(&s),
        }
    }

    fn save(&self, blob: &AddressBookBlob) -> Result<(), AddressBookError> {
        self.kv
            .set_item(ADDRESS_BOOK_LOCAL_STORAGE_KEY, &blob.encode()?)
            .map_err(AddressBookError::from)
    }

    pub fn list(&self) -> Result<Vec<Contact>, AddressBookError> {
        Ok(self.load()?.contacts)
    }

    pub fn add(&self, name: &str, email: &str) -> Result<Contact, AddressBookError> {
        let contact = parse_contact(name, email)?;
        let mut blob = self.load()?;
        blob.add(contact.clone())?;
        self.save(&blob)?;
        Ok(contact)
    }

    pub fn remove(&self, email: &str) -> Result<bool, AddressBookError> {
        let mut blob = self.load()?;
        let removed = blob.remove_email(email);
        if removed {
            self.save(&blob)?;
        }
        Ok(removed)
    }

    /// Stored contacts matching `query`, newest ranking from [`suggest_contacts`].
    pub fn suggest(&self, query: &str, limit: usize) -> Result<Vec<Contact>, AddressBookError> {
        let contacts = self.list()?;
        Ok(suggest_contacts(&contacts, query, limit)
            .into_iter()
            .cloned()
            .collect())
    }
}

/// Load contacts from origin storage. Empty when storage is missing or unreadable.
pub fn load_contacts() -> Vec<Contact> {
    try_load_contacts().unwrap_or_default()
}

/// Load contacts, surfacing storage errors (settings can show a banner).
pub fn try_load_contacts() -> Result<Vec<Contact>, AddressBookError> {
    AddressBookStore::<WebLocalStorage>::open()?.list()
}

/// Persist a new contact. See [`AddressBookError`] for validation failures.
pub fn add_contact(name: &str, email: &str) -> Result<Contact, AddressBookError> {
    AddressBookStore::<WebLocalStorage>::open()?.add(name, email)
}

/// Remove by mailbox. `Ok(false)` when the email was not stored.
pub fn remove_contact(email: &str) -> Result<bool, AddressBookError> {
    AddressBookStore::<WebLocalStorage>::open()?.remove(email)
}

/// Autocomplete hook over origin storage. Empty on storage failure.
pub fn load_contact_suggestions(query: &str, limit: usize) -> Vec<Contact> {
    AddressBookStore::<WebLocalStorage>::open()
        .and_then(|store| store.suggest(query, limit))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str, email: &str) -> Contact {
        parse_contact(name, email).expect("valid sample")
    }

    #[test]
    fn storage_key_is_versioned() {
        assert_eq!(ADDRESS_BOOK_LOCAL_STORAGE_KEY, "mailiner.addressbook.v1");
        assert_eq!(ADDRESS_BOOK_SCHEMA_VERSION, 1);
        assert!(ADDRESS_BOOK_LOCAL_STORAGE_KEY.starts_with("mailiner."));
    }

    #[test]
    fn parse_trims_and_allows_empty_name() {
        let c = parse_contact("  Ada Lovelace  ", "  ada@example.com ").unwrap();
        assert_eq!(c.name, "Ada Lovelace");
        assert_eq!(c.email, "ada@example.com");
        assert_eq!(c.display_label(), "Ada Lovelace");
        assert_eq!(c.formatted(), "Ada Lovelace <ada@example.com>");

        let comma = parse_contact("Smith, Alice", "alice@example.com").unwrap();
        assert_eq!(comma.formatted(), r#""Smith, Alice" <alice@example.com>"#);

        let bare = parse_contact("   ", "solo@example.com").unwrap();
        assert_eq!(bare.name, "");
        assert_eq!(bare.display_label(), "solo@example.com");
        assert_eq!(bare.formatted(), "solo@example.com");
        assert_eq!(bare.to_composer_address().name, None);
        assert_eq!(bare.to_composer_address().email, "solo@example.com");
    }

    #[test]
    fn parse_rejects_bad_email_and_oversize() {
        assert_eq!(
            parse_contact("Ada", "").unwrap_err(),
            AddressBookError::EmptyEmail
        );
        assert_eq!(
            parse_contact("Ada", "   ").unwrap_err(),
            AddressBookError::EmptyEmail
        );
        assert_eq!(
            parse_contact("Ada", "not-an-email").unwrap_err(),
            AddressBookError::InvalidEmail
        );
        assert_eq!(
            parse_contact("Ada", "Ada <ada@example.com>").unwrap_err(),
            AddressBookError::InvalidEmail
        );
        let long_name = "n".repeat(MAX_CONTACT_NAME_CHARS + 1);
        assert_eq!(
            parse_contact(&long_name, "a@b.co").unwrap_err(),
            AddressBookError::NameTooLong
        );
        let long_email = format!("{}@example.com", "a".repeat(MAX_CONTACT_EMAIL_CHARS));
        assert_eq!(
            parse_contact("Ada", &long_email).unwrap_err(),
            AddressBookError::EmailTooLong
        );
    }

    #[test]
    fn blob_encode_decode_roundtrip() {
        let mut blob = AddressBookBlob::empty();
        blob.add(sample("Ada", "ada@example.com")).unwrap();
        blob.add(sample("", "bob@example.com")).unwrap();

        let json = blob.encode().expect("encode");
        assert!(json.contains("\"schema_version\":1"), "json={json}");
        assert!(json.contains("ada@example.com"), "json={json}");

        let back = AddressBookBlob::decode(&json).expect("decode");
        assert_eq!(back.contacts.len(), 2);
        assert_eq!(back.contacts[0].email, "ada@example.com");
        assert_eq!(back.contacts[1].email, "bob@example.com");
    }

    #[test]
    fn blob_decode_rejects_future_schema() {
        let json = r#"{"schema_version":99,"contacts":[]}"#;
        let err = AddressBookBlob::decode(json).unwrap_err();
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
    fn blob_decode_drops_invalid_and_duplicate_rows() {
        let json = r#"{
            "schema_version":1,
            "contacts":[
                {"name":"Ada","email":"ada@example.com"},
                {"name":"Dup","email":"ADA@example.com"},
                {"name":"Bad","email":"nope"},
                {"name":"Missing email"},
                {"email":1},
                "not-an-object",
                {"name":"Bob","email":"bob@example.com"}
            ]
        }"#;
        let blob = AddressBookBlob::decode(json).unwrap();
        let emails: Vec<_> = blob.contacts.iter().map(|c| c.email.as_str()).collect();
        assert_eq!(emails, ["ada@example.com", "bob@example.com"]);
    }

    #[test]
    fn blob_decode_keeps_valid_rows_when_contacts_is_not_an_array() {
        let blob = AddressBookBlob::decode(r#"{"schema_version":1,"contacts":{}}"#).unwrap();
        assert!(blob.contacts.is_empty());
    }

    #[test]
    fn blob_decode_empty_or_missing_contacts() {
        let empty = AddressBookBlob::decode(r#"{"schema_version":1}"#).unwrap();
        assert!(empty.contacts.is_empty());
        let blank = AddressBookStore::open_memory().list().unwrap();
        assert!(blank.is_empty());
    }

    #[test]
    fn add_sorts_and_rejects_duplicate_and_full() {
        let mut blob = AddressBookBlob::empty();
        blob.add(sample("Zoe", "zoe@example.com")).unwrap();
        blob.add(sample("Ada", "ada@example.com")).unwrap();
        assert_eq!(blob.contacts[0].email, "ada@example.com");
        assert_eq!(blob.contacts[1].email, "zoe@example.com");
        assert_eq!(
            blob.add(sample("Ada Lovelace", "ADA@example.com"))
                .unwrap_err(),
            AddressBookError::Duplicate
        );

        let mut full = AddressBookBlob::empty();
        for i in 0..MAX_CONTACTS {
            full.add(sample("", &format!("u{i}@example.com"))).unwrap();
        }
        assert_eq!(
            full.add(sample("", "overflow@example.com")).unwrap_err(),
            AddressBookError::Full
        );
    }

    #[test]
    fn remove_is_case_insensitive() {
        let mut blob = AddressBookBlob::empty();
        blob.add(sample("Ada", "ada@example.com")).unwrap();
        assert!(blob.remove_email("ADA@example.com"));
        assert!(blob.contacts.is_empty());
        assert!(!blob.remove_email("ada@example.com"));
    }

    #[test]
    fn store_roundtrip_add_list_remove() {
        let store = AddressBookStore::open_memory();
        store.add("  Bob  ", " bob@example.com ").unwrap();
        store.add("Ada", "ada@example.com").unwrap();
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].name, "Ada");
        assert_eq!(listed[1].email, "bob@example.com");

        assert!(store.remove("BOB@example.com").unwrap());
        assert_eq!(store.list().unwrap().len(), 1);
        assert!(!store.remove("missing@example.com").unwrap());
        assert_eq!(
            store.add("Ada 2", "ada@example.com").unwrap_err(),
            AddressBookError::Duplicate
        );
    }

    #[test]
    fn suggest_ranks_prefix_over_substring() {
        let contacts = vec![
            sample("Ada Lovelace", "ada@example.com"),
            sample("Bob", "robert@example.com"),
            sample("Carol", "ada.help@team.example.com"),
            sample("Dan", "dan@elsewhere.test"),
        ];
        let hits = suggest_contacts(&contacts, "ada", 8);
        let emails: Vec<_> = hits.iter().map(|c| c.email.as_str()).collect();
        assert_eq!(emails, ["ada@example.com", "ada.help@team.example.com"]);

        let by_name = suggest_contacts(&contacts, "lov", 8);
        assert_eq!(by_name.len(), 1);
        assert_eq!(by_name[0].email, "ada@example.com");

        assert!(suggest_contacts(&contacts, "  ", 8).is_empty());
        assert!(suggest_contacts(&contacts, "ada", 0).is_empty());
        assert_eq!(suggest_contacts(&contacts, "a", 1).len(), 1);

        let dups = vec![
            sample("Ada", "ada@example.com"),
            sample("Ada Copy", "ADA@example.com"),
        ];
        let unique = suggest_contacts(&dups, "ada", 8);
        assert_eq!(unique.len(), 1);
        assert_eq!(unique[0].email, "ada@example.com");
    }

    #[test]
    fn store_suggest_is_the_autocomplete_hook() {
        let store = AddressBookStore::open_memory();
        store.add("Ada", "ada@example.com").unwrap();
        store.add("Bob", "bob@example.com").unwrap();
        let hits = store.suggest("bo", DEFAULT_SUGGESTION_LIMIT).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].to_composer_address().email, "bob@example.com");
        assert_eq!(hits[0].to_composer_address().name.as_deref(), Some("Bob"));
    }
}
