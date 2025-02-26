use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;

use super::input::*;

#[component]
pub fn Gallery() -> Element {
    let mut value = use_signal(|| None::<String>);
    let mut has_focus = use_signal(|| false);

    rsx! {
        h1 {
            class: class!(text_2xl font_bold),
            { "Input" }
        }

        div {
            class: class!(flex w_full gap_5),

            div {
                class: class!(flex flex_col grow gap_2),

                Input {
                    label: "Text",
                    input_type: InputType::Text,
                    placeholder: "Enter text",
                    size: InputSize::Small,
                    full_width: true,
                }

                Input {
                    label: "Email",
                    input_type: InputType::Email,
                    placeholder: "Enter email",
                    size: InputSize::Medium,
                    full_width: true,
                }

                Input {
                    label: "Password",
                    input_type: InputType::Password,
                    placeholder: "Enter password",
                    size: InputSize::Large,
                    full_width: true,
                }

                Input {
                    label: "Search",
                    input_type: InputType::Search,
                    placeholder: "Search",
                    full_width: true,
                }

                Input {
                    label: "Number",
                    input_type: InputType::Number,
                    placeholder: "Enter number",
                    full_width: true,
                }

                Input {
                    label: "URL",
                    input_type: InputType::Url,
                    placeholder: "Enter URL",
                    full_width: true,
                }
            }

            div {
                class: class!(flex flex_col grow gap_2),

                Input {
                    label: "Disabled",
                    input_type: InputType::Text,
                    placeholder: "Enter text",
                    full_width: true,
                    disabled: true,
                }

                Input {
                    label: "Error",
                    input_type: InputType::Text,
                    placeholder: "Enter text",
                    full_width: true,
                    error: "Error",
                }

                Input {
                    label: "Helper text",
                    input_type: InputType::Text,
                    placeholder: "Enter text",
                    full_width: true,
                    helper_text: "Helper text",
                }

                /*
                Input {
                    label: "Start icon",
                    input_type: InputType::Text,
                    placeholder: "Enter text",
                    full_width: true,
                    start_icon: Some(rsx!(Icon { name: "search" })),
                }

                Input {
                    label: "End icon",
                    input_type: InputType::Text,
                    placeholder: "Enter text",
                    full_width: true,
                    end_icon: Some(rsx!(Icon { name: "search" })),
                }

                Input {
                    label: "Start and end icon",
                    input_type: InputType::Text,
                    placeholder: "Enter text",
                    full_width: true,
                    start_icon: Some(rsx!(Icon { name: "search" })),
                    end_icon: Some(rsx!(Icon { name: "search" })),
                }
                */

                Input {
                    label: "Read only",
                    input_type: InputType::Text,
                    full_width: true,
                    value: "This is read-only input",
                    read_only: true,
                }
            }

            div {
                class: class!(flex flex_col grow gap_2),

                Input {
                    label: "Controlled",
                    input_type: InputType::Text,
                    size: InputSize::Small,
                    full_width: true,
                    on_change: move |new_value| {
                        value.set(Some(new_value));
                    },
                    on_focus: move |_| {
                        has_focus.set(true);
                    },
                    on_blur: move |_| {
                        has_focus.set(false);
                    },
                }

                span {
                    { "Value: " }
                    if let Some(v) = value() {
                        "{v}"
                    } else {
                        "None"
                    }
                }
                span {
                    { "Has focus: " }
                    if has_focus() {
                        "Yes"
                    } else {
                        "No"
                    }
                }
            }
        }
    }
}
