// components/toolbar.rs
use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;
use mailiner_css::*;

/// Props for toolbar item data
#[derive(PartialEq, Clone)]
pub struct ToolbarItemData {
    pub id: String,
    pub icon: Element,
    pub label: Option<String>,
    pub tooltip: Option<String>,
    pub disabled: Option<bool>,
    pub danger: Option<bool>,
}

/// Toolbar positions
#[derive(PartialEq, Clone)]
pub enum ToolbarPosition {
    Top,
    Bottom,
    Left,
    Right,
}

/// Toolbar sizes
#[derive(PartialEq, Clone)]
pub enum ToolbarSize {
    Small,
    Medium,
    Large,
}

/// Props for the Toolbar component
#[derive(Clone, Props, PartialEq)]
pub struct ToolbarProps {
    /// Array of toolbar item data
    items: Vec<ToolbarItemData>,
    
    /// Toolbar position
    position: Option<ToolbarPosition>,
    
    /// Toolbar size
    size: Option<ToolbarSize>,
    
    /// Whether to show labels
    show_labels: Option<bool>,
    
    /// Whether to show dividers between items
    show_dividers: Option<bool>,
    
    /// Additional CSS classes
    #[props(into)]
    class: Option<String>,
    
    /// Item click event handler
    on_item_click: Option<EventHandler<String>>,
}

