use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;

use super::checkbox::*;

#[component]
pub fn Gallery() -> Element {
    let mut is_checked = use_signal(|| false);

    rsx! {
        h1 {
            class: class!(text_2xl font_bold),
            { "CheckBox" }
        }

        div {
            class: class!(flex w_full gap_5),

            div {
                class: class!(flex flex_col grow gap_2),

                Checkbox {
                    label: "Checkbox",
                    checked: true,
                }

                Checkbox {
                    label: "Disabled",
                    disabled: true,
                }

                Checkbox {
                    label: "Indeterminate",
                    indeterminate: true,
                }
            }

            div {
                class: class!(flex flex_col grow gap_2),

                Checkbox {
                    label: "Toggle me",
                    on_change: move |checked| {
                        is_checked.set(checked)
                    },
                }

                span {
                    { "Checkbox is checked: " },
                    if is_checked() {
                        "yes"
                    } else {
                        "no"
                    }
                }
            }
        }
    }
}
