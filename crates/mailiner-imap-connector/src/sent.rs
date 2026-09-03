//! Mailbox roles from RFC 6154 special-use, then name heuristics.

use mailiner_core::{AccountId, Folder, FolderId, MailboxRole};

/// One LIST/LSUB row, enough to pick a role or Sent target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedMailbox {
    pub name: String,
    pub delimiter: Option<String>,
    pub no_select: bool,
    /// LIST/LSUB special-use only. `None` if none advertised; `Some(Other)` if unmapped.
    pub special_use: Option<MailboxRole>,
}

impl ListedMailbox {
    /// Special-use if any RFC 6154 flag was advertised, else name heuristics.
    pub fn role(&self) -> MailboxRole {
        match self.special_use {
            Some(role) => role,
            None => role_from_name(&self.name, self.delimiter.as_deref()),
        }
    }
}

/// Prefer `\Sent` (selectable). Else first selectable name that looks like Sent.
pub fn find_sent_mailbox(mailboxes: &[ListedMailbox]) -> Option<&str> {
    if let Some(m) = mailboxes
        .iter()
        .find(|m| !m.no_select && m.special_use == Some(MailboxRole::Sent))
    {
        return Some(m.name.as_str());
    }
    mailboxes
        .iter()
        .find(|m| !m.no_select && m.role() == MailboxRole::Sent)
        .map(|m| m.name.as_str())
}

/// Map LIST/LSUB attributes to a role. `None` means no special-use flag.
/// Unmapped RFC 6154 flags stay `Some(Other)` so name heuristics do not override them.
pub fn special_use_from_attrs<'a, I>(attrs: I) -> (bool, Option<MailboxRole>)
where
    I: IntoIterator<Item = &'a async_imap::types::NameAttribute<'a>>,
{
    use async_imap::types::NameAttribute;
    let mut no_select = false;
    let mut role: Option<MailboxRole> = None;
    for attr in attrs {
        let next = match attr {
            NameAttribute::NoSelect => {
                no_select = true;
                continue;
            }
            NameAttribute::All | NameAttribute::Archive => MailboxRole::Archive,
            NameAttribute::Drafts => MailboxRole::Drafts,
            NameAttribute::Sent => MailboxRole::Sent,
            NameAttribute::Trash => MailboxRole::Trash,
            NameAttribute::Flagged | NameAttribute::Junk => MailboxRole::Other,
            NameAttribute::Extension(name) => match extension_role(name) {
                MailboxRole::Other if !is_unmapped_special_use(name) => continue,
                mapped => mapped,
            },
            _ => continue,
        };
        role = Some(match role {
            Some(current)
                if next != MailboxRole::Other
                    && (current == MailboxRole::Other
                        || next.sort_rank() < current.sort_rank()) =>
            {
                next
            }
            Some(current) => current,
            None => next,
        });
    }
    (no_select, role)
}

fn extension_role(name: &str) -> MailboxRole {
    match name.trim_start_matches('\\').to_ascii_lowercase().as_str() {
        "inbox" => MailboxRole::Inbox,
        "all" | "archive" => MailboxRole::Archive,
        "outbox" => MailboxRole::Outbox,
        _ => MailboxRole::Other,
    }
}

fn is_unmapped_special_use(name: &str) -> bool {
    matches!(
        name.trim_start_matches('\\').to_ascii_lowercase().as_str(),
        "flagged" | "junk"
    )
}

/// Leaf names (ASCII lowercase) used when LIST has no special-use attribute.
const ROLE_FROM_LEAF: &[(&str, MailboxRole)] = &[
    ("archive", MailboxRole::Archive),
    ("archives", MailboxRole::Archive),
    ("all mail", MailboxRole::Archive),
    ("drafts", MailboxRole::Drafts),
    ("draft", MailboxRole::Drafts),
    ("draft messages", MailboxRole::Drafts),
    ("sent", MailboxRole::Sent),
    ("sent items", MailboxRole::Sent),
    ("sent mail", MailboxRole::Sent),
    ("sent-mail", MailboxRole::Sent),
    ("sent messages", MailboxRole::Sent),
    ("outbox", MailboxRole::Outbox),
    ("unsent messages", MailboxRole::Outbox),
    ("trash", MailboxRole::Trash),
    ("bin", MailboxRole::Trash),
    ("deleted", MailboxRole::Trash),
    ("deleted items", MailboxRole::Trash),
    ("deleted messages", MailboxRole::Trash),
    ("deleted mail", MailboxRole::Trash),
];

