//! Browser OAuth 2.0 authorization-code + PKCE (WASM `web_sys` only).
//!
//! Mailiner does not ship Google or Microsoft client IDs. The user pastes a
//! public client id; tokens stay in the account vault like IMAP passwords.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::account_config::{
    AccountConfig, AuthKind, Oauth2Provider, Oauth2Settings, Oauth2Tokens,
};

/// Refresh this many seconds before `expires_at`.
pub const REFRESH_SKEW_SECS: i64 = 60;

/// Same-origin path the provider must redirect to after consent.
pub const OAUTH_CALLBACK_PATH: &str = "/oauth/callback";

/// sessionStorage key for the in-flight PKCE request (no tokens).
pub const OAUTH_PENDING_KEY: &str = "mailiner.oauth.pending";

/// sessionStorage key for a same-tab callback result (auth code only).
pub const OAUTH_RESULT_KEY: &str = "mailiner.oauth.result";

/// `postMessage` type used by the popup callback page.
pub const OAUTH_MESSAGE_TYPE: &str = "mailiner-oauth";

const PKCE_VERIFIER_BYTES: usize = 32;
const STATE_BYTES: usize = 16;

/// User-safe OAuth errors. Display never includes tokens or codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OauthError {
    MissingClientId,
    MissingTokens,
    NeedsReauth,
    Provider(String),
    Network(String),
    BrowserOnly,
    Cancelled,
    StateMismatch,
}

impl OauthError {
    pub fn user_message(&self) -> &str {
        match self {
            Self::MissingClientId => {
                "OAuth client ID is required. Paste a public client ID from the provider console."
            }
            Self::MissingTokens => "Sign in with OAuth before testing or saving this account.",
            Self::NeedsReauth => "OAuth sign-in expired. Sign in again in account settings.",
            Self::Provider(msg) => msg.as_str(),
            Self::Network(msg) => msg.as_str(),
            Self::BrowserOnly => "OAuth sign-in is only available in the browser.",
            Self::Cancelled => "OAuth sign-in was cancelled.",
            Self::StateMismatch => "OAuth sign-in failed (state mismatch). Try again.",
        }
    }
}

impl fmt::Display for OauthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.user_message())
    }
}

impl std::error::Error for OauthError {}

/// PKCE pair (RFC 7636 S256).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

/// In-flight authorization request stored in sessionStorage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OauthPending {
    pub state: String,
    pub verifier: String,
    pub provider: Oauth2Provider,
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
    pub redirect_uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_path: Option<String>,
}

/// Auth-code result from `/oauth/callback` (never tokens).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OauthCallbackResult {
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Inputs for building the authorize URL / exchanging the code.
#[derive(Debug, Clone)]
pub struct OauthAuthorizeRequest {
    pub provider: Oauth2Provider,
    pub client_id: String,
    pub tenant: Option<String>,
    pub redirect_uri: String,
    pub return_path: Option<String>,
}

impl Oauth2Provider {
    pub fn authorize_endpoint(self, tenant: Option<&str>) -> String {
        match self {
            Self::Google => "https://accounts.google.com/o/oauth2/v2/auth".into(),
            Self::Microsoft => {
                let tenant = tenant
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .unwrap_or("common");
                format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize")
            }
        }
    }

    pub fn token_endpoint(self, tenant: Option<&str>) -> String {
        match self {
            Self::Google => "https://oauth2.googleapis.com/token".into(),
            Self::Microsoft => {
                let tenant = tenant
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .unwrap_or("common");
                format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token")
            }
        }
    }

    pub fn scopes(self) -> &'static str {
        match self {
            // Full IMAP/SMTP/POP access. `access_type=offline` is a query param.
            Self::Google => "https://mail.google.com/",
            Self::Microsoft => {
                "offline_access https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send"
            }
        }
    }
}

/// Redirect URI operators must register: `{origin}/oauth/callback`.
pub fn redirect_uri_from_origin(origin: &str) -> String {
    format!("{}{OAUTH_CALLBACK_PATH}", origin.trim_end_matches('/'))
}

/// RFC 7636 S256 challenge from a verifier.
pub fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

