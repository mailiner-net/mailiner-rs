//! Compile a mailbox search box into IMAP `SEARCH` keys.
//!
//! Bare words become `OR OR SUBJECT FROM TEXT`. Prefixes cover from / to /
//! subject / body / date / flags / attachments.

use chrono::{Datelike, NaiveDate};

use crate::models::{Envelope, MessageListFilter};

/// Parsed search-box query (AND of terms).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MailboxSearch {
    pub terms: Vec<SearchTerm>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchTerm {
    From(String),
    To(String),
    Cc(String),
    Subject(String),
    Body(String),
    /// Headers + body (`TEXT`), not the subject/from/text OR used for bare words.
    Text(String),
    /// Bare word: subject, from, or full-text (`TEXT`).
    Free(String),
    Since(NaiveDate),
    Before(NaiveDate),
    On(NaiveDate),
    Unread,
    Read,
    Flagged,
    Unflagged,
    HasAttachment,
}

/// IMAP `SEARCH` / `SORT` criteria compiled from chips + the search box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledSearch {
    /// Search keys (`ALL` when nothing is active).
    pub keys: String,
    /// Non-ASCII quoted strings need `CHARSET UTF-8` on `UID SEARCH`.
    pub needs_utf8: bool,
}

impl CompiledSearch {
    /// Argument to `UID SEARCH` (`CHARSET UTF-8 …` when needed).
    pub fn uid_search_query(&self) -> String {
        if self.needs_utf8 {
            format!("CHARSET UTF-8 {}", self.keys)
        } else {
            self.keys.clone()
        }
    }

    /// Search keys only. IMAP `SORT` already names the charset.
    pub fn sort_query(&self) -> &str {
        &self.keys
    }
}

impl MailboxSearch {
    pub fn parse(input: &str) -> Self {
        let mut terms = Vec::new();
        let mut rest = input;
        while let Some((token, next)) = next_token(rest) {
            rest = next;
            if let Some(term) = token_to_term(token) {
                terms.push(term);
            }
        }
        Self { terms }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    pub fn has_unread(&self) -> bool {
        self.terms.iter().any(|t| matches!(t, SearchTerm::Unread))
    }

    pub fn has_read(&self) -> bool {
        self.terms.iter().any(|t| matches!(t, SearchTerm::Read))
    }

    pub fn has_flagged(&self) -> bool {
        self.terms.iter().any(|t| matches!(t, SearchTerm::Flagged))
    }

    pub fn has_unflagged(&self) -> bool {
        self.terms
            .iter()
            .any(|t| matches!(t, SearchTerm::Unflagged))
    }

    /// `true` when a `\Seen` change would leave the message outside this SEARCH.
    pub fn drops_on_read_change(&self, filter: MessageListFilter, now_read: bool) -> bool {
        if now_read {
            filter.unread || self.has_unread()
        } else {
            self.has_read()
        }
    }

    /// `true` when a `\Flagged` change would leave the message outside this SEARCH.
    pub fn drops_on_flagged_change(&self, filter: MessageListFilter, now_flagged: bool) -> bool {
        if now_flagged {
            self.has_unflagged()
        } else {
            filter.flagged || self.has_flagged()
        }
    }

    /// Envelope match for [`crate::MockConnector`] (no per-message body).
    pub fn matches_envelope(&self, envelope: &Envelope) -> bool {
        self.terms
            .iter()
            .all(|term| term.matches_envelope(envelope))
    }
}

impl SearchTerm {
    fn matches_envelope(&self, envelope: &Envelope) -> bool {
        match self {
            Self::From(q) => address_contains(envelope.from.as_ref(), q),
            Self::To(q) => address_contains(envelope.to.as_ref(), q),
            Self::Cc(q) => address_contains(envelope.cc.as_ref(), q),
            Self::Subject(q) => field_contains(envelope.subject.as_deref().unwrap_or(""), q),
            Self::Body(q) | Self::Text(q) | Self::Free(q) => {
                field_contains(envelope.subject.as_deref().unwrap_or(""), q)
                    || address_contains(envelope.from.as_ref(), q)
                    || address_contains(envelope.to.as_ref(), q)
                    || address_contains(envelope.cc.as_ref(), q)
            }
            Self::Since(d) => envelope.date.date_naive() >= *d,
            Self::Before(d) => envelope.date.date_naive() < *d,
            Self::On(d) => envelope.date.date_naive() == *d,
            Self::Unread => !envelope.is_read,
            Self::Read => envelope.is_read,
            Self::Flagged => envelope.is_flagged,
            Self::Unflagged => !envelope.is_flagged,
            Self::HasAttachment => envelope.has_attachments,
        }
    }

