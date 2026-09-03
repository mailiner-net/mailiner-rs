//! Folder name validation and IMAP hierarchy-delimiter joining.

use thiserror::Error;

/// Why a user-supplied folder name cannot be used on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FolderNameError {
    #[error("Folder name cannot be empty")]
    Empty,
    #[error("Folder name cannot contain the hierarchy separator")]
    ContainsDelimiter,
    #[error("Folder name contains invalid characters")]
    InvalidCharacter,
    #[error("This server does not support nested folders")]
    NoHierarchy,
}

/// IMAP `INBOX` is case-insensitive and must not be deleted.
pub fn is_inbox_mailbox(name: &str) -> bool {
    name.eq_ignore_ascii_case("inbox")
}

/// Trim and reject empty names, controls, and the hierarchy delimiter.
pub fn validate_folder_name<'a>(
    name: &'a str,
    delimiter: Option<&str>,
) -> Result<&'a str, FolderNameError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(FolderNameError::Empty);
    }
    if name.chars().any(|c| c.is_control()) {
        return Err(FolderNameError::InvalidCharacter);
    }
    if delimiter
        .filter(|d| !d.is_empty())
        .is_some_and(|d| name.contains(d))
    {
        return Err(FolderNameError::ContainsDelimiter);
    }
    Ok(name)
}

/// `parent + delimiter + name`, or just `name` at the root.
pub fn join_mailbox_path(
    parent: Option<&str>,
    name: &str,
    delimiter: Option<&str>,
) -> Result<String, FolderNameError> {
    let name = validate_folder_name(name, delimiter)?;
    match parent.map(str::trim).filter(|p| !p.is_empty()) {
        None => Ok(name.to_string()),
        Some(parent) => {
            let Some(d) = delimiter.filter(|d| !d.is_empty()) else {
                return Err(FolderNameError::NoHierarchy);
            };
            Ok(format!("{parent}{d}{name}"))
        }
    }
}

/// Replace the last path segment of `full`, keeping any parent prefix.
pub fn rename_mailbox_path(
    full: &str,
    new_leaf: &str,
    delimiter: Option<&str>,
) -> Result<String, FolderNameError> {
    let new_leaf = validate_folder_name(new_leaf, delimiter)?;
    let (parent, _) = mailbox_parent_and_leaf(full, delimiter);
    match (parent, delimiter.filter(|d| !d.is_empty())) {
        (Some(parent), Some(d)) => Ok(format!("{parent}{d}{new_leaf}")),
        _ => Ok(new_leaf.to_string()),
    }
}

/// Split `full` into `(parent, leaf)` using the server delimiter.
///
/// A missing or empty delimiter is a flat name — do not split on `/`.
pub fn mailbox_parent_and_leaf<'a>(
    full: &'a str,
    delimiter: Option<&str>,
) -> (Option<&'a str>, &'a str) {
    match delimiter.filter(|d| !d.is_empty()) {
        Some(d) => match full.rsplit_once(d) {
            Some((parent, leaf)) if !parent.is_empty() && !leaf.is_empty() => (Some(parent), leaf),
            _ => (None, full),
        },
        None => (None, full),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_trims_and_rejects_empty() {
        assert_eq!(validate_folder_name("  Work  ", Some(".")).unwrap(), "Work");
        assert_eq!(
            validate_folder_name("   ", Some(".")),
            Err(FolderNameError::Empty)
        );
        assert_eq!(validate_folder_name("", None), Err(FolderNameError::Empty));
    }

    #[test]
    fn validate_rejects_delimiter_and_controls() {
        assert_eq!(
            validate_folder_name("foo.bar", Some(".")),
            Err(FolderNameError::ContainsDelimiter)
        );
        assert_eq!(
            validate_folder_name("foo/bar", Some("/")),
            Err(FolderNameError::ContainsDelimiter)
        );
        assert_eq!(
            validate_folder_name("foo\0bar", Some(".")),
            Err(FolderNameError::InvalidCharacter)
        );
        assert_eq!(
            validate_folder_name("foo\nbar", None),
            Err(FolderNameError::InvalidCharacter)
        );
        // Slash is fine when the server delimiter is a dot.
        assert_eq!(
            validate_folder_name("foo/bar", Some(".")).unwrap(),
            "foo/bar"
        );
    }

    #[test]
    fn join_root_and_nested() {
        assert_eq!(join_mailbox_path(None, "Work", Some(".")).unwrap(), "Work");
        assert_eq!(
            join_mailbox_path(Some("INBOX"), "Work", Some(".")).unwrap(),
            "INBOX.Work"
        );
        assert_eq!(
            join_mailbox_path(Some("[Gmail]"), "Work", Some("/")).unwrap(),
            "[Gmail]/Work"
        );
        assert_eq!(
            join_mailbox_path(Some("  "), "Work", Some(".")).unwrap(),
            "Work"
        );
    }

    #[test]
    fn join_requires_delimiter_for_parent() {
        assert_eq!(
            join_mailbox_path(Some("INBOX"), "Work", None),
            Err(FolderNameError::NoHierarchy)
        );
        assert_eq!(
            join_mailbox_path(Some("INBOX"), "Work", Some("")),
            Err(FolderNameError::NoHierarchy)
        );
    }

    #[test]
    fn join_rejects_bad_leaf() {
        assert_eq!(
            join_mailbox_path(Some("INBOX"), "Work.A", Some(".")),
            Err(FolderNameError::ContainsDelimiter)
        );
        assert_eq!(
            join_mailbox_path(None, "", Some(".")),
            Err(FolderNameError::Empty)
        );
    }

    #[test]
    fn rename_replaces_leaf_only() {
        assert_eq!(
            rename_mailbox_path("INBOX.Work", "Archive", Some(".")).unwrap(),
            "INBOX.Archive"
        );
        assert_eq!(
            rename_mailbox_path("INBOX/Work/A", "B", Some("/")).unwrap(),
            "INBOX/Work/B"
        );
        assert_eq!(
            rename_mailbox_path("Work", "Archive", Some(".")).unwrap(),
            "Archive"
        );
        assert_eq!(
            rename_mailbox_path("foo/bar", "Archive", None).unwrap(),
            "Archive"
        );
    }

    #[test]
    fn rename_rejects_delimiter_in_leaf() {
        assert_eq!(
            rename_mailbox_path("INBOX.Work", "A.B", Some(".")),
            Err(FolderNameError::ContainsDelimiter)
        );
    }

    #[test]
    fn parent_and_leaf_respects_delimiter() {
        assert_eq!(
            mailbox_parent_and_leaf("INBOX.Work", Some(".")),
            (Some("INBOX"), "Work")
        );
        assert_eq!(
            mailbox_parent_and_leaf("[Gmail]/Sent Mail", Some("/")),
            (Some("[Gmail]"), "Sent Mail")
        );
        assert_eq!(mailbox_parent_and_leaf("INBOX", Some(".")), (None, "INBOX"));
        assert_eq!(mailbox_parent_and_leaf("foo/bar", None), (None, "foo/bar"));
    }

    #[test]
    fn inbox_name_is_case_insensitive() {
        assert!(is_inbox_mailbox("INBOX"));
        assert!(is_inbox_mailbox("Inbox"));
        assert!(is_inbox_mailbox("inbox"));
        assert!(!is_inbox_mailbox("INBOX.Work"));
        assert!(!is_inbox_mailbox("Sent"));
    }
}
