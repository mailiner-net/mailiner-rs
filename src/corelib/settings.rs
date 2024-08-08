use std::collections::HashMap;

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::hooks::UsePersistent;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "auth_type")]
pub enum AuthMethod {
    Plain { username: String, password: String },
    Login { username: String, password: String },
    XOAuth2 { username: String, token: String },
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub enum Security {
    None,
    Tls,
    StartTls,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ImapConfiguration {
    pub server: String,
    pub port: u16,
    pub auth: AuthMethod,
    pub security: Security,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SmtpConfiguration {
    pub server: String,
    pub port: u16,
    pub auth: AuthMethod,
    pub security: Security,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct MailAccount {
    pub id: String,
    pub name: String,
    pub email: String,

    pub imap: ImapConfiguration,
    pub smtp: SmtpConfiguration,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
pub struct AppSettings {
    pub accounts: Vec<MailAccount>,
}

/// A hook that returns global AppSettings loaded from browser's local storage
/// When saving the settings, it will also save it back to local storage.
pub fn use_settings() -> UsePersistent<AppSettings> {
    use_context()
}

/// Returns a map of all mail accounts
pub fn use_accounts() -> Signal<HashMap<Uuid, MailAccount>> {
    use_context()
}

#[cfg(test)]
mod testing {
    use super::*;

    #[test]
    fn test_deserialize_settings() {
        let json = r#"
            {
                "accounts": [
                    {
                        "id": "00000000-0000-0000-0000-000000000000",
                        "name": "Test Account",
                        "email": "test@example.com",
                        "imap": {
                            "server": "imap.example.com",
                            "port": 993,
                            "auth": {
                                "auth_type": "Plain",
                                "username": "test@example.com",
                                "password": "password"
                            },
                            "security": "Tls"
                        },
                        "smtp": {
                            "server": "smtp.example.com",
                            "port": 587,
                            "auth": {
                                "auth_type": "Login",
                                "username": "test@example.com",
                                "password": "password"
                            },
                            "security": "StartTls"
                        }
                    }
                ]
            }"#;

        let settings: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.accounts.len(), 1);
        let accounts = settings.accounts;
        let &account = accounts.get(0).as_ref().unwrap();
        assert_eq!(account.id, "00000000-0000-0000-0000-000000000000");
        assert_eq!(account.name, "Test Account".to_owned());
        assert_eq!(account.email, "test@example.com".to_owned());
        assert_eq!(account.imap.server, "imap.example.com");
        assert_eq!(account.imap.port, 993);
        assert_eq!(
            account.imap.auth,
            AuthMethod::Plain {
                username: "test@example.com".to_owned(),
                password: "password".to_owned(),
            }
        );
        assert_eq!(account.imap.security, Security::Tls);
        assert_eq!(account.smtp.server, "smtp.example.com");
        assert_eq!(account.smtp.port, 587);
        assert_eq!(
            account.smtp.auth,
            AuthMethod::Login {
                username: "test@example.com".to_owned(),
                password: "password".to_owned(),
            }
        );
        assert_eq!(account.smtp.security, Security::StartTls);
    }
}
