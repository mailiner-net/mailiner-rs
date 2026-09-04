//! Local incoming-mail filter rules (`mailiner.ui.mailRules.v1`).
//!
//! Mailiner does not speak ManageSieve. Rules live in this browser and run
//! when envelopes are fetched or refreshed (folder open / IDLE / NOOP).

use std::collections::{HashMap, HashSet};

use mailiner_core::ids::AccountId;
use mailiner_core::{EmailAddress, Envelope, ImapKeyword};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(target_arch = "wasm32")]
use crate::account_store::WebLocalStorage;
use crate::account_store::{AccountStoreError, StringKvStore};
use crate::mailbox::MailboxId;

/// `localStorage` key for per-account filter rules.
pub const MAIL_RULES_KEY: &str = "mailiner.ui.mailRules.v1";
/// Schema version for [`MailRulesBlob`].
pub const MAIL_RULES_SCHEMA_VERSION: u32 = 1;
/// Cap on rules stored for one account.
pub const MAX_RULES_PER_ACCOUNT: usize = 50;

/// `localStorage` key for UIDs already processed by the local engine.
pub const MAIL_RULES_APPLIED_KEY: &str = "mailiner.ui.mailRulesApplied.v1";
/// Schema version for [`AppliedRulesBlob`].
pub const MAIL_RULES_APPLIED_SCHEMA_VERSION: u32 = 1;
/// Cap on remembered UIDs per account+mailbox (evict oldest).
pub const MAX_APPLIED_PER_MAILBOX: usize = 2_000;

/// One local incoming-mail filter.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MailRule {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Case-insensitive substring of From (name or address). Empty = ignore.
    #[serde(default)]
    pub match_from: String,
    /// Case-insensitive substring of To (name or address). Empty = ignore.
    #[serde(default)]
    pub match_to: String,
    /// Case-insensitive substring of Subject. Empty = ignore.
    #[serde(default)]
    pub match_subject: String,
    /// Optional IMAP keyword atom the message must already have.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_keyword: Option<String>,
    /// When true, only unread messages match.
    #[serde(default)]
    pub match_unread: bool,
    /// Destination mailbox id. `None` / empty = do not move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_move_to: Option<String>,
    #[serde(default)]
    pub action_mark_read: bool,
    #[serde(default)]
    pub action_star: bool,
    #[serde(default)]
    pub action_flag: bool,
    /// Built-in custom keyword atom to add (`$Todo`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_add_keyword: Option<String>,
}

fn default_true() -> bool {
    true
}

