use std::collections::{HashMap, HashSet};

use mailiner_core::{Folder, FolderCounts, FolderId, MailboxRole, is_inbox_mailbox};

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

impl MailboxId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl ToString for MailboxId {
    fn to_string(&self) -> String {
        self.0.clone()
    }
}

#[derive(Clone, Debug)]
pub struct MailboxNode {
    pub id: MailboxId,
    pub name: String,
    pub parent: Option<MailboxId>,
    pub children: Vec<MailboxId>,
    pub unread_count: usize,
    pub total_count: usize,
    /// True when unread is above the last count the user opened this folder with.
    pub has_new: bool,
    pub role: MailboxRole,
    /// False for `\\Noselect` / synthesized ancestors.
    pub selectable: bool,
}

impl MailboxNode {
    /// Special-use label when we know the role, otherwise the server name.
    pub fn title(&self) -> &str {
        self.role.label().unwrap_or(self.name.as_str())
    }
}

/// Upgrade pre-Archive / pre-Junk cache rows whose leaf name is a known special folder.
fn inferred_mailbox_role(name: &str, role: MailboxRole) -> MailboxRole {
    if role != MailboxRole::Other {
        return role;
    }
    match name.to_ascii_lowercase().as_str() {
        "archive" | "archives" | "all mail" => MailboxRole::Archive,
        "junk" | "spam" | "junk e-mail" | "junk email" => MailboxRole::Junk,
        _ => role,
    }
}

impl From<Folder> for MailboxNode {
    fn from(folder: Folder) -> Self {
        let role = inferred_mailbox_role(&folder.name, folder.role);
        Self {
            id: folder.id.into(),
            name: folder.name,
            parent: folder.parent_id.map(|id| id.into()),
            children: vec![],
            unread_count: 0,
            total_count: 0,
            has_new: false,
            role,
            selectable: folder.selectable,
        }
    }
}

