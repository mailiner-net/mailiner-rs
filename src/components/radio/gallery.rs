use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;

use super::radio::*;

#[component]
pub fn Gallery() -> Element {
    let mut selected = use_signal(|| None::<String>);

    rsx! {
        h1 {
            class: class!(text_2xl font_bold),
            { "Radio" }
        }

        div {
            class: class!(flex w_full gap_5),

            div {
                class: class!(flex flex_col grow gap_2),

                RadioGroup {
                    name: "group-1",
                    options: vec![
                        RadioOption {
                            label: "Option 1".to_string(),
                            value: "option-1".to_string(),
                            disabled: false,
                        },
                        RadioOption {
                            label: "Option 2".to_string(),
                            value: "option-2".to_string(),
                            disabled: true,
                        },
                        RadioOption {
                            label: "Option 3".to_string(),
                            value: "option-3".to_string(),
                            disabled: false,
                        }
                    ],
                    on_change: move |value| {
                        selected.set(Some(value));
                    },
                }

                span {
                    { "Selected value: " }
                    if let Some(value) = selected() {
                        "{value}"
                    } else {
                        "None"
                    }
                }
            }

            div {
                class: class!(flex flex_col grow gap_2),

                RadioGroup {
                    name: "group-2",
                    direction: RadioGroupDirection::Horizontal,
                    options: vec![
                        RadioOption {
                            label: "Option 1".to_string(),
                            value: "option-1".to_string(),
                            disabled: false,
                        },
                        RadioOption {
                            label: "Option 2".to_string(),
                            value: "option-2".to_string(),
                            disabled: false,
                        }
                    ]
                }
            }

            div {
                class: class!(flex flex_col grow gap_2),

                RadioGroup {
                    name: "group-3",
                    selected: "option-2",
                    options: vec![
                        RadioOption {
                            label: "Option 1".to_string(),
                            value: "option-1".to_string(),
                            disabled: false,
                        },
                        RadioOption {
                            label: "Option 2".to_string(),
                            value: "option-2".to_string(),
                            disabled: false,
                        },
                        RadioOption {
                            label: "Option 3".to_string(),
                            value: "option-3".to_string(),
                            disabled: false,
                        }
                    ],
                    disabled: true,
                }
            }
        }
    }
}
