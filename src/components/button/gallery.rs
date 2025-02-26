use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;

use super::button::*;

#[component]
pub fn Gallery() -> Element {
    rsx! {
        h1 {
            class: class!(text_2xl font_bold),
            { "Buttons" }
        }

        div {
            class: class!(flex w_full gap_5),

            div {
                class: class!(flex flex_col grow gap_2),

                Button {
                    text: "Primary",
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Medium,
                    full_width: true,
                }

                Button {
                    text: "Secondary",
                    variant: ButtonVariant::Secondary,
                    size: ButtonSize::Medium,
                    full_width: true,
                }

                Button {
                    text: "Danger",
                    variant: ButtonVariant::Danger,
                    size: ButtonSize::Medium,
                    full_width: true,
                }

                Button {
                    text: "Ghost",
                    variant: ButtonVariant::Ghost,
                    size: ButtonSize::Medium,
                    full_width: true,
                }
            }

            div {
                class: class!(flex flex_col grow gap_2),

                Button {
                    text: "Small",
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Small,
                    full_width: true,
                }

                Button {
                    text: "Medium",
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Medium,
                    full_width: true,
                }

                Button {
                    text: "Large",
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Large,
                    full_width: true,
                }
            }

            div {
                class: class!(flex flex_col grow gap_2),

                Button {
                    text: "Loading",
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Medium,
                    full_width: true,
                    loading: true,
                }
            }
        }
    }
}
