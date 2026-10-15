//! URL prefix for a deploy that is not at the host root.
//!
//! Dioxus reads this from `<meta name="DIOXUS_ASSET_ROOT">` in debug builds and
//! from `DIOXUS_ASSET_ROOT` baked in at compile time for release
//! (`dx build --base-path /app` or `[web.app] base_path`). The router and
//! `asset!()` files already use it. Files copied from `public/` and the OAuth
//! redirect URI do not, so they go through [`with_base`].

/// Site path with the deploy prefix applied.
///
/// `path` is a root path such as `/sw.js` or `/oauth/callback`. An empty base
/// leaves it unchanged. `app` or `/app/` becomes `/app/sw.js`.
pub fn with_base(path: &str) -> String {
    with_prefix(current().as_deref(), path)
}

/// Same as [`with_base`] with an explicit prefix. `None`, `""`, and `"."`
/// mean the host root (Dioxus uses `"."` as the empty `{base_path}` token).
pub fn with_prefix(base: Option<&str>, path: &str) -> String {
    let base = base.map(normalize_base).unwrap_or_default();
    let rel = path.trim().trim_start_matches('/');
    if base.is_empty() {
        if rel.is_empty() {
            "/".to_string()
        } else {
            format!("/{rel}")
        }
    } else if rel.is_empty() {
        format!("/{base}/")
    } else {
        format!("/{base}/{rel}")
    }
}

/// Trimmed deploy prefix (`app`), or `None` at the host root.
pub fn current() -> Option<String> {
    let raw = dioxus_cli_config::base_path()?;
    let trimmed = normalize_base(&raw);
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn normalize_base(base: &str) -> String {
    let base = base.trim().trim_matches('/');
    if base.is_empty() || base == "." {
        String::new()
    } else {
        base.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::with_prefix;

    #[test]
    fn public_shell_is_not_pinned_to_the_host_root() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../public/manifest.webmanifest")).unwrap();
        for key in ["id", "start_url", "scope"] {
            let value = manifest[key].as_str().unwrap();
            assert!(!value.starts_with('/'), "{key}={value}");
        }
        for icon in manifest["icons"].as_array().unwrap() {
            let src = icon["src"].as_str().unwrap();
            assert!(!src.starts_with('/'), "{src}");
        }

        let sw = include_str!("../public/sw.js");
        assert!(sw.contains("mailiner-shell-v4"));
        assert!(sw.contains("new URL(\"./\", self.location)"));
        assert!(!sw.contains("\"/index.html\""));
        assert!(!sw.contains("\"/manifest.webmanifest\""));
        assert!(!sw.contains("\"/icons/"));
    }

    #[test]
    fn root_base_keeps_absolute_paths() {
        assert_eq!(with_prefix(None, "/sw.js"), "/sw.js");
        assert_eq!(with_prefix(Some(""), "/"), "/");
        assert_eq!(
            with_prefix(Some("."), "/manifest.webmanifest"),
            "/manifest.webmanifest"
        );
        assert_eq!(
            with_prefix(Some("/"), "/icons/icon-192.png"),
            "/icons/icon-192.png"
        );
    }

    #[test]
    fn prefix_is_inserted_once() {
        assert_eq!(with_prefix(Some("app"), "/sw.js"), "/app/sw.js");
        assert_eq!(
            with_prefix(Some("/app/"), "/oauth/callback"),
            "/app/oauth/callback"
        );
        assert_eq!(with_prefix(Some("app"), "/"), "/app/");
        assert_eq!(
            with_prefix(Some("mail/app"), "/settings/accounts"),
            "/mail/app/settings/accounts"
        );
    }
}
