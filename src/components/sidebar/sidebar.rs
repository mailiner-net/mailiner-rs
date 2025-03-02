// components/sidebar.rs
use dioxus::prelude::*;
use dioxus_free_icons::{icons::ld_icons::{LdChevronDown, LdChevronRight}, Icon};
use dioxus_tailwindcss::prelude::*;
use mailiner_css::*;

/// Props for a sidebar item 
#[derive(PartialEq, Clone)]
pub struct SidebarItemData {
    pub id: String,
    pub label: String,
    pub icon: Option<Element>,
    pub badge: Option<String>,
    pub children: Option<Vec<SidebarItemData>>,
}

/// Props for the Sidebar component
#[derive(Clone, Props, PartialEq)]
pub struct SidebarProps {
    /// Array of sidebar item data
    items: Vec<SidebarItemData>,
    
    /// Currently selected item ID
    selected: Option<String>,
    
    /// Whether the sidebar is collapsed
    collapsed: Option<bool>,
    
    /// Optional header content
    header: Option<Element>,
    
    /// Optional footer content
    footer: Option<Element>,
    
    /// Item selection event handler
    on_select: Option<EventHandler<String>>,
    
    /// Collapse toggle event handler
    on_toggle_collapse: Option<EventHandler<bool>>,
}

/// Sidebar component for Mailiner UI
pub fn Sidebar(props: SidebarProps) -> Element {
    let is_collapsed = props.collapsed.unwrap_or(false);
    
    // Width class based on collapsed state
    let width_class = if is_collapsed {
        class!(w_16)
    } else {
        class!(w_64)
    };
    
    rsx! {
        aside {
            class: format!("{} {}",class!(bg_neutral_50 border_r border_neutral_200 h_full flex flex_col transition_all duration_200), width_class),
            
            // Sidebar header section
            if let Some(header) = &props.header {
                div { 
                    class: class!(p_4 border_b border_neutral_200),
                    {header}
                }
            }
            
            // Toggle collapse button
            if let Some(handler) = props.on_toggle_collapse {
                button {
                    class: class!(absolute right_0 top_4 translate_x_1_half bg_white rounded_full p_1 shadow_sm border border_neutral_200 hover(bg_neutral_50)),
                    onclick: move |_| handler.call(!is_collapsed),
                    
                    if is_collapsed {
                        Icon {
                            icon: LdChevronRight,
                            class: class!(h_4 w_4 text_neutral_600),
                        }
                    } else {
                        Icon {
                            icon: LdChevronDown,
                            class: class!(h_4 w_4 text_neutral_600),
                        }
                    }
                }
            }
            
            // Sidebar content - navigation items
            nav { 
                class: class!(flex_1 overflow_y_auto py_2),
                ul { 
                    class: class!(space_y_1),
                    {props.items.iter().map(|item| {
                        let is_selected = props.selected.as_ref()
                            .map(|sel| sel == &item.id)
                            .unwrap_or(false);
                            
                        rsx! {
                            SidebarItem {
                                item: item.clone(),
                                selected: is_selected,
                                collapsed: is_collapsed,
                                on_select: move |id| {
                                    if let Some(handler) = &props.on_select {
                                        handler.call(id);
                                    }
                                }
                            }
                        }
                    })}
                }
            }
            
            // Sidebar footer section
            if let Some(footer) = &props.footer {
                div { 
                    class: class!(p_4 border_t border_neutral_200 mt_auto),
                    {footer}
                }
            }
        }
    }
}

/// Props for a single sidebar item
#[derive(Clone, Props, PartialEq)]
pub struct SidebarItemProps {
    /// The item data
    item: SidebarItemData,
    
    /// Whether this item is selected
    selected: bool,
    
    /// Whether the sidebar is collapsed
    collapsed: bool,
    
    /// Item selection event handler
    on_select: Option<EventHandler<String>>,
}

