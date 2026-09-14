//! Step list for the reusable account-setup wizard.
//!
//! UI lives in `components/account_setup.rs`. This module is the pure
//! planning function so unit tests do not need Dioxus.

use crate::account::AccountId;
use crate::account_config::AccountConfig;

/// Where the setup wizard is hosted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupMode {
    /// Empty store, `/onboarding`. Includes Welcome and optional unlock.
    FirstRun,
    /// `/settings/accounts/new`. Skips Welcome and unlock.
    AddAccount,
}

/// One screen in the account-setup wizard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupStep {
    Welcome,
    Email,
    SignIn,
    Servers,
    Proxy,
    Unlock,
    Review,
}

/// Whether the Connection (proxy) step belongs in the list.
///
/// Hidden when a proxy URL is already known (debug prefill or last-used
/// copy) unless the user asked to change it.
pub fn include_proxy_step(proxy_url: &str, force: bool) -> bool {
    force || proxy_url.trim().is_empty()
}

/// Ordered steps for `mode`.
pub fn setup_steps(mode: SetupMode, include_proxy: bool) -> Vec<SetupStep> {
    let mut steps = Vec::with_capacity(7);
    if mode == SetupMode::FirstRun {
        steps.push(SetupStep::Welcome);
    }
    steps.push(SetupStep::Email);
    steps.push(SetupStep::SignIn);
    steps.push(SetupStep::Servers);
    if include_proxy {
        steps.push(SetupStep::Proxy);
    }
    if mode == SetupMode::FirstRun {
        steps.push(SetupStep::Unlock);
    }
    steps.push(SetupStep::Review);
    steps
}

/// Account whose proxy should be copied onto a new account.
///
/// Prefers `preferred` (the active account) when it has a proxy URL; otherwise
/// the most recently updated account that has one.
pub fn last_used_proxy_source<'a>(
    accounts: &'a [AccountConfig],
    preferred: Option<&AccountId>,
) -> Option<&'a AccountConfig> {
    if let Some(id) = preferred
        && let Some(found) = accounts.iter().find(|c| &c.id == id)
        && !found.proxy.base_url.trim().is_empty()
    {
        return Some(found);
    }
    accounts
        .iter()
        .filter(|c| !c.proxy.base_url.trim().is_empty())
        .max_by_key(|c| c.updated_at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_empty_proxy_includes_connection_step() {
        let steps = setup_steps(SetupMode::FirstRun, include_proxy_step("", false));
        assert_eq!(
            steps,
            vec![
                SetupStep::Welcome,
                SetupStep::Email,
                SetupStep::SignIn,
                SetupStep::Servers,
                SetupStep::Proxy,
                SetupStep::Unlock,
                SetupStep::Review,
            ]
        );
    }

    #[test]
    fn first_run_prefilled_proxy_skips_connection_step() {
        let steps = setup_steps(
            SetupMode::FirstRun,
            include_proxy_step("ws://localhost:9400/proxy", false),
        );
        assert_eq!(
            steps,
            vec![
                SetupStep::Welcome,
                SetupStep::Email,
                SetupStep::SignIn,
                SetupStep::Servers,
                SetupStep::Unlock,
                SetupStep::Review,
            ]
        );
    }

    #[test]
    fn add_account_with_last_used_proxy_skips_connection_step() {
        let steps = setup_steps(
            SetupMode::AddAccount,
            include_proxy_step("wss://proxy.example/proxy", false),
        );
        assert_eq!(
            steps,
            vec![
                SetupStep::Email,
                SetupStep::SignIn,
                SetupStep::Servers,
                SetupStep::Review,
            ]
        );
    }

    #[test]
    fn add_account_without_proxy_includes_connection_step() {
        let steps = setup_steps(SetupMode::AddAccount, include_proxy_step("  ", false));
        assert_eq!(
            steps,
            vec![
                SetupStep::Email,
                SetupStep::SignIn,
                SetupStep::Servers,
                SetupStep::Proxy,
                SetupStep::Review,
            ]
        );
    }

    #[test]
    fn force_proxy_keeps_connection_step_when_url_is_set() {
        assert!(include_proxy_step("ws://localhost:9400/proxy", true));
        let steps = setup_steps(SetupMode::AddAccount, true);
        assert!(steps.contains(&SetupStep::Proxy));
    }

    fn cfg(id: &str, url: &str, updated_hour: u32) -> AccountConfig {
        use crate::account_config::{DEFAULT_IMAP_PORT, ImapSettings, ImapTlsMode, ProxySettings};
        use chrono::TimeZone;
        let ts = chrono::Utc
            .with_ymd_and_hms(2024, 6, 15, updated_hour, 0, 0)
            .unwrap();
        AccountConfig {
            id: AccountId::new(id),
            display_name: id.into(),
            email: format!("{id}@example.com"),
            identities: Vec::new(),
            signature: None,
            auth_kind: crate::account_config::AuthKind::Password,
            oauth2: None,
            imap: ImapSettings::new(
                "imap.example.com".into(),
                DEFAULT_IMAP_PORT,
                format!("{id}@example.com"),
                "secret".into(),
                ImapTlsMode::Implicit,
            ),
            smtp: None,
            proxy: ProxySettings {
                base_url: url.into(),
                token: "tok".into(),
                remote_host: None,
                remote_port: None,
            },
            extra_ca_pems: Vec::new(),
            smime_identities: Vec::new(),
            created_at: ts,
            updated_at: ts,
        }
    }

    #[test]
    fn last_used_proxy_prefers_active_account() {
        let older = cfg("a", "wss://old.example/proxy", 10);
        let active = cfg("b", "wss://active.example/proxy", 8);
        let list = vec![older, active];
        let id = AccountId::new("b");
        let src = last_used_proxy_source(&list, Some(&id)).unwrap();
        assert_eq!(src.proxy.base_url, "wss://active.example/proxy");
    }

    #[test]
    fn last_used_proxy_falls_back_to_most_recent() {
        let older = cfg("a", "wss://old.example/proxy", 10);
        let newer = cfg("b", "wss://new.example/proxy", 12);
        let list = vec![older, newer];
        let src = last_used_proxy_source(&list, None).unwrap();
        assert_eq!(src.proxy.base_url, "wss://new.example/proxy");
    }

    #[test]
    fn last_used_proxy_skips_empty_urls() {
        let empty = cfg("a", "", 14);
        let with_url = cfg("b", "wss://ok.example/proxy", 10);
        let list = vec![empty, with_url];
        let src = last_used_proxy_source(&list, Some(&AccountId::new("a"))).unwrap();
        assert_eq!(src.proxy.base_url, "wss://ok.example/proxy");
    }
}
