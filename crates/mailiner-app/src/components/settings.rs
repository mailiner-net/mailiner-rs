//! General settings home: appearance, composer defaults, address book, privacy, shortcuts.

use dioxus::prelude::*;

use crate::Route;
use crate::account::{Account, AccountId};
use crate::address_book::{self, AddressBookError, Contact};
use crate::context::AppContext;
use crate::layout::reset_saved_layout;
use crate::shortcuts::{ShortcutGroup, shortcuts_in_group};
use crate::ui_prefs::{ComposeBodyMode, ComposePlacement, MessageListDensity};

fn account_from_label(account: &Account) -> String {
    if account.name.is_empty() {
        account.email.clone()
    } else {
        format!("{} <{}>", account.name, account.email)
    }
}

/// Full `/settings` page (accounts stay at `/settings/accounts`).
#[component]
pub fn SettingsPage() -> Element {
    let ctx = use_context::<AppContext>();
    let mut density = ctx.message_list_density;
    let current_density = *density.read();
    let mut body_mode = use_signal(crate::ui_prefs::load_compose_body_mode);
    let mut compose_placement = ctx.compose_placement;
    let mut default_from = use_signal(crate::ui_prefs::load_default_from_account);
    let mut allow_remote = use_signal(crate::ui_prefs::load_allow_remote_images);
    let mut layout_reset = use_signal(|| false);

    let mut accounts: Vec<_> = ctx.accounts.read().values().cloned().collect();
    accounts.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.id.as_str().cmp(b.id.as_str()))
    });
    let from_value = default_from
        .read()
        .as_ref()
        .filter(|id| accounts.iter().any(|a| &a.id == *id))
        .map(|id| id.as_str().to_string())
        .unwrap_or_default();

    rsx! {
        div {
            class: "bootstrap-shell onboarding-shell",
            div {
                class: "bootstrap-card onboarding-card settings-card",
                h1 { class: "bootstrap-title", "Settings" }
                p {
                    class: "bootstrap-muted",
                    "Appearance, composer, contacts, and privacy preferences are stored in this browser."
                }

                section {
                    class: "settings-section",
                    h2 { "Appearance" }
                    p {
                        class: "bootstrap-muted settings-hint",
                        "List density applies immediately. Pane sizes reset the next time you open mail."
                    }
                    div {
                        class: "onboarding-field",
                        label { r#for: "settings-density", "Message list density" }
                        select {
                            id: "settings-density",
                            value: "{current_density.as_key()}",
                            onchange: move |evt| {
                                if let Some(next) = MessageListDensity::from_key(&evt.value()) {
                                    crate::ui_prefs::save_message_list_density(next);
                                    density.set(next);
                                }
                            },
                            for option in MessageListDensity::ALL {
                                option {
                                    value: "{option.as_key()}",
                                    selected: option == current_density,
                                    "{option.label()}"
                                }
                            }
                        }
                    }
                    div {
                        class: "settings-actions",
                        button {
                            r#type: "button",
                            class: "onboarding-btn onboarding-btn-secondary",
                            onclick: move |_| {
                                reset_saved_layout();
                                layout_reset.set(true);
                            },
                            "Reset pane sizes"
                        }
                    }
                    if layout_reset() {
                        p {
                            class: "bootstrap-muted settings-reset-note",
                            "Folder width and message-list height will use the defaults when you return to mail."
                        }
                    }
                }

                section {
                    class: "settings-section",
                    h2 { "Composer" }
                    p {
                        class: "bootstrap-muted settings-hint",
                        "The composer is a plain-text editor. Rich sends an HTML alternative of the same text. Docked keeps the mailbox visible while you write."
                    }
                    div {
                        class: "onboarding-field",
                        label { r#for: "settings-compose-placement", "Compose window" }
                        select {
                            id: "settings-compose-placement",
                            value: "{compose_placement().as_key()}",
                            onchange: move |evt| {
                                if let Some(next) = ComposePlacement::from_key(&evt.value()) {
                                    crate::ui_prefs::save_compose_placement(next);
                                    compose_placement.set(next);
                                }
                            },
                            for option in ComposePlacement::ALL {
                                option {
                                    value: "{option.as_key()}",
                                    selected: option == compose_placement(),
                                    "{option.label()}"
                                }
                            }
                        }
                    }
                    div {
                        class: "onboarding-field",
                        label { r#for: "settings-compose-mode", "Default format" }
                        select {
                            id: "settings-compose-mode",
                            value: "{body_mode().as_key()}",
                            onchange: move |evt| {
                                if let Some(next) = ComposeBodyMode::from_key(&evt.value()) {
                                    crate::ui_prefs::save_compose_body_mode(next);
                                    body_mode.set(next);
                                }
                            },
                            for option in ComposeBodyMode::ALL {
                                option {
                                    value: "{option.as_key()}",
                                    selected: option == body_mode(),
                                    "{option.label()}"
                                }
                            }
                        }
                    }
                    div {
                        class: "onboarding-field",
                        label { r#for: "settings-default-from", "Default From" }
                        select {
                            id: "settings-default-from",
                            value: "{from_value}",
                            onchange: move |evt| {
                                let value = evt.value();
                                if value.is_empty() {
                                    crate::ui_prefs::save_default_from_account(None);
                                    default_from.set(None);
                                } else {
                                    let id = AccountId::new(value);
                                    crate::ui_prefs::save_default_from_account(Some(&id));
                                    default_from.set(Some(id));
                                }
                            },
                            option {
                                value: "",
                                selected: from_value.is_empty(),
                                "Active account"
                            }
                            for account in accounts.iter() {
                                option {
                                    value: "{account.id.as_str()}",
                                    selected: from_value == account.id.as_str(),
                                    "{account_from_label(account)}"
                                }
                            }
                        }
                    }
                }

                AddressBookSection {}

                section {
                    class: "settings-section",
                    h2 { "Privacy" }
                    p {
                        class: "bootstrap-muted settings-hint",
                        "Remote images can tell the sender that you opened a message. Blocked by default; you can still allow them on a single message."
                    }
                    div {
                        class: "onboarding-field",
                        label { r#for: "settings-remote-images", "Remote images" }
                        select {
                            id: "settings-remote-images",
                            value: if allow_remote() { "allow" } else { "block" },
                            onchange: move |evt| {
                                let next = evt.value() == "allow";
                                crate::ui_prefs::save_allow_remote_images(next);
                                allow_remote.set(next);
                            },
                            option {
                                value: "block",
                                selected: !allow_remote(),
                                "Block by default"
                            }
                            option {
                                value: "allow",
                                selected: allow_remote(),
                                "Allow by default"
                            }
                        }
                    }
                }

                section {
                    class: "settings-section",
                    h2 { "Keyboard shortcuts" }
                    p {
                        class: "bootstrap-muted settings-hint",
                        "Press ? from the mail view to open this list."
                    }
                    for group in ShortcutGroup::ALL {
                        section {
                            class: "shortcut-group settings-shortcut-group",
                            h3 { class: "shortcut-group-title", "{group.title()}" }
                            ul {
                                class: "shortcut-list",
                                for shortcut in shortcuts_in_group(*group) {
                                    li {
                                        class: "shortcut-row",
                                        span { class: "shortcut-desc", "{shortcut.description}" }
                                        kbd { class: "shortcut-key", "{shortcut.label}" }
                                    }
                                }
                            }
                        }
                    }
                }

                nav {
                    class: "bootstrap-nav accounts-nav",
                    Link {
                        to: Route::AccountsSettingsView {},
                        class: "onboarding-btn onboarding-btn-primary accounts-link-btn",
                        "Accounts"
                    }
                    Link {
                        to: Route::MainView {},
                        class: "onboarding-btn onboarding-btn-secondary accounts-link-btn",
                        "Back to mail"
                    }
                }
            }
        }
    }
}

/// Add/remove name+email contacts stored in origin `localStorage`.
#[component]
fn AddressBookSection() -> Element {
    let initial = use_hook(address_book::try_load_contacts);
    let storage_error = initial.as_ref().err().map(AddressBookError::to_string);
    let mut contacts = use_signal(|| initial.clone().unwrap_or_default());
    let mut name = use_signal(String::new);
    let mut email = use_signal(String::new);
    let mut action_error = use_signal(|| None::<String>);
    let blocked = storage_error.is_some();
    let listed = contacts();

    rsx! {
        section {
            class: "settings-section",
            h2 { "Address book" }
            p {
                class: "bootstrap-muted settings-hint",
                "Contacts stay in this browser."
            }
            if let Some(err) = storage_error {
                p {
                    class: "onboarding-status onboarding-status-error",
                    role: "alert",
                    "{err}"
                }
            }
            form {
                class: "settings-contact-form",
                onsubmit: move |evt| {
                    evt.prevent_default();
                    if blocked {
                        return;
                    }
                    action_error.set(None);
                    match address_book::add_contact(&name(), &email()) {
                        Ok(_) => {
                            contacts.set(address_book::load_contacts());
                            name.set(String::new());
                            email.set(String::new());
                        }
                        Err(e) => action_error.set(Some(e.to_string())),
                    }
                },
                div {
                    class: "onboarding-field",
                    label { r#for: "settings-contact-name", "Name" }
                    input {
                        id: "settings-contact-name",
                        r#type: "text",
                        autocomplete: "name",
                        value: "{name}",
                        disabled: blocked,
                        oninput: move |evt| name.set(evt.value()),
                    }
                }
                div {
                    class: "onboarding-field",
                    label { r#for: "settings-contact-email", "Email" }
                    input {
                        id: "settings-contact-email",
                        r#type: "email",
                        inputmode: "email",
                        autocomplete: "email",
                        spellcheck: "false",
                        autocapitalize: "off",
                        value: "{email}",
                        disabled: blocked,
                        required: true,
                        oninput: move |evt| email.set(evt.value()),
                    }
                }
                button {
                    r#type: "submit",
                    class: "onboarding-btn onboarding-btn-primary accounts-btn-sm settings-contact-add",
                    disabled: blocked,
                    "Add"
                }
            }
            if let Some(err) = action_error() {
                p {
                    class: "onboarding-status onboarding-status-error",
                    role: "alert",
                    "{err}"
                }
            }
            if listed.is_empty() {
                p {
                    class: "bootstrap-muted settings-contact-empty",
                    "No contacts yet."
                }
            } else {
                ul {
                    class: "settings-contact-list",
                    for contact in listed.iter() {
                        ContactRow {
                            key: "{contact.email}",
                            contact: contact.clone(),
                            contacts,
                            action_error,
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ContactRow(
    contact: Contact,
    mut contacts: Signal<Vec<Contact>>,
    mut action_error: Signal<Option<String>>,
) -> Element {
    let email = contact.email.clone();
    let name_label = contact.display_label().to_string();
    let show_email = !contact.name.trim().is_empty();

    rsx! {
        li {
            class: "settings-contact-row",
            div {
                class: "settings-contact-main",
                span { class: "settings-contact-name", "{name_label}" }
                if show_email {
                    span { class: "settings-contact-email", "{email}" }
                }
            }
            button {
                r#type: "button",
                class: "onboarding-btn onboarding-btn-secondary accounts-btn-sm",
                title: "Remove {email}",
                aria_label: "Remove {email}",
                onclick: move |_| {
                    action_error.set(None);
                    match address_book::remove_contact(&email) {
                        Ok(_) => contacts.set(address_book::load_contacts()),
                        Err(e) => action_error.set(Some(e.to_string())),
                    }
                },
                "Remove"
            }
        }
    }
}
