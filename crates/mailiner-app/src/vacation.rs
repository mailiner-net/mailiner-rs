//! Local vacation / out-of-office auto-reply (`mailiner.ui.vacation.v1`).
//!
//! Mailiner does not speak ManageSieve. Settings live in this browser and run
//! when envelopes are fetched or refreshed (folder open / IDLE / NOOP). The
//! client sends replies over SMTP — this is not a server-side Sieve script.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, NaiveDateTime, Utc};
use mailiner_composer::{
    BodyMode, ComposerAddress, DraftDocument, FromIdentity, flatten_addresses,
};
use mailiner_core::ids::AccountId;
use mailiner_core::{EmailAddress, Envelope};
use serde::{Deserialize, Serialize};

#[cfg(target_arch = "wasm32")]
use crate::account_store::WebLocalStorage;
use crate::account_store::{AccountStoreError, StringKvStore};

/// `localStorage` key for per-account vacation settings.
pub const VACATION_KEY: &str = "mailiner.ui.vacation.v1";
/// Schema version for [`VacationBlob`].
pub const VACATION_SCHEMA_VERSION: u32 = 1;

/// `localStorage` key for senders already auto-replied to in a vacation period.
pub const VACATION_REPLIED_KEY: &str = "mailiner.ui.vacationReplied.v1";
/// Schema version for [`VacationRepliedBlob`].
pub const VACATION_REPLIED_SCHEMA_VERSION: u32 = 1;
/// Cap on remembered senders per account + period (evict oldest).
pub const MAX_REPLIED_PER_PERIOD: usize = 5_000;

/// Default Subject when the user has not set one.
pub const DEFAULT_VACATION_SUBJECT: &str = "Out of office";
/// Default body when the user has not set one.
pub const DEFAULT_VACATION_BODY: &str = "I am currently away and will reply when I return.";

/// Why a message is not auto-replied to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VacationSkip {
    /// Vacation is off.
    Disabled,
    /// `now` is outside the optional start/end window.
    OutsideWindow,
    /// Envelope Date is before the vacation was armed / started.
    BeforeCutoff,
    /// No usable From / Reply-To address.
    NoSender,
    /// From is one of our identities.
    OwnAddress,
    /// Sender looks like a no-reply or mailer-daemon.
    NoReply,
    /// Already sent a vacation reply to this address in this period.
    AlreadyReplied,
}

/// Per-account vacation / out-of-office settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VacationSettings {
    #[serde(default)]
    pub enabled: bool,
    /// Inclusive start. `None` = no lower bound (see [`Self::armed_at`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<DateTime<Utc>>,
    /// Inclusive end. `None` = no upper bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<DateTime<Utc>>,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub body: String,
    /// When vacation was last turned on. Existing mail older than this is skipped
    /// so opening the inbox does not blast every first-page sender.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub armed_at: Option<DateTime<Utc>>,
}

impl VacationSettings {
    pub fn new() -> Self {
        Self {
            enabled: false,
            start: None,
            end: None,
            subject: DEFAULT_VACATION_SUBJECT.into(),
            body: DEFAULT_VACATION_BODY.into(),
            armed_at: None,
        }
    }

    pub fn trimmed(mut self) -> Self {
        self.subject = self.subject.trim().to_string();
        self.body = self.body.trim().to_string();
        self
    }

    /// Stable key for the current start/end window (replied-set identity).
    pub fn period_key(&self) -> String {
        format!(
            "{}|{}",
            self.start.map(|t| t.to_rfc3339()).unwrap_or_default(),
            self.end.map(|t| t.to_rfc3339()).unwrap_or_default()
        )
    }

    /// Enabled and `now` is inside the optional window.
    pub fn is_active(&self, now: DateTime<Utc>) -> bool {
        if !self.enabled {
            return false;
        }
        if let (Some(start), Some(end)) = (self.start, self.end)
            && start > end
        {
            return false;
        }
        if let Some(start) = self.start
            && now < start
        {
            return false;
        }
        if let Some(end) = self.end
            && now > end
        {
            return false;
        }
        true
    }