/// New PKCE verifier (32 random bytes → 43-char base64url) and challenge.
pub fn generate_pkce() -> Pkce {
    let verifier = URL_SAFE_NO_PAD.encode(random_bytes(PKCE_VERIFIER_BYTES));
    let challenge = pkce_challenge(&verifier);
    Pkce {
        verifier,
        challenge,
    }
}

pub fn generate_state() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes(STATE_BYTES))
}

/// True when the access token should be refreshed before AUTH.
pub fn token_needs_refresh(expires_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    match expires_at {
        None => false,
        Some(exp) => now + TimeDelta::seconds(REFRESH_SKEW_SECS) >= exp,
    }
}

pub fn build_authorize_url(pending: &OauthPending, challenge: &str) -> String {
    let endpoint = pending
        .provider
        .authorize_endpoint(pending.tenant.as_deref());
    let mut url = format!(
        "{endpoint}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        enc(&pending.client_id),
        enc(&pending.redirect_uri),
        enc(pending.provider.scopes()),
        enc(&pending.state),
        enc(challenge),
    );
    if pending.provider == Oauth2Provider::Google {
        url.push_str("&access_type=offline&prompt=consent");
    }
    url
}

pub fn token_request_body_authorization_code(
    client_id: &str,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> String {
    format!(
        "grant_type=authorization_code&client_id={}&code={}&redirect_uri={}&code_verifier={}",
        enc(client_id),
        enc(code),
        enc(redirect_uri),
        enc(verifier),
    )
}

pub fn token_request_body_refresh(client_id: &str, refresh_token: &str) -> String {
    format!(
        "grant_type=refresh_token&client_id={}&refresh_token={}",
        enc(client_id),
        enc(refresh_token),
    )
}

/// Parse a token-endpoint JSON body. Never logs the payload.
pub fn parse_token_response(
    body: &str,
    now: DateTime<Utc>,
    previous_refresh: Option<&str>,
) -> Result<Oauth2Tokens, OauthError> {
    let raw: TokenResponseJson = serde_json::from_str(body)
        .map_err(|_| OauthError::Provider("Invalid token response.".into()))?;
    if let Some(err) = raw.error.filter(|e| !e.is_empty()) {
        let _ = raw.error_description;
        return Err(OauthError::Provider(format!("OAuth token error: {err}")));
    }
    let access = raw.access_token.filter(|s| !s.is_empty()).ok_or_else(|| {
        OauthError::Provider("Token response did not include an access token.".into())
    })?;
    let refresh = raw
        .refresh_token
        .filter(|s| !s.is_empty())
        .or_else(|| previous_refresh.map(str::to_string));
    let expires_at = raw
        .expires_in
        .filter(|&s| s > 0)
        .map(|secs| now + TimeDelta::seconds(i64::from(secs).saturating_sub(0)));
    Ok(Oauth2Tokens {
        access_token: access,
        refresh_token: refresh,
        expires_at,
    })
}

#[derive(Deserialize)]
struct TokenResponseJson {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u32>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

/// Refresh when expired. Returns `true` if `config.oauth2.tokens` changed.
pub async fn ensure_fresh_oauth(config: &mut AccountConfig) -> Result<bool, OauthError> {
    if config.auth_kind != AuthKind::Oauth2 {
        return Ok(false);
    }
    let Some(oauth) = config.oauth2.as_ref() else {
        return Err(OauthError::MissingTokens);
    };
    if oauth.client_id.trim().is_empty() {
        return Err(OauthError::MissingClientId);
    }
    if oauth.tokens.access_token.is_empty() && oauth.tokens.refresh_token.is_none() {
        return Err(OauthError::MissingTokens);
    }
    if !token_needs_refresh(oauth.tokens.expires_at, Utc::now()) {
        if oauth.tokens.access_token.is_empty() {
            return Err(OauthError::NeedsReauth);
        }
        return Ok(false);
    }
    let Some(refresh) = oauth
        .tokens
        .refresh_token
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    else {
        return Err(OauthError::NeedsReauth);
    };
    let tokens = refresh_access_token(oauth, refresh).await?;
    if let Some(oauth) = config.oauth2.as_mut() {
        oauth.tokens = tokens;
    }
    Ok(true)
}

pub async fn refresh_access_token(
    oauth: &Oauth2Settings,
    refresh_token: &str,
) -> Result<Oauth2Tokens, OauthError> {
    let url = oauth
        .provider
        .token_endpoint(Some(oauth.tenant_or_common()));
    let body = token_request_body_refresh(oauth.client_id.trim(), refresh_token);
    let json = post_token_form(&url, &body).await?;
    parse_token_response(&json, Utc::now(), Some(refresh_token))
}

pub async fn exchange_authorization_code(
    pending: &OauthPending,
    code: &str,
) -> Result<Oauth2Tokens, OauthError> {
    let url = pending.provider.token_endpoint(pending.tenant.as_deref());
    let body = token_request_body_authorization_code(
        pending.client_id.trim(),
        code,
        &pending.redirect_uri,
        &pending.verifier,
    );
    let json = post_token_form(&url, &body).await?;
    parse_token_response(&json, Utc::now(), None)
}

async fn post_token_form(url: &str, body: &str) -> Result<String, OauthError> {
    if !url.starts_with("https://") {
        return Err(OauthError::Network("Token URL must be HTTPS.".into()));
    }
    #[cfg(target_arch = "wasm32")]
    {
        wasm_post_form(url, body).await
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (url, body);
        Err(OauthError::BrowserOnly)
    }
}

#[cfg(target_arch = "wasm32")]
async fn wasm_post_form(url: &str, body: &str) -> Result<String, OauthError> {
    use futures_util::future::{Either, select};
    use std::pin::pin;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestInit, RequestMode, Response};

    let window = web_sys::window().ok_or(OauthError::BrowserOnly)?;
    let opts = RequestInit::new();
    opts.set_method("POST");
    opts.set_mode(RequestMode::Cors);
    opts.set_body(&wasm_bindgen::JsValue::from_str(body));
    let headers = web_sys::Headers::new()
        .map_err(|_| OauthError::Network("Could not build the token request.".into()))?;
    headers
        .set("Content-Type", "application/x-www-form-urlencoded")
        .map_err(|_| OauthError::Network("Could not build the token request.".into()))?;
    opts.set_headers(&headers);
    let request = Request::new_with_str_and_init(url, &opts)
        .map_err(|_| OauthError::Network("Could not build the token request.".into()))?;
    let fetch = pin!(JsFuture::from(window.fetch_with_request(&request)));
    let timeout = pin!(gloo_timers::future::TimeoutFuture::new(20_000));
    let resp_val = match select(fetch, timeout).await {
        Either::Left((result, _)) => {
            result.map_err(|_| OauthError::Network("OAuth token request failed.".into()))?
        }
        Either::Right(_) => {
            return Err(OauthError::Network("OAuth token request timed out.".into()));
        }
    };
    let resp: Response = resp_val
        .dyn_into()
        .map_err(|_| OauthError::Network("Invalid token response.".into()))?;
    let text = resp
        .text()
        .map_err(|_| OauthError::Network("Could not read the token response.".into()))?;
    let text = JsFuture::from(text)
        .await
        .map_err(|_| OauthError::Network("Could not read the token response.".into()))?;
    let body = text
        .as_string()
        .ok_or_else(|| OauthError::Network("Token response was not text.".into()))?;
    if !resp.ok() {
        // Parse error JSON if possible; never include the raw body (may echo tokens).
        if let Ok(err) = parse_token_response(&body, Utc::now(), None) {
            let _ = err;
        }
        if let Ok(raw) = serde_json::from_str::<TokenResponseJson>(&body)
            && let Some(code) = raw.error.filter(|e| !e.is_empty())
        {
            return Err(OauthError::Provider(format!("OAuth token error: {code}")));
        }
        return Err(OauthError::Provider(format!(
            "OAuth token endpoint returned HTTP {}.",
            resp.status()
        )));
    }
    Ok(body)
}

fn enc(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn random_bytes(n: usize) -> Vec<u8> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rand::RngCore;
        let mut buf = vec![0u8; n];
        rand::rngs::OsRng.fill_bytes(&mut buf);
        buf
    }
    #[cfg(target_arch = "wasm32")]
    {
        let mut buf = vec![0u8; n];
        if let Some(window) = web_sys::window()
            && let Ok(crypto) = window.crypto()
        {
            let _ = crypto.get_random_values_with_u8_array(&mut buf);
        }
        buf
    }
}

