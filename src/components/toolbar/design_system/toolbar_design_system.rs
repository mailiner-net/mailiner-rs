use dioxus::prelude::*;
use dioxus_free_icons::icons::ld_icons::{
    LdBold, LdForward, LdItalic, LdPlus, LdReply, LdTrash2, LdUnderline,
};
use dioxus_free_icons::Icon;
use dioxus_tailwindcss::prelude::*;
use mailiner_css::*;

use crate::components::{
    ButtonGroupToolbar, Toolbar, ToolbarItemData, ToolbarPosition, ToolbarSize,
};

use super::ToolbarItemStates;
use super::ToolbarProps;

/// Main Design System Component for Toolbars
pub fn ToolbarDesignSystem() -> Element {
    rsx! {
        div { class: class!(min_h_screen bg_neutral_50),
            // Page header
            header {
                class: class!(bg_white border_b border_neutral_200 py_4 px_6 mb_6),
                h1 {
                    class: class!(text_2xl font_semibold text_neutral_800),
                    "Mailiner Toolbar Design System"
                }
            }

            // Main content container
            div { class: class!(container mx_auto px_4 pb_12),
                // Toolbar Section
                section { class: class!(mb_12),
                    h2 {
                        class: class!(text_xl font_medium text_neutral_800 mb_4 pb_2 border_b border_neutral_200),
                        "Toolbars"
                    }

                    // Props documentation
                    ToolbarProps {}

                    // Demo Toolbars
                    ToolbarExamples {}

                    // Documentation
                    div { class: class!(grid grid_cols_1 md(grid_cols_2) gap_8 mt_8),
                        // Position variants
                        ToolbarPositions {}

                        // Size variants
                        ToolbarSizes {}

                        // Item states
                        ToolbarItemStates {}
                    }
                }
            }
        }
    }
}

/// Component to demonstrate different toolbar examples
fn ToolbarExamples() -> Element {
    rsx! {
        div { class: class!(bg_white border border_neutral_200 p_6 rounded shadow_sm mb_8),
            // Top Toolbar with Labels
            div {
                class: class!(mb_8),
                h3 {
                    class: class!(text_lg font_medium text_neutral_700 mb_3),
                    "Top Toolbar (with labels)"
                }
                Toolbar {
                    position: ToolbarPosition::Top,
                    show_labels: true,
                    items: vec![
                        ToolbarItemData {
                            id: "new".to_string(),
                            label: Some("New".to_string()),
                            icon: rsx! {
                                Icon {
                                    icon: LdPlus,
                                    class: class!(w_5 h_5),
                                }
                            },
                            tooltip: Some("Create new email".to_string()),
                            disabled: None,
                            danger: None
                        },
                        ToolbarItemData {
                            id: "reply".to_string(),
                            label: Some("Reply".to_string()),
                            icon: rsx! {
                                Icon {
                                    icon: LdReply,
                                    class: class!(w_5 h_5),
                                }
                            },
                            tooltip: Some("Reply to sender".to_string()),
                            disabled: None,
                            danger: None
                        },
                        ToolbarItemData {
                            id: "forward".to_string(),
                            label: Some("Forward".to_string()),
                            icon: rsx! {
                                Icon {
                                    icon: LdForward,
                                    class: class!(w_5 h_5),
                                }
                            },
                            tooltip: Some("Forward email".to_string()),
                            disabled: None,
                            danger: None
                        },
                        ToolbarItemData {
                            id: "delete".to_string(),
                            label: Some("Delete".to_string()),
                            icon: rsx! {
                                Icon {
                                    icon: LdTrash2,
                                    class: class!(w_5 h_5),
                                }
                            },
                            tooltip: Some("Delete email".to_string()),
                            disabled: None,
                            danger: Some(true)
                        }
                    ]
                }
            }

            // Left Toolbar Demo
            div {
                class: class!(mb_8 flex),
                div {
                    class: class!(w_32),
                    h3 {
                        class: class!(text_lg font_medium text_neutral_700 mb_3),
                        "Left Toolbar"
                    }
                    Toolbar {
                        position: ToolbarPosition::Left,
                        size: ToolbarSize::Medium,
                        show_labels: false,
                        items: vec![
                            ToolbarItemData {
                                id: "format_bold".to_string(),
                                label: None,
                                icon: rsx! {
                                    Icon {
                                        icon: LdBold,
                                        class: class!(w_5 h_5),
                                    }
                                },
                                tooltip: Some("Bold".to_string()),
                                disabled: None,
                                danger: None
                            },
                            ToolbarItemData {
                                id: "format_italic".to_string(),
                                label: None,
                                icon: rsx! {
                                    Icon {
                                        icon: LdItalic,
                                        class: class!(w_5 h_5),
                                    }
                                },
                                tooltip: Some("Italic".to_string()),
                                disabled: None,
                                danger: None
                            },
                            ToolbarItemData {
                                id: "format_underline".to_string(),
                                label: None,
                                icon: rsx! {
                                    Icon {
                                        icon: LdUnderline,
                                        class: class!(w_5 h_5),
                                    }
                                },
                                tooltip: Some("Underline".to_string()),
                                disabled: Some(true),
                                danger: None
                            }
                        ]
                    }
                }

                // Description
                div {
                    class: class!(ml_8),
                    p {
                        class: class!(text_neutral_700 mb_2),
                        "Toolbars provide quick access to common actions."
                    }
                    ul {
                        class: class!(list_disc pl_5 text_sm text_neutral_600 space_y_1),
                        li { "Support for top/bottom/left/right positioning" }
                        li { "Optional labels for better clarity" }
                        li { "Tooltips for additional context" }
                        li { "Support for disabled and danger states" }
                    }
                }
            }

            // Button Group Toolbar
            div {
                h3 {
                    class: class!(text_lg font_medium text_neutral_700 mb_3),
                    "Button Group Toolbar"
                }
                ButtonGroupToolbar {
                    button {
                        class: class!(px_3 py_2 text_sm font_medium rounded_l border border_neutral_300 bg_white text_neutral_700 hover(bg_neutral_50)),
                        "Day"
                    }
                    button {
                        class: class!(px_3 py_2 text_sm font_medium border_t border_b border_neutral_300 bg_primary_50 text_primary_700),
                        "Week"
                    }
                    button {
                        class: class!(px_3 py_2 text_sm font_medium rounded_r border border_neutral_300 bg_white text_neutral_700 hover(bg_neutral_50)),
                        "Month"
                    }
                }
            }
        }
    }
}

