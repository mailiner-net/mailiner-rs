//! CSS / style sanitization helpers.

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
///
/// - Strip `@import`
/// - Strip `expression()`, `-moz-binding`, `behavior:`
/// - Remote `url(http…)`: strip when `!allow_remote`, keep when allow
/// - `url(cid:…)` left for attribute-level cid rewrite (style urls stripped if remote policy)
pub fn sanitize_css(css: &str, allow_remote: bool) -> String {
    let mut s = re_import().replace_all(css, "").into_owned();
    s = re_expression().replace_all(&s, "").into_owned();
    s = re_binding().replace_all(&s, "").into_owned();

    s = re_url()
        .replace_all(&s, |caps: &regex::Captures| {
            let url = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let lower = url.trim().to_ascii_lowercase();
            if lower.starts_with("cid:") {
                // leave cid for now; style cid resolution is limited in v1
                caps.get(0).unwrap().as_str().to_string()
            } else if lower.starts_with("data:") {
                // only allow data:image/* raster-ish
                if lower.starts_with("data:image/png")
                    || lower.starts_with("data:image/jpeg")
                    || lower.starts_with("data:image/gif")
                    || lower.starts_with("data:image/webp")
                {
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
                // relative / other — treat as remote for privacy
                if allow_remote {
                    caps.get(0).unwrap().as_str().to_string()
                } else {
                    String::new()
                }
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
        assert!(!s.to_ascii_lowercase().contains("@import"));
        assert!(s.contains("color:red"));
    }

    #[test]
    fn strips_remote_url_by_default() {
        let s = sanitize_css("background:url(https://evil/x.png)", false);
        assert!(!s.contains("https://evil"));
    }

    #[test]
    fn keeps_remote_when_allowed() {
        let s = sanitize_css("background:url(https://ok/x.png)", true);
        assert!(s.contains("https://ok"));
    }
}