    fn to_imap_key(&self) -> String {
        match self {
            Self::From(s) => format!("FROM {}", imap_quoted(s)),
            Self::To(s) => format!("TO {}", imap_quoted(s)),
            Self::Cc(s) => format!("CC {}", imap_quoted(s)),
            Self::Subject(s) => format!("SUBJECT {}", imap_quoted(s)),
            Self::Body(s) => format!("BODY {}", imap_quoted(s)),
            Self::Text(s) => format!("TEXT {}", imap_quoted(s)),
            Self::Free(s) => {
                let q = imap_quoted(s);
                format!("OR OR SUBJECT {q} FROM {q} TEXT {q}")
            }
            Self::Since(d) => format!("SINCE {}", imap_date(*d)),
            Self::Before(d) => format!("BEFORE {}", imap_date(*d)),
            Self::On(d) => format!("ON {}", imap_date(*d)),
            Self::Unread => "UNSEEN".into(),
            Self::Read => "SEEN".into(),
            Self::Flagged => "FLAGGED".into(),
            Self::Unflagged => "UNFLAGGED".into(),
            Self::HasAttachment => {
                "OR HEADER Content-Disposition \"attachment\" HEADER Content-Type \"multipart/mixed\""
                    .into()
            }
        }
    }

    fn needs_utf8(&self) -> bool {
        match self {
            Self::From(s)
            | Self::To(s)
            | Self::Cc(s)
            | Self::Subject(s)
            | Self::Body(s)
            | Self::Text(s)
            | Self::Free(s) => !s.is_ascii(),
            Self::Since(_)
            | Self::Before(_)
            | Self::On(_)
            | Self::Unread
            | Self::Read
            | Self::Flagged
            | Self::Unflagged
            | Self::HasAttachment => false,
        }
    }
}

/// Chips + search box → IMAP criteria (`ALL` when idle).
pub fn compile_list_search(filter: MessageListFilter, query: &str) -> CompiledSearch {
    let parsed = MailboxSearch::parse(query);
    let mut keys = Vec::new();
    let unread = filter.unread || parsed.has_unread();
    let flagged = filter.flagged || parsed.has_flagged();
    if unread {
        keys.push("UNSEEN".into());
    }
    if flagged {
        keys.push("FLAGGED".into());
    }
    for term in &parsed.terms {
        match term {
            SearchTerm::Unread | SearchTerm::Flagged => {}
            other => keys.push(other.to_imap_key()),
        }
    }
    let needs_utf8 = parsed.terms.iter().any(SearchTerm::needs_utf8);
    if keys.is_empty() {
        CompiledSearch {
            keys: "ALL".into(),
            needs_utf8,
        }
    } else {
        CompiledSearch {
            keys: keys.join(" "),
            needs_utf8,
        }
    }
}

/// Keys to AND onto unread-first `UNSEEN` / `SEEN` groups (flags stay on those groups).
pub fn compile_unread_sort_extra(query: &str) -> CompiledSearch {
    let parsed = MailboxSearch::parse(query);
    let mut keys = Vec::new();
    for term in &parsed.terms {
        match term {
            SearchTerm::Unread | SearchTerm::Flagged => {}
            other => keys.push(other.to_imap_key()),
        }
    }
    let needs_utf8 = parsed.terms.iter().any(SearchTerm::needs_utf8);
    if keys.is_empty() {
        CompiledSearch {
            keys: String::new(),
            needs_utf8,
        }
    } else {
        CompiledSearch {
            keys: keys.join(" "),
            needs_utf8,
        }
    }
}

/// Join IMAP search atoms, dropping empties.
pub fn join_search_keys(parts: &[&str]) -> String {
    let joined = parts
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if joined.is_empty() {
        "ALL".into()
    } else {
        joined
    }
}

pub fn mailbox_search_is_active(query: &str) -> bool {
    !MailboxSearch::parse(query).is_empty()
}

/// RFC 3501 quoted-string (`\` / `"` escaped; CR/LF stripped).
pub fn imap_quoted(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        if c == '\r' || c == '\n' {
            continue;
        }
        if c == '\\' || c == '"' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

pub fn imap_date(date: NaiveDate) -> String {
    format!(
        "{}-{}-{}",
        date.day(),
        month_abbr(date.month()),
        date.year()
    )
}

fn month_abbr(month: u32) -> &'static str {
    match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        _ => "Dec",
    }
}