impl MailRule {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: String::new(),
            enabled: true,
            match_from: String::new(),
            match_to: String::new(),
            match_subject: String::new(),
            match_keyword: None,
            match_unread: false,
            action_move_to: None,
            action_mark_read: false,
            action_star: false,
            action_flag: false,
            action_add_keyword: None,
        }
    }

    pub fn trimmed(mut self) -> Self {
        self.name = self.name.trim().to_string();
        self.match_from = self.match_from.trim().to_string();
        self.match_to = self.match_to.trim().to_string();
        self.match_subject = self.match_subject.trim().to_string();
        self.match_keyword = normalize_opt_atom(self.match_keyword);
        self.action_move_to = normalize_opt_id(self.action_move_to);
        self.action_add_keyword = normalize_opt_atom(self.action_add_keyword);
        self
    }

    pub fn has_match_criterion(&self) -> bool {
        !self.match_from.is_empty()
            || !self.match_to.is_empty()
            || !self.match_subject.is_empty()
            || self.match_keyword.as_ref().is_some_and(|s| !s.is_empty())
            || self.match_unread
    }

    pub fn has_action(&self) -> bool {
        self.action_move_to.as_ref().is_some_and(|s| !s.is_empty())
            || self.action_mark_read
            || self.action_star
            || self.action_flag
            || self
                .action_add_keyword
                .as_ref()
                .is_some_and(|s| !s.is_empty())
    }

    /// Destination mailbox when the move action is set.
    pub fn move_mailbox(&self) -> Option<MailboxId> {
        self.action_move_to
            .as_ref()
            .filter(|s| !s.is_empty())
            .map(|s| MailboxId::from(s.clone()))
    }

    pub fn add_keyword(&self) -> Option<ImapKeyword> {
        self.action_add_keyword
            .as_deref()
            .and_then(ImapKeyword::from_atom)
    }

    /// Display name, or a short summary of the match when unnamed.
    pub fn display_name(&self) -> String {
        let name = self.name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
        let mut parts = Vec::new();
        if !self.match_from.is_empty() {
            parts.push(format!("from contains “{}”", self.match_from));
        }
        if !self.match_to.is_empty() {
            parts.push(format!("to contains “{}”", self.match_to));
        }
        if !self.match_subject.is_empty() {
            parts.push(format!("subject contains “{}”", self.match_subject));
        }
        if let Some(kw) = self.match_keyword.as_deref().filter(|s| !s.is_empty()) {
            parts.push(format!("keyword {kw}"));
        }
        if self.match_unread {
            parts.push("unread".into());
        }
        if parts.is_empty() {
            "Untitled filter".into()
        } else {
            parts.join(", ")
        }
    }

    /// Short action summary for the settings list.
    pub fn action_summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(dest) = self.action_move_to.as_deref().filter(|s| !s.is_empty()) {
            parts.push(format!("move to {dest}"));
        }
        if self.action_mark_read {
            parts.push("mark read".into());
        }
        if self.action_star {
            parts.push("star".into());
        }
        if self.action_flag {
            parts.push("flag".into());
        }
        if let Some(kw) = self.action_add_keyword.as_deref().filter(|s| !s.is_empty()) {
            let label = ImapKeyword::from_atom(kw).map(|k| k.label()).unwrap_or(kw);
            parts.push(format!("label {label}"));
        }
        if parts.is_empty() {
            "no action".into()
        } else {
            parts.join(", ")
        }
    }

    /// Enabled + every set criterion matches (AND). Disabled rules never match.
    pub fn matches(&self, envelope: &Envelope) -> bool {
        if !self.enabled || !self.has_match_criterion() {
            return false;
        }
        if !self.match_from.is_empty()
            && !contains_ci(&address_haystack(envelope.from.as_ref()), &self.match_from)
        {
            return false;
        }
        if !self.match_to.is_empty()
            && !contains_ci(&address_haystack(envelope.to.as_ref()), &self.match_to)
        {
            return false;
        }
        if !self.match_subject.is_empty() {
            let subject = envelope.subject.as_deref().unwrap_or("");
            if !contains_ci(subject, &self.match_subject) {
                return false;
            }
        }
        if let Some(kw) = self.match_keyword.as_deref().filter(|s| !s.is_empty())
            && !envelope
                .keywords
                .iter()
                .any(|atom| atom.eq_ignore_ascii_case(kw))
        {
            return false;
        }
        if self.match_unread && envelope.is_read {
            return false;
        }
        true
    }
}

impl Default for MailRule {
    fn default() -> Self {
        Self::new()
    }
}

/// First matching enabled rule, if any.
pub fn first_matching_rule<'a>(rules: &'a [MailRule], envelope: &Envelope) -> Option<&'a MailRule> {
    rules.iter().find(|rule| rule.matches(envelope))
}

/// Planned application of one rule to one envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleHit<'a> {
    pub rule: &'a MailRule,
    pub uid: &'a str,
}

/// First matching rule per envelope. Disabled rules, empty criteria, and
/// already-applied UIDs are skipped. First match wins.
pub fn plan_rule_hits<'a>(
    rules: &'a [MailRule],
    envelopes: &'a [Envelope],
    applied_uids: &HashSet<String>,
) -> Vec<RuleHit<'a>> {
    envelopes
        .iter()
        .filter(|env| !applied_uids.contains(env.id.as_uid()))
        .filter_map(|env| {
            first_matching_rule(rules, env).map(|rule| RuleHit {
                rule,
                uid: env.id.as_uid(),
            })
        })
        .collect()
}

