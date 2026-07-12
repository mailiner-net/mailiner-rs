//! Attribution lines and quote wrappers.

use chrono::{DateTime, Utc};

use crate::model::ComposerAddress;

/// Format a simple attribution for reply/forward.
pub fn attribution_line(
    when: DateTime<Utc>,
    from: Option<&ComposerAddress>,
) -> String {
    let who = match from {
        Some(a) => match &a.name {
            Some(n) if !n.is_empty() => format!("{n} <{}>", a.email),
            _ => a.email.clone(),
        },
        None => "Unknown".to_string(),
    };
    // RFC-ish readable date
    let date = when.format("%a, %d %b %Y %H:%M:%S +0000");
    format!("On {date}, {who} wrote:")
}

/// Wrap plain body with `>` quote markers.
pub fn quote_plain(attribution: &str, body: &str) -> String {
    let mut out = String::new();
    out.push_str(attribution);
    out.push_str("\n\n");
    if body.is_empty() {
        out.push('>');
        return out;
    }
    for line in body.lines() {
        out.push('>');
        if !line.is_empty() {
            out.push(' ');
            out.push_str(line);
        }
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// Prefix subject with Re: / Fwd: after stripping existing markers.
pub fn subject_with_prefix(original: Option<&str>, prefix: &str) -> String {
    let raw = original.unwrap_or("").trim();
    let stripped = strip_subject_prefixes(raw);
    let p = if prefix.ends_with(':') {
        format!("{prefix} ")
    } else {
        format!("{prefix}: ")
    };
    if stripped.is_empty() {
        p.trim_end().to_string()
    } else {
        format!("{p}{stripped}")
    }
}

fn strip_subject_prefixes(s: &str) -> String {
    let mut rest = s.trim();
    loop {
        let lower = rest.to_ascii_lowercase();
        let next = if let Some(r) = lower.strip_prefix("re:") {
            r
        } else if let Some(r) = lower.strip_prefix("fwd:") {
            r
        } else if let Some(r) = lower.strip_prefix("fw:") {
            r
        } else {
            break;
        };
        // Map back to original length of prefix on rest
        let prefix_len = rest.len() - next.len();
        rest = rest[prefix_len..].trim_start();
    }
    rest.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_re_fwd() {
        assert_eq!(subject_with_prefix(Some("Re: Re: Hi"), "Re:"), "Re: Hi");
        assert_eq!(subject_with_prefix(Some("Fwd: Hello"), "Fwd:"), "Fwd: Hello");
        assert_eq!(subject_with_prefix(Some("Hello"), "Re:"), "Re: Hello");
    }

    #[test]
    fn quote_plain_marks_lines() {
        let q = quote_plain("On date, a wrote:", "line1\nline2");
        assert!(q.contains("> line1"), "{q}");
        assert!(q.contains("> line2"), "{q}");
    }
}
