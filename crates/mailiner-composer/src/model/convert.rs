//! Minimal plain ↔ HTML conversion for mode switch and export alternative.

/// Convert HTML fragment to plain text (lossy).
///
/// Rules (design v1):
/// - Block tags (`p`, `div`, `tr`, `li`, `br`, `h1`–`h6`, `blockquote`) → newline boundaries
/// - `li` → prefix `"• "`
/// - Strip all tags
/// - Decode common entities (named + numeric)
/// - Collapse 3+ newlines to 2
/// - Trim trailing space per line
pub fn html_to_plain(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut chars = html.char_indices().peekable();
    let mut in_tag = false;
    let mut tag_buf = String::new();

    while let Some((i, c)) = chars.next() {
        if !in_tag {
            if c == '<' {
                in_tag = true;
                tag_buf.clear();
                continue;
            }
            if c == '&' {
                if let Some((decoded, end)) = decode_entity_at(html, i) {
                    out.push_str(&decoded);
                    // Advance chars past the entity.
                    while let Some(&(j, _)) = chars.peek() {
                        if j < end {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    continue;
                }
            }
            out.push(c);
        } else if c == '>' {
            in_tag = false;
            let tag = tag_buf.trim().to_ascii_lowercase();
            let name = tag
                .trim_start_matches('/')
                .split(|ch: char| ch.is_whitespace() || ch == '/')
                .next()
                .unwrap_or("");
            let is_close = tag.starts_with('/');
            match name {
                "br" => out.push('\n'),
                "p" | "div" | "tr" | "blockquote" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    out.push('\n');
                }
                "li" if !is_close => {
                    out.push('\n');
                    out.push_str("• ");
                }
                "li" if is_close => out.push('\n'),
                _ => {}
            }
        } else {
            tag_buf.push(c);
        }
    }

    normalize_plain(&out)
}

fn decode_entity_at(s: &str, start: usize) -> Option<(String, usize)> {
    let rest = &s[start..];
    if !rest.starts_with('&') {
        return None;
    }
    let end_rel = rest.find(';')?;
    let entity = &rest[..=end_rel];
    let end = start + end_rel + 1;

    let decoded = match entity {
        "&amp;" => "&".to_string(),
        "&lt;" => "<".to_string(),
        "&gt;" => ">".to_string(),
        "&nbsp;" => " ".to_string(),
        "&quot;" => "\"".to_string(),
        "&apos;" => "'".to_string(),
        e if e.starts_with("&#x") || e.starts_with("&#X") => {
            let hex = e[3..e.len() - 1].trim();
            let code = u32::from_str_radix(hex, 16).ok()?;
            char::from_u32(code)?.to_string()
        }
        e if e.starts_with("&#") => {
            let num = e[2..e.len() - 1].trim();
            let code: u32 = num.parse().ok()?;
            char::from_u32(code)?.to_string()
        }
        _ => return None,
    };
    Some((decoded, end))
}

fn normalize_plain(s: &str) -> String {
    let mut collapsed = String::with_capacity(s.len());
    let mut nl = 0;
    for line in s.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            nl += 1;
            if nl <= 2 {
                collapsed.push('\n');
            }
        } else {
            if !collapsed.is_empty() && !collapsed.ends_with('\n') {
                collapsed.push('\n');
            }
            // If we already emitted blank newlines, we're positioned correctly.
            collapsed.push_str(line);
            nl = 0;
            collapsed.push('\n');
        }
    }
    // Collapse 3+ newlines again on the joined string
    let mut out = String::with_capacity(collapsed.len());
    let mut nrun = 0;
    for ch in collapsed.chars() {
        if ch == '\n' {
            nrun += 1;
            if nrun <= 2 {
                out.push(ch);
            }
        } else {
            nrun = 0;
            out.push(ch);
        }
    }
    out.trim_end().to_string()
}

/// Escape plain text into a minimal HTML fragment.
///
/// - Escape HTML
/// - Split on blank lines → `<p>…</p>`
/// - Single newlines → `<br>`
pub fn plain_to_html(plain: &str) -> String {
    let mut out = String::new();
    let paragraphs: Vec<&str> = plain.split("\n\n").collect();
    for para in paragraphs {
        out.push_str("<p>");
        let lines: Vec<&str> = para.split('\n').collect();
        for (li, line) in lines.iter().enumerate() {
            if li > 0 {
                out.push_str("<br>");
            }
            out.push_str(&escape_html(line));
        }
        out.push_str("</p>");
    }
    if out.is_empty() {
        out.push_str("<p></p>");
    }
    out
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_to_plain_basic() {
        let p = html_to_plain("<p>Hello <b>world</b></p><p>Line2</p>");
        assert!(p.contains("Hello world"), "{p}");
        assert!(p.contains("Line2"), "{p}");
    }

    #[test]
    fn html_to_plain_entities_and_li() {
        let p = html_to_plain("<ul><li>A&amp;B</li><li>C</li></ul>");
        assert!(p.contains("• A&B"), "{p}");
        assert!(p.contains("• C"), "{p}");
    }

    #[test]
    fn html_to_plain_utf8_and_numeric_entity() {
        let p = html_to_plain("<p>café &#65;</p>");
        assert!(p.contains("café"), "{p}");
        assert!(p.contains('A'), "{p}");
    }

    #[test]
    fn plain_to_html_paragraphs() {
        let h = plain_to_html("a\nb\n\nc");
        assert!(h.contains("<br>"), "{h}");
        assert!(h.contains("c"), "{h}");
    }

    #[test]
    fn plain_to_html_escapes() {
        let h = plain_to_html("<script>");
        assert!(h.contains("&lt;script&gt;"), "{h}");
    }
}
