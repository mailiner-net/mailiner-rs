use dioxus::prelude::*;
use dioxus_daisyui::prelude::*;
use uuid::Uuid;

use mailiner_core::imap_account_manager::{use_imap_account_manager, ImapAccount};
use mailiner_core::security::{Authentication, Security};

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
        Security::Ssl => "Ssl".to_owned(),
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
                        "Ssl" => Security::Ssl,
                        "StartTls" => Security::StartTls,
                        _ => Security::None,
                    })
                },
                option {
                    value: "None",
                    { "None" }
                },
                option {
                    value: "Ssl",
                    { "SSL/TLS (recommended)" }
                },
                option {
                    value: "StartTls",
                    { "STARTTLS" }
                },
            }
        }
    }
}

fn get_username(auth: &Authentication) -> String {
    match &auth {
        Authentication::Plain { username, .. } => username.clone(),
        Authentication::Login { username, .. } => username.clone(),
        _ => "".to_string(),
    }
}

fn get_password(auth: &Authentication) -> String {
    match &auth {
        Authentication::Plain { password, .. } => password.clone(),
        Authentication::Login { password, .. } => password.clone(),
        _ => "".to_string(),
    }
}

fn account_wizard(account_id: Option<String>) -> Element {
    let mut account_manager = use_imap_account_manager();
    let account = account_id
        .map(|id| {
            account_manager
                .read()
                .get_account(Uuid::parse_str(&id).unwrap())
        })
        .flatten();

    let name = use_signal(|| account.map(|a| a.read().name.clone()).unwrap_or_default());
    let imap_server = use_signal(|| {
        account
            .map(|a| a.read().hostname.clone())
            .unwrap_or_default()
    });
    let imap_port = use_signal(|| account.map(|a| a.read().port).unwrap_or_default());
    let imap_username = use_signal(|| {
        account
            .map(|a| get_username(&a.read().authentication))
            .unwrap_or_default()
    });
    let imap_password = use_signal(|| {
        account
            .map(|a| get_password(&a.read().authentication))
            .unwrap_or_default()
    });
    let imap_security = use_signal(|| account.map(|a| a.read().security).unwrap_or(Security::None));

    rsx! {
        { text_input("Name", "name", "Name", name) }
        { text_input("IMAP Server", "imap-server", "Server", imap_server) }
        { port_input("IMAP Port", "imap-port", imap_port) }
        { text_input("IMAP Username", "imap-username", "Username", imap_username) }
        { text_input("IMAP Password", "imap-password", "Password", imap_password) }
        { security_input("IMAP Security", "imap-security", imap_security) }
        /*
        { text_input("SMTP Server", "smtp-server", "Server", smtp_server) }
        { port_input("SMTP Port", "smtp-port", smtp_port) }
        { text_input("SMTP Username", "smtp-username", "Username", smtp_username) }
        { text_input("SMTP Password", "smtp-password", "Password", smtp_password) }
        { security_input("SMTP Security", "smtp-security", smtp_security) }
         */

        button {
            class: class!(btn btn_primary),
            value: "Save",
            onclick: move |_| {
                if let Some(mut account) = &account {
                    let mut mut_account = account.write();
                    mut_account.name = name.read().to_owned();
                    mut_account.hostname = imap_server.read().to_owned();
                    mut_account.port = imap_port.read().to_owned();
                    mut_account.authentication = Authentication::Plain {
                        username: imap_username.read().to_owned(),
                        password: imap_password.read().to_owned(),
                    };
                    mut_account.security = imap_security.read().to_owned();
                    account_manager.write().save();
                } else {
                    let new_account = ImapAccount {
                        id: Uuid::new_v4(),
                        name: name.read().to_owned(),
                        hostname: imap_server.read().to_owned(),
                        port: imap_port.read().to_owned(),
                        authentication: Authentication::Plain {
                            username: imap_username.read().to_owned(),
                            password: imap_password.read().to_owned(),
                        },
                        security: imap_security.read().to_owned(),
                    };
                    account_manager.write().add_account(new_account);
                }
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
