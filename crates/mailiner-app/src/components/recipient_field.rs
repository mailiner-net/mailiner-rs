//! Chip-style To / Cc / Bcc field.

use dioxus::html::Key;
use dioxus::prelude::*;

use mailiner_composer::model::draft::ComposerAddress;
use mailiner_composer::shell::recipient_field::{
    chip_is_valid, chip_label, chip_title, commit_input, remove_last_recipient, remove_recipient,
};

use super::icons::{Icon, IconKind};

#[component]
pub fn RecipientField(
    label: &'static str,
    chips: Signal<Vec<ComposerAddress>>,
    draft: Signal<String>,
    disabled: bool,
) -> Element {
    let mut chips = chips;
    let mut draft = draft;
    let mut input: Signal<Option<std::rc::Rc<MountedData>>> = use_signal(|| None);
    let chip_list = chips();
    let empty = chip_list.is_empty();

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
                },
                onkeydown: move |evt: KeyboardEvent| {
                    if disabled {
                        return;
                    }
                    match evt.key() {
                        Key::Enter => {
                            if evt.modifiers().ctrl() || evt.modifiers().meta() {
                                return;
                            }
                            evt.prevent_default();
                            let (next, leftover) = commit_input(&chips(), &draft(), true);
                            chips.set(next);
                            draft.set(leftover);
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
                    let (next, leftover) = commit_input(&chips(), &draft(), true);
                    chips.set(next);
                    draft.set(leftover);
                },
            }
        }
    }
}
