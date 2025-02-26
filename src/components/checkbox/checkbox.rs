use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;
use mailiner_css::*;

/// Props for the Checkbox component
#[derive(Clone, Props, PartialEq)]
pub struct CheckboxProps {
    /// Label text for the checkbox
    #[props(into)]
    label: Option<String>,

    /// Whether the checkbox is checked
    checked: Option<bool>,

    /// Whether the checkbox is disabled
    disabled: Option<bool>,

    /// Whether the checkbox is in an indeterminate state
    indeterminate: Option<bool>,

    /// Change event handler
    on_change: Option<EventHandler<bool>>,
}

/// The checkbox component for Mailiner UI
#[component]
pub fn Checkbox(props: CheckboxProps) -> Element {
    let mut checked = use_signal(|| props.checked.unwrap_or(false));
    let is_disabled = use_signal(|| props.disabled.unwrap_or(false));

    // Handle checkbox state changes
    let on_change = move |_| {
        if !is_disabled() {
            let new_state = !checked();
            checked.set(new_state);

            if let Some(handler) = &props.on_change {
                handler.call(new_state);
            }
        }
    };

    // Create an ID for associating the label with the input
    let id = format!("checkbox-{}", use_memo(|| uuid::Uuid::new_v4().to_string()));

    // Determine appearance classes based on state
    let appearance = if is_disabled() {
        if checked() {
            class!(bg_primary_300 border_primary_300 cursor_not_allowed)
        } else {
            class!(bg_neutral_100 border_neutral_300 cursor_not_allowed)
        }
    } else {
        if checked() {
            class!(bg_primary_500 border_primary_500 cursor_pointer)
        } else {
            class!(bg_white border_neutral_300 hover(border_primary_500) cursor_pointer)
        }
    };

    rsx! {
        div { class: "flex items-center",
            // Hidden actual checkbox for accessibility
            input {
                id: "{id}",
                type: "checkbox",
                checked: checked(),
                disabled: is_disabled,
                class: class!(absolute w_0 h_0 opacity_0),
                onchange: on_change
            }

            // Custom styled checkbox
            label {
                class: class!(flex items_center cursor_pointer),
                for: "{id}",

                // Custom checkbox element
                div {
                    class: format!("{} {}", class!(w_5 h_5 border rounded transition_colors mr_2 flex items_center justify_center), appearance),

                    // Checkmark (show when checked)
                    if checked() {
                        svg {
                            class: class!(w_3_half h_3_half text_white),
                            xmlns: "http://www.w3.org/2000/svg",
                            view_box: "0 0 24 24",
                            stroke_width: "3",
                            stroke: "currentColor",
                            fill: "none",

                            path {
                                stroke_linecap: "round",
                                stroke_linejoin: "round",
                                d: "M5 13l4 4L19 7"
                            }
                        }
                    }

                    // Indeterminate state (horizontal line)
                    if !checked() && props.indeterminate.unwrap_or(false) {
                        div {
                            class: class!(w_2_half h__half bg_primary_500 rounded_full)
                        }
                    }
                }

                // Label text (if provided)
                if let Some(label_text) = &props.label {
                    span {
                        class: if is_disabled() { class!(text_neutral_400) } else { class!(text_neutral_700) },
                        "{label_text}"
                    }
                }
            }
        }
    }
}
