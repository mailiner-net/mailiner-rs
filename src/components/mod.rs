use dioxus::prelude::*;
use dioxus_router::prelude::*;
use dioxus_tailwindcss::prelude::*;

mod button;
mod checkbox;
mod input;
mod radio;

pub use button::{Button, ButtonProps, ButtonSize, ButtonVariant};
pub use checkbox::{Checkbox, CheckboxProps};
pub use input::{Input, InputProps, InputSize, InputType};
pub use radio::{RadioButton, RadioButtonProps, RadioGroup, RadioGroupDirection, RadioGroupProps};

pub fn ComponentGallery() -> Element {
    rsx! {
        div {
            class: class!(m_5),
            style: "max-width: 50%",

            div {
                class: class!(flex flex_col gap_5),

                button::Gallery {}

                checkbox::Gallery {}

                radio::Gallery {}

                input::Gallery {}
            }
        }
    }
}