/// Individual sidebar item component
pub fn SidebarItem(props: SidebarItemProps) -> Element {
    let item = props.item.clone();
    let is_selected = props.selected;
    let is_collapsed = props.collapsed;
    let has_children = item.children.as_ref().map_or(false, |c| !c.is_empty());
    
    // State for whether a submenu is expanded
    let mut is_expanded = use_signal(|| is_selected);
    
    // Base classes for the item
    let base_classes = class!(flex items_center rounded px_3 py_2 transition_colors);
    
    // Additional classes based on state
    let state_classes = if is_selected {
        class!(bg_primary_50 text_primary_600)
    } else {
        class!(text_neutral_700 hover(bg_neutral_100))
    };
    
    // Justify content based on collapsed state
    let justify_classes = if is_collapsed {
        class!(justify_center)
    } else {
        class!(justify_between)
    };
    
    // Combine all classes
    let item_classes = format!("{} {} {}", base_classes, state_classes, justify_classes);
    
    // Handle item click
    let item_clone = item.clone();
    let on_click = move |_| {
        let item = item_clone.clone();
        if has_children {
            // Toggle submenu expansion
            is_expanded.set(!is_expanded());
        } else if let Some(handler) = &props.on_select {
            // Select this item
            handler.call(item.id);
        }
    };
    
    rsx! {
        li {
            // Item container
            div {
                class: format!("{} {}", item_classes, class!(cursor_pointer)),
                onclick: on_click,
                
                // Left side content (icon + label)
                div { 
                    class: class!(flex items_center),
                    
                    // Icon (if provided)
                    if let Some(icon) = &item.icon {
                        div { 
                            class: class!(shrink_0), 
                            {icon}
                        }
                    }
                    
                    // Label (hidden when collapsed unless no icon)
                    if !is_collapsed || item.icon.is_none() {
                        span { 
                            class: if item.icon.is_some() { class!(ml_2) } else { class!("") },
                            "{item.label}" 
                        }
                    }
                }
                
                // Right side content (badge + chevron) - hidden when collapsed
                if !is_collapsed {
                    div { 
                        class: class!(flex items_center),
                        
                        // Badge (if provided)
                        if let Some(badge_text) = &item.badge {
                            span { 
                                class: class!(ml_2 px_2 py__half text_xs rounded_full bg_primary_100 text_primary_600),
                                "{badge_text}" 
                            }
                        }
                        
                        // Chevron for submenu (if children exist)
                        if has_children {
                            Icon {
                                icon: LdChevronRight,
                                class: class!(h_4 w_4 ml_1 transition_transform),
                                style: if is_expanded() { "transform: rotate(90deg)" } else { "" },
                            }
                        }
                    }
                }
            }
            
            // Submenu items (if expanded and not collapsed)
            if has_children && is_expanded() && !is_collapsed {
                if let Some(children) = item.children {
                    ul { 
                        class: class!(ml_4 mt_1 space_y_1),
                        {children.iter().map(|child| {
                            let child = child.clone();
                            let is_child_selected = props.selected && item.id == child.id;
                            let state_classes = if is_child_selected {
                                class!(bg_primary_50 text_primary_600)
                            } else {
                                class!(text_neutral_700 hover(bg_neutral_100))
                            };
                            
                            rsx! {
                                li {
                                    div {
                                        class: format!("{} {}", class!(flex items_center rounded px_3 py_2 transition_colors cursor_pointer), state_classes),  
                                        onclick: move |_| {
                                            if let Some(handler) = &props.on_select {
                                                handler.call(child.id.clone());
                                            }
                                        },
                                        
                                        // Child icon (if provided)
                                        if let Some(icon) = &child.icon {
                                            div { 
                                                class: class!(shrink_0), 
                                                {icon}
                                            }
                                        }
                                        
                                        // Child label
                                        span { 
                                            class: if child.icon.is_some() { class!(ml_2) } else { class!("") },
                                            "{child.label}" 
                                        }
                                        
                                        // Child badge (if provided)
                                        if let Some(badge_text) = &child.badge {
                                            span { 
                                                class: class!(ml_auto px_2 py__half text_xs rounded_full bg_primary_100 text_primary_600),
                                                "{badge_text}" 
                                            }
                                        }
                                    }
                                }
                            }
                        })}
                    }
                }
            }
        }
    }
}