enum Token {
    Bare(String),
    Keyed(String, String),
}

fn next_token(input: &str) -> Option<(Token, &str)> {
    let s = input.trim_start();
    if s.is_empty() {
        return None;
    }
    if s.starts_with('"') {
        let (value, rest) = parse_quoted(s);
        return Some((Token::Bare(value), rest));
    }
    if let Some(colon) = s.find(':') {
        let key = &s[..colon];
        if is_search_key(key) {
            let after = &s[colon + 1..];
            if after.starts_with('"') {
                let (value, rest) = parse_quoted(after);
                return Some((Token::Keyed(key.to_ascii_lowercase(), value), rest));
            }
            let (value, rest) = split_ws(after);
            return Some((
                Token::Keyed(key.to_ascii_lowercase(), value.to_string()),
                rest,
            ));
        }
    }
    let (word, rest) = split_ws(s);
    Some((Token::Bare(word.to_string()), rest))
}

fn parse_quoted(s: &str) -> (String, &str) {
    let mut chars = s.char_indices();
    chars.next();
    let mut out = String::new();
    let mut escape = false;
    for (i, c) in chars {
        if escape {
            out.push(c);
            escape = false;
            continue;
        }
        match c {
            '\\' => escape = true,
            '"' => return (out, s.get(i + 1..).unwrap_or("")),
            _ => out.push(c),
        }
    }
    (out, "")
}

fn split_ws(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
}

fn is_search_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "from"
            | "to"
            | "cc"
            | "subject"
            | "subj"
            | "body"
            | "text"
            | "after"
            | "since"
            | "before"
            | "on"
            | "is"
            | "has"
    )
}

fn token_to_term(token: Token) -> Option<SearchTerm> {
    match token {
        Token::Bare(s) => {
            let s = s.trim();
            if s.is_empty() {
                None
            } else {
                Some(SearchTerm::Free(s.to_string()))
            }
        }
        Token::Keyed(key, value) => keyed_term(&key, value.trim()),
    }
}

fn keyed_term(key: &str, value: &str) -> Option<SearchTerm> {
    match key {
        "from" if !value.is_empty() => Some(SearchTerm::From(value.to_string())),
        "to" if !value.is_empty() => Some(SearchTerm::To(value.to_string())),
        "cc" if !value.is_empty() => Some(SearchTerm::Cc(value.to_string())),
        "subject" | "subj" if !value.is_empty() => Some(SearchTerm::Subject(value.to_string())),
        "body" if !value.is_empty() => Some(SearchTerm::Body(value.to_string())),
        "text" if !value.is_empty() => Some(SearchTerm::Text(value.to_string())),
        "after" | "since" => parse_search_date(value).map(SearchTerm::Since),
        "before" => parse_search_date(value).map(SearchTerm::Before),
        "on" => parse_search_date(value).map(SearchTerm::On),
        "is" => match value.to_ascii_lowercase().as_str() {
            "unread" | "unseen" => Some(SearchTerm::Unread),
            "read" | "seen" => Some(SearchTerm::Read),
            "flagged" => Some(SearchTerm::Flagged),
            "unflagged" => Some(SearchTerm::Unflagged),
            _ => None,
        },
        "has" => match value.to_ascii_lowercase().as_str() {
            "attachment" | "attachments" | "attach" => Some(SearchTerm::HasAttachment),
            _ => None,
        },
        _ => None,
    }
}