    /// Do not auto-reply to envelopes dated before this instant.
    pub fn effective_cutoff(&self) -> Option<DateTime<Utc>> {
        match (self.armed_at, self.start) {
            (Some(a), Some(s)) => Some(a.max(s)),
            (Some(a), None) => Some(a),
            (None, Some(s)) => Some(s),
            (None, None) => None,
        }
    }
}

impl Default for VacationSettings {
    fn default() -> Self {
        Self::new()
    }
}

/// Planned auto-reply for one envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VacationHit {
    pub uid: String,
    /// Address to send the reply to (Reply-To else From), normalized for display.
    pub sender: String,
}

/// First matching envelopes that should receive a vacation reply.
///
/// Reply-once-per-sender: the first hit for an address wins; later envelopes
/// from the same sender in this batch are skipped.
pub fn plan_vacation_hits(
    settings: &VacationSettings,
    now: DateTime<Utc>,
    envelopes: &[Envelope],
    own_addresses: &[String],
    replied: &HashSet<String>,
) -> Vec<VacationHit> {
    if !settings.is_active(now) {
        return Vec::new();
    }
    let mut seen = replied.clone();
    let mut hits = Vec::new();
    for env in envelopes {
        if let Ok(sender) = should_reply(settings, now, env, own_addresses, &seen) {
            seen.insert(normalize_email(&sender));
            hits.push(VacationHit {
                uid: env.id.as_uid().to_string(),
                sender,
            });
        }
    }
    hits
}

/// Decide whether `envelope` should get a vacation reply.
///
/// `Ok(sender)` is the Reply-To / From address to write to.
pub fn should_reply(
    settings: &VacationSettings,
    now: DateTime<Utc>,
    envelope: &Envelope,
    own_addresses: &[String],
    replied: &HashSet<String>,
) -> Result<String, VacationSkip> {
    if !settings.enabled {
        return Err(VacationSkip::Disabled);
    }
    if !settings.is_active(now) {
        return Err(VacationSkip::OutsideWindow);
    }
    if let Some(cutoff) = settings.effective_cutoff()
        && envelope.date < cutoff
    {
        return Err(VacationSkip::BeforeCutoff);
    }
    if let Some(from) = from_email(envelope)
        && is_own_address(&from, own_addresses)
    {
        return Err(VacationSkip::OwnAddress);
    }
    let sender = reply_target(envelope).ok_or(VacationSkip::NoSender)?;
    if is_own_address(&sender, own_addresses) {
        return Err(VacationSkip::OwnAddress);
    }
    if is_noreply_address(&sender) {
        return Err(VacationSkip::NoReply);
    }
    if already_replied(replied, &sender) {
        return Err(VacationSkip::AlreadyReplied);
    }
    Ok(sender)
}

/// Reply-To mailbox, else From.
pub fn reply_target(envelope: &Envelope) -> Option<String> {
    first_email(envelope.reply_to.as_ref()).or_else(|| first_email(envelope.from.as_ref()))
}

/// First From mailbox.
pub fn from_email(envelope: &Envelope) -> Option<String> {
    first_email(envelope.from.as_ref())
}

fn first_email(addr: Option<&EmailAddress>) -> Option<String> {
    addr.map(flatten_addresses)
        .and_then(|v| v.into_iter().next())
        .map(|a| a.email)
}

/// True when `email` matches one of our identities (ASCII case-insensitive).
pub fn is_own_address(email: &str, own_addresses: &[String]) -> bool {
    let email = email.trim();
    if email.is_empty() {
        return false;
    }
    own_addresses
        .iter()
        .any(|own| own.trim().eq_ignore_ascii_case(email))
}