/// Component to showcase toolbar positions
fn ToolbarPositions() -> Element {
    rsx! {
        div {
            class: class!(bg_white p_4 rounded border border_neutral_200 shadow_sm),
            h3 {
                class: class!(text_lg font_medium mb_3),
                "Toolbar Positions"
            }

            div {
                class: class!(grid grid_cols_2 gap_3),
                div {
                    class: class!(border border_neutral_200 rounded p_3),
                    h4 { class: class!(font_medium mb_1), "Top" }
                    p { class: class!(text_sm text_neutral_600), "Horizontal toolbar at the top" }
                    div {
                        class: class!(h_6 w_full bg_neutral_100 mt_2 flex items_center justify_center rounded text_xs text_neutral_500),
                        "ToolbarPosition::Top"
                    }
                }

                div {
                    class: class!(border border_neutral_200 rounded p_3),
                    h4 { class: class!(font_medium mb_1), "Bottom" }
                    p { class: class!(text_sm text_neutral_600), "Horizontal toolbar at the bottom" }
                    div {
                        class: class!(h_6 w_full bg_neutral_100 mt_2 flex items_center justify_center rounded text_xs text_neutral_500),
                        "ToolbarPosition::Bottom"
                    }
                }

                div {
                    class: class!(border border_neutral_200 rounded p_3),
                    h4 { class: class!(font_medium mb_1), "Left" }
                    p { class: class!(text_sm text_neutral_600), "Vertical toolbar on the left" }
                    div {
                        class: class!(h_6 w_full bg_neutral_100 mt_2 flex items_center justify_center rounded text_xs text_neutral_500),
                        "ToolbarPosition::Left"
                    }
                }

                div {
                    class: class!(border border_neutral_200 rounded p_3),
                    h4 { class: class!(font_medium mb_1), "Right" }
                    p { class: class!(text_sm text_neutral_600), "Vertical toolbar on the right" }
                    div {
                        class: class!(h_6 w_full bg_neutral_100 mt_2 flex items_center justify_center rounded text_xs text_neutral_500),
                        "ToolbarPosition::Right"
                    }
                }
            }
        }
    }
}

/// Component to showcase toolbar sizes
fn ToolbarSizes() -> Element {
    rsx! {
        div {
            class: class!(bg_white p_4 rounded border border_neutral_200 shadow_sm),
            h3 {
                class: class!(text_lg font_medium mb_3),
                "Toolbar Sizes"
            }

            div {
                class: class!(space_y_3),
                div {
                    class: class!(border border_neutral_200 rounded p_3),
                    div {
                        class: class!(flex justify_between items_center),
                        h4 { class: class!(font_medium), "Small" }
                        span { class: class!(text_xs text_neutral_500 px_2 py_1 bg_neutral_100 rounded), "ToolbarSize::Small" }
                    }
                    ul {
                        class: class!(mt_2 text_sm space_y_1),
                        li { class: class!(flex justify_between),
                            span { "Height (Top/Bottom)" }
                            span { class: class!(text_neutral_500), "h-10 (2.5rem)" }
                        }
                        li { class: class!(flex justify_between),
                            span { "Width (Left/Right)" }
                            span { class: class!(text_neutral_500), "w-10 (2.5rem)" }
                        }
                    }
                }

                div {
                    class: class!(border border_primary_200 bg_primary_50 rounded p_3),
                    div {
                        class: class!(flex justify_between items_center),
                        h4 { class: class!(font_medium text_primary_700), "Medium (Default)" }
                        span { class: class!(text_xs text_primary_600 px_2 py_1 bg_primary_100 rounded), "ToolbarSize::Medium" }
                    }
                    ul {
                        class: class!(mt_2 text_sm space_y_1),
                        li { class: class!(flex justify_between),
                            span { "Height (Top/Bottom)" }
                            span { class: class!(text_primary_600), "h-12 (3rem)" }
                        }
                        li { class: class!(flex justify_between),
                            span { "Width (Left/Right)" }
                            span { class: class!(text_primary_600), "w-12 (3rem)" }
                        }
                    }
                }

                div {
                    class: class!(border border_neutral_200 rounded p_3),
                    div {
                        class: class!(flex justify_between items_center),
                        h4 { class: class!(font_medium), "Large" }
                        span { class: class!(text_xs text_neutral_500 px_2 py_1 bg_neutral_100 rounded), "ToolbarSize::Large" }
                    }
                    ul {
                        class: class!(mt_2 text_sm space_y_1),
                        li { class: class!(flex justify_between),
                            span { "Height (Top/Bottom)" }
                            span { class: class!(text_neutral_500), "h-14 (3.5rem)" }
                        }
                        li { class: class!(flex justify_between),
                            span { "Width (Left/Right)" }
                            span { class: class!(text_neutral_500), "w-14 (3.5rem)" }
                        }
                    }
                }
            }
        }
    }
}
