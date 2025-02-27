// components/dialog.rs
use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;
use mailiner_css::*;

/// Dialog sizes
#[derive(PartialEq, Clone)]
pub enum DialogSize {
    Small,     // 400px max-width
    Medium,    // 560px max-width
    Large,     // 720px max-width
}

/// Dialog variants for different contexts
#[derive(PartialEq, Clone)]
pub enum DialogVariant {
    Default,
    Info,
    Warning,
    Danger,
    Success,
}

/// Props for the Dialog component
#[derive(Clone, Props, PartialEq)]
pub struct DialogProps {
    /// Whether the dialog is currently open
    open: Option<bool>,
    
    /// Dialog title
    #[props(into)]
    title: Option<String>,
    
    /// Dialog size
    size: Option<DialogSize>,
    
    /// Dialog variant
    variant: Option<DialogVariant>,
    
    /// Whether to show the close button
    closable: Option<bool>,
    
    /// Whether to close when clicking the backdrop
    close_on_backdrop: Option<bool>,
    
    /// Whether the dialog is modal (prevents interaction with background)
    modal: Option<bool>,
    
    /// Content to render in the dialog
    children: Element,
    
    /// Footer content (typically buttons)
    footer: Option<Element>,
    
    /// Close event handler
    on_close: Option<EventHandler<()>>,
}