/// Inbox, Archive, Drafts, Sent, Outbox, Trash, Junk, then remaining names A–Z.
pub fn build_mailbox_tree(
    folders: Vec<Folder>,
) -> (Vec<MailboxId>, HashMap<MailboxId, MailboxNode>) {
    let mut root_ids = Vec::new();
    let mut mboxes = HashMap::<MailboxId, MailboxNode>::new();

    for folder in folders {
        let mailbox_id: MailboxId = folder.id.clone().into();
        let role = inferred_mailbox_role(&folder.name, folder.role);
        mboxes
            .entry(mailbox_id.clone())
            .and_modify(|node| {
                node.parent = folder.parent_id.as_ref().map(|id| id.clone().into());
                node.name = folder.name.clone();
                node.role = role;
                node.selectable = folder.selectable;
            })
            .or_insert(MailboxNode {
                id: mailbox_id.clone(),
                name: folder.name.clone(),
                parent: folder.parent_id.as_ref().map(|id| id.clone().into()),
                children: vec![],
                unread_count: 0,
                total_count: 0,
                has_new: false,
                role,
                selectable: folder.selectable,
            });
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
                    has_new: false,
                    role: MailboxRole::Other,
                    selectable: false,
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

/// True when unread arrived after the user last opened this folder.
pub fn unread_badge_is_new(unread: usize, acknowledged: usize) -> bool {
    unread > acknowledged
}

/// Copy IMAP `STATUS` totals onto matching tree nodes.
pub fn apply_folder_counts(
    nodes: &mut HashMap<MailboxId, MailboxNode>,
    counts: &HashMap<FolderId, FolderCounts>,
) {
    for (folder_id, count) in counts {
        let id = MailboxId::from(folder_id.clone());
        if let Some(node) = nodes.get_mut(&id) {
            node.unread_count = count.unread_messages as usize;
            node.total_count = count.total_messages as usize;
        }
    }
}

/// Set [`MailboxNode::has_new`] from persisted acknowledged unread counts.
///
/// Only folders present in `counts` are updated, so a later STATUS cannot
/// flip a folder the user already opened this session.
pub fn apply_unread_new_state(
    nodes: &mut HashMap<MailboxId, MailboxNode>,
    counts: &HashMap<FolderId, FolderCounts>,
    acknowledged: &HashMap<MailboxId, usize>,
) {
    for folder_id in counts.keys() {
        let id = MailboxId::from(folder_id.clone());
        if let Some(node) = nodes.get_mut(&id) {
            let ack = acknowledged.get(&id).copied().unwrap_or(0);
            node.has_new = unread_badge_is_new(node.unread_count, ack);
        }
    }
}

/// Trash special-use folder that can be selected (not `\\Noselect`).
pub fn can_empty_trash(node: &MailboxNode) -> bool {
    node.selectable && node.role == MailboxRole::Trash
}

/// Inbox cannot be renamed or deleted (IMAP `INBOX` is special).
pub fn can_manage_folder(node: &MailboxNode) -> bool {
    node.role != MailboxRole::Inbox && !is_inbox_mailbox(node.id.as_str())
}

/// New id for `selected` after `old` was renamed to `new`.
pub fn remap_renamed_mailbox(
    old: &MailboxId,
    new: &MailboxId,
    selected: &MailboxId,
    nodes: &HashMap<MailboxId, MailboxNode>,
) -> Option<MailboxId> {
    if selected == old {
        return Some(new.clone());
    }
    if !mailbox_is_ancestor(old, selected, nodes) {
        return None;
    }
    let rest = selected.as_str().strip_prefix(old.as_str())?;
    Some(MailboxId::from(format!("{}{rest}", new.as_str())))
}

/// `id` and its descendants, deepest first (safe IMAP DELETE order).
pub fn mailbox_subtree_deepest_first(
    id: &MailboxId,
    nodes: &HashMap<MailboxId, MailboxNode>,
) -> Vec<MailboxId> {
    let mut out = Vec::new();
    fn walk(id: &MailboxId, nodes: &HashMap<MailboxId, MailboxNode>, out: &mut Vec<MailboxId>) {
        let Some(node) = nodes.get(id) else {
            return;
        };
        for child in &node.children {
            walk(child, nodes, out);
        }
        out.push(id.clone());
    }
    walk(id, nodes, &mut out);
    out
}

/// First mailbox with `role`, if any.
pub fn find_mailbox_with_role(
    nodes: &HashMap<MailboxId, MailboxNode>,
    role: MailboxRole,
) -> Option<MailboxId> {
    nodes
        .iter()
        .find(|(_, n)| n.selectable && n.role == role)
        .map(|(id, _)| id.clone())
}

/// Archive target: Archive/Archives, then All Mail, then any other Archive-role folder.
///
/// Also matches pre-Archive cache rows still tagged `Other`. Ties use mailbox id order.
pub fn find_archive_mailbox(nodes: &HashMap<MailboxId, MailboxNode>) -> Option<MailboxId> {
    let mut exact = Vec::new();
    let mut all_mail = Vec::new();
    let mut other = Vec::new();
    for (id, node) in nodes {
        if !node.selectable {
            continue;
        }
        let leaf = node.name.to_ascii_lowercase();
        let archive_role = node.role == MailboxRole::Archive;
        match leaf.as_str() {
            "archive" | "archives" if archive_role || node.role == MailboxRole::Other => {
                exact.push(id.clone());
            }
            "all mail" if archive_role || node.role == MailboxRole::Other => {
                all_mail.push(id.clone());
            }
            _ if archive_role => other.push(id.clone()),
            _ => {}
        }
    }
    fn first_sorted(ids: &mut [MailboxId]) -> Option<MailboxId> {
        ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        ids.first().cloned()
    }
    first_sorted(&mut exact)
        .or_else(|| first_sorted(&mut all_mail))
        .or_else(|| first_sorted(&mut other))
}

/// Junk target: Junk, then Spam, then Junk E-mail, then any other Junk-role folder.
///
/// Also matches pre-Junk cache rows still tagged `Other`. Ties use mailbox id order.
pub fn find_junk_mailbox(nodes: &HashMap<MailboxId, MailboxNode>) -> Option<MailboxId> {
    let mut junk = Vec::new();
    let mut spam = Vec::new();
    let mut junk_email = Vec::new();
    let mut other = Vec::new();
    for (id, node) in nodes {
        if !node.selectable {
            continue;
        }
        let leaf = node.name.to_ascii_lowercase();
        let junk_role = node.role == MailboxRole::Junk;
        match leaf.as_str() {
            "junk" if junk_role || node.role == MailboxRole::Other => junk.push(id.clone()),
            "spam" if junk_role || node.role == MailboxRole::Other => spam.push(id.clone()),
            "junk e-mail" | "junk email" if junk_role || node.role == MailboxRole::Other => {
                junk_email.push(id.clone());
            }
            _ if junk_role => other.push(id.clone()),
            _ => {}
        }
    }
    fn first_sorted(ids: &mut [MailboxId]) -> Option<MailboxId> {
        ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        ids.first().cloned()
    }
    first_sorted(&mut junk)
        .or_else(|| first_sorted(&mut spam))
        .or_else(|| first_sorted(&mut junk_email))
        .or_else(|| first_sorted(&mut other))
}

/// Mailbox to open after a folder list: saved id if it still exists, else Inbox, else first root.
pub fn resolve_startup_mailbox(
    saved: Option<&MailboxId>,
    nodes: &HashMap<MailboxId, MailboxNode>,
    roots: &[MailboxId],
) -> Option<MailboxId> {
    if let Some(id) = saved
        && nodes.get(id).is_some_and(|n| n.selectable)
    {
        return Some(id.clone());
    }
    if let Some(inbox) = find_mailbox_with_role(nodes, MailboxRole::Inbox) {
        return Some(inbox);
    }
    roots
        .iter()
        .find(|id| nodes.get(*id).is_some_and(|n| n.selectable))
        .cloned()
}

/// True when `ancestor` is a parent (any depth) of `target`.
pub fn mailbox_is_ancestor(
    ancestor: &MailboxId,
    target: &MailboxId,
    nodes: &HashMap<MailboxId, MailboxNode>,
) -> bool {
    let mut current = nodes.get(target).and_then(|n| n.parent.clone());
    while let Some(id) = current {
        if &id == ancestor {
            return true;
        }
        current = nodes.get(&id).and_then(|n| n.parent.clone());
    }
    false
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
        if node.selectable {
            out.push(MailboxEntry {
                id: id.clone(),
                title: title.clone(),
                name: node.name.clone(),
                path: path.clone(),
                depth,
                role: node.role,
            });
        }
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

/// Sidebar-tree IDs to keep for `query`: picker matches plus their ancestors.
///
/// `None` when `query` is empty (show the full tree).
pub fn mailbox_tree_filter_ids(
    roots: &[MailboxId],
    nodes: &HashMap<MailboxId, MailboxNode>,
    query: &str,
) -> Option<HashSet<MailboxId>> {
    if query.split_whitespace().next().is_none() {
        return None;
    }
    let entries = collect_mailbox_entries(roots, nodes);
    let mut visible = HashSet::new();
    for entry in filter_mailbox_entries(&entries, query) {
        let mut current = Some(entry.id.clone());
        while let Some(id) = current {
            if !visible.insert(id.clone()) {
                break;
            }
            current = nodes.get(&id).and_then(|n| n.parent.clone());
        }
    }
    Some(visible)
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
        if node.selectable {
            out.push((id.clone(), format!("{indent}{}", node.title())));
        }
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

    fn folder(id: &str, name: &str, parent: Option<&str>, role: MailboxRole) -> Folder {
        folder_sel(id, name, parent, role, true)
    }

    fn folder_sel(
        id: &str,
        name: &str,
        parent: Option<&str>,
        role: MailboxRole,
        selectable: bool,
    ) -> Folder {
        Folder {
            id: FolderId::new(id),
            account_id: AccountId::new("acc"),
            name: name.into(),
            parent_id: parent.map(FolderId::new),
            role,
            selectable,
        }
    }

    #[test]
    fn title_uses_role_label() {
        let inbox = folder("INBOX", "INBOX", None, MailboxRole::Inbox);
        let node = MailboxNode::from(inbox);
        assert_eq!(node.title(), "Inbox");
        let archive = folder("All Mail", "All Mail", None, MailboxRole::Archive);
        assert_eq!(MailboxNode::from(archive).title(), "Archive");
        let junk_other = folder("Junk", "Junk", None, MailboxRole::Other);
        assert_eq!(MailboxNode::from(junk_other).title(), "Junk");
        let junk = folder("Spam", "Spam", None, MailboxRole::Junk);
        assert_eq!(MailboxNode::from(junk).title(), "Junk");
        let lists = folder("Lists", "Lists", None, MailboxRole::Other);
        assert_eq!(MailboxNode::from(lists).title(), "Lists");
    }

    #[test]
    fn roots_sort_special_first() {
        let (roots, nodes) = build_mailbox_tree(vec![
            folder("Junk", "Junk", None, MailboxRole::Junk),
            folder("Sent", "Sent", None, MailboxRole::Sent),
            folder("INBOX", "INBOX", None, MailboxRole::Inbox),
            folder("Drafts", "Drafts", None, MailboxRole::Drafts),
            folder("Trash", "Trash", None, MailboxRole::Trash),
            folder("Outbox", "Outbox", None, MailboxRole::Outbox),
            folder("Archive", "Archive", None, MailboxRole::Archive),
        ]);
        let names: Vec<_> = roots
            .iter()
            .map(|id| nodes.get(id).unwrap().name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "INBOX", "Archive", "Drafts", "Sent", "Outbox", "Trash", "Junk"
            ]
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
    fn find_archive_role() {
        let (_, nodes) = build_mailbox_tree(vec![
            folder("INBOX", "INBOX", None, MailboxRole::Inbox),
            folder("Archive", "Archive", None, MailboxRole::Archive),
        ]);
        let archive = find_mailbox_with_role(&nodes, MailboxRole::Archive).unwrap();
        assert_eq!(archive.to_string(), "Archive");
        assert_eq!(find_archive_mailbox(&nodes).unwrap().to_string(), "Archive");
        let hidden = build_mailbox_tree(vec![folder_sel(
            "virtual-archive",
            "Archive",
            None,
            MailboxRole::Archive,
            false,
        )])
        .1;
        assert!(find_mailbox_with_role(&hidden, MailboxRole::Archive).is_none());
        assert!(find_archive_mailbox(&hidden).is_none());
    }

    #[test]
    fn find_archive_prefers_named_folder_over_all_mail() {
        let (_, nodes) = build_mailbox_tree(vec![
            folder(
                "[Gmail]/All Mail",
                "All Mail",
                Some("[Gmail]"),
                MailboxRole::Archive,
            ),
            folder("Archive", "Archive", None, MailboxRole::Archive),
        ]);
        assert_eq!(find_archive_mailbox(&nodes).unwrap().to_string(), "Archive");
    }

    #[test]
    fn find_archive_accepts_other_role_named_archive() {
        let mut nodes = HashMap::new();
        let id = MailboxId::from("Archive".to_string());
        nodes.insert(
            id.clone(),
            MailboxNode {
                id: id.clone(),
                name: "Archive".into(),
                parent: None,
                children: vec![],
                unread_count: 0,
                total_count: 0,
                has_new: false,
                role: MailboxRole::Other,
                selectable: true,
            },
        );
        assert_eq!(find_archive_mailbox(&nodes).unwrap().to_string(), "Archive");
    }

    #[test]
    fn find_archive_reads_pre_archive_cache_names() {
        let (_, nodes) = build_mailbox_tree(vec![
            folder("INBOX", "INBOX", None, MailboxRole::Inbox),
            folder("Archive", "Archive", None, MailboxRole::Other),
        ]);
        assert_eq!(
            nodes
                .get(&MailboxId::from("Archive".to_string()))
                .unwrap()
                .role,
            MailboxRole::Archive
        );
        assert_eq!(find_archive_mailbox(&nodes).unwrap().to_string(), "Archive");
    }

    #[test]
    fn find_archive_prefers_all_mail_over_other_archive_names() {
        let (_, nodes) = build_mailbox_tree(vec![
            folder("2023 Archive", "2023 Archive", None, MailboxRole::Archive),
            folder(
                "[Gmail]/All Mail",
                "All Mail",
                Some("[Gmail]"),
                MailboxRole::Archive,
            ),
        ]);
        assert_eq!(
            find_archive_mailbox(&nodes).unwrap().to_string(),
            "[Gmail]/All Mail"
        );
    }

    #[test]
    fn find_archive_falls_back_to_all_mail() {
        let (_, nodes) = build_mailbox_tree(vec![
            folder("INBOX", "INBOX", None, MailboxRole::Inbox),
            folder(
                "[Gmail]/All Mail",
                "All Mail",
                Some("[Gmail]"),
                MailboxRole::Archive,
            ),
        ]);
        assert_eq!(
            find_archive_mailbox(&nodes).unwrap().to_string(),
            "[Gmail]/All Mail"
        );
    }

    #[test]
    fn find_junk_role() {
        let (_, nodes) = build_mailbox_tree(vec![
            folder("INBOX", "INBOX", None, MailboxRole::Inbox),
            folder("Junk", "Junk", None, MailboxRole::Junk),
        ]);
        let junk = find_mailbox_with_role(&nodes, MailboxRole::Junk).unwrap();
        assert_eq!(junk.to_string(), "Junk");
        assert_eq!(find_junk_mailbox(&nodes).unwrap().to_string(), "Junk");
        let hidden = build_mailbox_tree(vec![folder_sel(
            "virtual-junk",
            "Junk",
            None,
            MailboxRole::Junk,
            false,
        )])
        .1;
        assert!(find_mailbox_with_role(&hidden, MailboxRole::Junk).is_none());
        assert!(find_junk_mailbox(&hidden).is_none());
    }

    #[test]
    fn find_junk_prefers_named_junk_over_spam() {
        let (_, nodes) = build_mailbox_tree(vec![
            folder("[Gmail]/Spam", "Spam", Some("[Gmail]"), MailboxRole::Junk),
            folder("Junk", "Junk", None, MailboxRole::Junk),
        ]);
        assert_eq!(find_junk_mailbox(&nodes).unwrap().to_string(), "Junk");
    }

    #[test]
    fn find_junk_accepts_other_role_named_junk() {
        let mut nodes = HashMap::new();
        let id = MailboxId::from("Junk".to_string());
        nodes.insert(
            id.clone(),
            MailboxNode {
                id: id.clone(),
                name: "Junk".into(),
                parent: None,
                children: vec![],
                unread_count: 0,
                total_count: 0,
                has_new: false,
                role: MailboxRole::Other,
                selectable: true,
            },
        );
        assert_eq!(find_junk_mailbox(&nodes).unwrap().to_string(), "Junk");
    }

    #[test]
    fn find_junk_reads_pre_junk_cache_names() {
        let (_, nodes) = build_mailbox_tree(vec![
            folder("INBOX", "INBOX", None, MailboxRole::Inbox),
            folder("Spam", "Spam", None, MailboxRole::Other),
        ]);
        assert_eq!(
            nodes
                .get(&MailboxId::from("Spam".to_string()))
                .unwrap()
                .role,
            MailboxRole::Junk
        );
        assert_eq!(find_junk_mailbox(&nodes).unwrap().to_string(), "Spam");
    }

    #[test]
    fn find_junk_prefers_spam_over_junk_email() {
        let (_, nodes) = build_mailbox_tree(vec![
            folder("Junk E-mail", "Junk E-mail", None, MailboxRole::Junk),
            folder("[Gmail]/Spam", "Spam", Some("[Gmail]"), MailboxRole::Junk),
        ]);
        assert_eq!(
            find_junk_mailbox(&nodes).unwrap().to_string(),
            "[Gmail]/Spam"
        );
    }

    #[test]
    fn resolve_startup_prefers_saved_when_present() {
        let (roots, nodes) = build_mailbox_tree(vec![
            folder("INBOX", "INBOX", None, MailboxRole::Inbox),
            folder("Archive", "Archive", None, MailboxRole::Other),
        ]);
        let saved = MailboxId::from("Archive".to_string());
        let chosen = resolve_startup_mailbox(Some(&saved), &nodes, &roots).unwrap();
        assert_eq!(chosen.as_str(), "Archive");
    }

    #[test]
    fn resolve_startup_falls_back_to_inbox_when_saved_missing() {
        let (roots, nodes) = build_mailbox_tree(vec![
            folder("INBOX", "INBOX", None, MailboxRole::Inbox),
            folder("Sent", "Sent", None, MailboxRole::Sent),
        ]);
        let saved = MailboxId::from("Gone".to_string());
        let chosen = resolve_startup_mailbox(Some(&saved), &nodes, &roots).unwrap();
        assert_eq!(chosen.as_str(), "INBOX");
        let first_time = resolve_startup_mailbox(None, &nodes, &roots).unwrap();
        assert_eq!(first_time.as_str(), "INBOX");
    }

    #[test]
    fn resolve_startup_uses_first_root_without_inbox() {
        let (roots, nodes) = build_mailbox_tree(vec![
            folder("Archive", "Archive", None, MailboxRole::Other),
            folder("Lists", "Lists", None, MailboxRole::Other),
        ]);
        let chosen = resolve_startup_mailbox(None, &nodes, &roots).unwrap();
        assert_eq!(chosen.as_str(), roots[0].as_str());
    }

    #[test]
    fn resolve_startup_empty_tree_is_none() {
        let nodes = HashMap::new();
        let roots: Vec<MailboxId> = Vec::new();
        assert!(resolve_startup_mailbox(None, &nodes, &roots).is_none());
    }

    #[test]
    fn resolve_startup_skips_unselectable_saved() {
        let (roots, nodes) = build_mailbox_tree(vec![
            folder("INBOX", "INBOX", None, MailboxRole::Inbox),
            folder_sel("[Gmail]", "[Gmail]", None, MailboxRole::Other, false),
        ]);
        let saved = MailboxId::from("[Gmail]".to_string());
        let chosen = resolve_startup_mailbox(Some(&saved), &nodes, &roots).unwrap();
        assert_eq!(chosen.as_str(), "INBOX");
    }

    #[test]
    fn flatten_and_collect_skip_unselectable() {
        let (roots, nodes) = build_mailbox_tree(vec![
            folder("INBOX", "INBOX", None, MailboxRole::Inbox),
            folder_sel("[Gmail]", "[Gmail]", None, MailboxRole::Other, false),
            folder(
                "[Gmail]/Sent Mail",
                "Sent Mail",
                Some("[Gmail]"),
                MailboxRole::Sent,
            ),
        ]);
        let flat: Vec<_> = flatten_mailboxes(&roots, &nodes)
            .into_iter()
            .map(|(id, _)| id.to_string())
            .collect();
        assert_eq!(flat, vec!["INBOX", "[Gmail]/Sent Mail"]);
        let entries: Vec<_> = collect_mailbox_entries(&roots, &nodes)
            .into_iter()
            .map(|e| e.id.to_string())
            .collect();
        assert_eq!(entries, vec!["INBOX", "[Gmail]/Sent Mail"]);
    }

    #[test]
    fn can_manage_folder_refuses_inbox() {
        let inbox = MailboxNode::from(folder("INBOX", "INBOX", None, MailboxRole::Inbox));
        assert!(!can_manage_folder(&inbox));
        let named = MailboxNode::from(folder("inbox", "inbox", None, MailboxRole::Other));
        assert!(!can_manage_folder(&named));
        let work = MailboxNode::from(folder("Work", "Work", None, MailboxRole::Other));
        assert!(can_manage_folder(&work));
        let sent = MailboxNode::from(folder("Sent", "Sent", None, MailboxRole::Sent));
        assert!(can_manage_folder(&sent));
    }

    #[test]
    fn remap_renamed_mailbox_updates_self_and_children() {
        let (_, nodes) = build_mailbox_tree(vec![
            folder("INBOX", "INBOX", None, MailboxRole::Inbox),
            folder("INBOX.Work", "Work", Some("INBOX"), MailboxRole::Other),
            folder("INBOX.Work.A", "A", Some("INBOX.Work"), MailboxRole::Other),
            folder("INBOX.Work2", "Work2", Some("INBOX"), MailboxRole::Other),
        ]);
        let old = MailboxId::from("INBOX.Work".to_string());
        let new = MailboxId::from("INBOX.Archive".to_string());
        assert_eq!(
            remap_renamed_mailbox(&old, &new, &old, &nodes)
                .unwrap()
                .as_str(),
            "INBOX.Archive"
        );
        assert_eq!(
            remap_renamed_mailbox(
                &old,
                &new,
                &MailboxId::from("INBOX.Work.A".to_string()),
                &nodes
            )
            .unwrap()
            .as_str(),
            "INBOX.Archive.A"
        );
        assert!(
            remap_renamed_mailbox(
                &old,
                &new,
                &MailboxId::from("INBOX.Work2".to_string()),
                &nodes
            )
            .is_none()
        );
        assert!(
            remap_renamed_mailbox(&old, &new, &MailboxId::from("INBOX".to_string()), &nodes)
                .is_none()
        );
    }

    #[test]
    fn subtree_deepest_first_includes_self() {
        let (_, nodes) = build_mailbox_tree(vec![
            folder("KDE", "KDE", None, MailboxRole::Other),
            folder("KDE.pim", "pim", Some("KDE"), MailboxRole::Other),
            folder(
                "KDE.pim.inbox",
                "inbox",
                Some("KDE.pim"),
                MailboxRole::Other,
            ),
        ]);
        let ids = mailbox_subtree_deepest_first(&MailboxId::from("KDE".to_string()), &nodes);
        let names: Vec<_> = ids.iter().map(|id| id.as_str()).collect();
        assert_eq!(names, ["KDE.pim.inbox", "KDE.pim", "KDE"]);
    }

    #[test]
    fn can_empty_trash_requires_selectable_trash() {
        let trash = MailboxNode::from(folder("Trash", "Trash", None, MailboxRole::Trash));
        assert!(can_empty_trash(&trash));
        let inbox = MailboxNode::from(folder("INBOX", "INBOX", None, MailboxRole::Inbox));
        assert!(!can_empty_trash(&inbox));
        let hidden = MailboxNode::from(folder_sel(
            "virtual-trash",
            "Trash",
            None,
            MailboxRole::Trash,
            false,
        ));
        assert!(!can_empty_trash(&hidden));
    }

    #[test]
    fn find_role_skips_unselectable() {
        let (_, nodes) = build_mailbox_tree(vec![folder_sel(
            "virtual-trash",
            "Trash",
            None,
            MailboxRole::Trash,
            false,
        )]);
        assert!(find_mailbox_with_role(&nodes, MailboxRole::Trash).is_none());
    }

    #[test]
    fn build_tree_keeps_children_when_child_arrives_first() {
        let (roots, nodes) = build_mailbox_tree(vec![
            folder("KDE.pim", "pim", Some("KDE"), MailboxRole::Other),
            folder("KDE", "KDE", None, MailboxRole::Other),
        ]);
        assert!(roots.iter().any(|id| id.as_str() == "KDE"));
        let kde = nodes
            .get(&MailboxId::from("KDE".to_string()))
            .expect("parent");
        assert!(
            kde.children.iter().any(|id| id.as_str() == "KDE.pim"),
            "synthesized parent children must survive the real folder: {:?}",
            kde.children
        );
    }

    #[test]
    fn mailbox_is_ancestor_walks_parents() {
        let (_, nodes) = build_mailbox_tree(vec![
            folder("KDE", "KDE", None, MailboxRole::Other),
            folder("KDE.pim", "pim", Some("KDE"), MailboxRole::Other),
            folder(
                "KDE.pim.inbox",
                "inbox",
                Some("KDE.pim"),
                MailboxRole::Other,
            ),
            folder("Trash", "Trash", None, MailboxRole::Trash),
        ]);
        let kde = MailboxId::from("KDE".to_string());
        let pim = MailboxId::from("KDE.pim".to_string());
        let inbox = MailboxId::from("KDE.pim.inbox".to_string());
        let trash = MailboxId::from("Trash".to_string());
        assert!(mailbox_is_ancestor(&kde, &inbox, &nodes));
        assert!(mailbox_is_ancestor(&pim, &inbox, &nodes));
        assert!(!mailbox_is_ancestor(&inbox, &kde, &nodes));
        assert!(!mailbox_is_ancestor(&kde, &trash, &nodes));
        assert!(!mailbox_is_ancestor(&kde, &kde, &nodes));
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

    fn tree_ids(set: &HashSet<MailboxId>) -> Vec<&str> {
        let mut ids: Vec<&str> = set.iter().map(|id| id.as_str()).collect();
        ids.sort_unstable();
        ids
    }

    #[test]
    fn tree_filter_empty_is_none() {
        let (roots, nodes) = build_mailbox_tree(vec![
            folder("INBOX", "INBOX", None, MailboxRole::Inbox),
            folder("Trash", "Trash", None, MailboxRole::Trash),
        ]);
        assert!(mailbox_tree_filter_ids(&roots, &nodes, "").is_none());
        assert!(mailbox_tree_filter_ids(&roots, &nodes, "  \t").is_none());
    }

    #[test]
    fn tree_filter_keeps_match_and_ancestors() {
        let (roots, nodes) = build_mailbox_tree(vec![
            folder("KDE", "KDE", None, MailboxRole::Other),
            folder("KDE.pim", "pim", Some("KDE"), MailboxRole::Other),
            folder(
                "KDE.pim.inbox",
                "inbox",
                Some("KDE.pim"),
                MailboxRole::Other,
            ),
            folder("Trash", "Trash", None, MailboxRole::Trash),
        ]);
        let visible = mailbox_tree_filter_ids(&roots, &nodes, "inbox").unwrap();
        assert_eq!(tree_ids(&visible), ["KDE", "KDE.pim", "KDE.pim.inbox"]);
        assert!(!visible.contains(&MailboxId::from("Trash".to_string())));
    }

    #[test]
    fn tree_filter_includes_unselectable_ancestor() {
        let (roots, nodes) = build_mailbox_tree(vec![
            folder("INBOX", "INBOX", None, MailboxRole::Inbox),
            folder_sel("[Gmail]", "[Gmail]", None, MailboxRole::Other, false),
            folder(
                "[Gmail]/Sent Mail",
                "Sent Mail",
                Some("[Gmail]"),
                MailboxRole::Sent,
            ),
        ]);
        let visible = mailbox_tree_filter_ids(&roots, &nodes, "sent").unwrap();
        assert_eq!(tree_ids(&visible), ["[Gmail]", "[Gmail]/Sent Mail"]);
    }

    #[test]
    fn tree_filter_unknown_is_empty() {
        let (roots, nodes) =
            build_mailbox_tree(vec![folder("INBOX", "INBOX", None, MailboxRole::Inbox)]);
        let visible = mailbox_tree_filter_ids(&roots, &nodes, "nope").unwrap();
        assert!(visible.is_empty());
    }

    #[test]
    fn filter_inbox_special_use_title() {
        let (roots, nodes) =
            build_mailbox_tree(vec![folder("INBOX", "INBOX", None, MailboxRole::Inbox)]);
        let entries = collect_mailbox_entries(&roots, &nodes);
        let filtered = filter_mailbox_entries(&entries, "inbox");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].title, "Inbox");
    }

    #[test]
    fn unread_badge_is_new_when_count_exceeds_acknowledged() {
        assert!(unread_badge_is_new(1, 0));
        assert!(unread_badge_is_new(6, 5));
        assert!(!unread_badge_is_new(0, 0));
        assert!(!unread_badge_is_new(5, 5));
        assert!(!unread_badge_is_new(3, 5));
    }

    #[test]
    fn apply_folder_counts_and_new_state() {
        let (_, mut nodes) = build_mailbox_tree(vec![
            folder("INBOX", "INBOX", None, MailboxRole::Inbox),
            folder("Archive", "Archive", None, MailboxRole::Other),
        ]);
        let inbox = FolderId::new("INBOX");
        let archive = FolderId::new("Archive");
        let mut counts = HashMap::new();
        counts.insert(
            inbox,
            FolderCounts {
                total_messages: 10,
                unread_messages: 4,
            },
        );
        counts.insert(
            archive,
            FolderCounts {
                total_messages: 2,
                unread_messages: 2,
            },
        );
        apply_folder_counts(&mut nodes, &counts);

        let inbox_id = MailboxId::from("INBOX".to_string());
        let archive_id = MailboxId::from("Archive".to_string());
        assert_eq!(nodes.get(&inbox_id).unwrap().unread_count, 4);
        assert_eq!(nodes.get(&inbox_id).unwrap().total_count, 10);

        let mut ack = HashMap::new();
        ack.insert(inbox_id.clone(), 4);
        apply_unread_new_state(&mut nodes, &counts, &ack);
        assert!(!nodes.get(&inbox_id).unwrap().has_new);
        assert!(nodes.get(&archive_id).unwrap().has_new);
    }
}