/// No-reply / do-not-reply / mailer-daemon / postmaster local-parts.
pub fn is_noreply_address(email: &str) -> bool {
    let email = email.trim().to_ascii_lowercase();
    if email.is_empty() {
        return true;
    }
    let local = email.split('@').next().unwrap_or(email.as_str());
    let compact: String = local
        .chars()
        .filter(|c| *c != '.' && *c != '-' && *c != '_')
        .collect();
    compact.contains("noreply")
        || compact.contains("donotreply")
        || local == "mailer-daemon"
        || compact == "mailerdaemon"
        || local == "postmaster"
}

pub fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

pub fn already_replied(replied: &HashSet<String>, email: &str) -> bool {
    replied.contains(&normalize_email(email))
}

/// Folders that are not incoming mail (vacation does not run here).
pub fn folder_skips_vacation(role: mailiner_core::MailboxRole) -> bool {
    crate::mail_rules::folder_skips_rules(role)
}

/// Plain-text vacation reply draft (In-Reply-To / References when present).
pub fn build_vacation_draft(
    identity: &FromIdentity,
    settings: &VacationSettings,
    envelope: &Envelope,
    sender: &str,
) -> DraftDocument {
    let mut draft = DraftDocument::new_empty(identity);
    draft.mode = BodyMode::Plain;
    draft.subject = if settings.subject.trim().is_empty() {
        DEFAULT_VACATION_SUBJECT.to_string()
    } else {
        settings.subject.clone()
    };
    draft.plain_body = settings.body.clone();
    draft.html_body.clear();
    draft.plain_cache_dirty = false;
    draft.to = vec![ComposerAddress {
        name: None,
        email: sender.trim().to_string(),
    }];
    apply_reply_threading(&mut draft, envelope);
    draft
}

fn apply_reply_threading(draft: &mut DraftDocument, env: &Envelope) {
    let Some(mid) = env
        .rfc_message_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    draft.in_reply_to = Some(mid.to_string());
    let mut refs = env.references.clone();
    if refs.is_empty()
        && let Some(parent) = env
            .in_reply_to
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    {
        refs.push(parent.to_string());
    }
    if !refs.iter().any(|r| msg_ids_equal(r, mid)) {
        refs.push(mid.to_string());
    }
    draft.references = refs;
}

fn msg_ids_equal(a: &str, b: &str) -> bool {
    fn bare(s: &str) -> &str {
        s.trim().trim_start_matches('<').trim_end_matches('>')
    }
    bare(a).eq_ignore_ascii_case(bare(b))
}

/// Parse an `<input type="datetime-local">` value as UTC (browser / host local).
pub fn parse_datetime_local(value: &str) -> Option<DateTime<Utc>> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let naive = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S"))
        .ok()?;
    local_naive_to_utc(naive)
}

/// Format `dt` for `<input type="datetime-local">`.
pub fn format_datetime_local(dt: DateTime<Utc>) -> String {
    utc_to_local_naive(dt).format("%Y-%m-%dT%H:%M").to_string()
}

fn local_naive_to_utc(naive: NaiveDateTime) -> Option<DateTime<Utc>> {
    #[cfg(target_arch = "wasm32")]
    {
        use chrono::Datelike;
        use chrono::Timelike;
        let js = js_sys::Date::new_with_year_month_day_hr_min_sec_milli(
            naive.year() as u32,
            (naive.month() as i32) - 1,
            naive.day() as i32,
            naive.hour() as i32,
            naive.minute() as i32,
            naive.second() as i32,
            0.0,
        );
        DateTime::from_timestamp_millis(js.get_time() as i64)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use chrono::Local;
        use chrono::TimeZone;
        Local
            .from_local_datetime(&naive)
            .single()
            .map(|dt| dt.with_timezone(&Utc))
    }
}

