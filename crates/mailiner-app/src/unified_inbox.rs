//! Unified inbox: merge recent Inbox mail from every account.
//!
//! The virtual folder is UI-only. Each row still carries its owning
//! `AccountId` + folder-scoped [`MessageId`] so opening a message targets
//! the correct IMAP session.

use mailiner_core::ids::{AccountId, FolderId, MessageId};
use mailiner_core::models::{Envelope, FolderCounts, MailboxRole};

use crate::mail_cache::CachedFolderTree;
use crate::mailbox::MailboxId;
use crate::message::Message;

/// Reserved mailbox id for the virtual All-inboxes view. Never sent to IMAP.
pub const UNIFIED_INBOX_MAILBOX_ID: &str = "__mailiner_unified_inbox__";

/// First-page size fetched per account (not the whole mailbox).
pub const UNIFIED_INBOX_PREFIX: usize = 20;

/// Virtual mailbox used when All inboxes is selected.
pub fn unified_mailbox_id() -> MailboxId {
    MailboxId::from(UNIFIED_INBOX_MAILBOX_ID.to_string())
}

pub fn is_unified_mailbox(id: &MailboxId) -> bool {
    id.as_str() == UNIFIED_INBOX_MAILBOX_ID
}

/// How one account's Inbox prefix was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixSource {
    /// Live `FETCH` of the Inbox prefix.
    Live,
    /// Last cached Inbox prefix (account not connected).
    Cache,
    /// No live session and no cache.
    Skipped,
    /// Live session failed and no usable cache.
    Failed,
}

/// One account's contribution to the unified list.
#[derive(Debug, Clone)]
pub struct AccountInboxPrefix {
    pub account_id: AccountId,
    pub folder_id: FolderId,
    pub envelopes: Vec<Envelope>,
    pub unread: Option<u64>,
    pub source: PrefixSource,
}

/// Muted sidebar/list note for an account that is not live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedInboxNote {
    pub account_id: AccountId,
    pub source: PrefixSource,
}

impl UnifiedInboxNote {
    pub fn message(&self, account_label: &str) -> String {
        match self.source {
            PrefixSource::Cache => format!("{account_label}: showing cached mail"),
            PrefixSource::Skipped => format!("{account_label}: not connected"),
            PrefixSource::Failed => format!("{account_label}: could not load inbox"),
            PrefixSource::Live => String::new(),
        }
    }
}

/// Account + mailbox + UID to SELECT / FETCH when a unified row is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenTarget {
    pub account_id: AccountId,
    pub mailbox_id: MailboxId,
    pub message_id: MessageId,
}

/// Inbox folder id from a cached tree, or `INBOX` when the snapshot has none.
pub fn inbox_folder_id(tree: Option<&CachedFolderTree>) -> FolderId {
    if let Some(tree) = tree
        && let Some(folder) = tree.folders.iter().find(|f| {
            f.selectable
                && (f.role == MailboxRole::Inbox || f.id.as_str().eq_ignore_ascii_case("INBOX"))
        })
    {
        return folder.id.clone();
    }
    FolderId::new("INBOX")
}

/// Inbox `UNSEEN` from a cached folder tree (`None` if the tree has no Inbox count).
pub fn inbox_unread_from_tree(tree: &CachedFolderTree) -> Option<u64> {
    let inbox = inbox_folder_id(Some(tree));
    tree.counts
        .get(inbox.as_str())
        .or_else(|| {
            tree.counts
                .iter()
                .find(|(id, _)| id.eq_ignore_ascii_case("INBOX"))
                .map(|(_, c)| c)
        })
        .map(|c| c.unread_messages)
}

/// Inbox `UNSEEN` from a `STATUS` map (background poll / live folder counts).
pub fn inbox_unread_from_status(
    counts: &std::collections::HashMap<FolderId, FolderCounts>,
) -> Option<u64> {
    counts
        .iter()
        .find(|(id, _)| id.as_str().eq_ignore_ascii_case("INBOX"))
        .map(|(_, c)| c.unread_messages)
}

