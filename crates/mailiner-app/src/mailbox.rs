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
}