fn utc_to_local_naive(dt: DateTime<Utc>) -> NaiveDateTime {
    #[cfg(target_arch = "wasm32")]
    {
        let js = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(
            dt.timestamp_millis() as f64
        ));
        NaiveDateTime::new(
            chrono::NaiveDate::from_ymd_opt(
                js.get_full_year() as i32,
                js.get_month() + 1,
                js.get_date(),
            )
            .unwrap_or_else(|| dt.date_naive()),
            chrono::NaiveTime::from_hms_opt(js.get_hours(), js.get_minutes(), js.get_seconds())
                .unwrap_or_else(|| dt.time()),
        )
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use chrono::Local;
        dt.with_timezone(&Local).naive_local()
    }
}

/// Persisted vacation settings per account.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct VacationBlob {
    pub schema_version: u32,
    #[serde(default)]
    pub settings: HashMap<AccountId, VacationSettings>,
}

impl VacationBlob {
    pub fn empty() -> Self {
        Self {
            schema_version: VACATION_SCHEMA_VERSION,
            settings: HashMap::new(),
        }
    }

    pub fn encode(&self) -> Result<String, AccountStoreError> {
        serde_json::to_string(self).map_err(|e| AccountStoreError::Serialization(e.to_string()))
    }

    pub fn decode(json: &str) -> Result<Self, AccountStoreError> {
        let blob: Self = serde_json::from_str(json)
            .map_err(|e| AccountStoreError::Serialization(e.to_string()))?;
        if blob.schema_version > VACATION_SCHEMA_VERSION {
            return Err(AccountStoreError::Serialization(format!(
                "unsupported vacation schema_version {} (max supported {})",
                blob.schema_version, VACATION_SCHEMA_VERSION
            )));
        }
        Ok(blob)
    }

    pub fn for_account(&self, account_id: &AccountId) -> VacationSettings {
        self.settings
            .get(account_id)
            .cloned()
            .unwrap_or_else(VacationSettings::new)
    }

    pub fn set_account(&mut self, account_id: AccountId, settings: VacationSettings) {
        let settings = settings.trimmed();
        if !settings.enabled
            && settings.start.is_none()
            && settings.end.is_none()
            && settings.subject == DEFAULT_VACATION_SUBJECT
            && settings.body == DEFAULT_VACATION_BODY
            && settings.armed_at.is_none()
        {
            self.settings.remove(&account_id);
        } else {
            self.settings.insert(account_id, settings);
        }
        self.schema_version = VACATION_SCHEMA_VERSION;
    }

    pub fn retain_accounts(&mut self, known: &HashSet<AccountId>) {
        self.settings.retain(|id, _| known.contains(id));
        self.schema_version = VACATION_SCHEMA_VERSION;
    }
}

/// Senders already auto-replied to (per account + vacation period).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct VacationRepliedBlob {
    pub schema_version: u32,
    /// Account id → period key → addresses (insertion order, oldest first).
    #[serde(default)]
    pub replied: HashMap<AccountId, HashMap<String, Vec<String>>>,
}

impl VacationRepliedBlob {
    pub fn empty() -> Self {
        Self {
            schema_version: VACATION_REPLIED_SCHEMA_VERSION,
            replied: HashMap::new(),
        }
    }

    pub fn encode(&self) -> Result<String, AccountStoreError> {
        serde_json::to_string(self).map_err(|e| AccountStoreError::Serialization(e.to_string()))
    }

    pub fn decode(json: &str) -> Result<Self, AccountStoreError> {
        let blob: Self = serde_json::from_str(json)
            .map_err(|e| AccountStoreError::Serialization(e.to_string()))?;
        if blob.schema_version > VACATION_REPLIED_SCHEMA_VERSION {
            return Err(AccountStoreError::Serialization(format!(
                "unsupported vacation-replied schema_version {} (max supported {})",
                blob.schema_version, VACATION_REPLIED_SCHEMA_VERSION
            )));
        }
        Ok(blob)
    }

    pub fn addresses(&self, account_id: &AccountId, period_key: &str) -> HashSet<String> {
        self.replied
            .get(account_id)
            .and_then(|m| m.get(period_key))
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect()
    }