/// Current page origin (`https://app.example`) or a placeholder off-WASM.
pub fn page_origin() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.location().origin().ok())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "https://<your-origin>".into())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        "https://<your-origin>".into()
    }
}

pub fn documented_redirect_uri() -> String {
    redirect_uri_from_origin(&page_origin())
}

/// Build a pending PKCE request (no network).
pub fn begin_authorize(req: &OauthAuthorizeRequest) -> Result<(OauthPending, String), OauthError> {
    let client_id = req.client_id.trim();
    if client_id.is_empty() {
        return Err(OauthError::MissingClientId);
    }
    let pkce = generate_pkce();
    let pending = OauthPending {
        state: generate_state(),
        verifier: pkce.verifier,
        provider: req.provider,
        client_id: client_id.to_string(),
        tenant: req
            .tenant
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string),
        redirect_uri: req.redirect_uri.clone(),
        return_path: req.return_path.clone(),
    };
    let url = build_authorize_url(&pending, &pkce.challenge);
    Ok((pending, url))
}

/// Open a popup (or same-tab fallback) and return tokens after the callback.
#[cfg(target_arch = "wasm32")]
pub async fn sign_in_with_popup(req: &OauthAuthorizeRequest) -> Result<Oauth2Tokens, OauthError> {
    let (pending, url) = begin_authorize(req)?;
    session_set_json(OAUTH_PENDING_KEY, &pending);

    let window = web_sys::window().ok_or(OauthError::BrowserOnly)?;
    let popup = window
        .open_with_url_and_target_and_features(
            &url,
            "mailiner-oauth",
            "popup=yes,width=520,height=720",
        )
        .ok()
        .flatten();

    if popup.is_none() {
        // Popup blocked: same-tab redirect. Caller resumes after callback.
        let _ = window.location().assign(&url);
        return Err(OauthError::Cancelled);
    }

    let result = wait_for_callback(&pending, popup.as_ref()).await;
    session_remove(OAUTH_PENDING_KEY);
    let result = result?;
    if let Some(err) = result.error.filter(|e| !e.is_empty()) {
        if err == "access_denied" {
            return Err(OauthError::Cancelled);
        }
        return Err(OauthError::Provider(format!("OAuth sign-in error: {err}")));
    }
    let code = result
        .code
        .filter(|c| !c.is_empty())
        .ok_or(OauthError::Cancelled)?;
    exchange_authorization_code(&pending, &code).await
}

