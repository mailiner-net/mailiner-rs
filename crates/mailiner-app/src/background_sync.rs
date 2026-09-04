//! Multi-account background sync policy.
//!
//! The selected account stays in IDLE (or NOOP). Other live sessions are kept
//! (up to [`MAX_CONNECTED_ACCOUNTS`]) and polled with IMAP `STATUS` so unread
//! badges stay current without fetching message lists.

use std::collections::HashMap;

use mailiner_core::ids::{AccountId, FolderId};
use mailiner_core::models::{FolderCounts, MailboxRole};

use crate::account_store::AccountStoreError;
use crate::mail_cache::{CachedFolderTree, MailCache};

/// Live IMAP sessions retained after account switches (selected + background).
pub const MAX_CONNECTED_ACCOUNTS: usize = 4;
/// How often to `STATUS` folders on non-selected connected accounts.
pub const BACKGROUND_STATUS_INTERVAL_MS: u32 = 90_000;

/// How `ensure_connected` treats other live sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnsureConnectedMode {
    /// Connect this account and keep other live sessions.
    ///
    /// After Ready, evict the least-recently used session if over
    /// [`MAX_CONNECTED_ACCOUNTS`]. Used by `SelectAccount`, `ConnectExisting`,
    /// `Bootstrap`, and `Reconnect`.
    Switch,
    /// Trial / first-save connect: never tears down other sessions or evicts.
    ///
    /// Callers (e.g. `CommitNewAccount`) must apply the connection cap only
    /// after **full** success (connect Ready **and** store upsert + set_active_id).
    /// On connect or store failure the prior active session remains intact.
    KeepActiveUntilReady,
}

impl EnsureConnectedMode {
    /// `Switch` used to LOGOUT every other session; it no longer does.
    pub fn disconnects_others(self) -> bool {
        false
    }

    /// Whether a successful connect should evict LRU sessions over the cap.
    pub fn evicts_over_cap(self) -> bool {
        matches!(self, Self::Switch)
    }
}

/// Session ids to LOGOUT so `connected` is at most `cap`, never including `keep`.
///
/// Recency is a monotonic counter (higher = more recently used). Missing entries
/// are treated as oldest. If `keep` would have been evicted, the next-oldest
/// session is dropped instead.
pub fn accounts_to_evict(
    connected: impl IntoIterator<Item = AccountId>,
    recency: &HashMap<AccountId, u64>,
    cap: usize,
    keep: &AccountId,
) -> Vec<AccountId> {
    let mut ids: Vec<AccountId> = connected.into_iter().collect();
    if ids.len() <= cap {
        return Vec::new();
    }
    let overflow = ids.len() - cap;
    ids.sort_by(|a, b| {
        recency
            .get(a)
            .copied()
            .unwrap_or(0)
            .cmp(&recency.get(b).copied().unwrap_or(0))
            .then_with(|| a.as_str().cmp(b.as_str()))
    });
    ids.into_iter()
        .filter(|id| id != keep)
        .take(overflow)
        .collect()
}

/// Inbox folder id from a cached tree, or `INBOX` when the snapshot has none.
///
/// Background polls stay cheap: one `STATUS` per account, no message list fetch.
pub fn background_status_targets(tree: &CachedFolderTree) -> Vec<FolderId> {
    let inbox = tree.folders.iter().find(|f| {
        f.selectable
            && (f.role == MailboxRole::Inbox || f.id.as_str().eq_ignore_ascii_case("INBOX"))
    });
    match inbox {
        Some(folder) => vec![folder.id.clone()],
        None => vec![FolderId::new("INBOX")],
    }
}

/// Copy IMAP `STATUS` totals onto a cached folder tree (no message list fetch).
pub fn apply_status_counts(tree: &mut CachedFolderTree, counts: &HashMap<FolderId, FolderCounts>) {
    for (id, count) in counts {
        tree.counts.insert(id.to_string(), *count);
    }
}