fn parse_search_date(s: &str) -> Option<NaiveDate> {
    const FMTS: [&str; 3] = ["%Y-%m-%d", "%Y/%m/%d", "%d-%b-%Y"];
    for fmt in FMTS {
        if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
            return Some(d);
        }
    }
    None
}

fn field_contains(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

fn address_contains(addr: Option<&crate::models::EmailAddress>, needle: &str) -> bool {
    addr.is_some_and(|a| field_contains(&a.to_string(), needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{AccountId, FolderId, MessageId};
    use crate::models::{EmailAddr, EmailAddress};
    use chrono::{TimeZone, Utc};

    fn env(
        subject: &str,
        from: &str,
        to: &str,
        is_read: bool,
        is_flagged: bool,
        attach: bool,
    ) -> Envelope {
        Envelope {
            id: MessageId::new(FolderId::new("INBOX"), "1"),
            account_id: AccountId::new("acc"),
            folder_id: FolderId::new("INBOX"),
            subject: Some(subject.into()),
            from: Some(EmailAddress::List(vec![EmailAddr {
                name: Some("Ada".into()),
                email: Some(from.into()),
            }])),
            to: Some(EmailAddress::List(vec![EmailAddr {
                name: None,
                email: Some(to.into()),
            }])),
            cc: None,
            bcc: None,
            reply_to: None,
            rfc_message_id: None,
            in_reply_to: None,
            references: vec![],
            date: Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap(),
            is_read,
            is_answered: false,
            is_starred: false,
            is_flagged,
            is_draft: false,
            is_deleted: false,
            keywords: Vec::new(),
            has_attachments: attach,
            size: None,
            snippet: None,
            auth_results: Default::default(),
        }
    }

    #[test]
    fn empty_query_is_all() {
        assert!(MailboxSearch::parse("").is_empty());
        assert!(MailboxSearch::parse("   ").is_empty());
        assert!(!mailbox_search_is_active(""));
        let c = compile_list_search(MessageListFilter::default(), "");
        assert_eq!(c.keys, "ALL");
        assert!(!c.needs_utf8);
        assert_eq!(c.uid_search_query(), "ALL");
    }

    #[test]
    fn bare_word_is_subject_from_or_text() {
        let c = compile_list_search(MessageListFilter::default(), "invoice");
        assert_eq!(
            c.keys,
            r#"OR OR SUBJECT "invoice" FROM "invoice" TEXT "invoice""#
        );
        assert_eq!(c.uid_search_query(), c.keys);
    }

    #[test]
    fn words_and_and_prefixes() {
        let c = compile_list_search(
            MessageListFilter::default(),
            r#"from:ada subject:"Q3 invoice""#,
        );
        assert_eq!(c.keys, r#"FROM "ada" SUBJECT "Q3 invoice""#);
    }

    #[test]
    fn quotes_and_escapes() {
        assert_eq!(imap_quoted(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(imap_quoted(r#"a\b"#), r#""a\\b""#);
        let c = compile_list_search(MessageListFilter::default(), r#""foo bar""#);
        assert_eq!(
            c.keys,
            r#"OR OR SUBJECT "foo bar" FROM "foo bar" TEXT "foo bar""#
        );
    }

    #[test]
    fn dates_and_flags_and_attachment() {
        let c = compile_list_search(
            MessageListFilter::default(),
            "after:2024-01-15 before:2024-12-01 is:unread has:attachment",
        );
        assert_eq!(
            c.keys,
            concat!(
                "UNSEEN SINCE 15-Jan-2024 BEFORE 1-Dec-2024 ",
                r#"OR HEADER Content-Disposition "attachment" HEADER Content-Type "multipart/mixed""#
            )
        );
    }

    #[test]
    fn chips_and_search_combine() {
        let filter = MessageListFilter {
            unread: true,
            flagged: true,
            has_attachment: true,
        };
        let c = compile_list_search(filter, "to:bob");
        assert_eq!(c.keys, r#"UNSEEN FLAGGED TO "bob""#);
    }

    #[test]
    fn utf8_adds_charset_on_uid_search_only() {
        let c = compile_list_search(MessageListFilter::default(), "über");
        assert!(c.needs_utf8);
        assert_eq!(
            c.uid_search_query(),
            r#"CHARSET UTF-8 OR OR SUBJECT "über" FROM "über" TEXT "über""#
        );
        assert_eq!(
            c.sort_query(),
            r#"OR OR SUBJECT "über" FROM "über" TEXT "über""#
        );
    }

    #[test]
    fn unread_sort_extra_omits_flag_keys() {
        let extra = compile_unread_sort_extra("from:ada is:unread is:flagged is:read invoice");
        assert_eq!(
            extra.keys,
            r#"FROM "ada" SEEN OR OR SUBJECT "invoice" FROM "invoice" TEXT "invoice""#
        );
        assert_eq!(
            join_search_keys(&["UNSEEN", extra.sort_query()]),
            r#"UNSEEN FROM "ada" SEEN OR OR SUBJECT "invoice" FROM "invoice" TEXT "invoice""#
        );
        let extra_unflagged = compile_unread_sort_extra("is:unflagged invoice");
        assert_eq!(
            extra_unflagged.keys,
            r#"UNFLAGGED OR OR SUBJECT "invoice" FROM "invoice" TEXT "invoice""#
        );
    }

    #[test]
    fn flag_terms_drop_when_search_would_exclude() {
        let unread = MailboxSearch::parse("is:unread");
        let read = MailboxSearch::parse("is:read");
        let flagged = MailboxSearch::parse("is:flagged");
        let unflagged = MailboxSearch::parse("is:unflagged");
        let empty = MailboxSearch::default();
        let chip_unread = MessageListFilter {
            unread: true,
            ..MessageListFilter::default()
        };
        assert!(unread.drops_on_read_change(MessageListFilter::default(), true));
        assert!(!unread.drops_on_read_change(MessageListFilter::default(), false));
        assert!(read.drops_on_read_change(MessageListFilter::default(), false));
        assert!(empty.drops_on_read_change(chip_unread, true));
        assert!(flagged.drops_on_flagged_change(MessageListFilter::default(), false));
        assert!(unflagged.drops_on_flagged_change(MessageListFilter::default(), true));
        assert!(!empty.drops_on_flagged_change(MessageListFilter::default(), false));
    }

    #[test]
    fn join_search_keys_skips_empty() {
        assert_eq!(
            join_search_keys(&["UNSEEN", "", "FLAGGED"]),
            "UNSEEN FLAGGED"
        );
        assert_eq!(join_search_keys(&["", ""]), "ALL");
    }

    #[test]
    fn envelope_match_covers_fields() {
        let e = env("Q3 Invoice", "ada@x.com", "bob@y.com", false, true, true);
        assert!(MailboxSearch::parse("invoice").matches_envelope(&e));
        assert!(MailboxSearch::parse("from:ada").matches_envelope(&e));
        assert!(MailboxSearch::parse("to:bob").matches_envelope(&e));
        assert!(MailboxSearch::parse("is:unread is:flagged has:attachment").matches_envelope(&e));
        assert!(!MailboxSearch::parse("from:cara").matches_envelope(&e));
        assert!(!MailboxSearch::parse("is:read").matches_envelope(&e));
        assert!(MailboxSearch::parse("after:2024-06-01 before:2024-07-01").matches_envelope(&e));
        assert!(!MailboxSearch::parse("on:2024-01-01").matches_envelope(&e));
    }

    #[test]
    fn invalid_date_or_empty_prefix_is_dropped() {
        assert!(MailboxSearch::parse("after:nope from:").is_empty());
        assert!(MailboxSearch::parse("is:whatever has:foo").is_empty());
    }

    #[test]
    fn imap_date_unpadded_day() {
        assert_eq!(
            imap_date(NaiveDate::from_ymd_opt(2024, 1, 5).unwrap()),
            "5-Jan-2024"
        );
        assert_eq!(
            imap_date(NaiveDate::from_ymd_opt(1994, 2, 1).unwrap()),
            "1-Feb-1994"
        );
    }
}
