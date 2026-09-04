//! Toolbar descriptors for the rich compose editor.

use super::commands::EditorCommand;

/// One toolbar control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolbarItem {
    /// Stable id (`bold`, `link`, …).
    pub id: &'static str,
    /// Short visible label (ASCII fallback when no icon is used).
    pub label: &'static str,
    /// Accessible name / tooltip.
    pub title: &'static str,
    /// Command to run.
    pub command: EditorCommand,
}

/// Default v1 toolbar: emphasis, lists, quote, heading, link.
pub fn default_toolbar() -> &'static [ToolbarItem] {
    &[
        ToolbarItem {
            id: "bold",
            label: "B",
            title: "Bold",
            command: EditorCommand::Bold,
        },
        ToolbarItem {
            id: "italic",
            label: "I",
            title: "Italic",
            command: EditorCommand::Italic,
        },
        ToolbarItem {
            id: "underline",
            label: "U",
            title: "Underline",
            command: EditorCommand::Underline,
        },
        ToolbarItem {
            id: "ul",
            label: "•",
            title: "Bulleted list",
            command: EditorCommand::InsertUnorderedList,
        },
        ToolbarItem {
            id: "ol",
            label: "1.",
            title: "Numbered list",
            command: EditorCommand::InsertOrderedList,
        },
        ToolbarItem {
            id: "quote",
            label: "“",
            title: "Quote",
            command: EditorCommand::FormatBlockQuote,
        },
        ToolbarItem {
            id: "heading",
            label: "H",
            title: "Heading",
            command: EditorCommand::FormatHeading,
        },
        ToolbarItem {
            id: "link",
            label: "Link",
            title: "Link",
            command: EditorCommand::CreateLink,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_toolbar_covers_v1_commands() {
        let ids: Vec<&str> = default_toolbar().iter().map(|i| i.id).collect();
        assert_eq!(
            ids,
            [
                "bold",
                "italic",
                "underline",
                "ul",
                "ol",
                "quote",
                "heading",
                "link"
            ]
        );
        assert!(default_toolbar()
            .iter()
            .any(|i| i.command == EditorCommand::Bold));
    }
}
