//! Body-mode helpers for the compose overlay.
//!
//! The app owns the Dioxus chrome (recipients, attachments). This module keeps
//! [`DraftDocument`] plain/HTML fields consistent when the user toggles modes,
//! persists, or sends.

use crate::editor::rich::{
    html_for_edit, html_for_edit_from_plain, html_for_export, html_from_editor, plain_alternative,
};
use crate::model::convert::{html_to_plain, plain_to_html};
use crate::model::{BodyMode, DraftDocument};
use crate::reply::discard_rich_quote;
use crate::sanitize::sanitize_for_edit;
use crate::shell::attachment_list::html_for_plain_with_inlines;

/// Result of a plain ↔ rich switch. Text is preserved (formatting may be lost).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchBodyResult {
    /// Mode after the switch.
    pub mode: BodyMode,
    /// Plain body to show in the textarea (or the HTML alternative).
    pub plain: String,
    /// Sanitized HTML to inject into the editor. Empty in plain mode.
    pub html: String,
}

/// Apply the user's default format when *opening* a draft.
///
/// Existing rich HTML (reply/forward/IMAP) is kept and re-sanitized. A plain
/// source is converted with [`plain_to_html`]. Switching the preference to
/// plain drops the rich quote via [`discard_rich_quote`].
pub fn apply_preferred_mode(draft: &mut DraftDocument, preferred: BodyMode) {
    match (draft.mode, preferred) {
        (BodyMode::Rich, BodyMode::Rich) => {
            if draft.html_body.trim().is_empty() {
                draft.html_body = sanitize_for_edit(&plain_to_html(&draft.plain_body));
            } else {
                draft.html_body = sanitize_for_edit(&draft.html_body);
            }
            if draft.plain_body.is_empty() {
                draft.plain_body = html_to_plain(&draft.html_body);
            }
            draft.plain_cache_dirty = false;
        }
        (BodyMode::Plain, BodyMode::Rich) => {
            draft.html_body = sanitize_for_edit(&plain_to_html(&draft.plain_body));
            draft.mode = BodyMode::Rich;
            draft.plain_cache_dirty = false;
        }
        (_, BodyMode::Plain) => {
            if draft.mode == BodyMode::Rich {
                if draft.plain_body.trim().is_empty() || draft.plain_cache_dirty {
                    draft.plain_body = html_to_plain(&draft.html_body);
                }
                discard_rich_quote(draft);
            } else {
                draft.mode = BodyMode::Plain;
                draft.html_body.clear();
                draft.plain_cache_dirty = false;
            }
        }
    }
}

/// Convert the live editor/textarea contents to `next` without dropping text.
pub fn switch_body_mode(
    draft: &DraftDocument,
    live_plain: &str,
    live_html: &str,
    next: BodyMode,
) -> SwitchBodyResult {
    match next {
        BodyMode::Plain => {
            let html = live_html_source(draft, live_plain, live_html);
            let mut plain = if html.trim().is_empty() {
                live_plain.to_string()
            } else {
                html_to_plain(&html)
            };
            if plain.trim().is_empty() && !live_plain.trim().is_empty() {
                plain = live_plain.to_string();
            }
            SwitchBodyResult {
                mode: BodyMode::Plain,
                plain,
                html: String::new(),
            }
        }
        BodyMode::Rich => {
            let html = if draft.mode == BodyMode::Rich {
                let src = live_html_source(draft, live_plain, live_html);
                html_for_edit(&src, &draft.inline_images)
            } else {
                html_for_edit_from_plain(live_plain, &draft.inline_images)
            };
            SwitchBodyResult {
                mode: BodyMode::Rich,
                plain: plain_alternative(&html),
                html,
            }
        }
    }
}

/// Write live editor/textarea contents onto `draft` (edit-sanitized).
pub fn capture_live_body(draft: &mut DraftDocument, live_plain: &str, live_html: &str) {
    match draft.mode {
        BodyMode::Plain => {
            draft.plain_body = live_plain.to_string();
            draft.html_body.clear();
            draft.plain_cache_dirty = false;
        }
        BodyMode::Rich => {
            let html = live_html_source(draft, live_plain, live_html);
            draft.html_body = html_from_editor(&html);
            draft.plain_body = html_to_plain(&draft.html_body);
            draft.plain_cache_dirty = false;
        }
    }
}

/// Capture live contents and rewrite HTML for MIME (`cid:` only on images).
pub fn prepare_export_bodies(draft: &mut DraftDocument, live_plain: &str, live_html: &str) {
    capture_live_body(draft, live_plain, live_html);
    match draft.mode {
        BodyMode::Rich => {
            draft.html_body = html_for_export(&draft.html_body, &draft.inline_images);
            draft.plain_body = html_to_plain(&draft.html_body);
            draft.plain_cache_dirty = false;
        }
        BodyMode::Plain => {
            if !draft.inline_images.is_empty() {
                draft.html_body =
                    html_for_plain_with_inlines(&draft.plain_body, &draft.inline_images);
            }
        }
    }
}