/// Folders that are not incoming mail (rules do not run here).
pub fn folder_skips_rules(role: mailiner_core::MailboxRole) -> bool {
    matches!(
        role,
        mailiner_core::MailboxRole::Sent
            | mailiner_core::MailboxRole::Drafts
            | mailiner_core::MailboxRole::Trash
            | mailiner_core::MailboxRole::Junk
            | mailiner_core::MailboxRole::Outbox
    )
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

fn address_haystack(addr: Option<&EmailAddress>) -> String {
    addr.map(|a| a.to_string()).unwrap_or_default()
}

fn normalize_opt_atom(value: Option<String>) -> Option<String> {
    value.and_then(|s| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    })
}

fn normalize_opt_id(value: Option<String>) -> Option<String> {
    normalize_opt_atom(value)
}

/// Persisted rules. Order inside each account list is evaluation order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MailRulesBlob {
    pub schema_version: u32,
    #[serde(default)]
    pub rules: HashMap<AccountId, Vec<MailRule>>,
}

impl MailRulesBlob {
    pub fn empty() -> Self {
        Self {
            schema_version: MAIL_RULES_SCHEMA_VERSION,
            rules: HashMap::new(),
        }
    }

    pub fn encode(&self) -> Result<String, AccountStoreError> {
        serde_json::to_string(self).map_err(|e| AccountStoreError::Serialization(e.to_string()))
    }

    pub fn decode(json: &str) -> Result<Self, AccountStoreError> {
        let blob: Self = serde_json::from_str(json)
            .map_err(|e| AccountStoreError::Serialization(e.to_string()))?;
        if blob.schema_version > MAIL_RULES_SCHEMA_VERSION {
            return Err(AccountStoreError::Serialization(format!(
                "unsupported mail-rules schema_version {} (max supported {})",
                blob.schema_version, MAIL_RULES_SCHEMA_VERSION
            )));
        }
        Ok(blob)
    }

    pub fn for_account(&self, account_id: &AccountId) -> Vec<MailRule> {
        self.rules.get(account_id).cloned().unwrap_or_default()
    }

    pub fn set_account(&mut self, account_id: AccountId, mut rules: Vec<MailRule>) {
        rules.truncate(MAX_RULES_PER_ACCOUNT);
        if rules.is_empty() {
            self.rules.remove(&account_id);
        } else {
            self.rules.insert(account_id, rules);
        }
        self.schema_version = MAIL_RULES_SCHEMA_VERSION;
    }

    pub fn upsert(&mut self, account_id: AccountId, rule: MailRule) -> Result<MailRule, RuleError> {
        let rule = rule.trimmed();
        if !rule.has_match_criterion() {
            return Err(RuleError::EmptyMatch);
        }
        if !rule.has_action() {
            return Err(RuleError::EmptyAction);
        }
        let mut rules = self.for_account(&account_id);
        if let Some(existing) = rules.iter_mut().find(|r| r.id == rule.id) {
            *existing = rule.clone();
        } else {
            if rules.len() >= MAX_RULES_PER_ACCOUNT {
                return Err(RuleError::Limit);
            }
            rules.push(rule.clone());
        }
        self.set_account(account_id, rules);
        Ok(rule)
    }

    pub fn remove(&mut self, account_id: &AccountId, rule_id: &str) -> bool {
        let mut rules = self.for_account(account_id);
        let before = rules.len();
        rules.retain(|r| r.id != rule_id);
        if rules.len() == before {
            return false;
        }
        self.set_account(account_id.clone(), rules);
        true
    }

    pub fn set_enabled(&mut self, account_id: &AccountId, rule_id: &str, enabled: bool) -> bool {
        let mut rules = self.for_account(account_id);
        let Some(rule) = rules.iter_mut().find(|r| r.id == rule_id) else {
            return false;
        };
        rule.enabled = enabled;
        self.set_account(account_id.clone(), rules);
        true
    }

    /// Move `rule_id` by `delta` positions (−1 = earlier / higher priority).
    pub fn move_rule(&mut self, account_id: &AccountId, rule_id: &str, delta: i32) -> bool {
        let mut rules = self.for_account(account_id);
        let Some(idx) = rules.iter().position(|r| r.id == rule_id) else {
            return false;
        };
        let dest = idx as i32 + delta;
        if dest < 0 || dest as usize >= rules.len() {
            return false;
        }
        let rule = rules.remove(idx);
        rules.insert(dest as usize, rule);
        self.set_account(account_id.clone(), rules);
        true
    }