#[cfg(target_arch = "wasm32")]
async fn wait_for_callback(
    pending: &OauthPending,
    popup: Option<&web_sys::Window>,
) -> Result<OauthCallbackResult, OauthError> {
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use web_sys::MessageEvent;

    let slot: Rc<RefCell<Option<OauthCallbackResult>>> = Rc::new(RefCell::new(None));
    let expected_state = pending.state.clone();
    let expected_origin = page_origin();
    let slot_cb = slot.clone();

    let closure = Closure::wrap(Box::new(move |event: MessageEvent| {
        if event.origin() != expected_origin {
            return;
        }
        let data = event.data();
        let raw = if let Some(s) = data.as_string() {
            s
        } else {
            match js_sys::JSON::stringify(&data) {
                Ok(js) => js.as_string().unwrap_or_default(),
                Err(_) => return,
            }
        };
        let Ok(msg) = serde_json::from_str::<OauthPostMessage>(&raw) else {
            return;
        };
        if msg.r#type != OAUTH_MESSAGE_TYPE || msg.state != expected_state {
            return;
        }
        *slot_cb.borrow_mut() = Some(OauthCallbackResult {
            state: msg.state,
            code: msg.code,
            error: msg.error,
        });
    }) as Box<dyn FnMut(MessageEvent)>);

    let window = web_sys::window().ok_or(OauthError::BrowserOnly)?;
    window
        .add_event_listener_with_callback("message", closure.as_ref().unchecked_ref())
        .map_err(|_| OauthError::Network("Could not listen for the OAuth callback.".into()))?;

    const TICK_MS: u32 = 200;
    const MAX_TICKS: u32 = 5 * 60 * 1000 / TICK_MS;
    let mut ticks = 0u32;
    let outcome = loop {
        if let Some(result) = slot.borrow_mut().take() {
            break Ok(result);
        }
        if let Some(result) = session_get_json::<OauthCallbackResult>(OAUTH_RESULT_KEY)
            && result.state == pending.state
        {
            session_remove(OAUTH_RESULT_KEY);
            break Ok(result);
        }
        if let Some(popup) = popup
            && popup.closed().unwrap_or(false)
        {
            gloo_timers::future::TimeoutFuture::new(250).await;
            if let Some(result) = slot.borrow_mut().take() {
                break Ok(result);
            }
            if let Some(result) = session_take_json::<OauthCallbackResult>(OAUTH_RESULT_KEY)
                && result.state == pending.state
            {
                break Ok(result);
            }
            break Err(OauthError::Cancelled);
        }
        ticks += 1;
        if ticks >= MAX_TICKS {
            break Err(OauthError::Network("OAuth sign-in timed out.".into()));
        }
        gloo_timers::future::TimeoutFuture::new(TICK_MS).await;
    };

    let _ = window.remove_event_listener_with_callback("message", closure.as_ref().unchecked_ref());
    drop(closure);
    outcome
}

