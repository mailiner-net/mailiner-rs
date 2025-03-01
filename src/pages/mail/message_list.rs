use dioxus::prelude::*;
use dioxus_free_icons::{icons::ld_icons::LdStar, Icon};
use dioxus_tailwindcss::prelude::*;
use std::rc::Rc;

use crate::mailiner_core::mail_controller::MailController;
use crate::mailiner_core::model::Message;

/// Component for displaying a list of messages
#[component]
pub fn MessageList() -> Element {
    let mut mail_controller = use_context::<MailController>();
    
    // Get messages for the selected mailbox (we need to clone them for the UI)
    let messages = mail_controller.get_selected_mailbox_messages()
        .into_iter()
        .map(|m| m.clone())
        .collect::<Vec<Message>>();
    
    let selected_message_id = mail_controller.get_selected_message_id();
    
    // If no message is selected and there are messages, select the first one
    if selected_message_id.is_none() && !messages.is_empty() {
        mail_controller.select_message(Some(messages[0].id.clone()));
    }
    
    // Convert to Rc for efficient sharing
    let messages_rc: Vec<Rc<Message>> = messages.into_iter().map(Rc::new).collect();
    
    rsx! {
        div {
            class: class!(w_96 border_r border_neutral_200 h_full flex flex_col bg_white),
            
            // Message list header
            div {
                class: class!(p_4 border_b border_neutral_200 flex justify_between items_center),
                h2 {
                    class: class!(text_lg font_semibold),
                    if let Some(mailbox) = mail_controller.get_selected_mailbox() {
                        "{mailbox.name}"
                    } else {
                        "No mailbox selected"
                    }
                }
                
                span {
                    class: class!(text_sm text_neutral_500),
                    "{messages_rc.len()} messages"
                }
            }
            
            // Message list
            div {
                class: class!(flex_1 overflow_y_auto),
                
                if messages_rc.is_empty() {
                    div {
                        class: class!(p_4 text_center text_neutral_500),
                        "No messages in this mailbox"
                    }
                } else {
                    ul {
                        class: class!(divide_y divide_neutral_200),
                        
                        {messages_rc.iter().map(|message| {
                            let message_id_select = message.id.clone();
                            let message_id_toggle = message.id.clone();
                            let is_selected = selected_message_id.as_ref().map_or(false, |id| id == &message.id);
                            let message_rc = message.clone();
                            
                            rsx! {
                                MessageListItem {
                                    message: message_rc,
                                    is_selected: is_selected,
                                    on_select: move |_| {
                                        let mut mail_controller = use_context::<MailController>();
                                        mail_controller.select_message(Some(message_id_select.clone()));
                                    },
                                    on_toggle_flag: move |_| {
                                        let mut mail_controller = use_context::<MailController>();
                                        mail_controller.toggle_flag(&message_id_toggle.clone());
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

/// Props for the MessageListItem component
#[derive(Props, Clone, PartialEq)]
pub struct MessageListItemProps {
    message: Rc<Message>,
    is_selected: bool,
    on_select: EventHandler<()>,
    on_toggle_flag: EventHandler<()>,
}

/// Component for displaying a single message in the list
#[component]
fn MessageListItem(props: MessageListItemProps) -> Element {
    let message = &*props.message;
    
    // Format the date
    let date_str = format_date(&message.date);
    
    // Get sender display name
    let sender_display = message.sender_name.clone().unwrap_or_else(|| message.sender.clone());
    
    // Base classes
    let base_classes = class!(p_4 cursor_pointer hover(bg_neutral_50));
    
    // Additional classes based on state
    let state_classes = if props.is_selected {
        class!(bg_blue_50 hover(bg_blue_100))
    } else if !message.is_read {
        class!(bg_white font_semibold)
    } else {
        class!(bg_white text_neutral_700)
    };
    
    rsx! {
        li {
            class: format!("{} {}", base_classes, state_classes),
            onclick: move |_| props.on_select.call(()),
            
            div {
                class: class!(flex items_start gap_3),
                
                // Flag/star button
                button {
                    class: class!(shrink_0 mt_1),
                    onclick: move |e| {
                        e.stop_propagation();
                        props.on_toggle_flag.call(());
                    },
                    
                    Icon {
                        icon: LdStar,
                        class: if message.is_flagged {
                            class!(h_4 w_4 text_amber_400)
                        } else {
                            class!(h_4 w_4 text_neutral_300)
                        }
                    }
                }
                
                // Message content
                div {
                    class: class!(flex_1 min_w_0),
                    
                    // Sender and date
                    div {
                        class: class!(flex justify_between items_baseline mb_1),
                        
                        // Sender
                        div {
                            class: class!(truncate font_medium),
                            "{sender_display}"
                        }
                        
                        // Date
                        div {
                            class: class!(text_xs text_neutral_500 shrink_0 ml_2),
                            "{date_str}"
                        }
                    }
                    
                    // Subject
                    div {
                        class: class!(truncate mb_1),
                        "{message.subject}"
                    }
                    
                    // Preview
                    div {
                        class: class!(text_sm text_neutral_500 truncate),
                        if let Some(body) = &message.body_text {
                            "{preview_text(body, 100)}"
                        }
                    }
                    
                    // Attachment indicator
                    if !message.attachments.is_empty() {
                        div {
                            class: class!(text_xs text_neutral_500 mt_1),
                            "📎 {message.attachments.len()} attachment(s)"
                        }
                    }
                }
            }
        }
    }
}

/// Format a date string for display
fn format_date(date_str: &str) -> String {
    // For a real implementation, parse the ISO date and format it nicely
    // For now, just return a simplified version
    if let Ok(date) = chrono::DateTime::parse_from_rfc3339(date_str) {
        let now = chrono::Utc::now();
        let duration = now.signed_duration_since(date);
        
        if duration.num_days() == 0 {
            // Today
            date.format("%H:%M").to_string()
        } else if duration.num_days() < 7 {
            // This week
            date.format("%a %H:%M").to_string()
        } else {
            // Older
            date.format("%b %d").to_string()
        }
    } else {
        // Fallback if parsing fails
        date_str.to_string()
    }
}

/// Create a preview of the message text
fn preview_text(text: &str, max_length: usize) -> String {
    if text.len() <= max_length {
        text.to_string()
    } else {
        format!("{}...", &text[..max_length])
    }
} 