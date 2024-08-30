use dioxus::prelude::*;
use dioxus_daisyui::prelude::*;
use uuid::Uuid;

use mailiner_core::settings::{
    use_accounts, AuthMethod, ImapConfiguration, MailAccount, Security, SmtpConfiguration,
};

fn text_input(
    label_var: &str,
    name: &str,
    placeholder_var: &str,
    signal: Signal<String>,
) -> Element {
    let mut signal = signal.clone();
    rsx! {
        label {
            class: class!(input input_bordered flex justify_items_center gap_2),
            { label_var },
            input {
                class: class!(grow),
                r#type: "text",
                name: name,
                placeholder: placeholder_var,
                value: signal,
                oninput: move |event| {
                    signal.set(event.value())
                }
            }
        }
    }
}

fn port_input(label_var: &str, name: &str, signal: Signal<u16>) -> Element {
    let mut signal = signal.clone();
    rsx! {
        label {
            class: class!(input input_bordered flex justify_items_center gap_2),
            { label_var }
            input {
                class: class!(grow),
                r#type: "number",
                min: 0,
                max: u16::MAX as i64,
                name: name,
                placeholder: "Port",
                value: signal.read().clone().to_string(),
                oninput: move |event| {
                    signal.set(event.value().parse().unwrap_or(0))
                }
            }
        }
    }
}

fn security_input(label_var: &str, name: &str, signal: Signal<Security>) -> Element {
    let mut signal = signal.clone();
    let value: String = match signal.read().clone() {
        Security::None => "None".to_owned(),
        Security::Tls => "Tls".to_owned(),
        Security::StartTls => "StartTls".to_owned(),
    };

    rsx! {
        label {
            class: class!(input input_bordered flex justify_items_center gap_2),
            { label_var },
            select {
                class: class!(grow),
                name: name,
                value: value,
                onchange: move |event| {
                    signal.set(match event.value().as_str() {
                        "None" => Security::None,
                        "Tls" => Security::Tls,
                        "StartTls" => Security::StartTls,
                        _ => Security::None,
                    })
                },
                option {
                    value: "None",
                    { "None" }
                },
                option {
                    value: "Tls",
                    { "TLS" }
                },
                option {
                    value: "StartTls",
                    { "STARTTLS" }
                },
            }
        }
    }
}

fn get_username(auth: &AuthMethod) -> String {
    match &auth {
        AuthMethod::Plain { username, .. } => username.clone(),
        AuthMethod::Login { username, .. } => username.clone(),
        _ => "".to_string(),
    }
}

fn get_password(auth: &AuthMethod) -> String {
    match &auth {
        AuthMethod::Plain { password, .. } => password.clone(),
        AuthMethod::Login { password, .. } => password.clone(),
        _ => "".to_string(),
    }
}

fn account_wizard(account_id: Option<String>) -> Element {
    let accounts = use_accounts();
    let account = account_id.and_then(|id| {
        accounts
            .read()
            .iter()
            .find(|(uuid, account)| uuid.to_string() == id)
            .map(|(_, account)| account)
            .cloned()
    });

    let email = use_signal(|| {
        account
            .as_ref()
            .map(|a| a.email.clone())
            .unwrap_or_default()
    });
    let name = use_signal(|| account.as_ref().map(|a| a.name.clone()).unwrap_or_default());
    let imap_server = use_signal(|| {
        account
            .as_ref()
            .map(|a| a.imap.server.clone())
            .unwrap_or_default()
    });
    let imap_port = use_signal(|| {
        account
            .as_ref()
            .map(|a| a.imap.port.clone())
            .unwrap_or_default()
    });
    let imap_username = use_signal(|| {
        account
            .as_ref()
            .map(|a| get_username(&a.imap.auth))
            .unwrap_or_default()
    });
    let imap_password = use_signal(|| {
        account
            .as_ref()
            .map(|a| get_password(&a.imap.auth))
            .unwrap_or_default()
    });
    let imap_security = use_signal(|| {
        account
            .as_ref()
            .map(|a| a.imap.security.clone())
            .unwrap_or(Security::None)
    });
    let smtp_server = use_signal(|| {
        account
            .as_ref()
            .map(|a| a.smtp.server.clone())
            .unwrap_or_default()
    });
    let smtp_port = use_signal(|| {
        account
            .as_ref()
            .map(|a| a.smtp.port.clone())
            .unwrap_or_default()
    });
    let smtp_username = use_signal(|| {
        account
            .as_ref()
            .map(|a| get_username(&a.smtp.auth))
            .unwrap_or_default()
    });
    let smtp_password = use_signal(|| {
        account
            .as_ref()
            .map(|a| get_password(&a.smtp.auth))
            .unwrap_or_default()
    });
    let smtp_security = use_signal(|| {
        account
            .as_ref()
            .map(|a| a.smtp.security.clone())
            .unwrap_or(Security::None)
    });

    rsx! {
        { text_input("Email Address", "email", "Email Address", email) }
        { text_input("Name", "name", "Name", name) }
        { text_input("IMAP Server", "imap-server", "Server", imap_server) }
        { port_input("IMAP Port", "imap-port", imap_port) }
        { text_input("IMAP Username", "imap-username", "Username", imap_username) }
        { text_input("IMAP Password", "imap-password", "Password", imap_password) }
        { security_input("IMAP Security", "imap-security", imap_security) }
        { text_input("SMTP Server", "smtp-server", "Server", smtp_server) }
        { port_input("SMTP Port", "smtp-port", smtp_port) }
        { text_input("SMTP Username", "smtp-username", "Username", smtp_username) }
        { text_input("SMTP Password", "smtp-password", "Password", smtp_password) }
        { security_input("SMTP Security", "smtp-security", smtp_security) }

        button {
            class: class!(btn btn_primary),
            value: "Save",
            onclick: move |_| {
                let new_account = MailAccount {
                    id: Uuid::new_v4().to_string(),
                    email: email.read().to_owned(),
                    name: name.read().to_owned(),
                    imap: ImapConfiguration {
                        server: imap_server.read().to_owned(),
                        port: imap_port.read().to_owned(),
                        auth: AuthMethod::Plain {
                            username: imap_username.read().to_owned(),
                            password: imap_password.read().to_owned(),
                        },
                        security: imap_security.read().to_owned(),
                    },
                    smtp: SmtpConfiguration {
                        server: smtp_server.read().to_owned(),
                        port: smtp_port.read().to_owned(),
                        auth: AuthMethod::Plain {
                            username: smtp_username.read().to_owned(),
                            password: smtp_password.read().to_owned(),
                        },
                        security: smtp_security.read().to_owned(),
                    },
                };

                /*
                let mut mut_settings = settings.get();
                if let Some(account) = &account {
                    for acc in mut_settings.accounts.iter_mut() {
                        if acc.id == account.id {
                            *acc = new_account;
                            break;
                        }
                    }
                } else {
                    mut_settings.accounts.push(new_account);
                }
                settings.set(mut_settings);
                */
            },

            { "Save Account" }
        }

    }
}

#[component]
pub fn NewAccount() -> Element {
    account_wizard(None)
}

#[component]
pub fn EditAccount(account_id: String) -> Element {
    account_wizard(Some(account_id))
}