/// Sum of Inbox UNSEEN across accounts.
pub fn sum_inbox_unread(unread: impl IntoIterator<Item = u64>) -> u64 {
    unread.into_iter().fold(0, u64::saturating_add)
}

/// Newest-first merge of per-account Inbox prefixes.
///
/// Ties: later `date` wins, then account id, then UID (all descending so the
/// sort is stable across equal timestamps).
pub fn merge_inbox_prefixes(
    prefixes: impl IntoIterator<Item = AccountInboxPrefix>,
) -> Vec<Message> {
    let mut rows: Vec<Envelope> = prefixes
        .into_iter()
        .flat_map(|prefix| prefix.envelopes)
        .collect();
    rows.sort_by(|a, b| {
        b.date
            .cmp(&a.date)
            .then_with(|| b.account_id.as_str().cmp(a.account_id.as_str()))
            .then_with(|| b.id.as_uid().cmp(a.id.as_uid()))
    });
    rows.into_iter().map(Message::from).collect()
}

/// Notes for accounts that did not contribute a live prefix.
pub fn notes_from_prefixes(prefixes: &[AccountInboxPrefix]) -> Vec<UnifiedInboxNote> {
    prefixes
        .iter()
        .filter(|p| !matches!(p.source, PrefixSource::Live))
        .map(|p| UnifiedInboxNote {
            account_id: p.account_id.clone(),
            source: p.source,
        })
        .collect()
}

/// IMAP target for a unified (or ordinary) list row.
pub fn open_target(message: &Message) -> OpenTarget {
    OpenTarget {
        account_id: message.envelope.account_id.clone(),
        mailbox_id: MailboxId::from(message.envelope.folder_id.clone()),
        message_id: message.id.clone(),
    }
}

/// Shared account+mailbox when every selected row belongs to the same folder.
///
/// `None` when the set is empty or spans more than one account/mailbox.
pub fn batch_open_target<'a>(
    messages: impl IntoIterator<Item = &'a Message>,
) -> Option<OpenTarget> {
    let mut iter = messages.into_iter();
    let first = open_target(iter.next()?);
    for message in iter {
        let next = open_target(message);
        if next.account_id != first.account_id || next.mailbox_id != first.mailbox_id {
            return None;
        }
    }
    Some(first)
}

