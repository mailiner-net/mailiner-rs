use std::collections::HashMap;

use mailiner_core::{Folder, FolderId, MailboxRole};

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct MailboxId(String);

impl From<String> for MailboxId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

impl From<FolderId> for MailboxId {
    fn from(id: FolderId) -> Self {
        Self(id.to_string())
    }
}

impl ToString for MailboxId {
    fn to_string(&self) -> String {
        self.0.clone()
    }
}

pub struct MailboxNode {
    pub id: MailboxId,
    pub name: String,
    pub parent: Option<MailboxId>,
    pub children: Vec<MailboxId>,
    pub unread_count: usize,
    pub total_count: usize,
    pub role: MailboxRole,
}

impl MailboxNode {
    /// Special-use label when we know the role, otherwise the server name.
    pub fn title(&self) -> &str {
        self.role.label().unwrap_or(self.name.as_str())
    }
}

impl From<Folder> for MailboxNode {
    fn from(folder: Folder) -> Self {
        Self {
            id: folder.id.into(),
            name: folder.name,
            parent: folder.parent_id.map(|id| id.into()),
            children: vec![],
            unread_count: 0,
            total_count: 0,
            role: folder.role,
        }
    }
}

/// Inbox, Drafts, Sent, Outbox, Trash, then remaining names A–Z.
pub fn build_mailbox_tree(folders: Vec<Folder>) -> (Vec<MailboxId>, HashMap<MailboxId, MailboxNode>) {
    let mut root_ids = Vec::new();
    let mut mboxes = HashMap::<MailboxId, MailboxNode>::new();

    for folder in folders {
        let mailbox_id: MailboxId = folder.id.clone().into();
        let role = folder.role;
        mboxes
            .entry(mailbox_id.clone())
            .and_modify(|node| {
                node.parent = folder.parent_id.as_ref().map(|id| id.clone().into());
                node.name = folder.name.clone();
                node.role = role;
            })
            .or_insert(MailboxNode {
                id: mailbox_id.clone(),
                name: folder.name.clone(),
                parent: folder.parent_id.as_ref().map(|id| id.clone().into()),
                children: vec![],
                unread_count: 0,
                total_count: 0,
                role,
            });
        mboxes.insert(mailbox_id.clone(), folder.clone().into());
        if let Some(parent_id) = folder.parent_id.clone() {
            mboxes
                .entry(parent_id.clone().into())
                .or_insert(MailboxNode {
                    id: parent_id.clone().into(),
                    name: parent_id.to_string(),
                    parent: None,
                    children: vec![],
                    unread_count: 0,
                    total_count: 0,
                    role: MailboxRole::Other,
                })
                .children
                .push(mailbox_id);
        } else {
            root_ids.push(mailbox_id);
        }
    }

    sort_mailbox_ids(&mut root_ids, &mboxes);
    let keys: Vec<MailboxId> = mboxes.keys().cloned().collect();
    for key in keys {
        let mut children = mboxes
            .get_mut(&key)
            .map(|n| std::mem::take(&mut n.children))
            .unwrap_or_default();
        sort_mailbox_ids(&mut children, &mboxes);
        if let Some(node) = mboxes.get_mut(&key) {
            node.children = children;
        }
    }

    (root_ids, mboxes)
}

/// First mailbox with `role`, if any.
pub fn find_mailbox_with_role(
    nodes: &HashMap<MailboxId, MailboxNode>,
    role: MailboxRole,
) -> Option<MailboxId> {
    nodes
        .iter()
        .find(|(_, n)| n.role == role)
        .map(|(id, _)| id.clone())
}

/// One mailbox in tree order, with a display path for filtering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailboxEntry {
    pub id: MailboxId,
    pub title: String,
    pub name: String,
    pub path: String,
    pub depth: usize,
    pub role: MailboxRole,
}

/// Depth-first list of every selectable mailbox.
pub fn collect_mailbox_entries(
    roots: &[MailboxId],
    nodes: &HashMap<MailboxId, MailboxNode>,
) -> Vec<MailboxEntry> {
    let mut out = Vec::new();
    fn walk(
        id: &MailboxId,
        depth: usize,
        parent_path: &str,
        nodes: &HashMap<MailboxId, MailboxNode>,
        out: &mut Vec<MailboxEntry>,
    ) {
        let Some(node) = nodes.get(id) else {
            return;
        };
        let title = node.title().to_string();
        let path = if parent_path.is_empty() {
            title.clone()
        } else {
            format!("{parent_path} / {title}")
        };
        out.push(MailboxEntry {
            id: id.clone(),
            title: title.clone(),
            name: node.name.clone(),
            path: path.clone(),
            depth,
            role: node.role,
        });
        for child in &node.children {
            walk(child, depth + 1, &path, nodes, out);
        }
    }
    for root in roots {
        walk(root, 0, "", nodes, &mut out);
    }
    out
}

