//! Clean a short body prefix for the message-list preview line.

/// Display cap (chars). Fetch a bit more wire octets so HTML/quotes still yield this.
pub const SNIPPET_MAX_CHARS: usize = 120;
/// `BODY.PEEK[section]<0.N>` window. Enough for ~120 chars after decode + HTML strip.
pub const SNIPPET_FETCH_OCTETS: usize = 2048;

/// Strip markup/quotes, collapse whitespace, and truncate for a list row.
pub fn clean_snippet(raw: &str) -> String {
    let without_tags = strip_html(raw);
    let without_quotes = strip_quoted_lines(&without_tags);
    let collapsed = collapse_ws(&without_quotes);
    truncate_chars(&collapsed, SNIPPET_MAX_CHARS)
}

fn strip_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let lower = input.to_ascii_lowercase();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if let Some(tag_end) = lower[i..].find('>') {
                let tag = &lower[i..i + tag_end];
                let skip_block = tag.starts_with("<script")
                    || tag.starts_with("<style")
                    || tag.starts_with("<head")
                    || tag.starts_with("<noscript");
                if skip_block {
                    let close = if tag.starts_with("<script") {
                        "</script>"
                    } else if tag.starts_with("<style") {
                        "</style>"
                    } else if tag.starts_with("<head") {
                        "</head>"
                    } else {
                        "</noscript>"
                    };
                    if let Some(rel) = lower[i + tag_end..].find(close) {
                        i += tag_end + rel + close.len();
                        out.push(' ');
                        continue;
                    }
                }
                // Block-ish tags become a word break so "foo</p><p>bar" stays two words.
                out.push(' ');
                i += tag_end + 1;
                continue;
            }
            // Unclosed `<` — keep the rest as text.
            out.push('<');
            i += 1;
            continue;
        }
        if bytes[i] == b'&'
            && let Some((ch, consumed)) = decode_entity(&input[i..])
        {
            out.push(ch);
            i += consumed;
            continue;
        }
        // input is valid UTF-8; copy the next char.
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn decode_entity(s: &str) -> Option<(char, usize)> {
    let rest = s.strip_prefix('&')?;
    if let Some(named) = rest.find(';').map(|n| &rest[..n]) {
        let consumed = named.len() + 2;
        let ch = match named {
            "nbsp" => ' ',
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" | "#39" => '\'',
            numeric if numeric.starts_with('#') => decode_numeric_entity(numeric)?,
            _ => return None,
        };
        return Some((ch, consumed));
    }
    None
}

fn decode_numeric_entity(body: &str) -> Option<char> {
    let digits = body.strip_prefix('#')?;
    let value = if let Some(hex) = digits
        .strip_prefix('x')
        .or_else(|| digits.strip_prefix('X'))
    {
        u32::from_str_radix(hex, 16).ok()?
    } else {
        digits.parse().ok()?
    };
    char::from_u32(value)
}

fn strip_quoted_lines(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for line in input.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('>') {
            continue;
        }
        if is_wrote_line(trimmed) {
            continue;
        }
        if trimmed == "--" || trimmed == "-- " {
            break;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
    }
    out
}

fn is_wrote_line(line: &str) -> bool {
    let t = line.trim();
    let lower = t.to_ascii_lowercase();
    lower.starts_with("on ") && lower.ends_with(" wrote:")
}

fn collapse_ws(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_space = false;
    for ch in input.chars() {
        if ch.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

fn truncate_chars(input: &str, max: usize) -> String {
    let mut out = String::new();
    for (i, ch) in input.chars().enumerate() {
        if i >= max {
            break;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_whitespace() {
        assert_eq!(
            clean_snippet("Hello,\n\n  world\tfrom\r\nMailiner"),
            "Hello, world from Mailiner"
        );
    }

    #[test]
    fn strips_html_tags_and_entities() {
        assert_eq!(
            clean_snippet("<p>Hello&nbsp;<b>world</b> &amp; friends&#39;s</p>"),
            "Hello world & friends's"
        );
    }

    #[test]
    fn strips_script_and_style() {
        let raw = "<html><head><style>p{color:red}</style></head>\
                   <body><script>alert(1)</script><p>Visible</p></body></html>";
        assert_eq!(clean_snippet(raw), "Visible");
    }

    #[test]
    fn strips_quoted_reply_and_signature() {
        let raw = "Thanks for the update.\n\
                   > quoted history\n\
                   >> still quoted\n\
                   On Mon, Jan 1, Alice wrote:\n\
                   -- \n\
                   Jane Doe\n\
                   Engineer";
        assert_eq!(clean_snippet(raw), "Thanks for the update.");
    }

    #[test]
    fn truncates_to_max_chars() {
        let long = "a".repeat(SNIPPET_MAX_CHARS + 40);
        let out = clean_snippet(&long);
        assert_eq!(out.chars().count(), SNIPPET_MAX_CHARS);
        assert!(out.chars().all(|c| c == 'a'));
    }

    #[test]
    fn empty_after_cleanup() {
        assert!(clean_snippet("   \n\t  ").is_empty());
        assert!(clean_snippet("<div>  </div>").is_empty());
        assert!(clean_snippet("> only quotes").is_empty());
    }

    #[test]
    fn keeps_unicode() {
        assert_eq!(clean_snippet("Café ☕\nnaïve"), "Café ☕ naïve");
    }

    #[test]
    fn unclosed_tag_does_not_eat_body() {
        assert_eq!(
            clean_snippet("before <notatag after"),
            "before <notatag after"
        );
    }
}
