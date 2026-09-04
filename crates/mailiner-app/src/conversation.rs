//! Client-side conversation grouping for the currently loaded folder.
//!
//! Thread id is the root Message-ID found by walking `In-Reply-To` and
//! `References` until no parent is in the loaded set. A message with no
//! walkable parent uses its own Message-ID, or a singleton keyed by IMAP UID.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::message::{Message, MessageId};

/// Stable id for one conversation in the open folder.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConversationId(String);

impl ConversationId {
    fn from_rfc_message_id(mid: &str) -> Self {
        Self(format!("mid:{mid}"))
    }

    fn from_imap_uid(uid: &str) -> Self {
        Self(format!("uid:{uid}"))
    }
}

/// Messages that share a thread root, members oldest → newest.
#[derive(Debug, Clone, PartialEq)]
pub struct Conversation {
    pub id: ConversationId,
    pub members: Vec<Arc<Message>>,
}

impl Conversation {
    pub fn count(&self) -> usize {
        self.members.len()
    }

    pub fn unread_count(&self) -> usize {
        self.members.iter().filter(|m| !m.is_read).count()
    }

    pub fn newest(&self) -> &Arc<Message> {
        self.members
            .last()
            .expect("conversation has at least one member")
    }

    /// Newest unread member, else the newest member.
    pub fn open_target(&self) -> &Arc<Message> {
        self.members
            .iter()
            .rev()
            .find(|m| !m.is_read)
            .unwrap_or_else(|| self.newest())
    }

    pub fn contains(&self, id: &MessageId) -> bool {
        self.members.iter().any(|m| m.id == *id)
    }
}

/// One virtualized row in conversation view.
#[derive(Debug, Clone, PartialEq)]
pub enum ConversationRow {
    /// Multi-message thread header (count badge + expand).
    Thread {
        conversation: Conversation,
        expanded: bool,
    },
    /// Singleton, or a member of an expanded thread.
    Message {
        conversation_id: ConversationId,
        message: Arc<Message>,
        indented: bool,
    },
}

impl ConversationRow {
    /// Message to select when this row is activated.
    pub fn select_target(&self) -> &Arc<Message> {
        match self {
            Self::Thread { conversation, .. } => conversation.open_target(),
            Self::Message { message, .. } => message,
        }
    }

    pub fn is_unread(&self) -> bool {
        match self {
            Self::Thread { conversation, .. } => conversation.unread_count() > 0,
            Self::Message { message, .. } => !message.is_read,
        }
    }

    /// Message ids this row represents for range selection.
    pub fn selected_ids(&self) -> Vec<MessageId> {
        match self {
            Self::Thread { conversation, .. } => {
                conversation.members.iter().map(|m| m.id.clone()).collect()
            }
            Self::Message { message, .. } => vec![message.id.clone()],
        }
    }
}

/// Group loaded folder messages into conversations.
///
/// Conversations are ordered by newest member date (then IMAP UID), then
/// pinned conversations (any pinned member) are moved to the front in pin
/// order.
pub fn group_conversations(
    messages: impl IntoIterator<Item = Arc<Message>>,
    pinned_uids: &[String],
) -> Vec<Conversation> {
    let messages: Vec<Arc<Message>> = messages.into_iter().collect();
    if messages.is_empty() {
        return Vec::new();
    }

    let mut by_mid: HashMap<String, usize> = HashMap::new();
    for (idx, msg) in messages.iter().enumerate() {
        if let Some(mid) = normalize_mid_opt(msg.envelope.rfc_message_id.as_deref()) {
            by_mid.entry(mid).or_insert(idx);
        }
    }

    let mut parent: Vec<usize> = (0..messages.len()).collect();
    let mut rank = vec![0u8; messages.len()];
    for (idx, msg) in messages.iter().enumerate() {
        for cand in parent_candidates(&msg.envelope.in_reply_to, &msg.envelope.references) {
            if let Some(&other) = by_mid.get(&cand) {
                uf_union(&mut parent, &mut rank, idx, other);
                break;
            }
        }
    }

    let mut buckets: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut order: Vec<usize> = Vec::new();
    for idx in 0..messages.len() {
        let root = uf_find(&mut parent, idx);
        buckets.entry(root).or_insert_with(|| {
            order.push(root);
            Vec::new()
        });
        if let Some(members) = buckets.get_mut(&root) {
            members.push(idx);
        }
    }

    let mut conversations: Vec<Conversation> = order
        .into_iter()
        .filter_map(|root| {
            let idxs = buckets.remove(&root)?;
            let mut members: Vec<Arc<Message>> =
                idxs.into_iter().map(|i| Arc::clone(&messages[i])).collect();
            members.sort_by(|a, b| {
                a.date
                    .cmp(&b.date)
                    .then_with(|| uid_ord(a).cmp(&uid_ord(b)))
            });
            let id = conversation_id_for(&members);
            Some(Conversation { id, members })
        })
        .collect();

    conversations.sort_by(|a, b| {
        b.newest()
            .date
            .cmp(&a.newest().date)
            .then_with(|| uid_ord(b.newest()).cmp(&uid_ord(a.newest())))
    });
    sort_conversations_pinned_first(&mut conversations, pinned_uids);
    conversations
}