    pub fn retain_accounts(&mut self, known: &HashSet<AccountId>) {
        self.rules.retain(|id, _| known.contains(id));
        self.schema_version = MAIL_RULES_SCHEMA_VERSION;
    }
}

/// Why a rule was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleError {
    EmptyMatch,
    EmptyAction,
    Limit,
}

impl RuleError {
    pub fn message(self) -> &'static str {
        match self {
            Self::EmptyMatch => "Add at least one match (from, to, subject, keyword, or unread).",
            Self::EmptyAction => {
                "Choose at least one action (move, mark read, star, flag, or label)."
            }
            Self::Limit => "This account already has the maximum number of filters.",
        }
    }
}

/// UIDs already processed by the local engine (per account + mailbox).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AppliedRulesBlob {
    pub schema_version: u32,
    /// Account id → mailbox id → UIDs (insertion order, oldest first).
    #[serde(default)]
    pub applied: HashMap<AccountId, HashMap<String, Vec<String>>>,
}

impl AppliedRulesBlob {
    pub fn empty() -> Self {
        Self {
            schema_version: MAIL_RULES_APPLIED_SCHEMA_VERSION,
            applied: HashMap::new(),
        }
    }

    pub fn encode(&self) -> Result<String, AccountStoreError> {
        serde_json::to_string(self).map_err(|e| AccountStoreError::Serialization(e.to_string()))
    }

    pub fn decode(json: &str) -> Result<Self, AccountStoreError> {
        let blob: Self = serde_json::from_str(json)
            .map_err(|e| AccountStoreError::Serialization(e.to_string()))?;
        if blob.schema_version > MAIL_RULES_APPLIED_SCHEMA_VERSION {
            return Err(AccountStoreError::Serialization(format!(
                "unsupported mail-rules-applied schema_version {} (max supported {})",
                blob.schema_version, MAIL_RULES_APPLIED_SCHEMA_VERSION
            )));
        }
        Ok(blob)
    }

    pub fn uids(&self, account_id: &AccountId, mailbox_id: &MailboxId) -> HashSet<String> {
        self.applied
            .get(account_id)
            .and_then(|m| m.get(mailbox_id.as_str()))
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect()
    }

    pub fn mark(
        &mut self,
        account_id: AccountId,
        mailbox_id: &MailboxId,
        uids: impl IntoIterator<Item = String>,
    ) {
        let mailbox = mailbox_id.as_str().to_string();
        let list = self
            .applied
            .entry(account_id)
            .or_default()
            .entry(mailbox)
            .or_default();
        for uid in uids {
            if uid.is_empty() || list.iter().any(|existing| existing == &uid) {
                continue;
            }
            list.push(uid);
        }
        if list.len() > MAX_APPLIED_PER_MAILBOX {
            let drop = list.len() - MAX_APPLIED_PER_MAILBOX;
            list.drain(0..drop);
        }
        self.schema_version = MAIL_RULES_APPLIED_SCHEMA_VERSION;
    }

    pub fn retain_accounts(&mut self, known: &HashSet<AccountId>) {
        self.applied.retain(|id, _| known.contains(id));
        self.schema_version = MAIL_RULES_APPLIED_SCHEMA_VERSION;
    }
}

fn load_rules_blob(kv: &dyn StringKvStore) -> Result<MailRulesBlob, AccountStoreError> {
    match kv.get_item(MAIL_RULES_KEY)? {
        None => Ok(MailRulesBlob::empty()),
        Some(s) if s.trim().is_empty() => Ok(MailRulesBlob::empty()),
        Some(s) => MailRulesBlob::decode(&s),
    }
}

fn save_rules_blob(kv: &dyn StringKvStore, blob: &MailRulesBlob) -> Result<(), AccountStoreError> {
    kv.set_item(MAIL_RULES_KEY, &blob.encode()?)
}

fn load_applied_blob(kv: &dyn StringKvStore) -> Result<AppliedRulesBlob, AccountStoreError> {
    match kv.get_item(MAIL_RULES_APPLIED_KEY)? {
        None => Ok(AppliedRulesBlob::empty()),
        Some(s) if s.trim().is_empty() => Ok(AppliedRulesBlob::empty()),
        Some(s) => AppliedRulesBlob::decode(&s),
    }
}

