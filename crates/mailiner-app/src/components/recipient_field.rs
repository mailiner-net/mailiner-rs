//! Chip-style To / Cc / Bcc field with contact / recent autocomplete.

use dioxus::html::Key;
use dioxus::prelude::*;

use mailiner_composer::emails_equal;
use mailiner_composer::model::draft::ComposerAddress;
use mailiner_composer::shell::recipient_field::{
    chip_is_valid, chip_label, chip_title, commit_input, remove_last_recipient, remove_recipient,
};

use super::icons::{Icon, IconKind};
use crate::address_book::{self, Contact};
use crate::context::AppContext;
use crate::recipient_suggest::{
    RecipientSuggestion, contacts_from_envelopes, default_suggestion_limit, load_recent_recipients,
    merge_recent_candidates, suggest_recipients, typed_overrides_suggestion,
};

fn harvested_from_open_mailbox(ctx: &AppContext) -> Vec<Contact> {
    let skip: Vec<String> = ctx
        .accounts
        .read()
        .values()
        .map(|account| account.email.clone())
        .collect();
    let skip_refs: Vec<&str> = skip.iter().map(String::as_str).collect();
    contacts_from_envelopes(
        ctx.messages.read().iter().map(|msg| &msg.envelope),
        &skip_refs,
    )
}

fn apply_suggestion(
    chips: &mut Signal<Vec<ComposerAddress>>,
    draft: &mut Signal<String>,
    suggestion: &RecipientSuggestion,
    open: &mut Signal<bool>,
) {
    let addr = suggestion.to_composer_address();
    let mut next = chips();
    if !next
        .iter()
        .any(|existing| emails_equal(&existing.email, &addr.email))
    {
        next.push(addr);
    }
    chips.set(next);
    draft.set(String::new());
    open.set(false);
}