/// Case-insensitive AND of whitespace-separated words against title, path, and id.
/// Rank: exact title, title prefix, title contains, path/id contains. Tree order breaks ties.
pub fn filter_mailbox_entries<'a>(
    entries: &'a [MailboxEntry],
    query: &str,
) -> Vec<&'a MailboxEntry> {
    let words: Vec<String> = query
        .split_whitespace()
        .map(|w| w.to_ascii_lowercase())
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return entries.iter().collect();
    }
    let mut scored: Vec<(u8, usize, &MailboxEntry)> = Vec::new();
    for (ord, entry) in entries.iter().enumerate() {
        if let Some(rank) = mailbox_match_rank(entry, &words) {
            scored.push((rank, ord, entry));
        }
    }
    scored.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, e)| e).collect()
}

fn mailbox_match_rank(entry: &MailboxEntry, words: &[String]) -> Option<u8> {
    let title = entry.title.to_ascii_lowercase();
    let name = entry.name.to_ascii_lowercase();
    let path = entry.path.to_ascii_lowercase();
    let id = entry.id.to_string().to_ascii_lowercase();
    let joined = query_join(words);
    if title == joined || name == joined {
        return Some(0);
    }
    if title.starts_with(&joined) || name.starts_with(&joined) {
        return Some(1);
    }
    if contains_all(&title, words) || contains_all(&name, words) {
        return Some(2);
    }
    if contains_all(&path, words) || contains_all(&id, words) {
        return Some(3);
    }
    None
}

fn query_join(words: &[String]) -> String {
    words.join(" ")
}

fn contains_all(hay: &str, words: &[String]) -> bool {
    words.iter().all(|w| hay.contains(w.as_str()))
}

/// Depth-first list of `(id, indented title)` for move pickers.
pub fn flatten_mailboxes(
    roots: &[MailboxId],
    nodes: &HashMap<MailboxId, MailboxNode>,
) -> Vec<(MailboxId, String)> {
    let mut out = Vec::new();
    fn walk(
        id: &MailboxId,
        depth: usize,
        nodes: &HashMap<MailboxId, MailboxNode>,
        out: &mut Vec<(MailboxId, String)>,
    ) {
        let Some(node) = nodes.get(id) else {
            return;
        };
        let indent = "\u{00a0}\u{00a0}".repeat(depth);
        out.push((id.clone(), format!("{indent}{}", node.title())));
        for child in &node.children {
            walk(child, depth + 1, nodes, out);
        }
    }
    for root in roots {
        walk(root, 0, nodes, &mut out);
    }
    out
}