/// Toolbar component for Mailiner UI
pub fn Toolbar(props: ToolbarProps) -> Element {
    let position = props.position.clone().unwrap_or(ToolbarPosition::Top);
    let size = props.size.clone().unwrap_or(ToolbarSize::Medium);
    let show_labels = props.show_labels.unwrap_or(true);
    let show_dividers = props.show_dividers.unwrap_or(true);
    
    // Base classes for the toolbar
    let base_classes = class!(bg_white border border_neutral_200 shadow_sm);
    
    // Position-specific classes
    let position_classes = match position {
        ToolbarPosition::Top => class!(flex_row),
        ToolbarPosition::Bottom => class!(flex_row),
        ToolbarPosition::Left => class!(flex_col),
        ToolbarPosition::Right => class!(flex_col),
    };
    
    // Size-specific classes
    let size_classes = match size {
        ToolbarSize::Small => match position {
            ToolbarPosition::Top | ToolbarPosition::Bottom => class!(h_10),
            ToolbarPosition::Left | ToolbarPosition::Right => class!(w_10),
        },
        ToolbarSize::Medium => match position {
            ToolbarPosition::Top | ToolbarPosition::Bottom => class!(h_12),
            ToolbarPosition::Left | ToolbarPosition::Right => class!(w_12),
        },
        ToolbarSize::Large => match position {
            ToolbarPosition::Top | ToolbarPosition::Bottom => class!(h_16),
            ToolbarPosition::Left | ToolbarPosition::Right => class!(w_16),
        },
    };
    
    // Padding and spacing classes
    let spacing_classes = match position {
        ToolbarPosition::Top | ToolbarPosition::Bottom => class!(px_2),
        ToolbarPosition::Left | ToolbarPosition::Right => class!(py_2),
    };
    
    // Additional classes from props
    let additional_classes = props.class.clone().unwrap_or_default();
    
    // Combine all classes
    let toolbar_classes = format!("{} {} {} {} {} {}",
                                class!(flex),
                                base_classes, 
                                position_classes, 
                                size_classes, 
                                spacing_classes,
                                additional_classes);
    
    let items = props.items.iter().enumerate().map(|(index, item)| {
            let item = item.clone();
                    let is_last = index == props.items.len() - 1;
                    let is_disabled = item.disabled.unwrap_or(false);
                    let is_danger = item.danger.unwrap_or(false);
                    
                    let item_render = rsx! {
                        // Toolbar item button
                        button {
                            class: format!("{} {}",
                                class!(/*group*/ relative flex items_center justify_center transition_colors rounded focus(outline_none) focus(ring_2) focus(ring_primary_500)),
                                if is_disabled {
                                    class!(cursor_not_allowed text_neutral_400)
                                } else if is_danger {
                                    class!(text_danger_600 hover(bg_danger_50))
                                } else {
                                    class!(text_neutral_700 hover(bg_neutral_100) active(bg_neutral_200))
                                }
                            ),
                            disabled: is_disabled,
                            "data-tooltip": item.tooltip.clone().unwrap_or_default(),
                            onclick: move |_| {
                                if !is_disabled {
                                    if let Some(handler) = &props.on_item_click {
                                        handler.call(item.id.clone());
                                    }
                                }
                            },
                            
                            // Button contents based on position
                            match position {
                                ToolbarPosition::Top | ToolbarPosition::Bottom => {
                                    if show_labels && item.label.is_some() {
                                        rsx! {
                                            div { 
                                                class: class!(flex flex_col items_center px_3 py_1),
                                                // Icon
                                                div { 
                                                    class: class!(shrink_0), 
                                                    {&item.icon}
                                                }
                                                
                                                // Label
                                                if let Some(label) = &item.label {
                                                    span { 
                                                        class: class!(text_xs mt_1), 
                                                        "{label}" 
                                                    }
                                                }
                                            }
                                        }
                                    } else {
                                        rsx! {
                                            div { 
                                                class: class!(flex items_center px_3 py_1),
                                                // Icon only
                                                div { 
                                                    class: class!(shrink_0), 
                                                    {&item.icon}
                                                }
                                            }
                                        }
                                    }
                                },
                                ToolbarPosition::Left | ToolbarPosition::Right => {
                                    if show_labels && item.label.is_some() {
                                        rsx! {
                                            div { 
                                                class: class!(flex flex_col items_center py_3 px_1),
                                                // Icon
                                                div { 
                                                    class: class!(shrink_0), 
                                                    {&item.icon}
                                                }
                                                
                                                // Label
                                                if let Some(label) = &item.label {
                                                    span { 
                                                        class: class!(text_xs mt_1), 
                                                        "{label}" 
                                                    }
                                                }
                                            }
                                        }
                                    } else {
                                        rsx! {
                                            div { 
                                                class: class!(flex items_center py_3 px_1),
                                                // Icon only
                                                div { 
                                                    class: class!(shrink_0), 
                                                    {&item.icon}
                                                }
                                            }
                                        }
                                    }
                                }
                            },
                            
                            // Tooltip (if provided)
                            if let Some(tooltip_text) = &item.tooltip {
                                div {
                                    class: format!("{} {}",
                                        class!(absolute hidden hover(block) bg_neutral_800 text_white text_xs rounded px_2 py_1 whitespace_nowrap z_10),
                                        match position {
                                            ToolbarPosition::Top => class!(bottom_full mb_1),
                                            ToolbarPosition::Bottom => class!(top_full mt_1),
                                            ToolbarPosition::Left => class!(right_full mr_1),
                                            ToolbarPosition::Right => class!(left_full ml_1),
                                        },
                                    ),
                                    "{tooltip_text}"
                                }
                            }
                        }
                    };

                    // Add divider if needed
                    if show_dividers && !is_last {
                        let divider_classes = match position {
                            ToolbarPosition::Top | ToolbarPosition::Bottom => 
                                class!(border_r border_neutral_200 h_6 my_auto mx_1),
                            ToolbarPosition::Left | ToolbarPosition::Right => 
                                class!(border_b border_neutral_200 w_6 mx_auto my_1),
                        };
                        
                        rsx! {
                            div { 
                                class: class!(flex items_center),
                                {item_render}
                            }
                            div { 
                                class: divider_classes 
                            }
                        }
                    } else {
                        rsx! {
                            {item_render}
                        }
                    }
                }).collect::<Vec<_>>();

    rsx! {
        div { 
            class: toolbar_classes,
            for item in items {
                {item}
            }
        }
    }
}

/// Props for button group toolbar
#[derive(Clone, Props, PartialEq)]
pub struct ButtonGroupToolbarProps {
    /// Child elements (typically buttons)
    children: Element,
    
    /// Additional CSS classes
    #[props(into)]
    class: Option<String>,
}

/// Button Group Toolbar component for Mailiner UI
/// Used for grouping related actions horizontally
pub fn ButtonGroupToolbar(props: ButtonGroupToolbarProps) -> Element {
    let additional_classes = props.class.clone().unwrap_or_default();
    
    rsx! {
        div {
            class: format!("{} {}", class!(flex items_center bg_white border border_neutral_200 rounded shadow_sm), additional_classes),
            {props.children}
        }
    }
}