    pub fn mark(
        &mut self,
        account_id: AccountId,
        period_key: &str,
        emails: impl IntoIterator<Item = String>,
    ) {
        let list = self
            .replied
            .entry(account_id)
            .or_default()
            .entry(period_key.to_string())
            .or_default();
        for email in emails {
            let email = normalize_email(&email);
            if email.is_empty() || list.iter().any(|existing| existing == &email) {
                continue;
            }
            list.push(email);
        }
        if list.len() > MAX_REPLIED_PER_PERIOD {
            let drop = list.len() - MAX_REPLIED_PER_PERIOD;
            list.drain(0..drop);
        }
        self.schema_version = VACATION_REPLIED_SCHEMA_VERSION;
    }

    pub fn retain_accounts(&mut self, known: &HashSet<AccountId>) {
        self.replied.retain(|id, _| known.contains(id));
        self.schema_version = VACATION_REPLIED_SCHEMA_VERSION;
    }
}

fn load_settings_blob(kv: &dyn StringKvStore) -> Result<VacationBlob, AccountStoreError> {
    match kv.get_item(VACATION_KEY)? {
        None => Ok(VacationBlob::empty()),
        Some(s) if s.trim().is_empty() => Ok(VacationBlob::empty()),
        Some(s) => VacationBlob::decode(&s),
    }
}

fn save_settings_blob(
    kv: &dyn StringKvStore,
    blob: &VacationBlob,
) -> Result<(), AccountStoreError> {
    kv.set_item(VACATION_KEY, &blob.encode()?)
}

fn load_replied_blob(kv: &dyn StringKvStore) -> Result<VacationRepliedBlob, AccountStoreError> {
    match kv.get_item(VACATION_REPLIED_KEY)? {
        None => Ok(VacationRepliedBlob::empty()),
        Some(s) if s.trim().is_empty() => Ok(VacationRepliedBlob::empty()),
        Some(s) => VacationRepliedBlob::decode(&s),
    }
}

fn save_replied_blob(
    kv: &dyn StringKvStore,
    blob: &VacationRepliedBlob,
) -> Result<(), AccountStoreError> {
    kv.set_item(VACATION_REPLIED_KEY, &blob.encode()?)
}

fn with_kv<T>(f: impl FnOnce(&dyn StringKvStore) -> Result<T, AccountStoreError>) -> Option<T> {
    #[cfg(target_arch = "wasm32")]
    {
        let storage = WebLocalStorage::try_open().ok()?;
        f(&storage).ok()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        host_kv::with(|kv| f(kv).ok())
    }
}

fn mutate_settings<T>(f: impl FnOnce(&mut VacationBlob) -> T) -> Option<T> {
    with_kv(|kv| {
        let mut blob = load_settings_blob(kv)?;
        let out = f(&mut blob);
        save_settings_blob(kv, &blob)?;
        Ok(out)
    })
}

/// Settings for `account_id` (defaults when unset).
pub fn load_settings(account_id: &AccountId) -> VacationSettings {
    with_kv(|kv| Ok(load_settings_blob(kv)?.for_account(account_id))).unwrap_or_default()
}

/// Persist settings. Records `armed_at` the first time vacation is enabled.
pub fn save_settings(account_id: AccountId, mut settings: VacationSettings) -> VacationSettings {
    let previous = load_settings(&account_id);
    settings = settings.trimmed();
    if settings.enabled && !previous.enabled {
        settings.armed_at = Some(Utc::now());
    } else if !settings.enabled {
        settings.armed_at = None;
    } else if settings.armed_at.is_none() {
        settings.armed_at = previous.armed_at.or_else(|| Some(Utc::now()));
    }
    let stored = settings.clone();
    let _ = mutate_settings(|blob| blob.set_account(account_id, settings));
    stored
}