fn save_applied_blob(
    kv: &dyn StringKvStore,
    blob: &AppliedRulesBlob,
) -> Result<(), AccountStoreError> {
    kv.set_item(MAIL_RULES_APPLIED_KEY, &blob.encode()?)
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

fn mutate_rules<T>(f: impl FnOnce(&mut MailRulesBlob) -> T) -> Option<T> {
    with_kv(|kv| {
        let mut blob = load_rules_blob(kv)?;
        let out = f(&mut blob);
        save_rules_blob(kv, &blob)?;
        Ok(out)
    })
}

/// Rules for `account_id`, in evaluation order.
pub fn load_rules(account_id: &AccountId) -> Vec<MailRule> {
    with_kv(|kv| Ok(load_rules_blob(kv)?.for_account(account_id))).unwrap_or_default()
}

pub fn save_rule(account_id: AccountId, rule: MailRule) -> Result<MailRule, RuleError> {
    match mutate_rules(|blob| blob.upsert(account_id, rule)) {
        Some(inner) => inner,
        None => Err(RuleError::EmptyMatch),
    }
}

pub fn remove_rule(account_id: &AccountId, rule_id: &str) -> bool {
    mutate_rules(|blob| blob.remove(account_id, rule_id)).unwrap_or(false)
}

pub fn set_rule_enabled(account_id: &AccountId, rule_id: &str, enabled: bool) -> bool {
    mutate_rules(|blob| blob.set_enabled(account_id, rule_id, enabled)).unwrap_or(false)
}

pub fn move_rule(account_id: &AccountId, rule_id: &str, delta: i32) -> bool {
    mutate_rules(|blob| blob.move_rule(account_id, rule_id, delta)).unwrap_or(false)
}

/// Drop rules whose account is no longer known.
pub fn retain_mail_rules(known: &HashSet<AccountId>) {
    let _ = mutate_rules(|blob| blob.retain_accounts(known));
    let _ = with_kv(|kv| {
        let mut blob = load_applied_blob(kv)?;
        blob.retain_accounts(known);
        save_applied_blob(kv, &blob)
    });
}

/// UIDs already processed in this folder.
pub fn load_applied_uids(account_id: &AccountId, mailbox_id: &MailboxId) -> HashSet<String> {
    with_kv(|kv| Ok(load_applied_blob(kv)?.uids(account_id, mailbox_id))).unwrap_or_default()
}

/// Remember UIDs so reopening the folder does not re-apply.
pub fn mark_applied(account_id: &AccountId, mailbox_id: &MailboxId, uids: &[String]) {
    if uids.is_empty() {
        return;
    }
    let _ = with_kv(|kv| {
        let mut blob = load_applied_blob(kv)?;
        blob.mark(account_id.clone(), mailbox_id, uids.iter().cloned());
        save_applied_blob(kv, &blob)
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
    use chrono::{DateTime, Utc};
    use mailiner_core::{AccountId, EmailAddr, FolderId, MessageId};

    fn ts() -> DateTime<Utc> {
        DateTime::from_timestamp(0, 0).unwrap()
    }

    fn envelope(uid: &str, from: &str, to: &str, subject: &str) -> Envelope {
        envelope_full(uid, from, to, subject, false, &[])
    }

    fn envelope_full(
        uid: &str,
        from: &str,
        to: &str,
        subject: &str,
        is_read: bool,
        keywords: &[&str],
    ) -> Envelope {
        Envelope {
            id: MessageId::new(FolderId::new("INBOX"), uid),
            account_id: AccountId::new("acc"),
            folder_id: FolderId::new("INBOX"),
            subject: Some(subject.into()),
            from: Some(EmailAddress::List(vec![EmailAddr {
                name: Some("Ada".into()),
                email: Some(from.into()),
            }])),
            to: Some(EmailAddress::List(vec![EmailAddr {
                name: None,
                email: Some(to.into()),
            }])),
            cc: None,
            bcc: None,
            reply_to: None,
            rfc_message_id: None,
            in_reply_to: None,
            references: vec![],
            date: ts(),
            is_read,
            is_answered: false,
            is_starred: false,
            is_flagged: false,
            is_draft: false,
            is_deleted: false,
            keywords: keywords.iter().map(|s| (*s).to_string()).collect(),
            has_attachments: false,
            size: None,
            snippet: None,
            auth_results: Default::default(),
        }
    }

    fn rule_from(needle: &str) -> MailRule {
        MailRule {
            match_from: needle.into(),
            action_mark_read: true,
            ..MailRule::new()
        }
    }

    #[test]
    fn from_contains_is_case_insensitive() {
        let rule = rule_from("NEWS@Example.COM");
        let env = envelope("1", "news@example.com", "me@example.com", "Hello");
        assert!(rule.matches(&env));
        let miss = envelope("2", "other@example.com", "me@example.com", "Hello");
        assert!(!rule.matches(&miss));
    }

    #[test]
    fn to_and_subject_contains() {
        let to_rule = MailRule {
            match_to: "list@".into(),
            action_star: true,
            ..MailRule::new()
        };
        let subject_rule = MailRule {
            match_subject: "invoice".into(),
            action_flag: true,
            ..MailRule::new()
        };
        let env = envelope("1", "a@b.com", "list@example.com", "Monthly Invoice");
        assert!(to_rule.matches(&env));
        assert!(subject_rule.matches(&env));
        let other = envelope("2", "a@b.com", "me@example.com", "Hello");
        assert!(!to_rule.matches(&other));
        assert!(!subject_rule.matches(&other));
    }

    #[test]
    fn first_matching_rule_wins() {
        let first = MailRule {
            name: "first".into(),
            match_from: "news".into(),
            action_mark_read: true,
            ..MailRule::new()
        };
        let second = MailRule {
            name: "second".into(),
            match_from: "news".into(),
            action_star: true,
            ..MailRule::new()
        };
        let env = envelope("1", "news@example.com", "me@x.com", "Hi");
        let rules = [first.clone(), second];
        let hit = first_matching_rule(&rules, &env).expect("match");
        assert_eq!(hit.id, first.id);
        assert!(hit.action_mark_read);
        assert!(!hit.action_star);
    }

    #[test]
    fn disabled_rules_are_skipped() {
        let disabled = MailRule {
            enabled: false,
            match_from: "news".into(),
            action_mark_read: true,
            ..MailRule::new()
        };
        let enabled = MailRule {
            match_from: "news".into(),
            action_star: true,
            ..MailRule::new()
        };
        let env = envelope("1", "news@example.com", "me@x.com", "Hi");
        assert!(!disabled.matches(&env));
        let rules = [disabled, enabled.clone()];
        let hit = first_matching_rule(&rules, &env).expect("enabled");
        assert_eq!(hit.id, enabled.id);
    }

    #[test]
    fn already_applied_uids_are_skipped() {
        let rule = rule_from("news");
        let env = envelope("42", "news@example.com", "me@x.com", "Hi");
        let applied = HashSet::from(["42".into()]);
        assert!(
            plan_rule_hits(
                std::slice::from_ref(&rule),
                std::slice::from_ref(&env),
                &applied
            )
            .is_empty()
        );
        assert_eq!(
            plan_rule_hits(
                std::slice::from_ref(&rule),
                std::slice::from_ref(&env),
                &HashSet::new()
            )
            .iter()
            .map(|h| h.uid)
            .collect::<Vec<_>>(),
            vec!["42"]
        );
    }

    #[test]
    fn unread_and_keyword_criteria() {
        let unread = MailRule {
            match_unread: true,
            action_mark_read: true,
            ..MailRule::new()
        };
        let keyword = MailRule {
            match_keyword: Some("$Todo".into()),
            action_star: true,
            ..MailRule::new()
        };
        let unread_env = envelope_full("1", "a@b.com", "c@d.com", "s", false, &[]);
        let read_env = envelope_full("2", "a@b.com", "c@d.com", "s", true, &[]);
        let todo_env = envelope_full("3", "a@b.com", "c@d.com", "s", true, &["$todo"]);
        assert!(unread.matches(&unread_env));
        assert!(!unread.matches(&read_env));
        assert!(keyword.matches(&todo_env));
        assert!(!keyword.matches(&unread_env));
    }

    #[test]
    fn empty_match_does_not_match_everything() {
        let rule = MailRule {
            action_mark_read: true,
            ..MailRule::new()
        };
        let env = envelope("1", "a@b.com", "c@d.com", "s");
        assert!(!rule.has_match_criterion());
        assert!(!rule.matches(&env));
    }

    #[test]
    fn blob_upsert_validates_and_roundtrips() {
        let acc = AccountId::new("acc");
        let mut blob = MailRulesBlob::empty();
        let bad = MailRule::new();
        assert_eq!(blob.upsert(acc.clone(), bad), Err(RuleError::EmptyMatch));
        let no_action = MailRule {
            match_from: "x".into(),
            ..MailRule::new()
        };
        assert_eq!(
            blob.upsert(acc.clone(), no_action),
            Err(RuleError::EmptyAction)
        );
        let rule = rule_from("boss");
        blob.upsert(acc.clone(), rule.clone()).expect("ok");
        assert_eq!(blob.for_account(&acc)[0].match_from, "boss");
        let json = blob.encode().expect("encode");
        let back = MailRulesBlob::decode(&json).expect("decode");
        assert_eq!(back, blob);
    }

    #[test]
    fn blob_decode_rejects_future_schema() {
        let err = MailRulesBlob::decode(r#"{"schema_version":99,"rules":{}}"#).unwrap_err();
        match err {
            AccountStoreError::Serialization(msg) => {
                assert!(msg.contains("99"), "{msg}");
            }
            other => panic!("expected Serialization, got {other:?}"),
        }
    }

    #[test]
    fn persist_load_reorder_and_retain() {
        host_kv::reset();
        let acc = AccountId::new("acc");
        let gone = AccountId::new("gone");
        let a = save_rule(acc.clone(), rule_from("a")).expect("a");
        let b = save_rule(acc.clone(), rule_from("b")).expect("b");
        save_rule(gone.clone(), rule_from("z")).expect("z");
        assert_eq!(load_rules(&acc).len(), 2);
        assert!(move_rule(&acc, &b.id, -1));
        let order: Vec<_> = load_rules(&acc).into_iter().map(|r| r.id).collect();
        assert_eq!(order, vec![b.id.clone(), a.id.clone()]);
        assert!(set_rule_enabled(&acc, &a.id, false));
        assert!(
            !load_rules(&acc)
                .iter()
                .find(|r| r.id == a.id)
                .unwrap()
                .enabled
        );
        assert!(remove_rule(&acc, &b.id));
        retain_mail_rules(&HashSet::from([acc.clone()]));
        assert_eq!(load_rules(&acc).len(), 1);
        assert!(load_rules(&gone).is_empty());
        host_kv::reset();
    }

    #[test]
    fn applied_set_roundtrip_and_cap() {
        host_kv::reset();
        let acc = AccountId::new("acc");
        let mb = MailboxId::from("INBOX".to_string());
        mark_applied(&acc, &mb, &["1".into(), "2".into()]);
        let got = load_applied_uids(&acc, &mb);
        assert!(got.contains("1") && got.contains("2"));
        let mut blob = AppliedRulesBlob::empty();
        let many: Vec<String> = (0..=MAX_APPLIED_PER_MAILBOX)
            .map(|i| i.to_string())
            .collect();
        blob.mark(acc.clone(), &mb, many);
        let uids = blob.uids(&acc, &mb);
        assert_eq!(uids.len(), MAX_APPLIED_PER_MAILBOX);
        assert!(!uids.contains("0"));
        assert!(uids.contains(&MAX_APPLIED_PER_MAILBOX.to_string()));
        host_kv::reset();
    }

    #[test]
    fn folder_skip_roles() {
        use mailiner_core::MailboxRole;
        assert!(folder_skips_rules(MailboxRole::Sent));
        assert!(folder_skips_rules(MailboxRole::Drafts));
        assert!(folder_skips_rules(MailboxRole::Trash));
        assert!(!folder_skips_rules(MailboxRole::Inbox));
        assert!(!folder_skips_rules(MailboxRole::Archive));
        assert!(!folder_skips_rules(MailboxRole::Other));
    }
}