/// Flatten conversations into list rows. Expanded threads list members
/// oldest → newest under the header.
pub fn flatten_conversations(
    conversations: &[Conversation],
    expanded: &HashSet<ConversationId>,
) -> Vec<ConversationRow> {
    let mut rows = Vec::new();
    for conversation in conversations {
        if conversation.count() <= 1 {
            let message = Arc::clone(conversation.newest());
            rows.push(ConversationRow::Message {
                conversation_id: conversation.id.clone(),
                message,
                indented: false,
            });
            continue;
        }
        let is_expanded = expanded.contains(&conversation.id);
        rows.push(ConversationRow::Thread {
            conversation: conversation.clone(),
            expanded: is_expanded,
        });
        if is_expanded {
            for message in &conversation.members {
                rows.push(ConversationRow::Message {
                    conversation_id: conversation.id.clone(),
                    message: Arc::clone(message),
                    indented: true,
                });
            }
        }
    }
    rows
}

pub fn conversation_for_message<'a>(
    conversations: &'a [Conversation],
    message_id: &MessageId,
) -> Option<&'a Conversation> {
    conversations.iter().find(|c| c.contains(message_id))
}

/// Visible-row index of `message_id`. Expanded members win over the header.
pub fn row_index_for_message(rows: &[ConversationRow], message_id: &MessageId) -> Option<usize> {
    let mut header = None;
    for (idx, row) in rows.iter().enumerate() {
        match row {
            ConversationRow::Thread {
                conversation,
                expanded,
            } => {
                if conversation.contains(message_id) {
                    if !*expanded {
                        return Some(idx);
                    }
                    header = Some(idx);
                }
            }
            ConversationRow::Message { message, .. } => {
                if message.id == *message_id {
                    return Some(idx);
                }
            }
        }
    }
    header
}

/// Strip whitespace and surrounding angle brackets from a Message-ID.
pub fn normalize_mid(raw: &str) -> String {
    let trimmed = raw.trim();
    let inner = trimmed
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(trimmed)
        .trim();
    inner.to_string()
}

fn normalize_mid_opt(raw: Option<&str>) -> Option<String> {
    let mid = normalize_mid(raw?);
    if mid.is_empty() { None } else { Some(mid) }
}

fn uf_find(parent: &mut [usize], i: usize) -> usize {
    let mut cur = i;
    while parent[cur] != cur {
        let p = parent[cur];
        parent[cur] = parent[p];
        cur = p;
    }
    cur
}

fn uf_union(parent: &mut [usize], rank: &mut [u8], a: usize, b: usize) {
    let mut ra = uf_find(parent, a);
    let mut rb = uf_find(parent, b);
    if ra == rb {
        return;
    }
    if rank[ra] < rank[rb] {
        std::mem::swap(&mut ra, &mut rb);
    }
    parent[rb] = ra;
    if rank[ra] == rank[rb] {
        rank[ra] = rank[ra].saturating_add(1);
    }
}

/// Root Message-ID when one member has no in-folder parent, else the
/// oldest member's Message-ID, else a singleton IMAP UID.
fn conversation_id_for(members: &[Arc<Message>]) -> ConversationId {
    let in_folder: HashSet<String> = members
        .iter()
        .filter_map(|m| normalize_mid_opt(m.envelope.rfc_message_id.as_deref()))
        .collect();
    let root = members
        .iter()
        .find(|m| {
            parent_candidates(&m.envelope.in_reply_to, &m.envelope.references)
                .iter()
                .all(|cand| !in_folder.contains(cand))
        })
        .unwrap_or(&members[0]);
    if let Some(mid) = normalize_mid_opt(root.envelope.rfc_message_id.as_deref()) {
        return ConversationId::from_rfc_message_id(&mid);
    }
    if let Some(mid) = members
        .iter()
        .find_map(|m| normalize_mid_opt(m.envelope.rfc_message_id.as_deref()))
    {
        return ConversationId::from_rfc_message_id(&mid);
    }
    ConversationId::from_imap_uid(members[0].id.as_uid())
}