fn sort_mailbox_ids(ids: &mut [MailboxId], mboxes: &HashMap<MailboxId, MailboxNode>) {
    ids.sort_by(|a, b| {
        let (ra, na) = mboxes
            .get(a)
            .map(|n| (n.role.sort_rank(), n.name.to_ascii_lowercase()))
            .unwrap_or((u8::MAX, String::new()));
        let (rb, nb) = mboxes
            .get(b)
            .map(|n| (n.role.sort_rank(), n.name.to_ascii_lowercase()))
            .unwrap_or((u8::MAX, String::new()));
        ra.cmp(&rb).then_with(|| na.cmp(&nb))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailiner_core::{AccountId, Folder};
    use chrono::{TimeZone, Utc};

    fn folder(id: &str, name: &str, parent: Option<&str>, role: MailboxRole) -> Folder {
        let ts = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        Folder {
            id: FolderId::new(id),
            account_id: AccountId::new("acc"),
            name: name.into(),
            parent_id: parent.map(FolderId::new),
            role,
            created_at: ts,
            updated_at: ts,
        }
    }

    #[test]
    fn title_uses_role_label() {
        let inbox = folder("INBOX", "INBOX", None, MailboxRole::Inbox);
        let node = MailboxNode::from(inbox);
        assert_eq!(node.title(), "Inbox");
        let junk = folder("Junk", "Junk", None, MailboxRole::Other);
        assert_eq!(MailboxNode::from(junk).title(), "Junk");
    }

    #[test]
    fn roots_sort_special_first() {
        let (roots, nodes) = build_mailbox_tree(vec![
            folder("Junk", "Junk", None, MailboxRole::Other),
            folder("Sent", "Sent", None, MailboxRole::Sent),
            folder("INBOX", "INBOX", None, MailboxRole::Inbox),
            folder("Drafts", "Drafts", None, MailboxRole::Drafts),
            folder("Trash", "Trash", None, MailboxRole::Trash),
            folder("Outbox", "Outbox", None, MailboxRole::Outbox),
            folder("Archive", "Archive", None, MailboxRole::Other),
        ]);
        let names: Vec<_> = roots
            .iter()
            .map(|id| nodes.get(id).unwrap().name.as_str())
            .collect();
        assert_eq!(
            names,
            ["INBOX", "Drafts", "Sent", "Outbox", "Trash", "Archive", "Junk"]
        );
    }

    #[test]
    fn children_sort_the_same_way() {
        let (roots, nodes) = build_mailbox_tree(vec![
            folder("INBOX", "INBOX", None, MailboxRole::Inbox),
            folder("INBOX.Zebra", "Zebra", Some("INBOX"), MailboxRole::Other),
            folder("INBOX.Sent", "Sent", Some("INBOX"), MailboxRole::Sent),
            folder("INBOX.Drafts", "Drafts", Some("INBOX"), MailboxRole::Drafts),
        ]);
        assert_eq!(roots.len(), 1);
        let kids = &nodes.get(&roots[0]).unwrap().children;
        let names: Vec<_> = kids
            .iter()
            .map(|id| nodes.get(id).unwrap().name.as_str())
            .collect();
        assert_eq!(names, ["Drafts", "Sent", "Zebra"]);
    }

    #[test]
    fn find_trash_role() {
        let (_, nodes) = build_mailbox_tree(vec![
            folder("INBOX", "INBOX", None, MailboxRole::Inbox),
            folder("Trash", "Trash", None, MailboxRole::Trash),
        ]);
        let trash = find_mailbox_with_role(&nodes, MailboxRole::Trash).unwrap();
        assert_eq!(trash.to_string(), "Trash");
        assert!(find_mailbox_with_role(&nodes, MailboxRole::Outbox).is_none());
    }

    #[test]
    fn flatten_indents_children() {
        let (roots, nodes) = build_mailbox_tree(vec![
            folder("INBOX", "INBOX", None, MailboxRole::Inbox),
            folder("INBOX.Work", "Work", Some("INBOX"), MailboxRole::Other),
            folder("Trash", "Trash", None, MailboxRole::Trash),
        ]);
        let flat = flatten_mailboxes(&roots, &nodes);
        let titles: Vec<_> = flat.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(titles, ["Inbox", "\u{00a0}\u{00a0}Work", "Trash"]);
    }

    #[test]
    fn collect_builds_paths() {
        let (roots, nodes) = build_mailbox_tree(vec![
            folder("KDE", "KDE", None, MailboxRole::Other),
            folder("KDE.pim", "pim", Some("KDE"), MailboxRole::Other),
        ]);
        let entries = collect_mailbox_entries(&roots, &nodes);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "KDE");
        assert_eq!(entries[1].path, "KDE / pim");
        assert_eq!(entries[1].depth, 1);
    }

    #[test]
    fn filter_empty_keeps_tree_order() {
        let (roots, nodes) = build_mailbox_tree(vec![
            folder("INBOX", "INBOX", None, MailboxRole::Inbox),
            folder("Trash", "Trash", None, MailboxRole::Trash),
        ]);
        let entries = collect_mailbox_entries(&roots, &nodes);
        let filtered = filter_mailbox_entries(&entries, "  ");
        assert_eq!(filtered.len(), entries.len());
        assert_eq!(filtered[0].title, "Inbox");
    }

    #[test]
    fn filter_ranks_title_above_path() {
        let (roots, nodes) = build_mailbox_tree(vec![
            folder("INBOX", "INBOX", None, MailboxRole::Inbox),
            folder("KDE", "KDE", None, MailboxRole::Other),
            folder("KDE.pim", "pim", Some("KDE"), MailboxRole::Other),
        ]);
        let entries = collect_mailbox_entries(&roots, &nodes);
        let filtered = filter_mailbox_entries(&entries, "pim");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].title, "pim");
        let multi = filter_mailbox_entries(&entries, "kde pim");
        assert_eq!(multi.len(), 1);
        assert_eq!(multi[0].path, "KDE / pim");
        assert!(filter_mailbox_entries(&entries, "nope").is_empty());
    }

    #[test]
    fn filter_inbox_special_use_title() {
        let (roots, nodes) = build_mailbox_tree(vec![folder(
            "INBOX",
            "INBOX",
            None,
            MailboxRole::Inbox,
        )]);
        let entries = collect_mailbox_entries(&roots, &nodes);
        let filtered = filter_mailbox_entries(&entries, "inbox");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].title, "Inbox");
    }
}
