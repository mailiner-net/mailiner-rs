// pages/dialog_design_system.rs
use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;
use mailiner_css::*;
use crate::components::{
    Button, ButtonVariant, Dialog, DialogSize, DialogVariant, ConfirmDialog,
    Input,
};

/// Design System Component for Dialogs
pub fn Gallery() -> Element {
    // Dialog open state
    let mut show_dialog = use_signal(|| false);
    let mut show_confirm_dialog = use_signal(|| false);
    
    rsx! {
        div { class: class!(min_h_screen bg_neutral_50),
            // Page header
            header { 
                class: class!(bg_white border_b border_neutral_200 py_4 px_6 mb_6),
                h1 { 
                    class: class!(text_2xl font_semibold text_neutral_800), 
                    "Mailiner Dialog Design System" 
                }
            }
            
            // Main content container
            div { class: class!(container mx_auto px_4 pb_12),
                // Dialog Section
                section { class: class!(mb_12),
                    h2 { 
                        class: class!(text_xl font_medium text_neutral_800 mb_4 pb_2 border_b border_neutral_200), 
                        "Dialogs" 
                    }
                    
                    div { 
                        class: class!(flex flex_wrap gap_4 mb_8),
                        Button {
                            text: "Open Dialog",
                            on_click: move |_| show_dialog.set(true)
                        }
                        
                        Button {
                            text: "Confirm Dialog",
                            variant: ButtonVariant::Danger,
                            on_click: move |_| show_confirm_dialog.set(true)
                        }
                    }
                    
                    // Standard Dialog
                    Dialog {
                        open: show_dialog(),
                        title: "Example Dialog",
                        size: DialogSize::Medium,
                        on_close: move |_| show_dialog.set(false),
                        footer: rsx! {
                            div { 
                                class: class!(flex justify_end space_x_3),
                                Button {
                                    text: "Cancel",
                                    variant: ButtonVariant::Secondary,
                                    on_click: move |_| show_dialog.set(false)
                                }
                                Button {
                                    text: "Save",
                                    on_click: move |_| show_dialog.set(false)
                                }
                            }
                        },
                        
                        div { 
                            class: class!(space_y_4),
                            p { 
                                class: class!(text_neutral_700), 
                                "This is an example dialog with a title, content area, and footer with actions."
                            }
                            
                            div {
                                class: class!(border border_neutral_200 rounded p_4 mt_4),
                                label { 
                                    class: class!(block text_sm font_medium text_neutral_700 mb_1),
                                    "Sample Form Field"
                                }
                                Input {
                                    placeholder: "Enter data..."
                                }
                            }
                        }
                    }
                    
                    // Confirmation Dialog
                    ConfirmDialog {
                        open: show_confirm_dialog(),
                        title: "Delete Item?",
                        message: "Are you sure you want to delete this item? This action cannot be undone.",
                        variant: DialogVariant::Danger,
                        confirm_text: "Delete",
                        on_confirm: move |_| show_confirm_dialog.set(false),
                        on_cancel: move |_| show_confirm_dialog.set(false)
                    }
                    
                    // Dialog Variants & Sizes
                    div { class: class!(grid grid_cols_1 md(grid_cols_2) gap_8 mt_8),
                        // Dialog Variants
                        div {
                            h3 { 
                                class: class!(text_lg font_medium text_neutral_700 mb_3), 
                                "Dialog Variants" 
                            }
                            div { 
                                class: class!(grid grid_cols_1 gap_4),
                                div { 
                                    class: class!(bg_white p_4 rounded border border_neutral_200),
                                    div { 
                                        class: class!(flex items_center mb_2),
                                        div { 
                                            class: class!(w_6 h_6 mr_2 text_primary_500),
                                            svg {
                                                xmlns: "http://www.w3.org/2000/svg",
                                                fill: "none",
                                                view_box: "0 0 24 24",
                                                stroke: "currentColor",
                                                
                                                path {
                                                    stroke_linecap: "round",
                                                    stroke_linejoin: "round",
                                                    stroke_width: "2",
                                                    d: "M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                                                }
                                            }
                                        }
                                        h4 { 
                                            class: class!(font_medium), 
                                            "Info Dialog" 
                                        }
                                    }
                                    p { 
                                        class: class!(text_sm text_neutral_700), 
                                        "Use for general information and notifications." 
                                    }
                                }
                                
                                div { 
                                    class: class!(bg_white p_4 rounded border border_neutral_200),
                                    div { 
                                        class: class!(flex items_center mb_2),
                                        div { 
                                            class: class!(w_6 h_6 mr_2 text_warning_500),
                                            svg {
                                                xmlns: "http://www.w3.org/2000/svg",
                                                fill: "none",
                                                view_box: "0 0 24 24",
                                                stroke: "currentColor",
                                                
                                                path {
                                                    stroke_linecap: "round",
                                                    stroke_linejoin: "round",
                                                    stroke_width: "2",
                                                    d: "M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
                                                }
                                            }
                                        }
                                        h4 { 
                                            class: class!(font_medium), 
                                            "Warning Dialog" 
                                        }
                                    }
                                    p { 
                                        class: class!(text_sm text_neutral_700), 
                                        "Use for warning messages that require attention." 
                                    }
                                }
                                
                                div { 
                                    class: class!(bg_white p_4 rounded border border_neutral_200),
                                    div { 
                                        class: class!(flex items_center mb_2),
                                        div { 
                                            class: class!(w_6 h_6 mr_2 text_danger_500),
                                            svg {
                                                xmlns: "http://www.w3.org/2000/svg",
                                                fill: "none",
                                                view_box: "0 0 24 24",
                                                stroke: "currentColor",
                                                
                                                path {
                                                    stroke_linecap: "round",
                                                    stroke_linejoin: "round",
                                                    stroke_width: "2",
                                                    d: "M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                                                }
                                            }
                                        }
                                        h4 { 
                                            class: class!(font_medium), 
                                            "Danger Dialog" 
                                        }
                                    }
                                    p { 
                                        class: class!(text_sm text_neutral_700), 
                                        "Use for destructive actions that require confirmation." 
                                    }
                                }
                                
                                div { 
                                    class: class!(bg_white p_4 rounded border border_neutral_200),
                                    div { 
                                        class: class!(flex items_center mb_2),
                                        div { 
                                            class: class!(w_6 h_6 mr_2 text_success_500),
                                            svg {
                                                xmlns: "http://www.w3.org/2000/svg",
                                                fill: "none",
                                                view_box: "0 0 24 24",
                                                stroke: "currentColor",
                                                
                                                path {
                                                    stroke_linecap: "round",
                                                    stroke_linejoin: "round",
                                                    stroke_width: "2",
                                                    d: "M5 13l4 4L19 7"
                                                }
                                            }
                                        }
                                        h4 { 
                                            class: class!(font_medium), 
                                            "Success Dialog" 
                                        }
                                    }
                                    p { 
                                        class: class!(text_sm text_neutral_700), 
                                        "Use for successful action confirmations." 
                                    }
                                }
                            }
                        }
                        
                        // Dialog Sizes and Features
                        div {
                            h3 { 
                                class: class!(text_lg font_medium text_neutral_700 mb_3), 
                                "Dialog Features" 
                            }
                            
                            // Dialog sizes
                            div { 
                                class: class!(bg_white p_4 rounded border border_neutral_200 mb_4),
                                h4 { 
                                    class: class!(font_medium mb_2), 
                                    "Dialog Sizes" 
                                }
                                
                                div { 
                                    class: class!(grid grid_cols_3 gap_2),
                                    div {
                                        class: class!(border border_neutral_200 rounded text_sm p_2 text_center),
                                        p { class: class!(font_medium), "Small" }
                                        p { class: class!(text_xs text_neutral_500), "max-w-md (448px)" }
                                    }
                                    div {
                                        class: class!(border border_primary_200 bg_primary_50 rounded text_sm p_2 text_center),
                                        p { class: class!(font_medium text_primary_700), "Medium" }
                                        p { class: class!(text_xs text_primary_600), "max-w-xl (576px)" }
                                    }
                                    div {
                                        class: class!(border border_neutral_200 rounded text_sm p_2 text_center),
                                        p { class: class!(font_medium), "Large" }
                                        p { class: class!(text_xs text_neutral_500), "max-w-3xl (768px)" }
                                    }
                                }
                            }
                            
                            // Dialog features
                            div { 
                                class: class!(bg_white p_4 rounded border border_neutral_200),
                                h4 { 
                                    class: class!(font_medium mb_2), 
                                    "Dialog Properties" 
                                }
                                
                                ul { 
                                    class: class!(space_y_2 list_disc pl_5 text_neutral_700),
                                    li { 
                                        span { class: class!(font_medium), "Modal" }
                                        ": Prevents interaction with background" 
                                    }
                                    li { 
                                        span { class: class!(font_medium), "Closable" }
                                        ": Shows close button in header" 
                                    }
                                    li { 
                                        span { class: class!(font_medium), "Close on backdrop" }
                                        ": Closes when clicking outside" 
                                    }
                                    li { 
                                        span { class: class!(font_medium), "Custom footer" }
                                        ": For action buttons" 
                                    }
                                    li { 
                                        span { class: class!(font_medium), "Variants" }
                                        ": Default, Info, Warning, Danger, Success" 
                                    }
                                    li { 
                                        span { class: class!(font_medium), "Responsive" }
                                        ": Adapts to screen sizes" 
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
