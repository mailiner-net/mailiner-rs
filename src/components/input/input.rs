// components/input.rs
use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;
use mailiner_css::*;

/// Input types supported by the Input component
#[derive(PartialEq, Clone)]
pub enum InputType {
    Text,
    Email,
    Password,
    Search,
    Number,
    Url,
}

impl InputType {
    pub fn as_str(&self) -> &'static str {
        match self {
            InputType::Text => "text",
            InputType::Email => "email",
            InputType::Password => "password",
            InputType::Search => "search",
            InputType::Number => "number",
            InputType::Url => "url",
        }
    }
}

/// Input sizes
#[derive(PartialEq, Clone)]
pub enum InputSize {
    Small,
    Medium,
    Large,
}

/// Props for the Input component
#[derive(Clone, Props, PartialEq)]
pub struct InputProps {
    /// Input type
    #[props(default = InputType::Text)]
    input_type: InputType,
    
    /// Input placeholder
    #[props(into)]
    placeholder: Option<String>,
    
    /// Input value
    #[props(into)]
    value: Option<String>,
    
    /// Default value (uncontrolled)
    #[props(into)]
    default_value: Option<String>,
    
    /// Input label
    #[props(into)]
    label: Option<String>,
    
    /// Helper text below the input
    #[props(into)]
    helper_text: Option<String>,
    
    /// Error message
    #[props(into)]
    error: Option<String>,
    
    /// Icon to show at the start of input
    start_icon: Option<Element>,
    
    /// Icon to show at the end of input
    end_icon: Option<Element>,
    
    /// Input size
    size: Option<InputSize>,
    
    /// Whether the input is disabled
    disabled: Option<bool>,
    
    /// Whether the input is read-only
    read_only: Option<bool>,
    
    /// Whether the input takes full width
    full_width: Option<bool>,
    
    /// Change event handler
    on_change: Option<EventHandler<String>>,
    
    /// Focus event handler
    on_focus: Option<EventHandler<FocusEvent>>,
    
    /// Blur event handler
    on_blur: Option<EventHandler<FocusEvent>>,
    
    /// Input event handler
    on_input: Option<EventHandler<FormEvent>>,
}

/// Input component for Mailiner UI
pub fn Input(props: InputProps) -> Element {
    let mut input_value = use_signal(|| props.value.clone().unwrap_or_else(|| 
        props.default_value.clone().unwrap_or_default()
    ));
    
    let is_disabled = props.disabled.unwrap_or(false);
    let is_error = props.error.is_some();
    let input_type = props.input_type.clone();
    
    // Generate unique ID for accessibility
    let id = format!("input-{}", use_memo(|| uuid::Uuid::new_v4().to_string()));
    
    // Handle input change
    let on_input = move |event: FormEvent| {
        let value = event.value();
        input_value.set(value.clone());
        
        if let Some(handler) = &props.on_change {
            handler.call(value);
        }
        
        if let Some(handler) = &props.on_input {
            handler.call(event);
        }
    };
    
    // Determine size classes
    let size_classes = match props.size.clone().unwrap_or(InputSize::Medium) {
        InputSize::Small => class!(h_8 text_sm px_2),
        InputSize::Medium => class!(h_10 text_base px_3),
        InputSize::Large => class!(h_12 text_lg px_4),
    };
    
    // Base input classes
    let base_classes = class!(w_full bg_white border rounded focus(outline_none) focus(transition_colors));
    
    // State-specific classes
    let state_classes = if is_disabled {
        class!(border_neutral_200 bg_neutral_100 text_neutral_400 cursor_not_allowed)
    } else if is_error {
        class!(border_danger_500 text_danger_700 focus(ring_2) focus(ring_danger_200) focus(border_danger_500))
    } else {
        class!(border_neutral_300 text_neutral_800 focus(ring_2) focus(ring_primary_100) focus(border_primary_500))
    };
    
    // Width classes
    let width_classes = if props.full_width.unwrap_or(true) {
        class!(w_full)
    } else {
        class!(w_auto)
    };
    
    // Input container padding adjustments for icons
    let container_padding = match (props.start_icon.is_some(), props.end_icon.is_some()) {
        (true, true) => class!(pl_9 pr_9),
        (true, false) => class!(pl_9),
        (false, true) => class!(pr_9),
        (false, false) => class!(""),
    };
    
    rsx! {
        div { class: format!("{} {}", class!(flex flex_col), width_classes),
            // Label (if provided)
            if let Some(label_text) = &props.label {
                label {
                    class: class!(block text_sm font_medium text_neutral_700 mb_1),
                    for: "{id}",
                    "{label_text}"
                }
            }
            
            // Input container for positioning icons
            div { class: class!(relative),
                // Start icon (if provided)
                if let Some(icon) = &props.start_icon {
                    div { 
                        class: class!(absolute inset_y_0 left_0 flex items_center pl_3 pointer_events_none text_neutral_500),
                        {icon}
                    }
                }
                
                // Input element
                input {
                    id: "{id}",
                    r#type: "{input_type.as_str()}",
                    value: "{input_value}",
                    placeholder: props.placeholder.clone().unwrap_or_default(),
                    disabled: is_disabled,
                    readonly: props.read_only.unwrap_or(false),
                    class: "{base_classes} {state_classes} {size_classes} {container_padding}",
                    oninput: on_input,
                    onfocus: move |evt| {
                        if let Some(handler) = &props.on_focus {
                            handler.call(evt);
                        }
                    },
                    onblur: move |evt| {
                        if let Some(handler) = &props.on_blur {
                            handler.call(evt);
                        }
                    }
                }
                
                // End icon (if provided)
                if let Some(icon) = &props.end_icon {
                    div { 
                        class: class!(absolute inset_y_0 right_0 flex items_center pr_3 text_neutral_500),
                        {icon}
                    }
                }
            }
            
            // Helper text or error message
            if let Some(error_text) = &props.error {
                p { class: class!(mt_1 text_sm text_danger_600), "{error_text}" }
            } else if let Some(helper_text) = &props.helper_text {
                p { class: class!(mt_1 text_sm text_neutral_500), "{helper_text}" }
            }
        }
    }
}
