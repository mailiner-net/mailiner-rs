//! Plain textarea body editor.
//!
//! When mounted, the textarea must set `spellcheck` to [`super::SPELLCHECK`].

use super::{spellcheck_attr, SpellcheckField};

/// HTML `spellcheck` value for the plain-text body textarea.
pub fn textarea_spellcheck() -> &'static str {
    spellcheck_attr(SpellcheckField::Body)
}

/// Authoritative plain body for persist / export while the textarea is live.
pub fn export_plain(live: &str) -> String {
    live.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn textarea_opts_into_spellcheck() {
        assert_eq!(textarea_spellcheck(), super::super::SPELLCHECK);
    }

    #[test]
    fn export_plain_keeps_user_text() {
        assert_eq!(export_plain("Hello\nWorld"), "Hello\nWorld");
    }
}
