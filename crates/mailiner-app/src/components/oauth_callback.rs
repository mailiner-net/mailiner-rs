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

        let return_path = pending
            .and_then(|p| p.return_path)
            .filter(|p| p.starts_with('/'))
            .unwrap_or_else(|| "/".into());
        status.set("Returning to Mailiner…".into());
        let _ = window.location().assign(&return_path);
        let _ = result;
    }
}
