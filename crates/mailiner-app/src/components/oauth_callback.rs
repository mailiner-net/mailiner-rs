//! `/oauth/callback` — receives the authorization code and returns it to the opener.

use dioxus::prelude::*;

#[cfg(target_arch = "wasm32")]
use crate::oauth::{
    OAUTH_MESSAGE_TYPE, OAUTH_PENDING_KEY, OAUTH_RESULT_KEY, OauthPending, OauthPostMessage,
    parse_callback_query,
};

/// Minimal page shown in the OAuth popup (or same-tab fallback).
#[component]
pub fn OauthCallbackPage() -> Element {
    let mut status = use_signal(|| "Finishing sign-in…".to_string());

    use_effect(move || {
        handle_callback(&mut status);
    });

    rsx! {
        main {
            class: "bootstrap-shell onboarding-shell",
            div {
                class: "bootstrap-card onboarding-card",
                h1 { class: "bootstrap-title", "OAuth sign-in" }
                p { class: "bootstrap-muted", "{status}" }
            }
        }
    }
}

/// In-app router path (`/`, `/settings/accounts`). Rejects protocol-relative
/// and absolute URLs so the same-tab return cannot leave this origin.
///
/// ASCII controls are rejected too. The URL parser removes tab and newlines
/// before resolving the reference, so `/\t/evil.example` would become
/// `//evil.example` and navigate off this origin.
fn is_app_route(path: &str) -> bool {
    if path.bytes().any(|b| b.is_ascii_control()) {
        return false;
    }
    path.starts_with('/') && !path.starts_with("//") && !path.contains('\\') && !path.contains(':')
}

fn handle_callback(status: &mut Signal<String>) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        status.set("OAuth sign-in is only available in the browser.".into());
    }
    #[cfg(target_arch = "wasm32")]
    {
        let Some(window) = web_sys::window() else {
            status.set("OAuth sign-in is only available in the browser.".into());
            return;
        };
        let search = window.location().search().unwrap_or_default();
        let result = parse_callback_query(&search);
        crate::oauth::session_set_json(OAUTH_RESULT_KEY, &result);

        let pending: Option<OauthPending> = crate::oauth::session_get_json(OAUTH_PENDING_KEY);
        if pending
            .as_ref()
            .is_some_and(|p| !p.state.is_empty() && p.state != result.state)
        {
            status.set("OAuth sign-in failed (state mismatch). You can close this window.".into());
            return;
        }

        let msg = OauthPostMessage {
            r#type: OAUTH_MESSAGE_TYPE.into(),
            state: result.state.clone(),
            code: result.code.clone(),
            error: result.error.clone(),
        };
        if let Ok(json) = serde_json::to_string(&msg)
            && let Ok(opener_val) = window.opener()
            && !opener_val.is_null()
        {
            use wasm_bindgen::JsCast;
            if let Ok(opener) = opener_val.dyn_into::<web_sys::Window>() {
                let origin = window.location().origin().unwrap_or_else(|_| "*".into());
                let _ = opener.post_message(&wasm_bindgen::JsValue::from_str(&json), &origin);
                let _ = window.close();
                status.set("You can close this window.".into());
                return;
            }
        }

        // Router path (`/settings`), not a browser path. `with_base` adds the
        // deploy prefix so a same-tab return from `/app/oauth/callback` does
        // not land on the host root.
        let return_path = pending
            .and_then(|p| p.return_path)
            .filter(|p| is_app_route(p))
            .unwrap_or_else(|| "/".into());
        let return_path = crate::base_path::with_base(&return_path);
        status.set("Returning to Mailiner…".into());
        let _ = window.location().assign(&return_path);
        let _ = result;
    }
}

#[cfg(test)]
mod tests {
    use super::is_app_route;

    #[test]
    fn return_path_stays_on_this_origin() {
        assert!(is_app_route("/"));
        assert!(is_app_route("/settings/accounts"));
        assert!(!is_app_route("//evil.example"));
        assert!(!is_app_route("///evil.example"));
        assert!(!is_app_route("/\t/evil.example"));
        assert!(!is_app_route("/\t//evil.example"));
        assert!(!is_app_route("/\n/evil.example"));
        assert!(!is_app_route("/\r/evil.example"));
        assert!(!is_app_route("https://evil.example/"));
        assert!(!is_app_route("/\\evil"));
    }
}