/// Payload posted by [`crate`] callback page. Codes only, never tokens.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OauthPostMessage {
    pub r#type: String,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Complete a same-tab return: exchange the stored code if state matches.
pub async fn complete_same_tab_signin() -> Result<Option<Oauth2Tokens>, OauthError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Ok(None)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let pending: OauthPending = match session_get_json(OAUTH_PENDING_KEY) {
            Some(p) => p,
            None => return Ok(None),
        };
        let result: OauthCallbackResult = match session_take_json(OAUTH_RESULT_KEY) {
            Some(r) => r,
            None => return Ok(None),
        };
        session_remove(OAUTH_PENDING_KEY);
        if result.state != pending.state {
            return Err(OauthError::StateMismatch);
        }
        if let Some(err) = result.error.filter(|e| !e.is_empty()) {
            if err == "access_denied" {
                return Err(OauthError::Cancelled);
            }
            return Err(OauthError::Provider(format!("OAuth sign-in error: {err}")));
        }
        let Some(code) = result.code.filter(|c| !c.is_empty()) else {
            return Err(OauthError::Cancelled);
        };
        let tokens = exchange_authorization_code(&pending, &code).await?;
        Ok(Some(tokens))
    }
}

#[cfg(target_arch = "wasm32")]
pub fn session_set_json(key: &str, value: &impl Serialize) {
    if let Some(storage) = session_storage()
        && let Ok(json) = serde_json::to_string(value)
    {
        let _ = storage.set_item(key, &json);
    }
}

#[cfg(target_arch = "wasm32")]
pub fn session_take_json<T: for<'de> Deserialize<'de>>(key: &str) -> Option<T> {
    let storage = session_storage()?;
    let raw = storage.get_item(key).ok().flatten()?;
    let _ = storage.remove_item(key);
    serde_json::from_str(&raw).ok()
}

#[cfg(target_arch = "wasm32")]
pub fn session_get_json<T: for<'de> Deserialize<'de>>(key: &str) -> Option<T> {
    let storage = session_storage()?;
    let raw = storage.get_item(key).ok().flatten()?;
    serde_json::from_str(&raw).ok()
}

#[cfg(target_arch = "wasm32")]
pub fn session_remove(key: &str) {
    if let Some(storage) = session_storage() {
        let _ = storage.remove_item(key);
    }
}

#[cfg(target_arch = "wasm32")]
fn session_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.session_storage().ok().flatten()
}

/// Parse `?code=&state=&error=` from a callback URL query (`?…` or bare).
pub fn parse_callback_query(query: &str) -> OauthCallbackResult {
    let q = query.strip_prefix('?').unwrap_or(query);
    let mut state = String::new();
    let mut code = None;
    let mut error = None;
    for pair in q.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(k);
        let val = percent_decode(v);
        match key.as_str() {
            "state" => state = val,
            "code" => code = Some(val).filter(|s| !s.is_empty()),
            "error" => error = Some(val).filter(|s| !s.is_empty()),
            _ => {}
        }
    }
    OauthCallbackResult { state, code, error }
}