fn live_html_source(draft: &DraftDocument, live_plain: &str, live_html: &str) -> String {
    if !live_html.trim().is_empty() {
        return live_html.to_string();
    }
    if !draft.html_body.trim().is_empty() {
        return draft.html_body.clone();
    }
    if !live_plain.is_empty() {
        return plain_to_html(live_plain);
    }
    String::new()
}

/// HTML to inject when (re)mounting the editor for `draft`.
pub fn editor_mount_html(draft: &DraftDocument) -> String {
    if draft.html_body.trim().is_empty() {
        html_for_edit_from_plain(&draft.plain_body, &draft.inline_images)
    } else {
        html_for_edit(&draft.html_body, &draft.inline_images)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::FromIdentity;

    fn draft() -> DraftDocument {
        DraftDocument::new_empty(&FromIdentity::new("Me", "me@example.com"))
    }

    #[test]
    fn toggle_plain_to_rich_keeps_text() {
        let mut d = draft();
        d.mode = BodyMode::Plain;
        d.plain_body = "Hello\n\nWorld".into();
        let switched = switch_body_mode(&d, "Hello\n\nWorld", "", BodyMode::Rich);
        assert_eq!(switched.mode, BodyMode::Rich);
        assert!(switched.html.contains("Hello"), "{}", switched.html);
        assert!(switched.html.contains("World"), "{}", switched.html);
        assert!(switched.plain.contains("Hello"), "{}", switched.plain);
        assert!(switched.plain.contains("World"), "{}", switched.plain);
    }

    #[test]
    fn toggle_rich_to_plain_keeps_text() {
        let mut d = draft();
        d.mode = BodyMode::Rich;
        d.html_body = "<p>Hello <b>world</b></p><p>Line 2</p>".into();
        let switched = switch_body_mode(&d, "", &d.html_body, BodyMode::Plain);
        assert_eq!(switched.mode, BodyMode::Plain);
        assert!(switched.html.is_empty());
        assert!(switched.plain.contains("Hello world"), "{}", switched.plain);
        assert!(switched.plain.contains("Line 2"), "{}", switched.plain);
    }

    #[test]
    fn toggle_roundtrip_does_not_drop_text() {
        let mut d = draft();
        d.mode = BodyMode::Plain;
        let live = "Café & tea <3";
        let rich = switch_body_mode(&d, live, "", BodyMode::Rich);
        d.mode = BodyMode::Rich;
        d.html_body = rich.html.clone();
        let plain = switch_body_mode(&d, "", &rich.html, BodyMode::Plain);
        assert!(plain.plain.contains("Café"), "{}", plain.plain);
        assert!(plain.plain.contains("tea"), "{}", plain.plain);
        assert!(
            plain.plain.contains("<3") || plain.plain.contains("&"),
            "{}",
            plain.plain
        );
    }

    #[test]
    fn preferred_rich_keeps_prefilled_html() {
        let mut d = draft();
        d.mode = BodyMode::Rich;
        d.html_body = "<p>Quoted <b>HTML</b></p>".into();
        d.plain_body = "Quoted HTML".into();
        apply_preferred_mode(&mut d, BodyMode::Rich);
        assert_eq!(d.mode, BodyMode::Rich);
        assert!(d.html_body.contains("Quoted"), "{}", d.html_body);
        assert!(
            d.html_body.contains("<b>") || d.html_body.contains("<strong>"),
            "{}",
            d.html_body
        );
    }

    #[test]
    fn preferred_rich_converts_plain_source() {
        let mut d = draft();
        d.mode = BodyMode::Plain;
        d.plain_body = "Just text".into();
        d.html_body.clear();
        apply_preferred_mode(&mut d, BodyMode::Rich);
        assert_eq!(d.mode, BodyMode::Rich);
        assert!(d.html_body.contains("Just text"), "{}", d.html_body);
    }

    #[test]
    fn preferred_plain_discards_rich_quote() {
        let mut d = draft();
        d.mode = BodyMode::Rich;
        d.html_body = "<p>Hi</p>".into();
        d.plain_body = "Hi".into();
        apply_preferred_mode(&mut d, BodyMode::Plain);
        assert_eq!(d.mode, BodyMode::Plain);
        assert!(d.html_body.is_empty());
        assert_eq!(d.plain_body, "Hi");
        assert!(d.inline_images.is_empty());
    }

    #[test]
    fn export_rich_sanitizes_and_builds_plain() {
        let mut d = draft();
        d.mode = BodyMode::Rich;
        prepare_export_bodies(&mut d, "", "<p>Hi<script>x()</script></p>");
        assert!(!d.html_body.to_ascii_lowercase().contains("script"));
        assert!(d.html_body.contains("Hi"), "{}", d.html_body);
        assert!(d.plain_body.contains("Hi"), "{}", d.plain_body);
    }

    #[test]
    fn export_plain_with_no_inlines_stays_plain() {
        let mut d = draft();
        d.mode = BodyMode::Plain;
        prepare_export_bodies(&mut d, "Hello", "<p>ignored</p>");
        assert_eq!(d.plain_body, "Hello");
        assert!(d.html_body.is_empty());
    }
}
