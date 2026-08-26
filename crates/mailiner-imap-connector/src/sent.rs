//! Mailbox roles from RFC 6154 special-use, then name heuristics.

use mailiner_core::MailboxRole;

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
}