/// Case-insensitive match for the All-inboxes tree filter.
pub fn unified_matches_filter(query: &str) -> bool {
    let q = query.trim().to_ascii_lowercase();
    q.is_empty() || "all inboxes".contains(&q) || "unified inbox".contains(&q)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use mailiner_core::models::Folder;
    use std::collections::HashMap;

    fn envelope(account: &str, folder: &str, uid: &str, secs: i64, subject: &str) -> Envelope {
        Envelope {
            id: MessageId::new(FolderId::new(folder), uid),
            account_id: AccountId::new(account),
            folder_id: FolderId::new(folder),
            subject: Some(subject.into()),
            from: None,
            to: None,
            cc: None,
            bcc: None,
            reply_to: None,
            rfc_message_id: None,
            in_reply_to: None,
            references: vec![],
            date: Utc.timestamp_opt(secs, 0).unwrap(),
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

    fn prefix(account: &str, envelopes: Vec<Envelope>, source: PrefixSource) -> AccountInboxPrefix {
        AccountInboxPrefix {
            account_id: AccountId::new(account),
            folder_id: FolderId::new("INBOX"),
            envelopes,
            unread: None,
            source,
        }
    }

    #[test]
    fn merge_sorts_two_accounts_by_date() {
        let a = prefix(
            "work",
            vec![
                envelope("work", "INBOX", "10", 100, "older work"),
                envelope("work", "INBOX", "11", 300, "newest work"),
            ],
            PrefixSource::Live,
        );
        let b = prefix(
            "home",
            vec![
                envelope("home", "INBOX", "2", 200, "middle home"),
                envelope("home", "INBOX", "3", 50, "oldest home"),
            ],
            PrefixSource::Cache,
        );
        let merged = merge_inbox_prefixes([a, b]);
        let subjects: Vec<&str> = merged.iter().map(|m| m.subject.as_str()).collect();
        assert_eq!(
            subjects,
            ["newest work", "middle home", "older work", "oldest home"]
        );
        assert_eq!(merged[0].envelope.account_id.as_str(), "work");
        assert_eq!(merged[1].envelope.account_id.as_str(), "home");
    }

    #[test]
    fn merge_keeps_same_uid_from_two_accounts() {
        let a = prefix(
            "work",
            vec![envelope("work", "INBOX", "12", 100, "work 12")],
            PrefixSource::Live,
        );
        let b = prefix(
            "home",
            vec![envelope("home", "INBOX", "12", 200, "home 12")],
            PrefixSource::Live,
        );
        let merged = merge_inbox_prefixes([a, b]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].id.as_uid(), "12");
        assert_eq!(merged[1].id.as_uid(), "12");
        assert_ne!(merged[0].envelope.account_id, merged[1].envelope.account_id);
    }

    #[test]
    fn unread_sum_adds_inbox_unseen() {
        assert_eq!(sum_inbox_unread([1, 4, 0]), 5);
        assert_eq!(sum_inbox_unread(std::iter::empty()), 0);
        assert_eq!(sum_inbox_unread([u64::MAX, 2]), u64::MAX);
    }

    #[test]
    fn inbox_unread_from_cached_tree() {
        let tree = CachedFolderTree::new(
            vec![Folder {
                id: FolderId::new("INBOX"),
                account_id: AccountId::new("a"),
                name: "INBOX".into(),
                parent_id: None,
                role: MailboxRole::Inbox,
                selectable: true,
                subscribed: true,
            }],
            HashMap::from([(
                "INBOX".into(),
                FolderCounts {
                    total_messages: 10,
                    unread_messages: 3,
                },
            )]),
        );
        assert_eq!(inbox_unread_from_tree(&tree), Some(3));
        assert_eq!(inbox_folder_id(Some(&tree)).as_str(), "INBOX");
        assert_eq!(inbox_folder_id(None).as_str(), "INBOX");
    }

    #[test]
    fn opening_a_row_targets_owning_account_and_uid() {
        let message = Message::from(envelope("work", "INBOX", "42", 1, "hello"));
        let target = open_target(&message);
        assert_eq!(target.account_id.as_str(), "work");
        assert_eq!(target.mailbox_id.as_str(), "INBOX");
        assert_eq!(target.message_id.as_uid(), "42");
        assert_eq!(target.message_id.folder_id().as_str(), "INBOX");
    }

    #[test]
    fn batch_target_requires_one_account_and_folder() {
        let work = Message::from(envelope("work", "INBOX", "1", 1, "a"));
        let work2 = Message::from(envelope("work", "INBOX", "2", 2, "b"));
        let home = Message::from(envelope("home", "INBOX", "1", 3, "c"));
        let same = batch_open_target([&work, &work2]).unwrap();
        assert_eq!(same.account_id.as_str(), "work");
        assert_eq!(same.message_id.as_uid(), "1");
        assert!(batch_open_target([&work, &home]).is_none());
        assert!(batch_open_target(std::iter::empty::<&Message>()).is_none());
    }

    #[test]
    fn notes_skip_live_accounts() {
        let notes = notes_from_prefixes(&[
            prefix("a", vec![], PrefixSource::Live),
            prefix("b", vec![], PrefixSource::Cache),
            prefix("c", vec![], PrefixSource::Skipped),
        ]);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].account_id.as_str(), "b");
        assert_eq!(notes[0].message("Home"), "Home: showing cached mail");
        assert_eq!(notes[1].message("Work"), "Work: not connected");
    }

    #[test]
    fn unified_filter_matches_label() {
        assert!(unified_matches_filter(""));
        assert!(unified_matches_filter("all"));
        assert!(unified_matches_filter("INBOX"));
        assert!(unified_matches_filter("unified"));
        assert!(!unified_matches_filter("sent"));
    }
}
