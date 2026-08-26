//! CSS sanitization helpers.

use regex::Regex;
use std::sync::OnceLock;

fn re_import() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)@import[^;]+;?").unwrap())
}

fn re_expression() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)expression\s*\([^)]*\)").unwrap())
}

fn re_binding() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)(-moz-binding|behavior)\s*:\s*[^;]+;?").unwrap())
}

fn re_url() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"(?i)url\s*\(\s*['"]?([^'")]+)['"]?\s*\)"#).unwrap())
}

/// Sanitize CSS text inside a `<style>` block or style attribute.
pub fn sanitize_css(css: &str, allow_remote: bool) -> String {
    let mut s = re_import().replace_all(css, "").into_owned();
    s = re_expression().replace_all(&s, "").into_owned();
    s = re_binding().replace_all(&s, "").into_owned();

    s = re_url()
        .replace_all(&s, |caps: &regex::Captures| {
            let url = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let lower = url.trim().to_ascii_lowercase();
            if lower.starts_with("javascript:") || lower.starts_with("vbscript:") {
                String::new()
            } else if lower.starts_with("cid:") {
                caps.get(0).unwrap().as_str().to_string()
            } else if lower.starts_with("data:") {
                if crate::is_safe_data_image(&lower) {
                    caps.get(0).unwrap().as_str().to_string()
                } else {
                    String::new()
                }
            } else if lower.starts_with("http://") || lower.starts_with("https://") {
                if allow_remote {
                    caps.get(0).unwrap().as_str().to_string()
                } else {
                    String::new()
                }
            } else {
                // relative / other — never keep under allow_remote either if not http(s)
                String::new()
            }
        })
        .into_owned();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_import() {
        let s = sanitize_css("@import url('https://x'); body{color:red}", false);
        assert!(!s.contains("@import"), "{s}");
        assert!(s.contains("color"), "{s}");
    }
}
