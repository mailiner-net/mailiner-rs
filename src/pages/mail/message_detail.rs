use dioxus::prelude::*;
use dioxus_free_icons::{
    icons::ld_icons::{LdReply, LdReplyAll, LdForward, LdTrash2, LdStar, LdArchive},
    Icon,
};
use dioxus_tailwindcss::prelude::*;
use std::rc::Rc;
use chrono::{DateTime, Utc, Local};

use crate::components::{Button, ButtonSize, ButtonVariant, Toolbar, ToolbarItemData, ToolbarPosition};
use crate::mailiner_core::mail_controller::MailController;
use crate::mailiner_core::model::Message;

/// Component for displaying the details of a message
#[component]
pub fn MessageDetail() -> Element {
    let mut mail_controller = use_context::<MailController>();
    
    // Get the selected message (clone it for the UI)
    let selected_message = mail_controller.get_selected_message().map(Rc::new);
    
    // Create toolbar items
    let toolbar_items = vec![
        ToolbarItemData {
            id: "reply".to_string(),
            icon: rsx! { Icon { class: class!(h_5 w_5), icon: LdReply } },
            label: Some("Reply".to_string()),
            tooltip: Some("Reply to sender".to_string()),
            disabled: None,
            danger: None,
        },
        ToolbarItemData {
            id: "reply-all".to_string(),
            icon: rsx! { Icon { class: class!(h_5 w_5), icon: LdReplyAll } },
            label: Some("Reply All".to_string()),
            tooltip: Some("Reply to all recipients".to_string()),
            disabled: None,
            danger: None,
        },
        ToolbarItemData {
            id: "forward".to_string(),
            icon: rsx! { Icon { class: class!(h_5 w_5), icon: LdForward } },
            label: Some("Forward".to_string()),
            tooltip: Some("Forward this message".to_string()),
            disabled: None,
            danger: None,
        },
        ToolbarItemData {
            id: "archive".to_string(),
            icon: rsx! { Icon { class: class!(h_5 w_5), icon: LdArchive } },
            label: Some("Archive".to_string()),
            tooltip: Some("Archive this message".to_string()),
            disabled: None,
            danger: None,
        },
        ToolbarItemData {
            id: "delete".to_string(),
            icon: rsx! { Icon { class: class!(h_5 w_5), icon: LdTrash2 } },
            label: Some("Delete".to_string()),
            tooltip: Some("Delete this message".to_string()),
            disabled: None,
            danger: Some(true),
        },
    ];
    
    rsx! {
        div {
            class: class!(flex_1 flex flex_col h_full bg_white),
            
            if let Some(message) = selected_message.clone() {
                // Message toolbar
                Toolbar {
                    items: toolbar_items,
                    position: ToolbarPosition::Top,
                    on_item_click: move |id: String| {
                        match id.as_str() {
                            "reply" => println!("Reply clicked"),
                            "reply-all" => println!("Reply All clicked"),
                            "forward" => println!("Forward clicked"),
                            "archive" => println!("Archive clicked"),
                            "delete" => println!("Delete clicked"),
                            _ => {}
                        }
                    },
                }
                
                // Message content
                div {
                    class: class!(flex_1 overflow_y_auto p_6),
                    
                    // Message header
                    {
                        let message_id = message.id.clone();
                        rsx! {
                            MessageHeader {
                                message: message.clone(),
                                on_toggle_flag: move |_| {
                                    mail_controller.toggle_flag(&message_id);
                                }
                            }
                        }
                    }
                    
                    // Message body
                    div {
                        class: class!(mt_6),
                        
                        if let Some(html_body) = &message.body_html {
                            // HTML body
                            div {
                                class: class!(max_w_none),
                                dangerous_inner_html: "{html_body}"
                            }
                        } else if let Some(text_body) = &message.body_text {
                            // Text body
                            div {
                                class: class!(whitespace_pre_wrap),
                                "{text_body}"
                            }
                        } else {
                            // No body
                            div {
                                class: class!(text_neutral_500 italic),
                                "No message content"
                            }
                        }
                    }
                    
                    // Attachments
                    if !message.attachments.is_empty() {
                        div {
                            class: class!(mt_6 border_t border_neutral_200 pt_4),
                            h3 {
                                class: class!(text_sm font_semibold mb_2),
                                "Attachments ({message.attachments.len()})"
                            }
                            
                            div {
                                class: class!(grid grid_cols_2 md(grid_cols_3) lg(grid_cols_4) gap_3),
                                
                                for attachment in message.attachments.clone() {
                                    {
                                        let file_size = format_file_size(attachment.size);
                                        
                                        rsx! {
                                            div {
                                                class: class!(border border_neutral_200 rounded p_3 flex flex_col),
                                                
                                                div {
                                                    class: class!(text_sm font_medium truncate),
                                                    "{attachment.filename}"
                                                }
                                                
                                                div {
                                                    class: class!(text_xs text_neutral_500 mt_1),
                                                    "{file_size} • {attachment.mime_type}"
                                                }
                                                
                                                Button {
                                                    variant: ButtonVariant::Secondary,
                                                    size: ButtonSize::Small,
                                                    on_click: move |_| {
                                                        println!("Download attachment: {}", attachment.filename);
                                                    },
                                                    text: "Download"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                // No message selected
                div {
                    class: class!(flex_1 flex items_center justify_center text_neutral_500),
                    "Select a message to view its contents"
                }
            }
        }
    }
}

/// Props for the MessageHeader component
#[derive(Props, Clone, PartialEq)]
pub struct MessageHeaderProps {
    message: Rc<Message>,
    on_toggle_flag: EventHandler<()>,
}

/// Component for displaying the header of a message
#[component]
fn MessageHeader(props: MessageHeaderProps) -> Element {
    let message = &*props.message;
    
    // Format the date
    let date = format_date(&message.date);
    
    // Get sender display name
    let sender_display = message.sender_name.clone().unwrap_or_else(|| message.sender.clone());
    
    rsx! {
        div {
            class: class!(border_b border_neutral_200 pb_4),
            
            // Subject and flag button
            div {
                class: class!(flex items_start justify_between mb_2),
                
                h1 {
                    class: class!(text_2xl font_bold),
                    "{message.subject}"
                }
                
                button {
                    class: class!(ml_2 p_1),
                    onclick: move |_| props.on_toggle_flag.call(()),
                    
                    Icon {
                        icon: LdStar,
                        class: if message.is_flagged {
                            class!(h_5 w_5 text_amber_400)
                        } else {
                            class!(h_5 w_5 text_neutral_300)
                        }
                    }
                }
            }
            
            // Sender info
            div {
                class: class!(flex items_center mb_2),
                
                div {
                    class: class!(h_10 w_10 rounded_full bg_neutral_200 flex items_center justify_center text_neutral_600 mr_3),
                    span {
                        class: class!(text_lg font_semibold),
                        // Display first letter of sender name
                        "{sender_display.chars().next().unwrap_or('?')}"
                    }
                }
                
                div {
                    div {
                        class: class!(font_medium),
                        "{sender_display}"
                    }
                    
                    div {
                        class: class!(text_sm text_neutral_500),
                        "{message.sender}"
                    }
                }
            }
            
            // Recipients and date
            div {
                class: class!(text_sm text_neutral_600 grid grid_cols_1 md(grid_cols_2) gap_2),
                
                div {
                    strong { "Date: " }
                    span { "{date}" }
                }
                
                div {
                    strong { "To: " }
                    span { "{message.recipients.join(\", \")}" }
                }
                
                if !message.cc.is_empty() {
                    div {
                        strong { "CC: " }
                        span { "{message.cc.join(\", \")}" }
                    }
                }
                
                if !message.bcc.is_empty() {
                    div {
                        strong { "BCC: " }
                        span { "{message.bcc.join(\", \")}" }
                    }
                }
            }
        }
    }
}

/// Format a date string for display
fn format_date(date_str: &str) -> String {
    // Parse the date string into a DateTime
    match DateTime::parse_from_rfc3339(date_str) {
        Ok(dt) => {
            // Convert to local time
            let local_dt = dt.with_timezone(&Local);
            // Format the date
            local_dt.format("%b %d, %Y %H:%M").to_string()
        },
        Err(_) => {
            // Fallback if parsing fails
            date_str.replace("T", " ").replace("Z", "")
        }
    }
}

/// Format a file size in bytes to a human-readable string
fn format_file_size(size_in_bytes: usize) -> String {
    const KB: usize = 1024;
    const MB: usize = KB * 1024;
    const GB: usize = MB * 1024;
    
    if size_in_bytes >= GB {
        format!("{:.2} GB", size_in_bytes as f64 / GB as f64)
    } else if size_in_bytes >= MB {
        format!("{:.2} MB", size_in_bytes as f64 / MB as f64)
    } else if size_in_bytes >= KB {
        format!("{:.2} KB", size_in_bytes as f64 / KB as f64)
    } else {
        format!("{} bytes", size_in_bytes)
    }
} 