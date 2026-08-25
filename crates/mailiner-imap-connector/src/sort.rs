//! Folder list order: IMAP `SORT` / `SEARCH`, plus a sequence fallback for date.

use std::collections::HashSet;
use std::fmt::Debug;

use async_imap::Session;
use imap_proto::{MailboxDatum, Response, Status};
use mailiner_core::MessageSort;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::ImapError;

/// Requested sort, or Date when Size/Sender need `SORT` the server does not have.
pub fn apply_sort_or_fallback(requested: MessageSort, has_sort: bool) -> MessageSort {
    if requested.needs_sort_capability() && !has_sort {
        MessageSort::Date
    } else {
        requested
    }
}

/// Unseen UIDs first, then seen. Each group newest-first (descending UID ≈ arrival).
pub fn unread_uid_order(unseen: HashSet<u32>, seen: HashSet<u32>) -> Vec<u32> {
    let mut unseen: Vec<u32> = unseen.into_iter().collect();
    let mut seen: Vec<u32> = seen.into_iter().collect();
    unseen.sort_unstable_by(|a, b| b.cmp(a));
    seen.sort_unstable_by(|a, b| b.cmp(a));
    unseen.extend(seen);
    unseen
}

/// Move `uid` between the unseen prefix and the seen suffix of an unread-first index.
///
/// `unread` is the length of the unseen prefix. Returns `(old_index, new_index)`.
pub fn move_uid_for_seen_flag(
    uids: &mut Vec<u32>,
    unread: &mut usize,
    uid: u32,
    now_read: bool,
) -> Option<(usize, usize)> {
    let from = uids.iter().position(|&u| u == uid)?;
    uids.remove(from);
    if from < *unread {
        *unread = unread.saturating_sub(1);
    }
    let dest = if now_read {
        *unread..uids.len()
    } else {
        0..*unread
    };
    let to = insert_uid_desc(uids, uid, dest);
    if !now_read {
        *unread += 1;
    }
    Some((from, to))
}

fn insert_uid_desc(uids: &mut Vec<u32>, uid: u32, range: std::ops::Range<usize>) -> usize {
    let pos = uids[range.clone()]
        .iter()
        .position(|&u| u < uid)
        .map(|i| range.start + i)
        .unwrap_or(range.end);
    uids.insert(pos, uid);
    pos
}

/// IMAP SORT criteria + search key for a sort that needs the SORT extension.
pub fn sort_command(sort: MessageSort) -> Option<(&'static str, &'static str)> {
    match sort {
        MessageSort::Size => Some(("REVERSE SIZE", "ALL")),
        MessageSort::Sender => Some(("FROM", "ALL")),
        MessageSort::Date | MessageSort::Unread => None,
    }
}

/// `UID SORT (criteria) UTF-8 query` → UIDs in sort order.
pub async fn uid_sort<S>(
    session: &mut Session<S>,
    criteria: &str,
    query: &str,
) -> Result<Vec<u32>, ImapError>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    let command = format!("UID SORT ({criteria}) UTF-8 {query}");
    let tag = session
        .run_command(&command)
        .await
        .map_err(|e| ImapError::Imap(format!("Failed to run {command}: {e}")))?;
    let mut ids = Vec::new();
    loop {
        let resp = session
            .read_response()
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to read SORT response: {e}")))?
            .ok_or_else(|| ImapError::Imap("IMAP connection closed during SORT".into()))?;
        match resp.parsed() {
            Response::MailboxData(MailboxDatum::Sort(cs)) => {
                ids.extend(cs.iter().copied());
            }
            Response::Done {
                tag: done_tag,
                status,
                information,
                ..
            } if done_tag == &tag => {
                return match status {
                    Status::Ok => Ok(ids),
                    _ => Err(ImapError::Imap(format!(
                        "UID SORT failed ({status:?}): {information:?}"
                    ))),
                };
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_only_when_sort_required() {
        assert_eq!(
            apply_sort_or_fallback(MessageSort::Size, false),
            MessageSort::Date
        );
        assert_eq!(
            apply_sort_or_fallback(MessageSort::Sender, true),
            MessageSort::Sender
        );
        assert_eq!(
            apply_sort_or_fallback(MessageSort::Unread, false),
            MessageSort::Unread
        );
        assert_eq!(
            apply_sort_or_fallback(MessageSort::Date, false),
            MessageSort::Date
        );
    }

    #[test]
    fn unread_order_unseen_desc_then_seen_desc() {
        let unseen = HashSet::from([3, 10, 1]);
        let seen = HashSet::from([8, 2]);
        assert_eq!(unread_uid_order(unseen, seen), vec![10, 3, 1, 8, 2]);
    }

    #[test]
    fn mark_read_moves_into_seen_group_by_uid() {
        let mut uids = vec![10, 3, 1, 8, 2];
        let mut unread = 3;
        assert_eq!(
            move_uid_for_seen_flag(&mut uids, &mut unread, 3, true),
            Some((1, 3))
        );
        assert_eq!(unread, 2);
        assert_eq!(uids, vec![10, 1, 8, 3, 2]);
    }

    #[test]
    fn mark_unread_moves_into_unseen_group_by_uid() {
        let mut uids = vec![10, 3, 1, 8, 2];
        let mut unread = 3;
        assert_eq!(
            move_uid_for_seen_flag(&mut uids, &mut unread, 8, false),
            Some((3, 1))
        );
        assert_eq!(unread, 4);
        assert_eq!(uids, vec![10, 8, 3, 1, 2]);
    }

    #[test]
    fn mark_unread_first_unseen_when_none() {
        let mut uids = vec![8, 2];
        let mut unread = 0;
        assert_eq!(
            move_uid_for_seen_flag(&mut uids, &mut unread, 8, false),
            Some((0, 0))
        );
        assert_eq!(unread, 1);
        assert_eq!(uids, vec![8, 2]);
    }

    #[test]
    fn sort_command_only_for_size_sender() {
        assert!(sort_command(MessageSort::Date).is_none());
        assert!(sort_command(MessageSort::Unread).is_none());
        assert_eq!(
            sort_command(MessageSort::Size),
            Some(("REVERSE SIZE", "ALL"))
        );
        assert_eq!(sort_command(MessageSort::Sender), Some(("FROM", "ALL")));
    }
}