/// Drop vacation data whose account is no longer known.
pub fn retain_vacation(known: &HashSet<AccountId>) {
    let _ = mutate_settings(|blob| blob.retain_accounts(known));
    let _ = with_kv(|kv| {
        let mut blob = load_replied_blob(kv)?;
        blob.retain_accounts(known);
        save_replied_blob(kv, &blob)
    });
}

/// Senders already auto-replied to in this vacation period.
pub fn load_replied(account_id: &AccountId, period_key: &str) -> HashSet<String> {
    with_kv(|kv| Ok(load_replied_blob(kv)?.addresses(account_id, period_key))).unwrap_or_default()
}

/// Remember senders so we do not auto-reply twice in this period.
pub fn mark_replied(account_id: &AccountId, period_key: &str, emails: &[String]) {
    if emails.is_empty() {
        return;
    }
    let _ = with_kv(|kv| {
        let mut blob = load_replied_blob(kv)?;
        blob.mark(account_id.clone(), period_key, emails.iter().cloned());
        save_replied_blob(kv, &blob)
    });
}

#[cfg(not(target_arch = "wasm32"))]
mod host_kv {
    use crate::account_store::MemoryKvStore;
    use std::cell::RefCell;

    thread_local! {
        static KV: RefCell<MemoryKvStore> = RefCell::new(MemoryKvStore::new());
    }

    pub fn with<T>(f: impl FnOnce(&MemoryKvStore) -> T) -> T {
        KV.with(|cell| f(&cell.borrow()))
    }

    #[cfg(test)]
    pub fn reset() {
        KV.with(|cell| *cell.borrow_mut() = MemoryKvStore::new());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Timelike};
    use mailiner_core::{AccountId, EmailAddr, FolderId, MessageId};

    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    fn envelope(uid: &str, from: &str) -> Envelope {
        envelope_at(uid, from, None, ts(1_700_000_000))
    }

    fn envelope_at(uid: &str, from: &str, reply_to: Option<&str>, date: DateTime<Utc>) -> Envelope {
        Envelope {
            id: MessageId::new(FolderId::new("INBOX"), uid),
            account_id: AccountId::new("acc"),
            folder_id: FolderId::new("INBOX"),
            subject: Some("Hello".into()),
            from: Some(EmailAddress::List(vec![EmailAddr {
                name: Some("Ada".into()),
                email: Some(from.into()),
            }])),
            to: Some(EmailAddress::List(vec![EmailAddr {
                name: None,
                email: Some("me@example.com".into()),
            }])),
            cc: None,
            bcc: None,
            reply_to: reply_to.map(|email| {
                EmailAddress::List(vec![EmailAddr {
                    name: None,
                    email: Some(email.into()),
                }])
            }),
            rfc_message_id: Some("<mid@example.com>".into()),
            in_reply_to: None,
            references: vec![],
            date,
            is_read: false,
            is_answered: false,
            is_starred: false,
            is_flagged: false,
            is_draft: false,
            is_deleted: false,
            keywords: vec![],
            has_attachments: false,
            size: None,
            snippet: None,
            auth_results: Default::default(),
        }
    }

    fn active_settings() -> VacationSettings {
        VacationSettings {
            enabled: true,
            start: None,
            end: None,
            subject: "Away".into(),
            body: "Back later.".into(),
            armed_at: Some(ts(1_600_000_000)),
        }
    }

    fn own() -> Vec<String> {
        vec!["me@example.com".into(), "alias@example.com".into()]
    }

    #[test]
    fn window_active_when_enabled_and_unbounded() {
        let settings = active_settings();
        assert!(settings.is_active(ts(1_700_000_000)));
        let off = VacationSettings {
            enabled: false,
            ..settings.clone()
        };
        assert!(!off.is_active(ts(1_700_000_000)));
        assert_eq!(
            should_reply(
                &off,
                ts(1_700_000_000),
                &envelope("1", "a@b.com"),
                &own(),
                &HashSet::new()
            ),
            Err(VacationSkip::Disabled)
        );
    }

