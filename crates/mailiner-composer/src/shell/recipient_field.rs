//! Parse, commit, and remove helpers for chip-style recipient fields.

use crate::model::draft::{is_valid_email_v1, ComposerAddress};
use crate::model::recipients::{dedupe_addresses, emails_equal};

/// Parse one typed token into a chip.
///
/// Accepts `email`, `Name <email>`, and `"Quoted Name" <email>`.
/// Empty / whitespace-only input yields `None`. Invalid mailboxes are still
/// returned so the UI can highlight them.
pub fn parse_recipient(raw: &str) -> Option<ComposerAddress> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some(addr) = parse_angle_addr(raw) {
        return Some(addr);
    }
    Some(ComposerAddress::email_only(raw))
}

/// Commit typed text onto an existing chip list.
///
/// Unquoted commas separate tokens. When `commit_tail` is false (live input),
/// the last token stays in `draft` so the user can keep typing. When true
/// (Enter / blur), the tail is committed too. Duplicate emails (ASCII
/// case-insensitive) are dropped, keeping the first chip.
pub fn commit_input(
    chips: &[ComposerAddress],
    draft: &str,
    commit_tail: bool,
) -> (Vec<ComposerAddress>, String) {
    let tokens = split_unquoted_commas(draft);
    if tokens.is_empty() {
        return (chips.to_vec(), String::new());
    }

    let (complete, leftover) = if commit_tail {
        (tokens.as_slice(), "")
    } else {
        let last = tokens.len() - 1;
        (&tokens[..last], tokens[last].trim_start())
    };

    let mut out = dedupe_addresses(chips.to_vec());
    for token in complete {
        if let Some(addr) = parse_recipient(token) {
            push_unique(&mut out, addr);
        }
    }
    (out, leftover.to_string())
}

/// Remove the chip at `index`. Out-of-range index is a no-op.
pub fn remove_recipient(chips: &[ComposerAddress], index: usize) -> Vec<ComposerAddress> {
    if index >= chips.len() {
        return chips.to_vec();
    }
    chips
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != index)
        .map(|(_, a)| a.clone())
        .collect()
}

/// Remove the last chip. Empty input stays empty.
pub fn remove_last_recipient(chips: &[ComposerAddress]) -> Vec<ComposerAddress> {
    match chips.split_last() {
        Some((_, rest)) => rest.to_vec(),
        None => Vec::new(),
    }
}

/// Visible chip text: display name when present, otherwise the mailbox.
pub fn chip_label(addr: &ComposerAddress) -> &str {
    addr.name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(addr.email.as_str())
}

/// Tooltip / accessible detail: `Name <email>` when a name exists.
pub fn chip_title(addr: &ComposerAddress) -> String {
    match addr
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        Some(name) => format!("{name} <{}>", addr.email),
        None => addr.email.clone(),
    }
}

/// Whether this chip's mailbox passes [`is_valid_email_v1`] and is trimmed.
pub fn chip_is_valid(addr: &ComposerAddress) -> bool {
    addr.email == addr.email.trim() && is_valid_email_v1(&addr.email)
}

fn parse_angle_addr(raw: &str) -> Option<ComposerAddress> {
    let open = raw.rfind('<')?;
    if !raw.ends_with('>') || open + 1 >= raw.len() {
        return None;
    }
    let email = raw[open + 1..raw.len() - 1].trim();
    if email.is_empty() {
        return None;
    }
    let name = unquote_display_name(&raw[..open]);
    Some(ComposerAddress {
        name,
        email: email.to_string(),
    })
}

fn unquote_display_name(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
        let unescaped = unescape_quoted(&raw[1..raw.len() - 1]);
        if unescaped.is_empty() {
            None
        } else {
            Some(unescaped)
        }
    } else {
        Some(raw.to_string())
    }
}

fn unescape_quoted(inner: &str) -> String {
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn split_unquoted_commas(raw: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut escape = false;
    for (i, c) in raw.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if in_quotes {
            if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_quotes = false;
            }
            continue;
        }
        if c == '"' {
            in_quotes = true;
        } else if c == ',' {
            tokens.push(&raw[start..i]);
            start = i + c.len_utf8();
        }
    }
    tokens.push(&raw[start..]);
    tokens
}

