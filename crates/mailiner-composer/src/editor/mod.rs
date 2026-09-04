//! Plain and rich body editors.

pub mod commands;
pub mod mount;
pub mod plain;
pub mod rich;
pub mod toolbar;

pub use commands::{normalize_link_href, EditorCommand};
pub use mount::{
    exec_editor_command, focus_editor, insert_editor_html, mount_editor, prompt_link_href,
    read_editor_html, set_editor_enabled, EDITOR_HOST_ID,
};
pub use toolbar::{default_toolbar, ToolbarItem};

/// HTML `spellcheck` value for compose prose (subject, textarea, contenteditable).
///
/// Address fields (To/Cc/Bcc) should omit this or use [`SPELLCHECK_OFF`] —
/// mailbox strings are not dictionary words.
pub const SPELLCHECK: &str = "true";

/// HTML `spellcheck` value that disables the browser dictionary.
pub const SPELLCHECK_OFF: &str = "false";

/// Compose field used to choose a browser `spellcheck` attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpellcheckField {
    /// To / Cc / Bcc mailbox list.
    Address,
    /// Subject line.
    Subject,
    /// Plain textarea or rich contenteditable body.
    Body,
}

/// HTML `spellcheck` attribute for `field`.
///
/// No `lang` override: the browser uses the document / UI language.
pub fn spellcheck_attr(field: SpellcheckField) -> &'static str {
    match field {
        SpellcheckField::Address => SPELLCHECK_OFF,
        SpellcheckField::Subject | SpellcheckField::Body => SPELLCHECK,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prose_fields_opt_into_spellcheck() {
        assert_eq!(spellcheck_attr(SpellcheckField::Subject), "true");
        assert_eq!(spellcheck_attr(SpellcheckField::Body), "true");
        assert_eq!(spellcheck_attr(SpellcheckField::Address), "false");
    }
}
