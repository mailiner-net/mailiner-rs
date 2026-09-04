//! Client-side list-filter helpers.
//!
//! Complements IMAP SEARCH (Unread/Flagged chips) and inspects the sparse
//! list cache for text query and attachment (no portable IMAP SEARCH).

use mailiner_core::MessageListFilter;

use crate::message::{Message, MessageId};

/// True when `query` has at least one non-whitespace word.
pub fn text_filter_is_active(query: &str) -> bool {
    query.split_whitespace().next().is_some()
}

/// Case-insensitive AND of whitespace-separated words against subject or from.
///
/// Each word may hit either field (so `ada invoice` matches Ada's invoice).
/// An empty / whitespace query matches every message.
pub fn message_matches_text_filter(message: &Message, query: &str) -> bool {
    let words = filter_words(query);
    if words.is_empty() {
        return true;
    }
    let subject = message.subject.to_lowercase();
    let from = message.from.to_lowercase();
    words
        .iter()
        .all(|w| subject.contains(w.as_str()) || from.contains(w.as_str()))
}

/// Cached rows that satisfy `query`, preserving source order and indices.
pub fn matching_loaded_messages<'a, I>(items: I, query: &str) -> Vec<(usize, &'a Message)>
where
    I: IntoIterator<Item = (usize, &'a Message)>,
{
    items
        .into_iter()
        .filter(|(_, m)| message_matches_text_filter(m, query))
        .collect()
}

pub fn message_matches_filter(message: &Message, filter: MessageListFilter) -> bool {
    filter.matches(message.is_read, message.is_flagged, message.has_attachments)
}

/// Items that satisfy `filter`, preserving the incoming order and indices.
pub fn matching_messages<'a, I>(items: I, filter: MessageListFilter) -> Vec<(usize, &'a Message)>
where
    I: IntoIterator<Item = (usize, &'a Message)>,
{
    items
        .into_iter()
        .filter(|(_, m)| message_matches_filter(m, filter))
        .collect()
}

/// Next source index when moving by `delta` among `matching` source indices.
///
/// `current` is the focused row's source index. When it is missing from
/// `matching` (hidden by the filter), `delta >= 0` lands on the first match
/// and `delta < 0` on the last.
pub fn adjacent_matching_index(
    matching: &[usize],
    current: Option<usize>,
    delta: i32,
) -> Option<usize> {
    if matching.is_empty() {
        return None;
    }
    let pos = current.and_then(|c| matching.iter().position(|&i| i == c));
    match pos {
        None => {
            if delta >= 0 {
                Some(matching[0])
            } else {
                Some(matching[matching.len() - 1])
            }
        }
        Some(i) => {
            let next = i as i64 + i64::from(delta);
            if next < 0 || next >= matching.len() as i64 {
                None
            } else {
                Some(matching[next as usize])
            }
        }
    }
}

/// Inclusive filtered-row range between two source indices.
///
/// `anchor` / `end` are source-list indices of visible (matching) rows.
/// If `anchor` is not in `matching` (focus was hidden by the filter), the
/// range starts at the first match.
pub fn matching_ids_in_filtered_range(
    matching: &[(usize, MessageId)],
    anchor: usize,
    end: usize,
) -> Vec<MessageId> {
    if matching.is_empty() {
        return Vec::new();
    }
    let pos = |src: usize| matching.iter().position(|(i, _)| *i == src);
    let a = pos(anchor).unwrap_or(0);
    let b = pos(end).unwrap_or(a);
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    matching[lo..=hi].iter().map(|(_, id)| id.clone()).collect()
}

fn filter_words(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use mailiner_core::{AccountId, EmailAddr, EmailAddress, Envelope, FolderId, MessageId};

    fn msg(uid: &str, subject: &str, from_name: &str, from_email: &str) -> Message {
        msg_flags(uid, subject, from_name, from_email, false, false, false)
    }

    fn msg_flags(
        uid: &str,
        subject: &str,
        from_name: &str,
        from_email: &str,
        is_read: bool,
        is_flagged: bool,
        has_attachments: bool,
    ) -> Message {
        let now = DateTime::from_timestamp(0, 0).unwrap();
        let envelope = Envelope {
            id: MessageId::new(FolderId::new("INBOX"), uid),
            account_id: AccountId::new("acc"),
            folder_id: FolderId::new("INBOX"),
            subject: Some(subject.into()),
            from: Some(EmailAddress::List(vec![EmailAddr {
                name: Some(from_name.into()),
                email: Some(from_email.into()),
            }])),
            to: None,
            cc: None,
            bcc: None,
            reply_to: None,
            rfc_message_id: None,
            in_reply_to: None,
            references: vec![],
            date: now,
            is_read,
            is_answered: false,
            is_starred: false,
            is_flagged,
            is_draft: false,
            is_deleted: false,
            has_attachments,
            size: None,
            snippet: None,
            auth_results: Default::default(),
        };
        Message::from(envelope)
    }

    #[test]
    fn empty_query_is_inactive_and_matches_all() {
        assert!(!text_filter_is_active(""));
        assert!(!text_filter_is_active("  \t"));
        let m = msg("1", "Hello", "Ada", "ada@example.com");
        assert!(message_matches_text_filter(&m, ""));
        assert!(message_matches_text_filter(&m, "   "));
    }

    #[test]
    fn substring_is_case_insensitive() {
        let m = msg("1", "Quarterly Invoice", "Ada Lovelace", "ada@example.com");
        assert!(message_matches_text_filter(&m, "invoice"));
        assert!(message_matches_text_filter(&m, "ADA"));
        assert!(message_matches_text_filter(&m, "Example.COM"));
        assert!(!message_matches_text_filter(&m, "bob"));
    }

    #[test]
    fn unicode_case_folding() {
        let m = msg("1", "Überweisung", "Søren", "soren@test.io");
        assert!(message_matches_text_filter(&m, "über"));
        assert!(message_matches_text_filter(&m, "ÜBERWEISUNG"));
        assert!(message_matches_text_filter(&m, "søren"));
        assert!(message_matches_text_filter(&m, "SØREN"));
        assert!(!message_matches_text_filter(&m, "uber"));
    }

    #[test]
    fn words_and_across_subject_and_from() {
        let m = msg("1", "Q3 Invoice", "Ada Lovelace", "ada@example.com");
        assert!(message_matches_text_filter(&m, "ada invoice"));
        assert!(message_matches_text_filter(&m, "lovelace q3"));
        assert!(!message_matches_text_filter(&m, "ada missing"));
    }

    #[test]
    fn matching_preserves_source_indices() {
        let msgs = [
            msg("0", "Hello", "Ada", "ada@example.com"),
            msg("1", "Report", "Bob", "bob@example.com"),
            msg("3", "Hello again", "Cara", "cara@example.com"),
        ];
        let got = matching_loaded_messages(msgs.iter().enumerate(), "hello");
        let ids: Vec<_> = got.iter().map(|(i, m)| (*i, m.subject.as_str())).collect();
        assert_eq!(ids, vec![(0, "Hello"), (2, "Hello again")]);
    }

    #[test]
    fn adjacent_walks_matching_only() {
        let matching = [0usize, 3, 8];
        assert_eq!(adjacent_matching_index(&matching, Some(0), 1), Some(3));
        assert_eq!(adjacent_matching_index(&matching, Some(3), 1), Some(8));
        assert_eq!(adjacent_matching_index(&matching, Some(8), 1), None);
        assert_eq!(adjacent_matching_index(&matching, Some(8), -1), Some(3));
        assert_eq!(adjacent_matching_index(&matching, Some(0), -1), None);
        // Hidden current row: enter the filtered set from the start / end.
        assert_eq!(adjacent_matching_index(&matching, Some(4), 1), Some(0));
        assert_eq!(adjacent_matching_index(&matching, Some(4), -1), Some(8));
        assert_eq!(adjacent_matching_index(&matching, None, 1), Some(0));
        assert!(adjacent_matching_index(&[], Some(0), 1).is_none());
    }

    #[test]
    fn filtered_range_uses_visible_order() {
        let msgs = [
            msg("0", "Alpha", "Ada", "ada@test.io"),
            msg("1", "Beta", "Bob", "bob@test.io"),
            msg("3", "Alpha two", "Cara", "cara@test.io"),
            msg("4", "Alpha three", "Dan", "dan@test.io"),
        ];
        let matching: Vec<(usize, MessageId)> =
            matching_loaded_messages(msgs.iter().enumerate(), "alpha")
                .into_iter()
                .map(|(i, m)| (i, m.id.clone()))
                .collect();
        // Source indices of the Alpha rows are 0, 2, 3 (Beta/Bob is hidden).
        let ids = matching_ids_in_filtered_range(&matching, 0, 3);
        assert_eq!(
            ids,
            vec![msgs[0].id.clone(), msgs[2].id.clone(), msgs[3].id.clone()]
        );
        let reverse = matching_ids_in_filtered_range(&matching, 3, 0);
        assert_eq!(reverse, ids);
        // Hidden anchor falls back to the first match.
        let from_hidden = matching_ids_in_filtered_range(&matching, 1, 2);
        assert_eq!(from_hidden, vec![msgs[0].id.clone(), msgs[2].id.clone()]);
    }

    #[test]
    fn empty_filter_keeps_order() {
        let msgs = [
            msg_flags("0", "0", "A", "a@x", false, false, false),
            msg_flags("1", "1", "B", "b@x", true, true, false),
            msg_flags("3", "3", "C", "c@x", false, true, true),
        ];
        let items = msgs.iter().enumerate();
        let got = matching_messages(items, MessageListFilter::default());
        let ids: Vec<_> = got.iter().map(|(i, m)| (*i, m.subject.as_str())).collect();
        assert_eq!(ids, vec![(0, "0"), (1, "1"), (2, "3")]);
    }

    #[test]
    fn unread_and_attachment_and() {
        let msgs = [
            msg_flags("0", "0", "A", "a@x", false, false, false),
            msg_flags("1", "1", "B", "b@x", true, true, false),
            msg_flags("3", "3", "C", "c@x", false, true, true),
            msg_flags("4", "4", "D", "d@x", true, false, true),
        ];
        let filter = MessageListFilter {
            unread: true,
            has_attachment: true,
            ..MessageListFilter::default()
        };
        let got = matching_messages(msgs.iter().enumerate(), filter);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].1.subject, "3");
        assert!(!got[0].1.is_read);
        assert!(got[0].1.has_attachments);
    }

    #[test]
    fn flagged_only() {
        let msgs = [
            msg_flags("0", "0", "A", "a@x", false, false, false),
            msg_flags("1", "1", "B", "b@x", true, true, false),
            msg_flags("3", "3", "C", "c@x", false, true, true),
        ];
        let filter = MessageListFilter {
            flagged: true,
            ..MessageListFilter::default()
        };
        let subjects: Vec<_> = matching_messages(msgs.iter().enumerate(), filter)
            .into_iter()
            .map(|(_, m)| m.subject.as_str())
            .collect();
        assert_eq!(subjects, vec!["1", "3"]);
    }
}