fn push_unique(out: &mut Vec<ComposerAddress>, addr: ComposerAddress) {
    if out.iter().any(|a| emails_equal(&a.email, &addr.email)) {
        return;
    }
    out.push(addr);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emails(chips: &[ComposerAddress]) -> Vec<&str> {
        chips.iter().map(|a| a.email.as_str()).collect()
    }

    #[test]
    fn parse_email_only() {
        let a = parse_recipient("  user.name+tag@example.com ").unwrap();
        assert_eq!(a.email, "user.name+tag@example.com");
        assert_eq!(a.name, None);
        assert!(chip_is_valid(&a));
    }

    #[test]
    fn parse_name_angle_addr() {
        let a = parse_recipient("Ada Lovelace <ada@example.com>").unwrap();
        assert_eq!(a.name.as_deref(), Some("Ada Lovelace"));
        assert_eq!(a.email, "ada@example.com");
        assert_eq!(chip_label(&a), "Ada Lovelace");
        assert_eq!(chip_title(&a), "Ada Lovelace <ada@example.com>");
    }

    #[test]
    fn parse_quoted_name_with_comma() {
        let a = parse_recipient(r#""Smith, Alice" <alice@example.com>"#).unwrap();
        assert_eq!(a.name.as_deref(), Some("Smith, Alice"));
        assert_eq!(a.email, "alice@example.com");
    }

    #[test]
    fn parse_quoted_name_unescapes() {
        let a = parse_recipient(r#""Say \"Hi\"" <hi@example.com>"#).unwrap();
        assert_eq!(a.name.as_deref(), Some(r#"Say "Hi""#));
    }

    #[test]
    fn parse_angle_only() {
        let a = parse_recipient("<solo@example.com>").unwrap();
        assert_eq!(a.name, None);
        assert_eq!(a.email, "solo@example.com");
        assert_eq!(chip_label(&a), "solo@example.com");
        assert_eq!(chip_title(&a), "solo@example.com");
    }

    #[test]
    fn parse_empty_is_none() {
        assert!(parse_recipient("").is_none());
        assert!(parse_recipient("   ").is_none());
    }

    #[test]
    fn parse_invalid_kept_for_highlight() {
        let a = parse_recipient("not-an-email").unwrap();
        assert_eq!(a.email, "not-an-email");
        assert!(!chip_is_valid(&a));
        let b = parse_recipient("Ada <nope>").unwrap();
        assert_eq!(b.name.as_deref(), Some("Ada"));
        assert_eq!(b.email, "nope");
        assert!(!chip_is_valid(&b));
    }

    #[test]
    fn parse_empty_angle_mailbox_keeps_raw_token() {
        let a = parse_recipient("<>").unwrap();
        assert_eq!(a.email, "<>");
        assert_eq!(a.name, None);
        assert_eq!(chip_label(&a), "<>");
        assert!(!chip_is_valid(&a));
        let b = parse_recipient("< >").unwrap();
        assert_eq!(b.email, "< >");
        assert_eq!(chip_label(&b), "< >");
    }

    #[test]
    fn commit_live_leaves_tail() {
        let (chips, draft) = commit_input(&[], "ada@example.com, bob", false);
        assert_eq!(emails(&chips), ["ada@example.com"]);
        assert_eq!(draft, "bob");
    }

    #[test]
    fn commit_live_trailing_comma_clears_draft() {
        let (chips, draft) = commit_input(&[], "ada@example.com,  ", false);
        assert_eq!(emails(&chips), ["ada@example.com"]);
        assert_eq!(draft, "");
    }

    #[test]
    fn commit_tail_on_enter() {
        let (chips, draft) = commit_input(&[], "ada@example.com", true);
        assert_eq!(emails(&chips), ["ada@example.com"]);
        assert_eq!(draft, "");
    }

    #[test]
    fn commit_quoted_comma_is_not_a_separator() {
        let (chips, draft) =
            commit_input(&[], r#""Smith, Alice" <a@b.com>, bob@example.com"#, true);
        assert_eq!(chips.len(), 2);
        assert_eq!(chips[0].name.as_deref(), Some("Smith, Alice"));
        assert_eq!(chips[0].email, "a@b.com");
        assert_eq!(chips[1].email, "bob@example.com");
        assert_eq!(draft, "");
    }

    #[test]
    fn commit_skips_empty_tokens_and_dedupes() {
        let existing = vec![ComposerAddress::email_only("Ada@Example.com")];
        let (chips, draft) = commit_input(&existing, "ada@example.com,, bob@example.com", true);
        assert_eq!(emails(&chips), ["Ada@Example.com", "bob@example.com"]);
        assert_eq!(draft, "");
    }

    #[test]
    fn commit_dedupes_existing_chips() {
        let existing = vec![
            ComposerAddress::email_only("a@b.com"),
            ComposerAddress::email_only("A@b.com"),
        ];
        let (chips, draft) = commit_input(&existing, "", true);
        assert_eq!(emails(&chips), ["a@b.com"]);
        assert_eq!(draft, "");
    }

    #[test]
    fn commit_empty_is_noop() {
        let existing = vec![ComposerAddress::email_only("a@b.com")];
        let (chips, draft) = commit_input(&existing, "   ", true);
        assert_eq!(emails(&chips), ["a@b.com"]);
        assert_eq!(draft, "");
    }

    #[test]
    fn remove_at_and_last() {
        let chips = vec![
            ComposerAddress::email_only("a@b.com"),
            ComposerAddress::email_only("c@d.com"),
            ComposerAddress::email_only("e@f.com"),
        ];
        assert_eq!(emails(&remove_recipient(&chips, 1)), ["a@b.com", "e@f.com"]);
        assert_eq!(
            emails(&remove_last_recipient(&chips)),
            ["a@b.com", "c@d.com"]
        );
        assert_eq!(emails(&remove_recipient(&chips, 9)), emails(&chips));
        assert!(remove_last_recipient(&[]).is_empty());
    }
}
