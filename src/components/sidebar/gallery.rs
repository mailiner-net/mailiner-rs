// pages/sidebar_design_system.rs
use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;
use dioxus_free_icons::Icon;
use dioxus_free_icons::icons::ld_icons::{LdMail, LdChevronRight, LdChevronsRight, LdFolder, LdSquarePen, LdArchive};
use mailiner_css::*;

use crate::components::{
    Sidebar, SidebarItemData,
};

/// Design System Component for Sidebars
pub fn Gallery() -> Element {
    // Sidebar collapsed state
    let mut sidebar_collapsed = use_signal(|| false);
    
    rsx! {
        div { 
            class: class!(min_h_screen bg_neutral_50),
            // Page header
            header { 
                class: class!(bg_white border_b border_neutral_200 py_4 px_6 mb_6),
                h1 { 
                    class: class!(text_2xl font_semibold text_neutral_800), 
                    "Mailiner Sidebar Design System" 
                }
            }
            
            // Main content container
            div { 
                class: class!(container mx_auto px_4 pb_12),
                // Sidebar Section
                section { class: class!(mb_12),
                    h2 { 
                        class: class!(text_xl font_medium text_neutral_800 mb_4 pb_2 border_b border_neutral_200), 
                        "Sidebar" 
                    }
                    
                    // Sidebar Demo and Features
                    div { 
                        class: class!(grid grid_cols_1 lg(grid_cols_2) gap_8 mb_8),
                        // Interactive Sidebar Demo
                        div { 
                            class: class!(border rounded bg_white shadow_sm flex h_96 relative),
                            // Sidebar component with sample items
                            Sidebar {
                                collapsed: sidebar_collapsed(),
                                selected: "inbox",
                                on_toggle_collapse: move |state| sidebar_collapsed.set(state),
                                items: vec![
                                    SidebarItemData {
                                        id: "inbox".to_string(),
                                        label: "Inbox".to_string(),
                                        icon: Some(rsx! {
                                            Icon {
                                                icon: LdMail,
                                                class: class!(w_5 h_5),
                                            }
                                        }),
                                        badge: Some("42".to_string()),
                                        children: None
                                    },
                                    SidebarItemData {
                                        id: "sent".to_string(),
                                        label: "Sent".to_string(),
                                        icon: Some(rsx! {
                                            Icon {
                                                icon: LdChevronsRight,
                                                class: class!(w_5 h_5),
                                            }
                                        }),
                                        badge: None,
                                        children: None
                                    },
                                    SidebarItemData {
                                        id: "folders".to_string(),
                                        label: "Folders".to_string(),
                                        icon: Some(rsx! {
                                            Icon {
                                                icon: LdFolder,
                                                class: class!(w_5 h_5),
                                            }
                                        }),
                                        badge: None,
                                        children: Some(vec![
                                            SidebarItemData {
                                                id: "work".to_string(),
                                                label: "Work".to_string(),
                                                icon: None,
                                                badge: None,
                                                children: None
                                            },
                                            SidebarItemData {
                                                id: "personal".to_string(),
                                                label: "Personal".to_string(),
                                                icon: None,
                                                badge: None,
                                                children: None
                                            }
                                        ])
                                    },
                                    SidebarItemData {
                                        id: "drafts".to_string(),
                                        label: "Drafts".to_string(),
                                        icon: Some(rsx! {
                                            Icon {
                                                icon: LdSquarePen,
                                                class: class!(w_5 h_5),
                                            }
                                        }),
                                        badge: Some("3".to_string()),
                                        children: None
                                    },
                                    SidebarItemData {
                                        id: "archived".to_string(),
                                        label: "Archived".to_string(),
                                        icon: Some(rsx! {
                                            Icon {
                                                icon: LdArchive,
                                                class: class!(w_5 h_5),
                                            }
                                        }),
                                        badge: None,
                                        children: None
                                    }
                                ],
                                header: rsx! {
                                    div {
                                        class: class!(text_xl font_semibold text_primary_600),
                                        if sidebar_collapsed() {
                                            "M"
                                        } else {
                                            "Mailiner"
                                        }
                                    }
                                },
                                footer: rsx!{
                                    div { 
                                        class: class!(flex items_center),
                                        div { 
                                            class: class!(w_8 h_8 bg_primary_100 rounded_full flex items_center justify_center text_primary_600 font_medium mr_2),
                                            "JS"
                                        }
                                        if !sidebar_collapsed() {
                                            div {
                                                div { 
                                                    class: class!(text_sm font_medium), 
                                                    "John Smith" 
                                                }
                                                div { 
                                                    class: class!(text_xs text_neutral_500), 
                                                    "john@example.com" 
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            
                            div { 
                                class: class!(flex_1 p_4 ml_4),
                                h4 { 
                                    class: class!(text_lg font_medium mb_2), 
                                    "Interactive Sidebar" 
                                }
                                p { 
                                    class: class!(text_neutral_600), 
                                    "Click the toggle button to collapse/expand." 
                                }
                                p { 
                                    class: class!(text_neutral_600 mt_2), 
                                    "Click on 'Folders' to see nested items." 
                                }
                            }
                        }
                        
                        // Sidebar features and props
                        div { 
                            class: class!(space_y_6),
                            // States and modes
                            div {
                                h3 { 
                                    class: class!(text_lg font_medium text_neutral_700 mb_3), 
                                    "Sidebar States" 
                                }
                                
                                div { 
                                    class: class!(grid grid_cols_2 gap_3),
                                    div { 
                                        class: class!(bg_white p_3 rounded border border_neutral_200),
                                        h4 { class: class!(font_medium text_sm mb_1), "Expanded" }
                                        p { class: class!(text_xs text_neutral_600), "Full width view with text labels" }
                                        p { class: class!(text_xs text_neutral_500 mt_1), "Default width: 16rem (w-64)" }
                                    }
                                    
                                    div { 
                                        class: class!(bg_white p_3 rounded border border_neutral_200),
                                        h4 { class: class!(font_medium text_sm mb_1), "Collapsed" }
                                        p { class: class!(text_xs text_neutral_600), "Icon-only narrow view" }
                                        p { class: class!(text_xs text_neutral_500 mt_1), "Width: 4rem (w-16)" }
                                    }
                                    
                                    div { 
                                        class: class!(bg_white p_3 rounded border border_primary_200 bg_primary_50),
                                        h4 { class: class!(font_medium text_sm mb_1 text_primary_700), "Selected Item" }
                                        p { class: class!(text_xs text_primary_600), "Highlighted with accent color" }
                                        p { class: class!(text_xs text_primary_600 mt_1), "Background: bg-primary-50" }
                                    }
                                    
                                    div { 
                                        class: class!(bg_white p_3 rounded border border_neutral_200),
                                        h4 { class: class!(font_medium text_sm mb_1), "Hover State" }
                                        p { class: class!(text_xs text_neutral_600), "Light background on hover" }
                                        p { class: class!(text_xs text_neutral_500 mt_1), "Background: bg-neutral-100" }
                                    }
                                }
                            }
                            
                            // Sidebar features
                            div { 
                                class: class!(bg_white border border_neutral_200 p_4 rounded shadow_sm),
                                h3 { 
                                    class: class!(text_lg font_medium mb_3), 
                                    "Sidebar Features" 
                                }
                                
                                ul { 
                                    class: class!(space_y_3 list_disc pl_5 text_neutral_700),
                                    li { "Collapsible with toggle button" }
                                    li { "Selected item state with highlight" }
                                    li { "Support for nested items (collapsible sections)" }
                                    li { "Optional badges for counters or status indicators" }
                                    li { "Optional icons for visual recognition" }
                                    li { "Custom header and footer sections" }
                                    li { "Compact collapsed state for space efficiency" }
                                    li { "Consistent styling with the rest of the UI" }
                                }
                            }
                            
                            // Item Types
                            div { 
                                class: class!(bg_white border border_neutral_200 p_4 rounded shadow_sm),
                                h3 { 
                                    class: class!(text_lg font_medium mb_3), 
                                    "Sidebar Item Types" 
                                }
                                
                                div { 
                                    class: class!(space_y_3),
                                    // Standard Item
                                    div { 
                                        class: class!(border border_neutral_200 rounded_md p_2),
                                        p { class: class!(text_sm font_medium mb_1), "Standard Item" }
                                        div { 
                                            class: class!(flex items_center rounded p_2 text_neutral_700 bg_neutral_50),
                                            div { 
                                                class: class!(w_5 h_5 text_neutral_500 mr_2),
                                                Icon {
                                                    icon: LdMail,
                                                    class: class!(w_5 h_5),
                                                }
                                            }
                                            span { "Inbox" }
                                        }
                                    }
                                    
                                    // Item with Badge
                                    div { 
                                        class: class!(border border_neutral_200 rounded_md p_2),
                                        p { class: class!(text_sm font_medium mb_1), "Item with Badge" }
                                        div { 
                                            class: class!(flex items_center justify_between rounded p_2 text_neutral_700 bg_neutral_50),
                                            div { 
                                                class: class!(flex items_center),
                                                div { 
                                                    class: class!(w_5 h_5 text_neutral_500 mr_2),
                                                    Icon {
                                                        icon: LdMail,
                                                        class: class!(w_5 h_5),
                                                    }
                                                }
                                                span { "Inbox" }
                                            }
                                            span { 
                                                class: class!(px_2 py__half text_xs rounded_full bg_primary_100 text_primary_600),
                                                "42" 
                                            }
                                        }
                                    }
                                    
                                    // Parent Item with Children
                                    div { 
                                        class: class!(border border_neutral_200 rounded_md p_2),
                                        p { class: class!(text_sm font_medium mb_1), "Parent Item with Children" }
                                        div { 
                                            class: class!(flex items_center justify_between rounded p_2 text_neutral_700 bg_neutral_50),
                                            div { 
                                                class: class!(flex items_center),
                                                div { 
                                                    class: class!(w_5 h_5 text_neutral_500 mr_2),
                                                    Icon {
                                                        icon: LdFolder,
                                                        class: class!(w_5 h_5),
                                                    }
                                                }
                                                span { "Folders" }
                                            }
                                            div { 
                                                class: class!(w_4 h_4 text_neutral_500),
                                                Icon {
                                                    icon: LdChevronRight,
                                                    class: class!(w_5 h_5),
                                                }
                                            }
                                        }
                                        
                                        // Child items
                                        div { 
                                            class: class!(ml_4 mt_1 space_y_1),
                                            div { 
                                                class: class!(flex items_center rounded p_2 text_neutral_700 bg_neutral_50),
                                                span { "Work" }
                                            }
                                            div { 
                                                class: class!(flex items_center rounded p_2 text_neutral_700 bg_neutral_50),
                                                span { "Personal" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    
                    // Props Documentation
                    div { 
                        class: class!(bg_white border border_neutral_200 p_4 rounded shadow_sm),
                        h3 { 
                            class: class!(text_lg font_medium mb_3), 
                            "Sidebar Props" 
                        }
                        
                        div { 
                            class: class!(overflow_x_auto),
                            table { 
                                class: class!(min_w_full divide_y divide_neutral_200),
                                thead { 
                                    class: class!(bg_neutral_50),
                                    tr {
                                        th { 
                                            class: class!(px_3 py_2 text_left text_sm font_medium text_neutral_700), 
                                            "Prop" 
                                        }
                                        th { 
                                            class: class!(px_3 py_2 text_left text_sm font_medium text_neutral_700), 
                                            "Type" 
                                        }
                                        th { 
                                            class: class!(px_3 py_2 text_left text_sm font_medium text_neutral_700), 
                                            "Default" 
                                        }
                                        th { 
                                            class: class!(px_3 py_2 text_left text_sm font_medium text_neutral_700), 
                                            "Description" 
                                        }
                                    }
                                }
                                tbody { 
                                    class: class!(divide_y divide_neutral_200 bg_white),
                                    tr {
                                        td { 
                                            class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_800 font_medium), 
                                            "items" 
                                        }
                                        td { 
                                            class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                            "Vec<SidebarItemData>" 
                                        }
                                        td { 
                                            class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                            "Required" 
                                        }
                                        td { 
                                            class: class!(px_3 py_2 text_sm text_neutral_600), 
                                            "Array of sidebar item data to display" 
                                        }
                                    }
                                    tr {
                                        td { 
                                            class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_800 font_medium), 
                                            "selected" 
                                        }
                                        td { 
                                            class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                            "Option<String>" 
                                        }
                                        td { 
                                            class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                            "None" 
                                        }
                                        td { 
                                            class: class!(px_3 py_2 text_sm text_neutral_600), 
                                            "Currently selected item ID" 
                                        }
                                    }
                                    tr {
                                        td { 
                                            class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_800 font_medium), 
                                            "collapsed" 
                                        }
                                        td { 
                                            class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                            "Option<bool>" 
                                        }
                                        td { 
                                            class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                            "false" 
                                        }
                                        td { 
                                            class: class!(px_3 py_2 text_sm text_neutral_600), 
                                            "Whether the sidebar is in collapsed state" 
                                        }
                                    }
                                    tr {
                                        td { 
                                            class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_800 font_medium), 
                                            "header" 
                                        }
                                        td { 
                                            class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                            "Option<Element>" 
                                        }
                                        td { 
                                            class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                            "None" 
                                        }
                                        td { 
                                            class: class!(px_3 py_2 text_sm text_neutral_600), 
                                            "Optional header content to display at the top" 
                                        }
                                    }
                                    tr {
                                        td { 
                                            class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_800 font_medium), 
                                            "footer" 
                                        }
                                        td { 
                                            class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                            "Option<Element>" 
                                        }
                                        td { 
                                            class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                            "None" 
                                        }
                                        td { 
                                            class: class!(px_3 py_2 text_sm text_neutral_600), 
                                            "Optional footer content to display at the bottom" 
                                        }
                                    }
                                    tr {
                                        td { 
                                            class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_800 font_medium), 
                                            "on_select" 
                                        }
                                        td { 
                                            class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                            "Option<EventHandler<String>>" 
                                        }
                                        td { 
                                            class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                            "None" 
                                        }
                                        td { 
                                            class: class!(px_3 py_2 text_sm text_neutral_600), 
                                            "Event handler when an item is selected" 
                                        }
                                    }
                                    tr {
                                        td { 
                                            class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_800 font_medium), 
                                            "on_toggle_collapse" 
                                        }
                                        td { 
                                            class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                            "Option<EventHandler<bool>>" 
                                        }
                                        td { 
                                            class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                            "None" 
                                        }
                                        td { 
                                            class: class!(px_3 py_2 text_sm text_neutral_600), 
                                            "Event handler when collapse toggle is clicked" 
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
