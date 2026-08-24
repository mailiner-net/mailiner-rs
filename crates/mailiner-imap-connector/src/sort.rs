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
