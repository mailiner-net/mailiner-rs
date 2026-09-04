//! General settings home: appearance, composer, filters, vacation, address book, privacy, shortcuts.

use dioxus::prelude::*;

use crate::Route;
use crate::account::{Account, AccountId};
use crate::address_book::{self, AddressBookError, Contact};
use crate::context::AppContext;
use crate::layout::reset_saved_layout;
use crate::mail_rules::{self, MailRule};
use crate::mailbox::{MailboxId, flatten_mailboxes, mailbox_is_action_target};
use crate::shortcuts::{
    ShortcutGroup, ShortcutId, effective_shortcuts_in, remap_shortcut, reset_all_shortcuts,
    reset_shortcut,
};
use crate::ui_prefs::{
    ComposeBodyMode, ComposePlacement, MailLayout, MessageListDensity, MessageListView,
    ShortcutMapBlob,
};
use crate::vacation::{self, VacationSettings};
use mailiner_core::ImapKeyword;

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
    let mut list_view = ctx.message_list_view;
    let current_view = *list_view.read();
    let mut expanded_conversations = ctx.expanded_conversations;
    let mut mail_layout = ctx.mail_layout;
    let current_layout = *mail_layout.read();
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
        main {
            class: "bootstrap-shell onboarding-shell",
            div {
                class: "bootstrap-card onboarding-card settings-card",
                h1 { class: "bootstrap-title", "Settings" }
                p {
                    class: "bootstrap-muted",
                    "Appearance, composer, filters, vacation, contacts, privacy, and shortcut preferences are stored in this browser."
                }

                section {
                    class: "settings-section",
                    h2 { "Appearance" }
                    p {
                        class: "bootstrap-muted settings-hint",
                        "List density and mail layout apply immediately on desktop. Phone and tablet widths show one pane at a time. Pane sizes reset the next time you open mail."
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
                        class: "onboarding-field",
                        label { r#for: "settings-list-view", "Message list grouping" }
                        select {
                            id: "settings-list-view",
                            value: "{current_view.as_key()}",
                            onchange: move |evt| {
                                if let Some(next) = MessageListView::from_key(&evt.value()) {
                                    crate::ui_prefs::save_message_list_view(next);
                                    list_view.set(next);
                                    if next == MessageListView::Flat {
                                        expanded_conversations.write().clear();
                                    }
                                }
                            },
                            for option in MessageListView::ALL {
                                option {
                                    value: "{option.as_key()}",
                                    selected: option == current_view,
                                    "{option.label()}"
                                }
                            }
                        }
                    }
                    div {
                        class: "onboarding-field",
                        label { r#for: "settings-mail-layout", "Mail layout" }
                        select {
                            id: "settings-mail-layout",
                            value: "{current_layout.as_key()}",
                            onchange: move |evt| {
                                if let Some(next) = MailLayout::from_key(&evt.value()) {
                                    crate::ui_prefs::save_mail_layout(next);
                                    mail_layout.set(next);
                                }
                            },
                            for option in MailLayout::ALL {
                                option {
                                    value: "{option.as_key()}",
                                    selected: option == current_layout,
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
                            "Folder width and message-list size will use the defaults when you return to mail."
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

                FiltersSection {}

                VacationSection {}

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

                ShortcutSettingsSection {}

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

/// Remap a subset of [`crate::shortcuts::GLOBAL_SHORTCUTS`]; the rest stay fixed.
#[component]
fn ShortcutSettingsSection() -> Element {
    let mut blob = use_signal(crate::ui_prefs::load_shortcut_map);
    let mut capturing = use_signal(|| None::<ShortcutId>);
    let mut action_error = use_signal(|| None::<String>);
    let listed = effective_shortcuts_in(&blob());
    let has_remaps = !blob().remaps.is_empty();

    rsx! {
        section {
            class: "settings-section",
            h2 { "Keyboard shortcuts" }
            p {
                class: "bootstrap-muted settings-hint",
                "Click a key to change it. Esc cancels. Press ? from the mail view to open the help list."
            }
            if has_remaps {
                div {
                    class: "settings-actions",
                    button {
                        r#type: "button",
                        class: "onboarding-btn onboarding-btn-secondary accounts-btn-sm",
                        onclick: move |_| {
                            reset_all_shortcuts();
                            blob.set(crate::ui_prefs::load_shortcut_map());
                            capturing.set(None);
                            action_error.set(None);
                        },
                        "Restore defaults"
                    }
                }
            }
            if let Some(err) = action_error() {
                p {
                    class: "onboarding-status onboarding-status-error",
                    role: "alert",
                    "{err}"
                }
            }
            for group in ShortcutGroup::ALL {
                section {
                    class: "shortcut-group settings-shortcut-group",
                    h3 { class: "shortcut-group-title", "{group.title()}" }
                    ul {
                        class: "shortcut-list",
                        for shortcut in listed.iter().filter(|s| s.group == *group) {
                            ShortcutSettingsRow {
                                key: "{shortcut.id.as_key()}",
                                id: shortcut.id,
                                description: shortcut.description,
                                label: shortcut.label.clone(),
                                remappable: shortcut.id.remappable(),
                                remapped: shortcut.remapped,
                                blob,
                                capturing,
                                action_error,
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ShortcutSettingsRow(
    id: ShortcutId,
    description: &'static str,
    label: String,
    remappable: bool,
    remapped: bool,
    mut blob: Signal<ShortcutMapBlob>,
    mut capturing: Signal<Option<ShortcutId>>,
    mut action_error: Signal<Option<String>>,
) -> Element {
    let is_capturing = capturing() == Some(id);
    let bind_class = if is_capturing {
        "shortcut-bind shortcut-bind-capturing"
    } else {
        "shortcut-bind"
    };
    let aria = if is_capturing {
        format!("Press a new key for {description}")
    } else {
        format!("Change shortcut for {description}, currently {label}")
    };

    rsx! {
        li {
            class: "shortcut-row settings-shortcut-row",
            span { class: "shortcut-desc", "{description}" }
            div {
                class: "settings-shortcut-actions",
                if remappable {
                    button {
                        r#type: "button",
                        class: "{bind_class}",
                        autofocus: is_capturing,
                        aria_label: "{aria}",
                        aria_pressed: if is_capturing { "true" } else { "false" },
                        onclick: move |_| {
                            action_error.set(None);
                            capturing.set(Some(id));
                        },
                        onblur: move |_| {
                            if capturing.peek().as_ref() == Some(&id) {
                                capturing.set(None);
                            }
                        },
                        onkeydown: move |evt: KeyboardEvent| {
                            if capturing.peek().as_ref() != Some(&id) {
                                return;
                            }
                            if evt.key() == Key::Escape {
                                evt.prevent_default();
                                evt.stop_propagation();
                                capturing.set(None);
                                return;
                            }
                            if evt.key() == Key::Tab {
                                return;
                            }
                            evt.prevent_default();
                            evt.stop_propagation();
                            let Some((name, shift)) = capture_key_from_event(&evt) else {
                                return;
                            };
                            match remap_shortcut(id, &name, shift) {
                                Ok(()) => {
                                    blob.set(crate::ui_prefs::load_shortcut_map());
                                    capturing.set(None);
                                    action_error.set(None);
                                }
                                Err(e) => action_error.set(Some(e.message())),
                            }
                        },
                        if is_capturing {
                            "Press a key…"
                        } else {
                            kbd { class: "shortcut-key", "{label}" }
                        }
                    }
                    if remapped {
                        button {
                            r#type: "button",
                            class: "onboarding-btn onboarding-btn-secondary accounts-btn-sm",
                            title: "Restore default for {description}",
                            aria_label: "Restore default for {description}",
                            onclick: move |_| {
                                match reset_shortcut(id) {
                                    Ok(()) => {
                                        blob.set(crate::ui_prefs::load_shortcut_map());
                                        capturing.set(None);
                                        action_error.set(None);
                                    }
                                    Err(e) => action_error.set(Some(e.message())),
                                }
                            },
                            "Reset"
                        }
                    }
                } else {
                    kbd { class: "shortcut-key", "{label}" }
                }
            }
        }
    }
}

fn capture_key_from_event(evt: &KeyboardEvent) -> Option<(String, bool)> {
    if evt.modifiers().ctrl() || evt.modifiers().alt() || evt.modifiers().meta() {
        return None;
    }
    let name = evt.key().to_string();
    if name.is_empty() {
        return None;
    }
    Some((name, evt.modifiers().shift()))
}

#[derive(Clone, PartialEq, Eq)]
struct RuleForm {
    id: Option<String>,
    name: String,
    enabled: bool,
    match_from: String,
    match_to: String,
    match_subject: String,
    match_keyword: String,
    match_unread: bool,
    action_move_to: String,
    action_mark_read: bool,
    action_star: bool,
    action_flag: bool,
    action_add_keyword: String,
}

impl RuleForm {
    fn blank() -> Self {
        Self {
            id: None,
            name: String::new(),
            enabled: true,
            match_from: String::new(),
            match_to: String::new(),
            match_subject: String::new(),
            match_keyword: String::new(),
            match_unread: false,
            action_move_to: String::new(),
            action_mark_read: false,
            action_star: false,
            action_flag: false,
            action_add_keyword: String::new(),
        }
    }

    fn from_rule(rule: &MailRule) -> Self {
        Self {
            id: Some(rule.id.clone()),
            name: rule.name.clone(),
            enabled: rule.enabled,
            match_from: rule.match_from.clone(),
            match_to: rule.match_to.clone(),
            match_subject: rule.match_subject.clone(),
            match_keyword: rule.match_keyword.clone().unwrap_or_default(),
            match_unread: rule.match_unread,
            action_move_to: rule.action_move_to.clone().unwrap_or_default(),
            action_mark_read: rule.action_mark_read,
            action_star: rule.action_star,
            action_flag: rule.action_flag,
            action_add_keyword: rule.action_add_keyword.clone().unwrap_or_default(),
        }
    }

    fn into_rule(self) -> MailRule {
        let mut rule = MailRule::new();
        if let Some(id) = self.id {
            rule.id = id;
        }
        rule.name = self.name;
        rule.enabled = self.enabled;
        rule.match_from = self.match_from;
        rule.match_to = self.match_to;
        rule.match_subject = self.match_subject;
        rule.match_keyword = if self.match_keyword.is_empty() {
            None
        } else {
            Some(self.match_keyword)
        };
        rule.match_unread = self.match_unread;
        rule.action_move_to = if self.action_move_to.is_empty() {
            None
        } else {
            Some(self.action_move_to)
        };
        rule.action_mark_read = self.action_mark_read;
        rule.action_star = self.action_star;
        rule.action_flag = self.action_flag;
        rule.action_add_keyword = if self.action_add_keyword.is_empty() {
            None
        } else {
            Some(self.action_add_keyword)
        };
        rule
    }
}

/// Local incoming-mail filters (not ManageSieve).
#[component]
fn FiltersSection() -> Element {
    let ctx = use_context::<AppContext>();
    let mut accounts: Vec<_> = ctx.accounts.read().values().cloned().collect();
    accounts.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.id.as_str().cmp(b.id.as_str()))
    });
    let selected = ctx.selected_account.read().clone();
    let mut account_id = use_signal(|| selected.clone());
    let current = account_id
        .read()
        .as_ref()
        .filter(|id| accounts.iter().any(|a| &a.id == *id))
        .cloned()
        .or_else(|| selected.clone())
        .or_else(|| accounts.first().map(|a| a.id.clone()));
    let mut rules = use_signal(|| {
        current
            .as_ref()
            .map(mail_rules::load_rules)
            .unwrap_or_default()
    });
    let mut form = use_signal(|| None::<RuleForm>);
    let mut action_error = use_signal(|| None::<String>);
    let show_all = *ctx.show_all_folders.read();
    let folder_account_is_active = current.as_ref() == selected.as_ref();
    let folders = if folder_account_is_active {
        let nodes = ctx.mailbox_nodes.read();
        let roots = ctx.mailbox_roots.read();
        flatten_mailboxes(&roots, &nodes)
            .into_iter()
            .filter(|(id, _)| {
                nodes
                    .get(id)
                    .is_some_and(|n| mailbox_is_action_target(n, show_all))
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let listed = rules();

    rsx! {
        section {
            class: "settings-section",
            h2 { "Filters" }
            p {
                class: "bootstrap-muted settings-hint",
                "Sieve is not spoken yet. These rules run locally in this browser when a folder is opened or new mail arrives (IDLE / NOOP). The first matching enabled rule wins. Each message is processed once."
            }
            if accounts.is_empty() {
                p { class: "bootstrap-muted", "Add an account to create filters." }
            } else {
                if accounts.len() > 1 {
                    div {
                        class: "onboarding-field",
                        label { r#for: "settings-filter-account", "Account" }
                        select {
                            id: "settings-filter-account",
                            value: "{current.as_ref().map(|id| id.as_str()).unwrap_or(\"\")}",
                            onchange: move |evt| {
                                let id = AccountId::new(evt.value());
                                account_id.set(Some(id.clone()));
                                rules.set(mail_rules::load_rules(&id));
                                form.set(None);
                                action_error.set(None);
                            },
                            for account in accounts.iter() {
                                option {
                                    value: "{account.id.as_str()}",
                                    selected: current.as_ref() == Some(&account.id),
                                    "{account_from_label(account)}"
                                }
                            }
                        }
                    }
                }
                div {
                    class: "settings-actions",
                    button {
                        r#type: "button",
                        class: "onboarding-btn onboarding-btn-primary accounts-btn-sm",
                        disabled: current.is_none() || form().is_some(),
                        onclick: move |_| {
                            form.set(Some(RuleForm::blank()));
                            action_error.set(None);
                        },
                        "Add filter"
                    }
                }
                if let Some(err) = action_error() {
                    p {
                        class: "onboarding-status onboarding-status-error",
                        role: "alert",
                        "{err}"
                    }
                }
                if let Some(draft) = form() {
                    FilterRuleForm {
                        key: "{draft.id.as_deref().unwrap_or(\"new\")}",
                        draft,
                        folders: folders.clone(),
                        folders_ready: folder_account_is_active && !folders.is_empty(),
                        form,
                        rules,
                        account_id,
                        action_error,
                    }
                }
                if listed.is_empty() && form().is_none() {
                    p {
                        class: "bootstrap-muted settings-contact-empty",
                        "No filters yet."
                    }
                } else if !listed.is_empty() {
                    ul {
                        class: "settings-filter-list",
                        for (idx, rule) in listed.iter().enumerate() {
                            FilterRuleRow {
                                key: "{rule.id}",
                                rule: rule.clone(),
                                index: idx,
                                count: listed.len(),
                                account_id,
                                rules,
                                form,
                                action_error,
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Local out-of-office auto-reply (not ManageSieve).
#[component]
fn VacationSection() -> Element {
    let ctx = use_context::<AppContext>();
    let mut accounts: Vec<_> = ctx.accounts.read().values().cloned().collect();
    accounts.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.id.as_str().cmp(b.id.as_str()))
    });
    let selected = ctx.selected_account.read().clone();
    let mut account_id = use_signal(|| selected.clone());
    let current = account_id
        .read()
        .as_ref()
        .filter(|id| accounts.iter().any(|a| &a.id == *id))
        .cloned()
        .or_else(|| selected.clone())
        .or_else(|| accounts.first().map(|a| a.id.clone()));
    let loaded = current
        .as_ref()
        .map(vacation::load_settings)
        .unwrap_or_default();
    let mut enabled = use_signal(|| loaded.enabled);
    let mut start = use_signal(|| {
        loaded
            .start
            .map(vacation::format_datetime_local)
            .unwrap_or_default()
    });
    let mut end = use_signal(|| {
        loaded
            .end
            .map(vacation::format_datetime_local)
            .unwrap_or_default()
    });
    let mut subject = use_signal(|| loaded.subject.clone());
    let mut body = use_signal(|| loaded.body.clone());
    let mut status = use_signal(|| None::<Result<String, String>>);
    let persist_acc_toggle = current.clone();
    let persist_acc_save = current.clone();

    rsx! {
        section {
            class: "settings-section",
            h2 { "Vacation" }
            p {
                class: "bootstrap-muted settings-hint",
                "Out of office is local to this browser. Mailiner sends a reply over SMTP when new mail arrives (folder open / IDLE / NOOP). This is not a server Sieve script — the app must be open. Each sender is replied to once per vacation period."
            }
            if accounts.is_empty() {
                p { class: "bootstrap-muted", "Add an account to configure vacation." }
            } else {
                if accounts.len() > 1 {
                    div {
                        class: "onboarding-field",
                        label { r#for: "settings-vacation-account", "Account" }
                        select {
                            id: "settings-vacation-account",
                            value: "{current.as_ref().map(|id| id.as_str()).unwrap_or(\"\")}",
                            onchange: move |evt| {
                                let id = AccountId::new(evt.value());
                                account_id.set(Some(id.clone()));
                                apply_vacation_form(
                                    vacation::load_settings(&id),
                                    enabled,
                                    start,
                                    end,
                                    subject,
                                    body,
                                );
                                status.set(None);
                            },
                            for account in accounts.iter() {
                                option {
                                    value: "{account.id.as_str()}",
                                    selected: current.as_ref() == Some(&account.id),
                                    "{account_from_label(account)}"
                                }
                            }
                        }
                    }
                }
                div {
                    class: "onboarding-checkbox-field",
                    label {
                        class: "onboarding-checkbox-label",
                        input {
                            r#type: "checkbox",
                            checked: enabled(),
                            onchange: move |evt| {
                                enabled.set(evt.checked());
                                let Some(acc) = persist_acc_toggle.clone() else {
                                    return;
                                };
                                match persist_vacation(
                                    acc,
                                    evt.checked(),
                                    &start(),
                                    &end(),
                                    subject(),
                                    body(),
                                ) {
                                    Ok(saved) => {
                                        apply_vacation_form(
                                            saved, enabled, start, end, subject, body,
                                        );
                                        status.set(Some(Ok("Vacation settings saved.".into())));
                                    }
                                    Err(msg) => status.set(Some(Err(msg))),
                                }
                            },
                        }
                        "Enabled"
                    }
                }
                div {
                    class: "onboarding-field",
                    label { r#for: "settings-vacation-start", "Start (optional)" }
                    input {
                        id: "settings-vacation-start",
                        r#type: "datetime-local",
                        value: "{start}",
                        oninput: move |evt| start.set(evt.value()),
                    }
                }
                div {
                    class: "onboarding-field",
                    label { r#for: "settings-vacation-end", "End (optional)" }
                    input {
                        id: "settings-vacation-end",
                        r#type: "datetime-local",
                        value: "{end}",
                        oninput: move |evt| end.set(evt.value()),
                    }
                }
                div {
                    class: "onboarding-field",
                    label { r#for: "settings-vacation-subject", "Subject" }
                    input {
                        id: "settings-vacation-subject",
                        r#type: "text",
                        value: "{subject}",
                        placeholder: "Out of office",
                        oninput: move |evt| subject.set(evt.value()),
                    }
                }
                div {
                    class: "onboarding-field",
                    label { r#for: "settings-vacation-body", "Message" }
                    textarea {
                        id: "settings-vacation-body",
                        value: "{body}",
                        placeholder: "I am currently away and will reply when I return.",
                        rows: "5",
                        oninput: move |evt| body.set(evt.value()),
                    }
                }
                p {
                    class: "bootstrap-muted settings-hint",
                    "Reply once per sender is always on for the current start/end window."
                }
                div {
                    class: "settings-actions",
                    button {
                        r#type: "button",
                        class: "onboarding-btn onboarding-btn-primary accounts-btn-sm",
                        disabled: current.is_none(),
                        onclick: move |_| {
                            let Some(acc) = persist_acc_save.clone() else {
                                return;
                            };
                            match persist_vacation(
                                acc,
                                enabled(),
                                &start(),
                                &end(),
                                subject(),
                                body(),
                            ) {
                                Ok(saved) => {
                                    apply_vacation_form(
                                        saved, enabled, start, end, subject, body,
                                    );
                                    status.set(Some(Ok("Vacation settings saved.".into())));
                                }
                                Err(msg) => status.set(Some(Err(msg))),
                            }
                        },
                        "Save vacation"
                    }
                }
                if let Some(result) = status() {
                    match result {
                        Ok(msg) => rsx! {
                            p { class: "bootstrap-muted settings-reset-note", "{msg}" }
                        },
                        Err(msg) => rsx! {
                            p {
                                class: "onboarding-status onboarding-status-error",
                                role: "alert",
                                "{msg}"
                            }
                        },
                    }
                }
            }
        }
    }
}

fn apply_vacation_form(
    next: VacationSettings,
    mut enabled: Signal<bool>,
    mut start: Signal<String>,
    mut end: Signal<String>,
    mut subject: Signal<String>,
    mut body: Signal<String>,
) {
    enabled.set(next.enabled);
    start.set(
        next.start
            .map(vacation::format_datetime_local)
            .unwrap_or_default(),
    );
    end.set(
        next.end
            .map(vacation::format_datetime_local)
            .unwrap_or_default(),
    );
    subject.set(next.subject);
    body.set(next.body);
}

fn persist_vacation(
    acc: AccountId,
    enabled: bool,
    start: &str,
    end: &str,
    subject: String,
    body: String,
) -> Result<VacationSettings, String> {
    let parsed_start = if start.trim().is_empty() {
        None
    } else {
        Some(
            vacation::parse_datetime_local(start)
                .ok_or_else(|| "Start time is not a valid date.".to_string())?,
        )
    };
    let parsed_end = if end.trim().is_empty() {
        None
    } else {
        Some(
            vacation::parse_datetime_local(end)
                .ok_or_else(|| "End time is not a valid date.".to_string())?,
        )
    };
    if let (Some(s), Some(e)) = (parsed_start, parsed_end)
        && s > e
    {
        return Err("Start must be before the end of the vacation window.".into());
    }
    Ok(vacation::save_settings(
        acc,
        VacationSettings {
            enabled,
            start: parsed_start,
            end: parsed_end,
            subject,
            body,
            armed_at: None,
        },
    ))
}

#[component]
fn FilterRuleRow(
    rule: MailRule,
    index: usize,
    count: usize,
    mut account_id: Signal<Option<AccountId>>,
    mut rules: Signal<Vec<MailRule>>,
    mut form: Signal<Option<RuleForm>>,
    mut action_error: Signal<Option<String>>,
) -> Element {
    let name = rule.display_name();
    let actions = rule.action_summary();
    let enabled = rule.enabled;
    let rule_id = rule.id.clone();
    let rule_id_up = rule.id.clone();
    let rule_id_down = rule.id.clone();
    let rule_id_del = rule.id.clone();
    let edit_rule = rule.clone();

    rsx! {
        li {
            class: "settings-filter-row",
            class: if !enabled { "is-disabled" },
            label {
                class: "onboarding-checkbox-label settings-filter-enable",
                input {
                    r#type: "checkbox",
                    checked: enabled,
                    aria_label: "Enable {name}",
                    onchange: move |evt| {
                        let Some(acc) = account_id() else { return; };
                        if mail_rules::set_rule_enabled(&acc, &rule_id, evt.checked()) {
                            rules.set(mail_rules::load_rules(&acc));
                        }
                    },
                }
                span { class: "sr-only", "Enabled" }
            }
            div {
                class: "settings-filter-main",
                span { class: "settings-filter-name", "{name}" }
                span { class: "settings-filter-actions-summary", "{actions}" }
            }
            div {
                class: "settings-filter-row-actions",
                button {
                    r#type: "button",
                    class: "onboarding-btn onboarding-btn-secondary accounts-btn-sm",
                    disabled: index == 0,
                    title: "Run earlier",
                    aria_label: "Move {name} earlier",
                    onclick: move |_| {
                        let Some(acc) = account_id() else { return; };
                        if mail_rules::move_rule(&acc, &rule_id_up, -1) {
                            rules.set(mail_rules::load_rules(&acc));
                        }
                    },
                    "Up"
                }
                button {
                    r#type: "button",
                    class: "onboarding-btn onboarding-btn-secondary accounts-btn-sm",
                    disabled: index + 1 >= count,
                    title: "Run later",
                    aria_label: "Move {name} later",
                    onclick: move |_| {
                        let Some(acc) = account_id() else { return; };
                        if mail_rules::move_rule(&acc, &rule_id_down, 1) {
                            rules.set(mail_rules::load_rules(&acc));
                        }
                    },
                    "Down"
                }
                button {
                    r#type: "button",
                    class: "onboarding-btn onboarding-btn-secondary accounts-btn-sm",
                    onclick: move |_| {
                        form.set(Some(RuleForm::from_rule(&edit_rule)));
                        action_error.set(None);
                    },
                    "Edit"
                }
                button {
                    r#type: "button",
                    class: "onboarding-btn onboarding-btn-secondary accounts-btn-sm",
                    title: "Delete {name}",
                    aria_label: "Delete {name}",
                    onclick: move |_| {
                        let Some(acc) = account_id() else { return; };
                        if mail_rules::remove_rule(&acc, &rule_id_del) {
                            rules.set(mail_rules::load_rules(&acc));
                            if form
                                .peek()
                                .as_ref()
                                .is_some_and(|f| f.id.as_deref() == Some(rule_id_del.as_str()))
                            {
                                form.set(None);
                            }
                            action_error.set(None);
                        }
                    },
                    "Delete"
                }
            }
        }
    }
}

#[component]
fn FilterRuleForm(
    draft: RuleForm,
    folders: Vec<(MailboxId, String)>,
    folders_ready: bool,
    mut form: Signal<Option<RuleForm>>,
    mut rules: Signal<Vec<MailRule>>,
    account_id: Signal<Option<AccountId>>,
    mut action_error: Signal<Option<String>>,
) -> Element {
    let mut draft = use_signal(|| draft);
    let is_edit = draft.read().id.is_some();
    let title = if is_edit { "Edit filter" } else { "New filter" };
    let dest_value = draft.read().action_move_to.clone();
    let dest_known =
        dest_value.is_empty() || folders.iter().any(|(id, _)| id.as_str() == dest_value);
    let keyword_match = draft.read().match_keyword.clone();
    let keyword_add = draft.read().action_add_keyword.clone();

    rsx! {
        form {
            class: "settings-filter-form",
            onsubmit: move |evt| {
                evt.prevent_default();
                let Some(acc) = account_id() else {
                    action_error.set(Some("Select an account first.".into()));
                    return;
                };
                match mail_rules::save_rule(acc.clone(), draft().into_rule()) {
                    Ok(_) => {
                        rules.set(mail_rules::load_rules(&acc));
                        form.set(None);
                        action_error.set(None);
                    }
                    Err(e) => action_error.set(Some(e.message().into())),
                }
            },
            h3 { class: "settings-filter-form-title", "{title}" }
            div {
                class: "onboarding-field",
                label { r#for: "settings-filter-name", "Name" }
                input {
                    id: "settings-filter-name",
                    r#type: "text",
                    value: "{draft.read().name}",
                    placeholder: "Optional",
                    oninput: move |evt| draft.write().name = evt.value(),
                }
            }
            div {
                class: "onboarding-checkbox-field",
                label {
                    class: "onboarding-checkbox-label",
                    input {
                        r#type: "checkbox",
                        checked: draft.read().enabled,
                        onchange: move |evt| draft.write().enabled = evt.checked(),
                    }
                    "Enabled"
                }
            }
            p { class: "bootstrap-muted settings-hint", "Match (all set fields, case-insensitive)" }
            div {
                class: "onboarding-field",
                label { r#for: "settings-filter-from", "From contains" }
                input {
                    id: "settings-filter-from",
                    r#type: "text",
                    value: "{draft.read().match_from}",
                    oninput: move |evt| draft.write().match_from = evt.value(),
                }
            }
            div {
                class: "onboarding-field",
                label { r#for: "settings-filter-to", "To contains" }
                input {
                    id: "settings-filter-to",
                    r#type: "text",
                    value: "{draft.read().match_to}",
                    oninput: move |evt| draft.write().match_to = evt.value(),
                }
            }
            div {
                class: "onboarding-field",
                label { r#for: "settings-filter-subject", "Subject contains" }
                input {
                    id: "settings-filter-subject",
                    r#type: "text",
                    value: "{draft.read().match_subject}",
                    oninput: move |evt| draft.write().match_subject = evt.value(),
                }
            }
            div {
                class: "onboarding-field",
                label { r#for: "settings-filter-keyword", "Has keyword" }
                select {
                    id: "settings-filter-keyword",
                    value: "{keyword_match}",
                    onchange: move |evt| draft.write().match_keyword = evt.value(),
                    option { value: "", selected: keyword_match.is_empty(), "Any" }
                    for keyword in ImapKeyword::ALL {
                        option {
                            value: "{keyword.atom()}",
                            selected: keyword_match == keyword.atom(),
                            "{keyword.label()}"
                        }
                    }
                }
            }
            div {
                class: "onboarding-checkbox-field",
                label {
                    class: "onboarding-checkbox-label",
                    input {
                        r#type: "checkbox",
                        checked: draft.read().match_unread,
                        onchange: move |evt| draft.write().match_unread = evt.checked(),
                    }
                    "Only unread"
                }
            }
            p { class: "bootstrap-muted settings-hint", "Then" }
            div {
                class: "onboarding-field",
                label { r#for: "settings-filter-move", "Move to folder" }
                select {
                    id: "settings-filter-move",
                    value: "{dest_value}",
                    onchange: move |evt| draft.write().action_move_to = evt.value(),
                    option { value: "", selected: dest_value.is_empty(), "Do not move" }
                    if !dest_known && !dest_value.is_empty() {
                        option {
                            value: "{dest_value}",
                            selected: true,
                            "{dest_value}"
                        }
                    }
                    for (id, title) in folders.iter() {
                        option {
                            value: "{id.as_str()}",
                            selected: dest_value == id.as_str(),
                            "{title}"
                        }
                    }
                }
            }
            if !folders_ready {
                p {
                    class: "bootstrap-muted settings-hint",
                    "Open this account in mail to load folders for the move list."
                }
            }
            div {
                class: "onboarding-checkbox-field",
                label {
                    class: "onboarding-checkbox-label",
                    input {
                        r#type: "checkbox",
                        checked: draft.read().action_mark_read,
                        onchange: move |evt| draft.write().action_mark_read = evt.checked(),
                    }
                    "Mark as read"
                }
            }
            div {
                class: "onboarding-checkbox-field",
                label {
                    class: "onboarding-checkbox-label",
                    input {
                        r#type: "checkbox",
                        checked: draft.read().action_star,
                        onchange: move |evt| draft.write().action_star = evt.checked(),
                    }
                    "Star"
                }
            }
            div {
                class: "onboarding-checkbox-field",
                label {
                    class: "onboarding-checkbox-label",
                    input {
                        r#type: "checkbox",
                        checked: draft.read().action_flag,
                        onchange: move |evt| draft.write().action_flag = evt.checked(),
                    }
                    "Flag"
                }
            }
            div {
                class: "onboarding-field",
                label { r#for: "settings-filter-add-keyword", "Add keyword" }
                select {
                    id: "settings-filter-add-keyword",
                    value: "{keyword_add}",
                    onchange: move |evt| draft.write().action_add_keyword = evt.value(),
                    option { value: "", selected: keyword_add.is_empty(), "None" }
                    for keyword in ImapKeyword::ALL {
                        option {
                            value: "{keyword.atom()}",
                            selected: keyword_add == keyword.atom(),
                            "{keyword.label()}"
                        }
                    }
                }
            }
            div {
                class: "settings-actions",
                button {
                    r#type: "submit",
                    class: "onboarding-btn onboarding-btn-primary accounts-btn-sm",
                    if is_edit { "Save filter" } else { "Add filter" }
                }
                button {
                    r#type: "button",
                    class: "onboarding-btn onboarding-btn-secondary accounts-btn-sm",
                    onclick: move |_| {
                        form.set(None);
                        action_error.set(None);
                    },
                    "Cancel"
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
