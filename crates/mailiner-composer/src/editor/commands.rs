//! Formatting commands for the rich compose editor.

/// A v1 toolbar / keyboard formatting command.
///
/// Executed via `document.execCommand` in the WASM mount layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorCommand {
    /// Toggle bold.
    Bold,
    /// Toggle italic.
    Italic,
    /// Toggle underline.
    Underline,
    /// Toggle a bulleted list.
    InsertUnorderedList,
    /// Toggle a numbered list.
    InsertOrderedList,
    /// Wrap the selection in a block quote.
    FormatBlockQuote,
    /// Turn the current block into a heading (`h2`).
    FormatHeading,
    /// Restore a paragraph block.
    FormatParagraph,
    /// Create a hyperlink (needs a URL argument).
    CreateLink,
    /// Remove the link around the selection.
    Unlink,
}

impl EditorCommand {
    /// `document.execCommand` name.
    pub fn exec_name(self) -> &'static str {
        match self {
            Self::Bold => "bold",
            Self::Italic => "italic",
            Self::Underline => "underline",
            Self::InsertUnorderedList => "insertUnorderedList",
            Self::InsertOrderedList => "insertOrderedList",
            Self::FormatBlockQuote | Self::FormatHeading | Self::FormatParagraph => "formatBlock",
            Self::CreateLink => "createLink",
            Self::Unlink => "unlink",
        }
    }

    /// Value argument for `formatBlock` / `createLink`. `None` means empty string.
    pub fn exec_value(self) -> Option<&'static str> {
        match self {
            Self::FormatBlockQuote => Some("blockquote"),
            Self::FormatHeading => Some("h2"),
            Self::FormatParagraph => Some("p"),
            _ => None,
        }
    }
}

/// Normalize a typed link target for `createLink`.
///
/// Accepts `http(s):`, `mailto:`, in-page `#` fragments, and bare hosts
/// (prefixed with `https://`). Rejects `javascript:`, `vbscript:`, `data:`,
/// protocol-relative URLs, and empty input.
pub fn normalize_link_href(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let lower = raw.to_ascii_lowercase();
    if lower.starts_with("javascript:")
        || lower.starts_with("vbscript:")
        || lower.starts_with("data:")
        || lower.starts_with("//")
    {
        return None;
    }
    if lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:")
        || lower.starts_with('#')
    {
        return Some(raw.to_string());
    }
    if lower.contains(':') {
        return None;
    }
    Some(format!("https://{raw}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_names_match_document_commands() {
        assert_eq!(EditorCommand::Bold.exec_name(), "bold");
        assert_eq!(EditorCommand::Italic.exec_name(), "italic");
        assert_eq!(EditorCommand::Underline.exec_name(), "underline");
        assert_eq!(
            EditorCommand::InsertUnorderedList.exec_name(),
            "insertUnorderedList"
        );
        assert_eq!(
            EditorCommand::InsertOrderedList.exec_name(),
            "insertOrderedList"
        );
        assert_eq!(EditorCommand::FormatBlockQuote.exec_name(), "formatBlock");
        assert_eq!(
            EditorCommand::FormatBlockQuote.exec_value(),
            Some("blockquote")
        );
        assert_eq!(EditorCommand::FormatHeading.exec_value(), Some("h2"));
        assert_eq!(EditorCommand::CreateLink.exec_name(), "createLink");
        assert_eq!(EditorCommand::Unlink.exec_name(), "unlink");
    }

    #[test]
    fn normalize_link_accepts_safe_schemes() {
        assert_eq!(
            normalize_link_href("https://example.com/a"),
            Some("https://example.com/a".into())
        );
        assert_eq!(
            normalize_link_href("http://example.com"),
            Some("http://example.com".into())
        );
        assert_eq!(
            normalize_link_href("mailto:a@b.com"),
            Some("mailto:a@b.com".into())
        );
        assert_eq!(normalize_link_href("#section"), Some("#section".into()));
        assert_eq!(
            normalize_link_href("example.com/x"),
            Some("https://example.com/x".into())
        );
    }

    #[test]
    fn normalize_link_rejects_unsafe() {
        assert!(normalize_link_href("").is_none());
        assert!(normalize_link_href("   ").is_none());
        assert!(normalize_link_href("javascript:alert(1)").is_none());
        assert!(normalize_link_href("JAVASCRIPT:alert(1)").is_none());
        assert!(normalize_link_href("data:text/html,x").is_none());
        assert!(normalize_link_href("//evil.example/x").is_none());
        assert!(normalize_link_href("ftp://files.example/x").is_none());
    }
}
