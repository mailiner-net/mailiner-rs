//! Plain-text signature append (RFC 3676 sigdash).

use crate::model::draft::DraftDocument;

/// RFC 3676 signature delimiter: two hyphens and a space, on its own line.
pub const SIGDASH: &str = "-- ";

/// Append `signature` after `body`, separated by a sigdash line.
///
/// Empty / whitespace-only signatures are a no-op (body unchanged).
pub fn append_plain_signature(body: &str, signature: Option<&str>) -> String {
    let Some(sig) = signature.map(str::trim).filter(|s| !s.is_empty()) else {
        return body.to_string();
    };
    if body.is_empty() {
        format!("\n{SIGDASH}\n{sig}")
    } else {
        format!("{}\n{SIGDASH}\n{sig}", body.trim_end())
    }
}

/// Apply [`append_plain_signature`] to a freshly built draft's plain body.
///
/// Call only when *building* the initial draft so a reopen does not rewrite
/// user edits.
pub fn apply_plain_signature(draft: &mut DraftDocument, signature: Option<&str>) {
    draft.plain_body = append_plain_signature(&draft.plain_body, signature);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::FromIdentity;
    use crate::model::draft::DraftDocument;

    #[test]
    fn missing_signature_is_noop() {
        assert_eq!(append_plain_signature("hello", None), "hello");
        assert_eq!(append_plain_signature("hello", Some("")), "hello");
        assert_eq!(append_plain_signature("hello", Some("  \n  ")), "hello");
        assert_eq!(append_plain_signature("", None), "");
    }

    #[test]
    fn appends_sigdash_after_body() {
        assert_eq!(
            append_plain_signature("Hello", Some("Jane Doe")),
            "Hello\n-- \nJane Doe"
        );
        assert_eq!(
            append_plain_signature("Hello\n\n", Some("Jane Doe")),
            "Hello\n-- \nJane Doe"
        );
        assert_eq!(
            append_plain_signature("", Some("Jane Doe")),
            "\n-- \nJane Doe"
        );
    }

    #[test]
    fn apply_updates_draft_plain_body() {
        let id = FromIdentity::new("Me", "me@example.com");
        let mut draft = DraftDocument::new_empty(&id);
        draft.plain_body = "Hi".into();
        apply_plain_signature(&mut draft, Some("Jane"));
        assert_eq!(draft.plain_body, "Hi\n-- \nJane");
        apply_plain_signature(&mut draft, None);
        assert_eq!(draft.plain_body, "Hi\n-- \nJane");
    }
}
