use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;
use mailiner_css::*;

/// Props for a single radio button
#[derive(Clone, Props, PartialEq)]
pub struct RadioButtonProps {
    /// Label text for the radio button
    label: String,

    /// Value associated with this radio button
    value: String,

    /// Group name to associate radio buttons together
    group: String,

    /// Whether this radio is selected
    selected: Option<bool>,

    /// Whether this radio is disabled
    disabled: Option<bool>,

    /// Change event handler
    on_change: Option<EventHandler<String>>,
}

/// Radio Button component for Mailiner UI
pub fn RadioButton(props: RadioButtonProps) -> Element {
    let is_selected = props.selected.unwrap_or(false);
    let is_disabled = props.disabled.unwrap_or(false);

    // Generate unique ID for accessibility
    let id = format!(
        "radio-{}-{}",
        props.group,
        use_memo(|| uuid::Uuid::new_v4().to_string())
    );

    // Handle radio change
    let value = props.value.clone();
    let on_change = move |_| {
        if !is_disabled && !is_selected {
            if let Some(handler) = &props.on_change {
                handler.call(value.clone());
            }
        }
    };

    // Determine appearance classes based on state
    let appearance = if is_disabled {
        if is_selected {
            class!(border_neutral_400 cursor_not_allowed)
        } else {
            class!(border_neutral_300 cursor_not_allowed)
        }
    } else {
        if is_selected {
            class!(border_primary_500 cursor_pointer)
        } else {
            class!(border_neutral_300 hover(border_primary_500) cursor_pointer)
        }
    };

    rsx! {
        div {
            class: class!(flex items_center),
            // Hidden actual radio button for accessibility
            input {
                id: "{id}",
                type: "radio",
                name: "{props.group}",
                value: "{props.value}",
                checked: is_selected,
                disabled: is_disabled,
                class: class!(absolute w_0 h_0 opacity_0),
                onchange: on_change
            }

            // Custom styled radio button
            label {
                class: class!(flex items_center cursor_pointer),
                for: "{id}",

                // Custom radio element (circle)
                div {
                    class: format!("{} {}", class!(w_5 h_5 border rounded_full transition_colors mr_2 flex items_center justify_center), appearance),

                    // Inner dot (show when selected)
                    if is_selected {
                        div {
                            class: if is_disabled {
                                class!(w_2_half h_2_half rounded_full bg_neutral_400)
                            } else {
                                class!(w_2_half h_2_half rounded_full bg_primary_500)
                            }
                        }
                    }
                }

                // Label text
                span {
                    class: if is_disabled { class!(text_neutral_400) } else { class!(text_neutral_700) },
                    "{props.label}"
                }
            }
        }
    }
}

/// Props for a radio group
#[derive(Clone, Props, PartialEq)]
pub struct RadioGroupProps {
    /// Group name to associate radio buttons
    #[props(into)]
    name: String,

    /// Selected value
    #[props(into)]
    selected: Option<String>,

    /// Whether the group is disabled
    disabled: Option<bool>,

    /// Options for the radio group
    options: Vec<RadioOption>,

    /// Layout direction
    direction: Option<RadioGroupDirection>,

    /// Change event handler
    on_change: Option<EventHandler<String>>,
}

/// Layout direction for radio groups
#[derive(PartialEq, Clone)]
pub enum RadioGroupDirection {
    Horizontal,
    Vertical,
}

/// Radio option data structure
#[derive(PartialEq, Clone)]
pub struct RadioOption {
    pub label: String,
    pub value: String,
    pub disabled: bool,
}

/// Radio Group component for Mailiner UI
pub fn RadioGroup(props: RadioGroupProps) -> Element {
    let mut selected = use_signal(|| props.selected.unwrap_or_default());
    let is_disabled = props.disabled.unwrap_or(false);

    // Handle radio change
    let on_change = move |value: String| {
        selected.set(value.clone());

        if let Some(handler) = &props.on_change {
            handler.call(value);
        }
    };

    // Container class based on direction
    let container_class = match props
        .direction
        .clone()
        .unwrap_or(RadioGroupDirection::Vertical)
    {
        RadioGroupDirection::Horizontal => class!(flex flex_row space_x_4),
        RadioGroupDirection::Vertical => class!(flex flex_col space_y_2),
    };

    rsx! {
        div {
            class: "{container_class}",
            
            for option in props.options.iter() {
                RadioButton {
                    label: option.label.clone(),
                    value: option.value.clone(),
                    group: props.name.clone(),
                    selected: selected() == option.value,
                    disabled: option.disabled || is_disabled,
                    on_change: on_change.clone()
                }
            }
        }
    }
}
