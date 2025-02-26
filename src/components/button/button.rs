// components/button.rs
use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;
use mailiner_css::*;

/// Button variants for the Mailiner UI
#[derive(PartialEq, Clone)]
pub enum ButtonVariant {
    Primary,
    Secondary,
    Danger,
    Ghost,
}

/// Button sizes for different contexts
#[derive(PartialEq, Clone)]
pub enum ButtonSize {
    Small,
    Medium,
    Large,
}

/// Props for the Button component
#[derive(Clone, Props, PartialEq)]
pub struct ButtonProps {
    /// Text to display on the button
    #[props(into)]
    text: String,

    /// Optional icon to display before the text
    icon: Option<Element>,

    /// Button variant (Primary, Secondary, Danger, Ghost)
    variant: Option<ButtonVariant>,

    /// Button size (Small, Medium, Large)
    size: Option<ButtonSize>,

    /// Whether the button should take full width
    full_width: Option<bool>,

    /// Whether the button is in a loading state
    loading: Option<bool>,

    /// Whether the button is disabled
    disabled: Option<bool>,

    /// Click event handler
    on_click: Option<EventHandler<MouseEvent>>,
}

/// Button component for Mailiner UI
#[component]
pub fn Button(props: ButtonProps) -> Element {
    let is_disabled = props.disabled.unwrap_or(false) || props.loading.unwrap_or(false);

    // Define base classes that apply to all buttons
    let base_classes = class!(inline_flex items_center justify_center font_medium transition_colors focus(outline_none) focus(ring_2) focus(ring_offset_2) focus(ring_primary_500));

    // Determine variant-specific classes
    let variant_classes = match props.variant.clone().unwrap_or(ButtonVariant::Primary) {
        ButtonVariant::Primary => {
            if is_disabled {
                class!(bg_primary_300 text_white cursor_not_allowed)
            } else {
                class!(bg_primary_500 text_white hover(bg_primary_600) active(bg_primary_700))
            }
        }
        ButtonVariant::Secondary => {
            if is_disabled {
                class!(bg_neutral_200 text_neutral_400 cursor_not_allowed)
            } else {
                class!(bg_neutral_200 text_neutral_700 hover(bg_neutral_300) active(bg_neutral_400))
            }
        }
        ButtonVariant::Danger => {
            if is_disabled {
                class!(bg_danger_300 text_white cursor_not_allowed)
            } else {
                class!(bg_danger_500 text_white hover(bg_danger_600) active(bg_danger_700))
            }
        }
        ButtonVariant::Ghost => {
            if is_disabled {
                class!(bg_transparent text_neutral_400 cursor_not_allowed)
            } else {
                class!(bg_transparent text_neutral_700 hover(bg_neutral_100) active(bg_neutral_200))
            }
        }
    };

    // Determine size-specific classes
    let size_classes = match props.size.clone().unwrap_or(ButtonSize::Medium) {
        ButtonSize::Small => class!(text_sm py_1 px_2),
        ButtonSize::Medium => class!(text_base py_2 px_3),
        ButtonSize::Large => class!(text_base py_2_half px_4),
    };

    // Determine width classes
    let width_classes = if props.full_width.unwrap_or(false) {
        class!(w_full)
    } else {
        class!("")
    };

    // Build the final class string
    let class = format!(
        "{} {} {} {} {}",
        base_classes, variant_classes, size_classes, width_classes, class!(rounded)
    );

    rsx! {
        button {
            class: "{class}",
            disabled: is_disabled,
            onclick: move |evt| {
                if !is_disabled {
                    if let Some(handler) = &props.on_click {
                        handler.call(evt);
                    }
                }
            },

            // Show loading spinner if loading
            if props.loading.unwrap_or(false) {
                // Simple loading spinner (can be replaced with an animated SVG)
                span { class: class!(mr_2 animate_spin),
                    "◌"
                }
            }

            // Show icon if provided
            if let Some(icon) = &props.icon {
                span { class: class!(mr_2),
                    {icon}
                }
            }

            // Button text
            span {
                "{props.text}"
            }
        }
    }
}

