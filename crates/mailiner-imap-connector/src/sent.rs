//! Mailbox roles from RFC 6154 special-use, then name heuristics.

use chrono::Utc;
use mailiner_core::{AccountId, Folder, FolderId, MailboxRole};

/// One LIST/LSUB row, enough to pick a role or Sent target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedMailbox {
    pub name: String,
    pub delimiter: Option<String>,
    pub no_select: bool,
    /// From LIST/LSUB attributes only (`Other` if none).
    pub special_use: MailboxRole,
}

impl ListedMailbox {
    /// Special-use if present, else name heuristics.
    pub fn role(&self) -> MailboxRole {
        if self.special_use != MailboxRole::Other {
            self.special_use
        } else {
            role_from_name(&self.name, self.delimiter.as_deref())
        }
    }
}

/// Prefer `\Sent` (selectable). Else first selectable name that looks like Sent.
pub fn find_sent_mailbox(mailboxes: &[ListedMailbox]) -> Option<&str> {
    if let Some(m) = mailboxes
        .iter()
        .find(|m| !m.no_select && m.special_use == MailboxRole::Sent)
    {
        return Some(m.name.as_str());
    }
    mailboxes
        .iter()
        .find(|m| !m.no_select && m.role() == MailboxRole::Sent)
        .map(|m| m.name.as_str())
}

/// Map LIST/LSUB attributes to a role. Multiple flags keep the earliest sort rank.
pub fn special_use_from_attrs<'a, I>(attrs: I) -> (bool, MailboxRole)
where
    I: IntoIterator<Item = &'a async_imap::types::NameAttribute<'a>>,
{
    use async_imap::types::NameAttribute;
    let mut no_select = false;
    let mut role = MailboxRole::Other;
    for attr in attrs {
        let next = match attr {
            NameAttribute::NoSelect => {
                no_select = true;
                continue;
            }
            NameAttribute::Drafts => MailboxRole::Drafts,
            NameAttribute::Sent => MailboxRole::Sent,
            NameAttribute::Trash => MailboxRole::Trash,
            NameAttribute::Extension(name) => extension_role(name),
            _ => MailboxRole::Other,
        };
        if next != MailboxRole::Other
            && (role == MailboxRole::Other || next.sort_rank() < role.sort_rank())
        {
            role = next;
        }
    }
    (no_select, role)
}

fn extension_role(name: &str) -> MailboxRole {
    match name.trim_start_matches('\\').to_ascii_lowercase().as_str() {
        "inbox" => MailboxRole::Inbox,
        "outbox" => MailboxRole::Outbox,
        _ => MailboxRole::Other,
    }
}

pub fn role_from_name(name: &str, delim: Option<&str>) -> MailboxRole {
    if name.eq_ignore_ascii_case("inbox") {
        return MailboxRole::Inbox;
    }
    match last_segment(name, delim).to_ascii_lowercase().as_str() {
        "inbox" => MailboxRole::Inbox,
        "drafts" | "draft" => MailboxRole::Drafts,
        "sent" | "sent items" | "sent mail" | "sent messages" => MailboxRole::Sent,
        "outbox" => MailboxRole::Outbox,
        "trash" | "bin" | "deleted" | "deleted items" | "deleted messages" => MailboxRole::Trash,
        _ => MailboxRole::Other,
    }
}

/// Build UI folders from a full `LIST`.
///
/// Selectable mailboxes are included. `\Noselect` rows and missing path
/// prefixes are emitted as stubs so nested children keep their ancestor chain.
/// A `NIL` / empty delimiter means a flat name — do not split on `/`.
pub fn folders_from_listed(account_id: &AccountId, listed: &[ListedMailbox]) -> Vec<Folder> {
    let now = Utc::now();
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
                    now,
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
                        now,
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
    now: chrono::DateTime<Utc>,
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
        created_at: now,
        updated_at: now,
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
    fn inbox_from_name() {
        assert_eq!(role_from_name("INBOX", None), MailboxRole::Inbox);
        assert_eq!(role_from_name("Inbox", Some("/")), MailboxRole::Inbox);
    }

    #[test]
    fn drafts_outbox_trash_from_name() {
        assert_eq!(role_from_name("Drafts", Some("/")), MailboxRole::Drafts);
        assert_eq!(
            role_from_name("INBOX.Drafts", Some(".")),
            MailboxRole::Drafts
        );
        assert_eq!(role_from_name("Outbox", Some("/")), MailboxRole::Outbox);
        assert_eq!(
            role_from_name("Deleted Items", Some("/")),
            MailboxRole::Trash
        );
    }

    #[test]
    fn special_use_beats_name() {
        let listed = mb("Archive", Some("/"), false, MailboxRole::Sent);
        assert_eq!(listed.role(), MailboxRole::Sent);
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