/// Dialog component for Mailiner UI
pub fn Dialog(props: DialogProps) -> Element {
    let is_open = props.open.unwrap_or(false);
    let closable = props.closable.unwrap_or(true);
    let close_on_backdrop = props.close_on_backdrop.unwrap_or(true);
    let is_modal = props.modal.unwrap_or(true);
    
    // Handle close event
    let on_close = move || {
        if let Some(handler) = &props.on_close {
            handler.call(());
        }
    };
    
    // Handle backdrop click
    let on_backdrop_click = move |_| {
        if close_on_backdrop {
            on_close();
        }
    };
    
    // Prevent click propagation to backdrop
    let prevent_propagation = move |event: MouseEvent| {
        event.stop_propagation();
    };
    
    // Determine size classes
    let size_classes = match props.size.clone().unwrap_or(DialogSize::Medium) {
        DialogSize::Small => class!(max_w_md),    // 448px
        DialogSize::Medium => class!(max_w_xl),    // 576px
        DialogSize::Large => class!(max_w_3xl),    // 768px
    };
    
    // Determine variant-specific styles
    let (header_bg, icon) = match props.variant.clone().unwrap_or(DialogVariant::Default) {
        DialogVariant::Default => (class!(bg_white), None),
        DialogVariant::Info => (class!(bg_primary_50), Some(
            rsx! {
                svg {
                    class: class!(w_6 h_6 text_primary_500),
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
        )),
        DialogVariant::Warning => (class!(bg_warning_50), Some(
            rsx! {
                svg {
                    class: class!(w_6 h_6 text_warning_500),
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
        )),
        DialogVariant::Danger => (class!(bg_danger_50), Some(
            rsx! {
                svg {
                    class: class!(w_6 h_6 text_danger_500),
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
        )),
        DialogVariant::Success => (class!(bg_success_50), Some(
            rsx! {
                svg {
                    class: class!(w_6 h_6 text_success_500),
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
        )),
    };
    
    // Only render if the dialog is open
    if !is_open {
        return rsx! {};
    }
    
    rsx! {
        // Backdrop and positioning container
        div {
            class: class!(fixed inset_0 flex items_center justify_center z_50),
            onclick: on_backdrop_click,
            
            // Dark semi-transparent backdrop
            div {
                class: class!(fixed inset_0 bg_black bg_opacity_50 transition_opacity),
                "aria-hidden": "true"
            }
            
            // Dialog content container
            div {
                class: format!("{} {}", class!(relative z_10 w_full rounded_lg shadow_lg overflow_hidden bg_white), size_classes),
                onclick: prevent_propagation,
                role: "dialog",
                "aria-modal": if is_modal { "true" } else { "false" },
                
                // Dialog header
                div {
                    class: format!("{} {}", class!(px_4 py_3 sm(px_6) border_b border_neutral_200 flex items_center justify_between), header_bg),
                    
                    // Title with optional icon
                    div { 
                        class: class!(flex items_center),
                        
                        // Icon (if variant provides one)
                        if let Some(variant_icon) = icon {
                            div { 
                                class: class!(mr_3), 
                                {variant_icon}
                            }
                        }
                        
                        // Dialog title
                        if let Some(title_text) = &props.title {
                            h3 { 
                                class: class!(text_lg font_medium text_neutral_800), 
                                "{title_text}" 
                            }
                        }
                    }
                    
                    // Close button
                    if closable {
                        button {
                            class: class!(rounded text_neutral_500 hover(text_neutral_700) focus(outline_none)),
                            "aria-label": "Close",
                            onclick: move |_| on_close(),
                            
                            svg {
                                class: class!(h_5 w_5),
                                xmlns: "http://www.w3.org/2000/svg",
                                fill: "none",
                                view_box: "0 0 24 24",
                                stroke: "currentColor",
                                
                                path {
                                    stroke_linecap: "round",
                                    stroke_linejoin: "round",
                                    stroke_width: "2",
                                    d: "M6 18L18 6M6 6l12 12"
                                }
                            }
                        }
                    }
                }
                
                // Dialog content
                div { 
                    class: class!(px_4 py_4 sm(p_6) overflow_y_auto),
                    {props.children}
                }
                
                // Dialog footer (if provided)
                if let Some(footer) = &props.footer {
                    div { 
                        class: class!(px_4 py_3 sm(px_6) border_t border_neutral_200 bg_neutral_50),
                        {footer}
                    }
                }
            }
        }
    }
}

/// Props for a simple confirmation dialog
#[derive(Clone, Props, PartialEq)]
pub struct ConfirmDialogProps {
    /// Whether the dialog is open
    open: bool,
    
    /// Dialog title
    #[props(into)]
    title: String,
    
    /// Dialog message
    #[props(into)]
    message: String,
    
    /// Confirm button text
    #[props(into)]
    confirm_text: Option<String>,
    
    /// Cancel button text
    #[props(into)]
    cancel_text: Option<String>,
    
    /// Dialog variant
    variant: Option<DialogVariant>,
    
    /// Confirm action handler
    on_confirm: Option<EventHandler<()>>,
    
    /// Cancel action handler
    on_cancel: Option<EventHandler<()>>,
}

/// Confirmation dialog component for Mailiner UI
pub fn ConfirmDialog(props: ConfirmDialogProps) -> Element {
    let variant = props.variant.clone().unwrap_or(DialogVariant::Default);
    
    // Create footer with confirm/cancel buttons
    let footer = rsx! {
        div { 
            class: class!(flex justify_end space_x_3),
            // Cancel button
            button {
                class: class!(px_4 py_2 bg_white border border_neutral_300 rounded text_neutral_700 hover(bg_neutral_50)),
                onclick: move |_| {
                    if let Some(handler) = &props.on_cancel {
                        handler.call(());
                    }
                },
                "{props.cancel_text.clone().unwrap_or_else(|| \"Cancel\".to_string())}"
            }
            
            // Confirm button with variant-specific styling
            button {
                class: match variant {
                    DialogVariant::Danger => class!(px_4 py_2 bg_danger_500 text_white rounded hover(bg_danger_600)),
                    DialogVariant::Warning => class!(px_4 py_2 bg_warning_500 text_white rounded hover(bg_warning_600)),
                    DialogVariant::Success => class!(px_4 py_2 bg_success_500 text_white rounded hover(bg_success_600)),
                    _ => class!(px_4 py_2 bg_primary_500 text_white rounded hover(bg_primary_600)),
                },
                onclick: move |_| {
                    if let Some(handler) = &props.on_confirm {
                        handler.call(());
                    }
                },
                "{props.confirm_text.clone().unwrap_or_else(|| \"Confirm\".to_string())}"
            }
        }
    };
    
    rsx! {
        Dialog {
            open: Some(props.open),
            title: Some(props.title.clone()),
            variant: Some(variant),
            size: Some(DialogSize::Small),
            on_close: move |_| {
                if let Some(handler) = &props.on_cancel {
                    handler.call(());
                }
            },
            footer: Some(footer),
            
            p { 
                class: class!(text_neutral_700), 
                "{props.message}" 
            }
        }
    }
}