fn percent_decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s)
        .decode_utf8()
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| s.to_string())
        .replace('+', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn pkce_challenge_matches_rfc7636_appendix_b() {
        // RFC 7636 Appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            pkce_challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn generate_pkce_is_url_safe_and_round_trips() {
        let pkce = generate_pkce();
        assert!(pkce.verifier.len() >= 43);
        assert_eq!(pkce.challenge, pkce_challenge(&pkce.verifier));
        assert!(
            pkce.verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    #[test]
    fn token_expiry_uses_skew() {
        let now = Utc.with_ymd_and_hms(2026, 9, 4, 12, 0, 0).unwrap();
        assert!(!token_needs_refresh(None, now));
        assert!(token_needs_refresh(Some(now), now));
        assert!(token_needs_refresh(Some(now + TimeDelta::seconds(30)), now));
        assert!(!token_needs_refresh(
            Some(now + TimeDelta::seconds(120)),
            now
        ));
        assert!(token_needs_refresh(Some(now - TimeDelta::seconds(1)), now));
    }

    #[test]
    fn parse_token_response_keeps_previous_refresh() {
        let now = Utc.with_ymd_and_hms(2026, 9, 4, 12, 0, 0).unwrap();
        let tokens = parse_token_response(
            r#"{"access_token":"ya29.new","expires_in":3600}"#,
            now,
            Some("1//old-refresh"),
        )
        .unwrap();
        assert_eq!(tokens.access_token, "ya29.new");
        assert_eq!(tokens.refresh_token.as_deref(), Some("1//old-refresh"));
        assert_eq!(tokens.expires_at, Some(now + TimeDelta::seconds(3600)));
    }

    #[test]
    fn parse_token_response_rotates_refresh() {
        let now = Utc.with_ymd_and_hms(2026, 9, 4, 12, 0, 0).unwrap();
        let tokens = parse_token_response(
            r#"{"access_token":"a","refresh_token":"1//new","expires_in":60}"#,
            now,
            Some("1//old"),
        )
        .unwrap();
        assert_eq!(tokens.refresh_token.as_deref(), Some("1//new"));
    }

    #[test]
    fn parse_token_response_error_is_provider() {
        let now = Utc.with_ymd_and_hms(2026, 9, 4, 12, 0, 0).unwrap();
        let err = parse_token_response(
            r#"{"error":"invalid_grant","error_description":"expired"}"#,
            now,
            None,
        )
        .unwrap_err();
        assert!(matches!(err, OauthError::Provider(ref msg) if msg.contains("invalid_grant")));
        assert!(!format!("{err}").contains("expired"));
    }

    #[test]
    fn authorize_url_includes_pkce_and_google_offline() {
        let pending = OauthPending {
            state: "st".into(),
            verifier: "ver".into(),
            provider: Oauth2Provider::Google,
            client_id: "cid.apps.googleusercontent.com".into(),
            tenant: None,
            redirect_uri: "https://app.example/oauth/callback".into(),
            return_path: None,
        };
        let url = build_authorize_url(&pending, "chal");
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        assert!(url.contains("code_challenge=chal"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
        assert!(!url.contains("ver"), "verifier must not be in the URL");
    }

    #[test]
    fn microsoft_endpoints_use_tenant() {
        assert_eq!(
            Oauth2Provider::Microsoft.token_endpoint(Some("contoso")),
            "https://login.microsoftonline.com/contoso/oauth2/v2.0/token"
        );
        assert!(
            Oauth2Provider::Microsoft
                .authorize_endpoint(None)
                .contains("/common/")
        );
    }

    #[test]
    fn redirect_uri_from_origin_appends_callback() {
        assert_eq!(
            redirect_uri_from_origin("https://mail.example"),
            "https://mail.example/oauth/callback"
        );
        assert_eq!(
            redirect_uri_from_origin("http://localhost:8080/"),
            "http://localhost:8080/oauth/callback"
        );
    }

    #[test]
    fn parse_callback_query_reads_code_and_error() {
        let ok = parse_callback_query("?code=abc%2Fdef&state=xyz");
        assert_eq!(ok.code.as_deref(), Some("abc/def"));
        assert_eq!(ok.state, "xyz");
        let err = parse_callback_query("error=access_denied&state=xyz");
        assert_eq!(err.error.as_deref(), Some("access_denied"));
        assert!(err.code.is_none());
    }

    #[test]
    fn oauth_error_display_has_no_secrets() {
        let err = OauthError::Provider("OAuth token error: invalid_grant".into());
        let s = format!("{err}");
        assert!(!s.to_ascii_lowercase().contains("token="));
        assert!(!s.contains("ya29"));
    }
}