/// Merge `STATUS` totals into a non-selected account's folder snapshot.
///
/// No-op when the account has no cached tree yet (nothing to badge).
pub async fn merge_status_into_cache(
    cache: &dyn MailCache,
    account_id: &AccountId,
    counts: &HashMap<FolderId, FolderCounts>,
) -> Result<(), AccountStoreError> {
    let Some(mut tree) = cache.load_folders(account_id).await? else {
        return Ok(());
    };
    apply_status_counts(&mut tree, counts);
    cache.save_folders(account_id, &tree).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailiner_core::models::{Folder, MailboxRole};

    use crate::mail_cache::InMemoryMailCache;

    fn acc(id: &str) -> AccountId {
        AccountId::new(id)
    }

    fn recency(pairs: &[(&str, u64)]) -> HashMap<AccountId, u64> {
        pairs.iter().map(|(id, n)| (acc(id), *n)).collect()
    }

    fn folder(account: &str, id: &str, subscribed: bool) -> Folder {
        Folder {
            id: FolderId::new(id),
            account_id: AccountId::new(account),
            name: id.into(),
            parent_id: None,
            role: if id.eq_ignore_ascii_case("INBOX") {
                MailboxRole::Inbox
            } else {
                MailboxRole::Other
            },
            selectable: true,
            subscribed,
        }
    }

    #[test]
    fn switch_no_longer_disconnects_others() {
        assert!(!EnsureConnectedMode::Switch.disconnects_others());
        assert!(!EnsureConnectedMode::KeepActiveUntilReady.disconnects_others());
        assert!(EnsureConnectedMode::Switch.evicts_over_cap());
        assert!(!EnsureConnectedMode::KeepActiveUntilReady.evicts_over_cap());

        let recency = recency(&[("a", 1), ("b", 2), ("c", 3)]);
        let evicted = accounts_to_evict(
            [acc("a"), acc("b"), acc("c")],
            &recency,
            MAX_CONNECTED_ACCOUNTS,
            &acc("c"),
        );
        assert!(
            evicted.is_empty(),
            "switch under the cap must keep other sessions: {evicted:?}"
        );
    }

    #[test]
    fn cap_evicts_lru_oldest() {
        let recency = recency(&[("a", 1), ("b", 2), ("c", 3), ("d", 4), ("e", 5)]);
        let evicted = accounts_to_evict(
            [acc("a"), acc("b"), acc("c"), acc("d"), acc("e")],
            &recency,
            MAX_CONNECTED_ACCOUNTS,
            &acc("e"),
        );
        assert_eq!(evicted, vec![acc("a")]);
    }

    #[test]
    fn cap_evicts_next_oldest_when_keep_is_lru() {
        let recency = recency(&[("a", 1), ("b", 2), ("c", 3), ("d", 4), ("e", 5)]);
        let evicted = accounts_to_evict(
            [acc("a"), acc("b"), acc("c"), acc("d"), acc("e")],
            &recency,
            MAX_CONNECTED_ACCOUNTS,
            &acc("a"),
        );
        assert_eq!(evicted, vec![acc("b")]);
        assert!(!evicted.contains(&acc("a")));
    }

    #[test]
    fn cap_evicts_untracked_before_touched() {
        let recency = recency(&[("b", 2), ("c", 3), ("d", 4), ("e", 5)]);
        let evicted = accounts_to_evict(
            [acc("a"), acc("b"), acc("c"), acc("d"), acc("e")],
            &recency,
            MAX_CONNECTED_ACCOUNTS,
            &acc("e"),
        );
        assert_eq!(evicted, vec![acc("a")]);
    }

    #[test]
    fn status_updates_unread_on_non_selected_account() {
        let cache = InMemoryMailCache::new();
        let account = acc("background");
        let tree = CachedFolderTree::new(
            vec![
                folder("background", "INBOX", true),
                folder("background", "Archive", true),
            ],
            HashMap::from([(
                "INBOX".into(),
                FolderCounts {
                    total_messages: 10,
                    unread_messages: 1,
                },
            )]),
        );
        futures_executor::block_on(cache.save_folders(&account, &tree)).unwrap();

        let counts = HashMap::from([
            (
                FolderId::new("INBOX"),
                FolderCounts {
                    total_messages: 12,
                    unread_messages: 4,
                },
            ),
            (
                FolderId::new("Archive"),
                FolderCounts {
                    total_messages: 3,
                    unread_messages: 2,
                },
            ),
        ]);
        futures_executor::block_on(merge_status_into_cache(&cache, &account, &counts)).unwrap();

        let loaded = futures_executor::block_on(cache.load_folders(&account))
            .unwrap()
            .expect("cached tree");
        assert_eq!(
            loaded.counts.get("INBOX").map(|c| c.unread_messages),
            Some(4)
        );
        assert_eq!(
            loaded.counts.get("Archive").map(|c| c.unread_messages),
            Some(2)
        );
        assert_eq!(
            loaded.counts.get("INBOX").map(|c| c.total_messages),
            Some(12)
        );
    }

    #[test]
    fn background_status_targets_inbox_only() {
        let tree = CachedFolderTree::new(
            vec![
                folder("a", "INBOX", true),
                folder("a", "Sent", true),
                folder("a", "Archive", true),
            ],
            HashMap::new(),
        );
        assert_eq!(
            background_status_targets(&tree),
            vec![FolderId::new("INBOX")]
        );

        let empty = CachedFolderTree::new(Vec::new(), HashMap::new());
        assert_eq!(
            background_status_targets(&empty),
            vec![FolderId::new("INBOX")]
        );
    }
}
