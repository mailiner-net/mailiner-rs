//! Sent-mailbox discovery (RFC 6154 special-use, then name heuristics).

/// One LIST/LSUB row, enough to pick a Sent target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedMailbox {
    pub name: String,
    pub delimiter: Option<String>,
    pub no_select: bool,
    pub special_use_sent: bool,
}

/// Prefer `\Sent` (selectable). Else first selectable name that looks like Sent.
pub fn find_sent_mailbox(mailboxes: &[ListedMailbox]) -> Option<&str> {
    if let Some(m) = mailboxes
        .iter()
        .find(|m| m.special_use_sent && !m.no_select)
    {
        return Some(m.name.as_str());
    }
    mailboxes
        .iter()
        .find(|m| !m.no_select && name_looks_like_sent(&m.name, m.delimiter.as_deref()))
        .map(|m| m.name.as_str())
}

fn name_looks_like_sent(name: &str, delim: Option<&str>) -> bool {
    let last = last_segment(name, delim).to_ascii_lowercase();
    matches!(
        last.as_str(),
        "sent" | "sent items" | "sent mail" | "sent messages"
    )
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

    fn mb(name: &str, delim: Option<&str>, no_select: bool, special_use_sent: bool) -> ListedMailbox {
        ListedMailbox {
            name: name.into(),
            delimiter: delim.map(str::to_string),
            no_select,
            special_use_sent,
        }
    }

    #[test]
    fn special_use_wins() {
        let boxes = [
            mb("INBOX", Some("."), false, false),
            mb("INBOX.Sent", Some("."), false, false),
            mb("[Gmail]/Sent Mail", Some("/"), false, true),
        ];
        assert_eq!(find_sent_mailbox(&boxes), Some("[Gmail]/Sent Mail"));
    }

    #[test]
    fn skips_noselect_special_use() {
        let boxes = [
            mb("virtual-sent", Some("/"), true, true),
            mb("Sent", Some("/"), false, false),
        ];
        assert_eq!(find_sent_mailbox(&boxes), Some("Sent"));
    }

    #[test]
    fn name_inbox_dot_sent() {
        let boxes = [
            mb("INBOX", Some("."), false, false),
            mb("INBOX.Sent", Some("."), false, false),
        ];
        assert_eq!(find_sent_mailbox(&boxes), Some("INBOX.Sent"));
    }

    #[test]
    fn name_sent_items() {
        let boxes = [mb("Sent Items", Some("/"), false, false)];
        assert_eq!(find_sent_mailbox(&boxes), Some("Sent Items"));
    }

    #[test]
    fn name_gmail_sent_mail_without_flag() {
        let boxes = [mb("[Gmail]/Sent Mail", Some("/"), false, false)];
        assert_eq!(find_sent_mailbox(&boxes), Some("[Gmail]/Sent Mail"));
    }

    #[test]
    fn does_not_match_unsent_or_inbox() {
        let boxes = [
            mb("INBOX", Some("/"), false, false),
            mb("Unsent", Some("/"), false, false),
            mb("Drafts", Some("/"), false, false),
        ];
        assert_eq!(find_sent_mailbox(&boxes), None);
    }

    #[test]
    fn none_when_empty() {
        assert_eq!(find_sent_mailbox(&[]), None);
    }
}