    #[test]
    fn window_inactive_before_start_and_after_end() {
        let settings = VacationSettings {
            enabled: true,
            start: Some(ts(100)),
            end: Some(ts(200)),
            armed_at: Some(ts(100)),
            ..VacationSettings::new()
        };
        assert!(!settings.is_active(ts(99)));
        assert!(settings.is_active(ts(100)));
        assert!(settings.is_active(ts(200)));
        assert!(!settings.is_active(ts(201)));
        assert_eq!(
            should_reply(
                &settings,
                ts(99),
                &envelope_at("1", "a@b.com", None, ts(150)),
                &own(),
                &HashSet::new()
            ),
            Err(VacationSkip::OutsideWindow)
        );
        assert_eq!(
            should_reply(
                &settings,
                ts(150),
                &envelope_at("1", "a@b.com", None, ts(150)),
                &own(),
                &HashSet::new()
            ),
            Ok("a@b.com".into())
        );
    }

    #[test]
    fn inverted_window_is_inactive() {
        let settings = VacationSettings {
            enabled: true,
            start: Some(ts(200)),
            end: Some(ts(100)),
            ..VacationSettings::new()
        };
        assert!(!settings.is_active(ts(150)));
    }

    #[test]
    fn already_replied_skip() {
        let settings = active_settings();
        let env = envelope("1", "Ada@Example.COM");
        let replied = HashSet::from(["ada@example.com".into()]);
        assert_eq!(
            should_reply(&settings, ts(1_700_000_000), &env, &own(), &replied),
            Err(VacationSkip::AlreadyReplied)
        );
        assert_eq!(
            should_reply(&settings, ts(1_700_000_000), &env, &own(), &HashSet::new()),
            Ok("Ada@Example.COM".into())
        );
    }

    #[test]
    fn no_reply_skip() {
        let settings = active_settings();
        let now = ts(1_700_000_000);
        for addr in [
            "noreply@example.com",
            "no-reply@example.com",
            "do-not-reply@lists.example",
            "mailer-daemon@example.com",
            "postmaster@example.com",
        ] {
            assert_eq!(
                should_reply(
                    &settings,
                    now,
                    &envelope("1", addr),
                    &own(),
                    &HashSet::new()
                ),
                Err(VacationSkip::NoReply),
                "{addr}"
            );
        }
        assert!(is_noreply_address("no_reply@example.com"));
        assert!(!is_noreply_address("ada@example.com"));
    }

    #[test]
    fn own_address_skip() {
        let settings = active_settings();
        let now = ts(1_700_000_000);
        assert_eq!(
            should_reply(
                &settings,
                now,
                &envelope("1", "me@example.com"),
                &own(),
                &HashSet::new()
            ),
            Err(VacationSkip::OwnAddress)
        );
        assert_eq!(
            should_reply(
                &settings,
                now,
                &envelope("2", "ALIAS@example.com"),
                &own(),
                &HashSet::new()
            ),
            Err(VacationSkip::OwnAddress)
        );
    }

    #[test]
    fn plan_replies_once_per_sender_in_batch() {
        let settings = active_settings();
        let envs = [
            envelope("1", "ada@example.com"),
            envelope("2", "ADA@example.com"),
            envelope("3", "bob@example.com"),
        ];
        let hits = plan_vacation_hits(&settings, ts(1_700_000_000), &envs, &own(), &HashSet::new());
        assert_eq!(
            hits.iter()
                .map(|h| (h.uid.as_str(), h.sender.as_str()))
                .collect::<Vec<_>>(),
            vec![("1", "ada@example.com"), ("3", "bob@example.com")]
        );
    }