#[component]
pub fn RecipientField(
    label: String,
    chips: Signal<Vec<ComposerAddress>>,
    draft: Signal<String>,
    disabled: bool,
) -> Element {
    let ctx = use_context::<AppContext>();
    let contacts = use_hook(address_book::load_contacts);
    let recents = use_hook(load_recent_recipients);
    let mut chips = chips;
    let mut draft = draft;
    let mut input: Signal<Option<std::rc::Rc<MountedData>>> = use_signal(|| None);
    let mut highlight = use_signal(|| 0usize);
    let mut list_open = use_signal(|| false);
    let chip_list = chips();
    let empty = chip_list.is_empty();
    let query = draft();
    let harvested = harvested_from_open_mailbox(&ctx);
    let recents = merge_recent_candidates(&recents, &harvested);
    let suggestions = suggest_recipients(
        &contacts,
        &recents,
        &query,
        &chip_list,
        default_suggestion_limit(),
    );
    let suggestion_count = suggestions.len();
    let active_idx = if suggestion_count == 0 {
        0
    } else {
        highlight().min(suggestion_count - 1)
    };
    let show_list = !disabled && list_open() && suggestion_count > 0;
    let list_id = format!("recipient-suggest-{label}");
    let active_id = if show_list {
        Some(format!("{list_id}-{active_idx}"))
    } else {
        None
    };

    rsx! {
        div {
            class: if disabled { "recipient-field is-disabled" } else { "recipient-field" },
            onclick: move |_| {
                if disabled {
                    return;
                }
                if let Some(el) = input() {
                    spawn(async move {
                        let _ = el.set_focus(true).await;
                    });
                }
            },
            for (i, addr) in chip_list.into_iter().enumerate() {
                {
                    let valid = chip_is_valid(&addr);
                    let label_text = chip_label(&addr).to_string();
                    let title = chip_title(&addr);
                    rsx! {
                        span {
                            key: "{addr.email}",
                            class: if valid { "recipient-chip" } else { "recipient-chip is-invalid" },
                            title: "{title}",
                            aria_invalid: if valid { "false" } else { "true" },
                            span { class: "recipient-chip-label", "{label_text}" }
                            button {
                                class: "recipient-chip-remove",
                                r#type: "button",
                                title: "Remove {label_text}",
                                aria_label: "Remove {label_text}",
                                disabled,
                                onclick: move |evt| {
                                    evt.prevent_default();
                                    evt.stop_propagation();
                                    if disabled {
                                        return;
                                    }
                                    chips.set(remove_recipient(&chips(), i));
                                },
                                Icon { size: 12, icon: IconKind::XMark }
                            }
                        }
                    }
                }
            }
            input {
                class: "recipient-field-input",
                r#type: "text",
                inputmode: "email",
                role: "combobox",
                aria_autocomplete: "list",
                aria_expanded: if show_list { "true" } else { "false" },
                aria_controls: "{list_id}",
                aria_activedescendant: active_id.as_deref().unwrap_or(""),
                aria_haspopup: "listbox",
                value: draft(),
                disabled,
                onmounted: move |evt| input.set(Some(evt.data())),
                placeholder: if empty { "name@example.com" } else { "" },
                aria_label: "{label}",
                autocomplete: "off",
                spellcheck: "false",
                autocapitalize: "off",
                oninput: move |e| {
                    if disabled {
                        return;
                    }
                    let (next, leftover) = commit_input(&chips(), &e.value(), false);
                    chips.set(next);
                    draft.set(leftover);
                    highlight.set(0);
                    list_open.set(true);
                },
                onkeydown: move |evt: KeyboardEvent| {
                    if disabled {
                        return;
                    }
                    match evt.key() {
                        Key::ArrowDown if suggestion_count > 0 => {
                            evt.prevent_default();
                            evt.stop_propagation();
                            if list_open() {
                                highlight.set((highlight() + 1) % suggestion_count);
                            } else {
                                highlight.set(0);
                                list_open.set(true);
                            }
                        }
                        Key::ArrowUp if suggestion_count > 0 => {
                            evt.prevent_default();
                            evt.stop_propagation();
                            if list_open() {
                                highlight.set(
                                    (highlight() + suggestion_count - 1) % suggestion_count,
                                );
                            } else {
                                highlight.set(suggestion_count - 1);
                                list_open.set(true);
                            }
                        }
                        Key::Escape if show_list => {
                            evt.prevent_default();
                            evt.stop_propagation();
                            list_open.set(false);
                        }
                        Key::Tab if show_list => {
                            if let Some(suggestion) = suggestions.get(active_idx) {
                                if !typed_overrides_suggestion(&draft(), &suggestion.contact.email) {
                                    apply_suggestion(
                                        &mut chips,
                                        &mut draft,
                                        suggestion,
                                        &mut list_open,
                                    );
                                } else {
                                    list_open.set(false);
                                }
                            }
                        }
                        Key::Enter => {
                            if evt.modifiers().ctrl() || evt.modifiers().meta() {
                                return;
                            }
                            evt.prevent_default();
                            if show_list {
                                if let Some(suggestion) = suggestions.get(active_idx) {
                                    if !typed_overrides_suggestion(
                                        &draft(),
                                        &suggestion.contact.email,
                                    ) {
                                        apply_suggestion(
                                            &mut chips,
                                            &mut draft,
                                            suggestion,
                                            &mut list_open,
                                        );
                                        return;
                                    }
                                }
                            }
                            let (next, leftover) = commit_input(&chips(), &draft(), true);
                            chips.set(next);
                            draft.set(leftover);
                            list_open.set(false);
                        }
                        Key::Backspace if draft().is_empty() && !chips().is_empty() => {
                            evt.prevent_default();
                            chips.set(remove_last_recipient(&chips()));
                        }
                        _ => {}
                    }
                },
                onblur: move |_| {
                    if disabled {
                        return;
                    }
                    list_open.set(false);
                    let (next, leftover) = commit_input(&chips(), &draft(), true);
                    chips.set(next);
                    draft.set(leftover);
                },
            }
            if show_list {
                ul {
                    id: "{list_id}",
                    class: "recipient-suggest",
                    role: "listbox",
                    aria_label: "{label} suggestions",
                    for (i, suggestion) in suggestions.iter().enumerate() {
                        {
                            let suggestion = suggestion.clone();
                            let option_id = format!("{list_id}-{i}");
                            let active = i == active_idx;
                            let name = suggestion.display_label().to_string();
                            let email = suggestion.contact.email.clone();
                            let show_email = !suggestion.contact.name.trim().is_empty();
                            let source = suggestion.source_label();
                            rsx! {
                                li {
                                    key: "{suggestion.contact.email}",
                                    id: "{option_id}",
                                    class: if active {
                                        "recipient-suggest-option is-active"
                                    } else {
                                        "recipient-suggest-option"
                                    },
                                    role: "option",
                                    aria_selected: if active { "true" } else { "false" },
                                    onmousedown: move |evt| {
                                        evt.prevent_default();
                                    },
                                    onclick: move |evt| {
                                        evt.prevent_default();
                                        evt.stop_propagation();
                                        apply_suggestion(
                                            &mut chips,
                                            &mut draft,
                                            &suggestion,
                                            &mut list_open,
                                        );
                                    },
                                    span { class: "recipient-suggest-name", "{name}" }
                                    span { class: "recipient-suggest-meta",
                                        if show_email {
                                            span { class: "recipient-suggest-email", "{email}" }
                                        }
                                        span { class: "recipient-suggest-source", "{source}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