/// Role from the last path segment when SPECIAL-USE is absent. `INBOX` matches the full name.
pub fn role_from_name(name: &str, delim: Option<&str>) -> MailboxRole {
    if name.eq_ignore_ascii_case("inbox") {
        return MailboxRole::Inbox;
    }
    let leaf = last_segment(name, delim).to_ascii_lowercase();
    ROLE_FROM_LEAF
        .iter()
        .find(|(n, _)| *n == leaf)
        .map(|(_, role)| *role)
        .unwrap_or(MailboxRole::Other)
}

/// Build UI folders from a full `LIST`.
///
/// Selectable mailboxes are included. `\Noselect` rows and missing path
/// prefixes are emitted as stubs so nested children keep their ancestor chain.
/// A `NIL` / empty delimiter means a flat name — do not split on `/`.
pub fn folders_from_listed(account_id: &AccountId, listed: &[ListedMailbox]) -> Vec<Folder> {
    let by_name: std::collections::HashMap<&str, &ListedMailbox> =
        listed.iter().map(|m| (m.name.as_str(), m)).collect();
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for m in listed.iter().filter(|m| !m.no_select) {
        let delim = m.delimiter.as_deref().filter(|d| !d.is_empty());
        match delim {
            None => {
                push_folder(
                    &mut out,
                    &mut seen,
                    account_id,
                    &m.name,
                    None,
                    None,
                    m.role(),
                    true,
                );
            }
            Some(d) => {
                let chunks: Vec<&str> = m.name.split(d).collect();
                for i in 1..=chunks.len() {
                    let full = chunks[..i].join(d);
                    let parent = if i > 1 {
                        Some(chunks[..i - 1].join(d))
                    } else {
                        None
                    };
                    let listed_row = by_name.get(full.as_str());
                    let role = listed_row
                        .map(|row| row.role())
                        .unwrap_or(MailboxRole::Other);
                    let selectable = listed_row.is_some_and(|row| !row.no_select);
                    let leaf = chunks[i - 1];
                    push_folder(
                        &mut out,
                        &mut seen,
                        account_id,
                        &full,
                        Some(leaf),
                        parent.as_deref(),
                        role,
                        selectable,
                    );
                }
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn push_folder(
    out: &mut Vec<Folder>,
    seen: &mut std::collections::HashSet<String>,
    account_id: &AccountId,
    full_name: &str,
    leaf: Option<&str>,
    parent: Option<&str>,
    role: MailboxRole,
    selectable: bool,
) {
    if !seen.insert(full_name.to_string()) {
        if selectable {
            if let Some(existing) = out.iter_mut().find(|f| f.id.as_str() == full_name) {
                existing.selectable = true;
                existing.role = role;
            }
        }
        return;
    }
    out.push(Folder {
        id: FolderId::new(full_name.to_string()),
        account_id: account_id.clone(),
        name: leaf.unwrap_or(full_name).to_string(),
        parent_id: parent.map(|p| FolderId::new(p.to_string())),
        role,
        selectable,
    });
}

fn last_segment<'a>(name: &'a str, delim: Option<&str>) -> &'a str {
    match delim {
        Some(d) if !d.is_empty() && name.contains(d) => name.rsplit(d).next().unwrap_or(name),
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mb(
        name: &str,
        delim: Option<&str>,
        no_select: bool,
        special_use: MailboxRole,
    ) -> ListedMailbox {
        mb_attr(
            name,
            delim,
            no_select,
            (special_use != MailboxRole::Other).then_some(special_use),
        )
    }

    fn mb_attr(
        name: &str,
        delim: Option<&str>,
        no_select: bool,
        special_use: Option<MailboxRole>,
    ) -> ListedMailbox {
        ListedMailbox {
            name: name.into(),
            delimiter: delim.map(str::to_string),
            no_select,
            special_use,
        }
    }

    #[test]
    fn special_use_wins() {
        let boxes = [
            mb("INBOX", Some("."), false, MailboxRole::Other),
            mb("INBOX.Sent", Some("."), false, MailboxRole::Other),
            mb("[Gmail]/Sent Mail", Some("/"), false, MailboxRole::Sent),
        ];
        assert_eq!(find_sent_mailbox(&boxes), Some("[Gmail]/Sent Mail"));
    }

    #[test]
    fn skips_noselect_special_use() {
        let boxes = [
            mb("virtual-sent", Some("/"), true, MailboxRole::Sent),
            mb("Sent", Some("/"), false, MailboxRole::Other),
        ];
        assert_eq!(find_sent_mailbox(&boxes), Some("Sent"));
    }

    #[test]
    fn name_inbox_dot_sent() {
        let boxes = [
            mb("INBOX", Some("."), false, MailboxRole::Other),
            mb("INBOX.Sent", Some("."), false, MailboxRole::Other),
        ];
        assert_eq!(find_sent_mailbox(&boxes), Some("INBOX.Sent"));
    }

    #[test]
    fn name_sent_items() {
        let boxes = [mb("Sent Items", Some("/"), false, MailboxRole::Other)];
        assert_eq!(find_sent_mailbox(&boxes), Some("Sent Items"));
    }

    #[test]
    fn name_gmail_sent_mail_without_flag() {
        let boxes = [mb(
            "[Gmail]/Sent Mail",
            Some("/"),
            false,
            MailboxRole::Other,
        )];
        assert_eq!(find_sent_mailbox(&boxes), Some("[Gmail]/Sent Mail"));
    }

    #[test]
    fn does_not_match_unsent_or_inbox() {
        let boxes = [
            mb("INBOX", Some("/"), false, MailboxRole::Other),
            mb("Unsent", Some("/"), false, MailboxRole::Other),
            mb("Drafts", Some("/"), false, MailboxRole::Other),
        ];
        assert_eq!(find_sent_mailbox(&boxes), None);
    }

    #[test]
    fn none_when_empty() {
        assert_eq!(find_sent_mailbox(&[]), None);
    }

    #[test]
    fn role_from_name_table() {
        assert!(!ROLE_FROM_LEAF.is_empty());
        for &(leaf, role) in ROLE_FROM_LEAF {
            assert_eq!(role_from_name(leaf, None), role, "{leaf}");
            assert_eq!(
                role_from_name(&leaf.to_ascii_uppercase(), Some("/")),
                role,
                "{leaf} uppercase"
            );
            assert_eq!(
                role_from_name(&format!("INBOX.{leaf}"), Some(".")),
                role,
                "INBOX.{leaf}"
            );
            assert_eq!(
                role_from_name(&format!("[Gmail]/{leaf}"), Some("/")),
                role,
                "[Gmail]/{leaf}"
            );
        }
    }

    #[test]
    fn role_from_name_inbox_is_full_name() {
        assert_eq!(role_from_name("INBOX", None), MailboxRole::Inbox);
        assert_eq!(role_from_name("Inbox", Some("/")), MailboxRole::Inbox);
        assert_eq!(role_from_name("inbox", Some(".")), MailboxRole::Inbox);
        assert_eq!(
            role_from_name("Projects/Inbox", Some("/")),
            MailboxRole::Other
        );
        assert_eq!(role_from_name("INBOX.inbox", Some(".")), MailboxRole::Other);
    }

    #[test]
    fn role_from_name_rejects_near_misses() {
        for name in [
            "Unsent",
            "Sentimental",
            "My Drafts",
            "Drafting",
            "Trashcan",
            "Work",
            "Lists",
        ] {
            assert_eq!(role_from_name(name, None), MailboxRole::Other, "{name}");
        }
    }

    #[test]
    fn role_from_name_nil_delimiter_is_flat() {
        assert_eq!(role_from_name("foo/Drafts", None), MailboxRole::Other);
        assert_eq!(role_from_name("Sent Items", None), MailboxRole::Sent);
    }

    #[test]
    fn archive_from_name() {
        assert_eq!(role_from_name("Archive", Some("/")), MailboxRole::Archive);
        assert_eq!(role_from_name("Archives", Some("/")), MailboxRole::Archive);
        assert_eq!(
            role_from_name("INBOX.Archive", Some(".")),
            MailboxRole::Archive
        );
        assert_eq!(
            role_from_name("[Gmail]/All Mail", Some("/")),
            MailboxRole::Archive
        );
    }

    #[test]
    fn archive_special_use_from_attrs() {
        use async_imap::types::NameAttribute;
        let (_, role) = special_use_from_attrs([NameAttribute::Archive].iter());
        assert_eq!(role, Some(MailboxRole::Archive));
        let (_, role) = special_use_from_attrs([NameAttribute::All].iter());
        assert_eq!(role, Some(MailboxRole::Archive));
        let (_, role) = special_use_from_attrs(
            [NameAttribute::Extension(std::borrow::Cow::Borrowed(
                "\\Archive",
            ))]
            .iter(),
        );
        assert_eq!(role, Some(MailboxRole::Archive));
    }

    #[test]
    fn special_use_beats_name() {
        let listed = mb("Archive", Some("/"), false, MailboxRole::Sent);
        assert_eq!(listed.role(), MailboxRole::Sent);
        let trash_named_sent = mb("Sent", Some("/"), false, MailboxRole::Trash);
        assert_eq!(trash_named_sent.role(), MailboxRole::Trash);
        let drafts_named_inbox = mb("INBOX", None, false, MailboxRole::Drafts);
        assert_eq!(drafts_named_inbox.role(), MailboxRole::Drafts);
    }

    #[test]
    fn special_use_from_attrs_named_flags() {
        use async_imap::types::NameAttribute;
        let cases = [
            (NameAttribute::Drafts, Some(MailboxRole::Drafts)),
            (NameAttribute::Sent, Some(MailboxRole::Sent)),
            (NameAttribute::Trash, Some(MailboxRole::Trash)),
        ];
        for (attr, want) in cases {
            let (no_select, role) = special_use_from_attrs([attr].iter());
            assert!(!no_select);
            assert_eq!(role, want);
        }
    }

    #[test]
    fn special_use_from_attrs_extensions_and_noselect() {
        use async_imap::types::NameAttribute;
        use std::borrow::Cow;
        let (_, role) =
            special_use_from_attrs([NameAttribute::Extension(Cow::Borrowed("\\Inbox"))].iter());
        assert_eq!(role, Some(MailboxRole::Inbox));
        let (_, role) =
            special_use_from_attrs([NameAttribute::Extension(Cow::Borrowed("outbox"))].iter());
        assert_eq!(role, Some(MailboxRole::Outbox));
        let (no_select, role) = special_use_from_attrs([NameAttribute::NoSelect].iter());
        assert!(no_select);
        assert_eq!(role, None);
        let (_, role) = special_use_from_attrs(
            [NameAttribute::Extension(Cow::Borrowed("\\HasNoChildren"))].iter(),
        );
        assert_eq!(role, None);
    }

    #[test]
    fn special_use_from_attrs_keeps_earliest_sort_rank() {
        use async_imap::types::NameAttribute;
        let (_, role) = special_use_from_attrs([NameAttribute::Sent, NameAttribute::Drafts].iter());
        assert_eq!(role, Some(MailboxRole::Drafts));
        let (_, role) = special_use_from_attrs([NameAttribute::Trash, NameAttribute::Sent].iter());
        assert_eq!(role, Some(MailboxRole::Sent));
    }

    #[test]
    fn unmapped_special_use_skips_name_heuristic() {
        use async_imap::types::NameAttribute;
        use std::borrow::Cow;
        for attr in [NameAttribute::Junk, NameAttribute::Flagged] {
            let (_, role) = special_use_from_attrs([attr].iter());
            assert_eq!(role, Some(MailboxRole::Other));
        }
        let (_, role) =
            special_use_from_attrs([NameAttribute::Extension(Cow::Borrowed("\\Junk"))].iter());
        assert_eq!(role, Some(MailboxRole::Other));
        let listed = mb_attr("Deleted Mail", Some("/"), false, Some(MailboxRole::Other));
        assert_eq!(listed.role(), MailboxRole::Other);
        let named = mb("Deleted Mail", Some("/"), false, MailboxRole::Other);
        assert_eq!(named.role(), MailboxRole::Trash);
    }

    #[test]
    fn folders_from_listed_skips_noselect() {
        let account = AccountId::new("acc");
        let listed = [
            mb("INBOX", Some("/"), false, MailboxRole::Inbox),
            mb("[Gmail]", Some("/"), true, MailboxRole::Other),
            mb("[Gmail]/Sent Mail", Some("/"), false, MailboxRole::Sent),
            mb("Unsubscribed", Some("/"), false, MailboxRole::Other),
        ];
        let folders = folders_from_listed(&account, &listed);
        let names: Vec<_> = folders.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(
            names,
            vec!["INBOX", "[Gmail]", "[Gmail]/Sent Mail", "Unsubscribed"]
        );
        let sent = folders
            .iter()
            .find(|f| f.role == MailboxRole::Sent)
            .unwrap();
        assert_eq!(sent.name, "Sent Mail");
        assert_eq!(
            sent.parent_id.as_ref().map(|id| id.as_str()),
            Some("[Gmail]")
        );
        assert!(folders.iter().any(|f| f.id.as_str() == "[Gmail]"));
        let gmail = folders.iter().find(|f| f.id.as_str() == "[Gmail]").unwrap();
        assert!(!gmail.selectable);
        assert!(sent.selectable);
    }

    #[test]
    fn folders_from_listed_nested_noselect_keeps_chain() {
        let account = AccountId::new("acc");
        let listed = [
            mb("A", Some("/"), true, MailboxRole::Other),
            mb("A/B", Some("/"), true, MailboxRole::Other),
            mb("A/B/C", Some("/"), false, MailboxRole::Other),
        ];
        let folders = folders_from_listed(&account, &listed);
        let names: Vec<_> = folders.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(names, vec!["A", "A/B", "A/B/C"]);
        let b = folders.iter().find(|f| f.id.as_str() == "A/B").unwrap();
        assert_eq!(b.parent_id.as_ref().map(|id| id.as_str()), Some("A"));
        assert!(
            !folders
                .iter()
                .find(|f| f.id.as_str() == "A")
                .unwrap()
                .selectable
        );
        assert!(!b.selectable);
        assert!(
            folders
                .iter()
                .find(|f| f.id.as_str() == "A/B/C")
                .unwrap()
                .selectable
        );
    }

    #[test]
    fn folders_from_listed_upgrades_stub_when_later_row_is_selectable() {
        let account = AccountId::new("acc");
        let listed = [
            mb("Work/A", Some("/"), false, MailboxRole::Other),
            mb("Work", Some("/"), false, MailboxRole::Other),
        ];
        let folders = folders_from_listed(&account, &listed);
        let work = folders.iter().find(|f| f.id.as_str() == "Work").unwrap();
        assert!(work.selectable);
        assert!(
            folders
                .iter()
                .find(|f| f.id.as_str() == "Work/A")
                .unwrap()
                .selectable
        );
    }

    #[test]
    fn folders_from_listed_uses_name_when_special_use_missing() {
        let account = AccountId::new("acc");
        let listed = [
            mb("INBOX", Some("/"), false, MailboxRole::Other),
            mb("Drafts", Some("/"), false, MailboxRole::Other),
            mb("Sent Items", Some("/"), false, MailboxRole::Other),
            mb("Outbox", Some("/"), false, MailboxRole::Other),
            mb("Deleted Items", Some("/"), false, MailboxRole::Other),
            mb("Work", Some("/"), false, MailboxRole::Other),
        ];
        let folders = folders_from_listed(&account, &listed);
        let role_of = |id: &str| {
            folders
                .iter()
                .find(|f| f.id.as_str() == id)
                .map(|f| f.role)
                .unwrap()
        };
        assert_eq!(role_of("INBOX"), MailboxRole::Inbox);
        assert_eq!(role_of("Drafts"), MailboxRole::Drafts);
        assert_eq!(role_of("Sent Items"), MailboxRole::Sent);
        assert_eq!(role_of("Outbox"), MailboxRole::Outbox);
        assert_eq!(role_of("Deleted Items"), MailboxRole::Trash);
        assert_eq!(role_of("Work"), MailboxRole::Other);
    }

    #[test]
    fn folders_from_listed_nil_delimiter_is_flat() {
        let account = AccountId::new("acc");
        let listed = [mb("foo/bar", None, false, MailboxRole::Other)];
        let folders = folders_from_listed(&account, &listed);
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].id.as_str(), "foo/bar");
        assert!(folders[0].parent_id.is_none());
        assert_eq!(folders[0].name, "foo/bar");
    }
}