    #[test]
    fn skips_mail_before_cutoff() {
        let settings = VacationSettings {
            enabled: true,
            start: Some(ts(100)),
            armed_at: Some(ts(150)),
            ..VacationSettings::new()
        };
        let old = envelope_at("1", "a@b.com", None, ts(140));
        let fresh = envelope_at("2", "a@b.com", None, ts(160));
        assert_eq!(
            should_reply(&settings, ts(200), &old, &own(), &HashSet::new()),
            Err(VacationSkip::BeforeCutoff)
        );
        assert_eq!(
            should_reply(&settings, ts(200), &fresh, &own(), &HashSet::new()),
            Ok("a@b.com".into())
        );
    }

    #[test]
    fn reply_to_wins_over_from() {
        let settings = active_settings();
        let env = envelope_at(
            "1",
            "from@example.com",
            Some("reply@example.com"),
            ts(1_700_000_000),
        );
        assert_eq!(
            should_reply(&settings, ts(1_700_000_000), &env, &own(), &HashSet::new()),
            Ok("reply@example.com".into())
        );
    }

    #[test]
    fn draft_sets_threading_headers() {
        let identity = FromIdentity::new("Me", "me@example.com");
        let settings = active_settings();
        let mut env = envelope("1", "ada@example.com");
        env.references = vec!["<parent@example.com>".into()];
        let draft = build_vacation_draft(&identity, &settings, &env, "ada@example.com");
        assert_eq!(draft.subject, "Away");
        assert_eq!(draft.plain_body, "Back later.");
        assert_eq!(draft.mode, BodyMode::Plain);
        assert_eq!(draft.to[0].email, "ada@example.com");
        assert_eq!(draft.in_reply_to.as_deref(), Some("<mid@example.com>"));
        assert_eq!(
            draft.references,
            vec![
                "<parent@example.com>".to_string(),
                "<mid@example.com>".into()
            ]
        );
    }

    #[test]
    fn persist_roundtrip_and_retain() {
        host_kv::reset();
        let acc = AccountId::new("acc");
        let gone = AccountId::new("gone");
        let saved = save_settings(
            acc.clone(),
            VacationSettings {
                enabled: true,
                subject: "  Away  ".into(),
                body: "Later.".into(),
                ..VacationSettings::new()
            },
        );
        assert!(saved.enabled);
        assert_eq!(saved.subject, "Away");
        assert!(saved.armed_at.is_some());
        assert_eq!(load_settings(&acc).subject, "Away");
        save_settings(gone.clone(), active_settings());
        mark_replied(&acc, &saved.period_key(), &["ada@example.com".into()]);
        assert!(load_replied(&acc, &saved.period_key()).contains("ada@example.com"));
        retain_vacation(&HashSet::from([acc.clone()]));
        assert!(load_settings(&acc).enabled);
        assert!(!load_settings(&gone).enabled);
        host_kv::reset();
    }

    #[test]
    fn blob_decode_rejects_future_schema() {
        let err = VacationBlob::decode(r#"{"schema_version":99,"settings":{}}"#).unwrap_err();
        match err {
            AccountStoreError::Serialization(msg) => {
                assert!(msg.contains("99"), "{msg}");
            }
            other => panic!("expected Serialization, got {other:?}"),
        }
    }

    #[test]
    fn datetime_local_roundtrip() {
        let src = Utc.with_ymd_and_hms(2026, 9, 4, 15, 30, 0).unwrap();
        let formatted = format_datetime_local(src);
        let back = parse_datetime_local(&formatted).expect("parse");
        assert_eq!(back.with_timezone(&Utc).minute(), src.minute());
        assert!(parse_datetime_local("").is_none());
        assert!(parse_datetime_local("not-a-date").is_none());
    }

    #[test]
    fn folder_skip_roles() {
        use mailiner_core::MailboxRole;
        assert!(folder_skips_vacation(MailboxRole::Sent));
        assert!(folder_skips_vacation(MailboxRole::Drafts));
        assert!(folder_skips_vacation(MailboxRole::Trash));
        assert!(folder_skips_vacation(MailboxRole::Junk));
        assert!(!folder_skips_vacation(MailboxRole::Inbox));
        assert!(!folder_skips_vacation(MailboxRole::Archive));
    }
}