/// Parent ids to walk: `In-Reply-To`, then `References` newest → oldest.
///
/// Newest-first on References finds the immediate parent when the root is
/// not in the folder; a single References value still satisfies
/// references-only linking.
fn parent_candidates(in_reply_to: &Option<String>, references: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(mid) = normalize_mid_opt(in_reply_to.as_deref()) {
        out.push(mid);
    }
    for raw in references.iter().rev() {
        if let Some(mid) = normalize_mid_opt(Some(raw))
            && !out.contains(&mid)
        {
            out.push(mid);
        }
    }
    out
}

fn uid_ord(message: &Message) -> (u64, &str) {
    let uid = message.id.as_uid();
    (uid.parse::<u64>().unwrap_or(0), uid)
}

fn sort_conversations_pinned_first(conversations: &mut [Conversation], pinned_uids: &[String]) {
    if conversations.len() < 2 || pinned_uids.is_empty() {
        return;
    }
    let mut used = vec![false; conversations.len()];
    let mut next = Vec::with_capacity(conversations.len());
    for uid in pinned_uids {
        if let Some(idx) = conversations
            .iter()
            .enumerate()
            .find(|(i, c)| !used[*i] && c.members.iter().any(|m| m.id.as_uid() == uid.as_str()))
        {
            used[idx.0] = true;
            next.push(conversations[idx.0].clone());
        }
    }
    if next.is_empty() {
        return;
    }
    for (i, keep) in used.iter().enumerate() {
        if !keep {
            next.push(conversations[i].clone());
        }
    }
    conversations.clone_from_slice(&next);
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use mailiner_core::{AccountId, Envelope, FolderId, MessageId};

    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    fn msg(
        uid: &str,
        mid: Option<&str>,
        in_reply_to: Option<&str>,
        references: &[&str],
        date: i64,
        unread: bool,
    ) -> Arc<Message> {
        let envelope = Envelope {
            id: MessageId::new(FolderId::new("INBOX"), uid),
            account_id: AccountId::new("acc"),
            folder_id: FolderId::new("INBOX"),
            subject: Some(format!("s{uid}")),
            from: None,
            to: None,
            cc: None,
            bcc: None,
            reply_to: None,
            rfc_message_id: mid.map(str::to_string),
            in_reply_to: in_reply_to.map(str::to_string),
            references: references.iter().map(|s| (*s).to_string()).collect(),
            date: ts(date),
            is_read: !unread,
            is_answered: false,
            is_starred: false,
            is_flagged: false,
            is_draft: false,
            is_deleted: false,
            keywords: Vec::new(),
            has_attachments: false,
            size: None,
            snippet: None,
            auth_results: Default::default(),
        };
        Arc::new(Message::from(envelope))
    }

    fn ids(conv: &Conversation) -> Vec<&str> {
        conv.members.iter().map(|m| m.id.as_uid()).collect()
    }

    fn grouped(messages: Vec<Arc<Message>>) -> Vec<Conversation> {
        group_conversations(messages, &[])
    }

    #[test]
    fn linear_reply_chain_is_one_conversation() {
        let a = msg("1", Some("<a@x>"), None, &[], 10, false);
        let b = msg("2", Some("<b@x>"), Some("<a@x>"), &["<a@x>"], 20, true);
        let c = msg(
            "3",
            Some("<c@x>"),
            Some("<b@x>"),
            &["<a@x>", "<b@x>"],
            30,
            true,
        );
        let convs = grouped(vec![c, b, a]);
        assert_eq!(convs.len(), 1);
        assert_eq!(ids(&convs[0]), vec!["1", "2", "3"]);
        assert_eq!(convs[0].unread_count(), 2);
        assert_eq!(convs[0].open_target().id.as_uid(), "3");
        assert_eq!(convs[0].newest().id.as_uid(), "3");
    }

    #[test]
    fn missing_parent_keeps_walkable_descendants_together() {
        let orphan = msg("1", Some("<a@x>"), Some("<missing@x>"), &[], 10, false);
        let reply = msg("2", Some("<b@x>"), Some("<a@x>"), &["<a@x>"], 20, true);
        let other = msg("3", Some("<c@x>"), Some("<gone@x>"), &[], 30, false);
        let convs = grouped(vec![other, reply, orphan]);
        assert_eq!(convs.len(), 2);
        assert_eq!(ids(&convs[0]), vec!["3"]);
        assert_eq!(ids(&convs[1]), vec!["1", "2"]);
    }

    #[test]
    fn two_roots_are_separate_conversations() {
        let a = msg("1", Some("<a@x>"), None, &[], 10, false);
        let b = msg("2", Some("<b@x>"), None, &[], 40, false);
        let convs = grouped(vec![b.clone(), a]);
        assert_eq!(convs.len(), 2);
        assert_eq!(ids(&convs[0]), vec!["2"]);
        assert_eq!(ids(&convs[1]), vec!["1"]);
        assert_eq!(convs[0].newest().date, ts(40));
    }

    #[test]
    fn references_only_link_joins_the_parent() {
        let root = msg("1", Some("<root@x>"), None, &[], 10, false);
        let reply = msg("2", Some("<r@x>"), None, &["<root@x>"], 20, true);
        let convs = grouped(vec![reply, root]);
        assert_eq!(convs.len(), 1);
        assert_eq!(ids(&convs[0]), vec!["1", "2"]);
    }

    #[test]
    fn singleton_without_message_id() {
        let lone = msg("99", None, None, &[], 10, false);
        let convs = grouped(vec![lone]);
        assert_eq!(convs.len(), 1);
        assert_eq!(ids(&convs[0]), vec!["99"]);
        assert_eq!(convs[0].id, ConversationId::from_imap_uid("99"));
    }

    #[test]
    fn normalize_mid_strips_brackets() {
        assert_eq!(normalize_mid(" <id@x> "), "id@x");
        assert_eq!(normalize_mid("id@x"), "id@x");
        assert!(normalize_mid_opt(Some("  ")).is_none());
    }

    #[test]
    fn flatten_expands_members_oldest_to_newest() {
        let a = msg("1", Some("<a@x>"), None, &[], 10, false);
        let b = msg("2", Some("<b@x>"), Some("<a@x>"), &["<a@x>"], 20, true);
        let convs = grouped(vec![b, a]);
        let collapsed = flatten_conversations(&convs, &HashSet::new());
        assert_eq!(collapsed.len(), 1);
        assert!(matches!(
            &collapsed[0],
            ConversationRow::Thread {
                expanded: false,
                ..
            }
        ));

        let mut open = HashSet::new();
        open.insert(convs[0].id.clone());
        let rows = flatten_conversations(&convs, &open);
        assert_eq!(rows.len(), 3);
        match &rows[0] {
            ConversationRow::Thread { expanded, .. } => assert!(*expanded),
            other => panic!("expected header, got {other:?}"),
        }
        match &rows[1] {
            ConversationRow::Message {
                message, indented, ..
            } => {
                assert_eq!(message.id.as_uid(), "1");
                assert!(*indented);
            }
            other => panic!("expected first member, got {other:?}"),
        }
        match &rows[2] {
            ConversationRow::Message { message, .. } => {
                assert_eq!(message.id.as_uid(), "2");
            }
            other => panic!("expected second member, got {other:?}"),
        }
        assert_eq!(
            row_index_for_message(&rows, &convs[0].members[1].id),
            Some(2)
        );
        assert_eq!(
            row_index_for_message(&collapsed, &convs[0].members[0].id),
            Some(0)
        );
    }

    #[test]
    fn open_target_prefers_newest_unread() {
        let a = msg("1", Some("<a@x>"), None, &[], 10, true);
        let b = msg("2", Some("<b@x>"), Some("<a@x>"), &["<a@x>"], 20, false);
        let c = msg(
            "3",
            Some("<c@x>"),
            Some("<b@x>"),
            &["<a@x>", "<b@x>"],
            30,
            false,
        );
        let convs = grouped(vec![c, b, a]);
        assert_eq!(convs[0].open_target().id.as_uid(), "1");
    }

    #[test]
    fn pinned_conversation_sorts_first() {
        let a = msg("1", Some("<a@x>"), None, &[], 10, false);
        let b = msg("2", Some("<b@x>"), None, &[], 40, false);
        let convs = group_conversations(vec![b, a], &["1".into()]);
        assert_eq!(ids(&convs[0]), vec!["1"]);
        assert_eq!(ids(&convs[1]), vec!["2"]);
    }

    #[test]
    fn cycle_does_not_loop() {
        let a = msg("1", Some("<a@x>"), Some("<b@x>"), &["<b@x>"], 10, false);
        let b = msg("2", Some("<b@x>"), Some("<a@x>"), &["<a@x>"], 20, false);
        let convs = grouped(vec![a, b]);
        assert_eq!(convs.len(), 1);
        assert_eq!(convs[0].count(), 2);
    }
}
