use dioxus::prelude::*;
use dioxus_free_icons::{
    icons::ld_icons::{LdMail, LdInbox, LdSend, LdTrash2, LdFolder, LdArchive, LdPencil},
    Icon,
};
use dioxus_tailwindcss::prelude::*;

use crate::components::{Sidebar, SidebarItemData, Toolbar, ToolbarItemData, ToolbarPosition};
use crate::mailiner_core::mail_controller::MailController;
use crate::mailiner_core::model::{Account, Mailbox};

/// Main layout for the mail client
#[component]
pub fn MailLayout() -> Element {
    let mut mail_controller = use_context::<MailController>();
    let mut is_sidebar_collapsed = use_signal(|| false);
    
    // Get accounts for the sidebar
    let accounts = mail_controller.get_accounts();
    
    // Get selected IDs
    let selected_account_id = mail_controller.get_selected_account_id();
    let selected_mailbox_id = mail_controller.get_selected_mailbox_id();
    
    // If no account is selected, select the first one
    if selected_account_id.is_none() && !accounts.is_empty() {
        mail_controller.select_account(Some(accounts[0].id.clone()));
    }
    
    // Create sidebar items from accounts and mailboxes
    let sidebar_items = create_sidebar_items(&accounts);
    
    // Determine the selected sidebar item ID
    let selected_sidebar_item = if let Some(mailbox_id) = &selected_mailbox_id {
        Some(mailbox_id.clone())
    } else if let Some(account_id) = &selected_account_id {
        Some(account_id.clone())
    } else {
        None
    };
    
    // Create toolbar items
    let toolbar_items = vec![
        ToolbarItemData {
            id: "compose".to_string(),
            icon: rsx! { Icon { class: class!(h_5 w_5), icon: LdPencil } },
            label: Some("Compose".to_string()),
            tooltip: Some("Compose new email".to_string()),
            disabled: None,
            danger: None,
        },
        ToolbarItemData {
            id: "refresh".to_string(),
            icon: rsx! { Icon { class: class!(h_5 w_5), icon: LdInbox } },
            label: Some("Refresh".to_string()),
            tooltip: Some("Refresh mailbox".to_string()),
            disabled: None,
            danger: None,
        },
    ];
    
    rsx! {
        div {
            class: class!(flex h_screen bg_neutral_50),
            
            // Sidebar
            Sidebar {
                items: sidebar_items,
                selected: selected_sidebar_item,
                collapsed: is_sidebar_collapsed(),
                header: rsx! {
                    div {
                        class: class!(flex items_center),
                        Icon {
                            icon: LdMail,
                            class: class!(h_6 w_6 text_blue_600 mr_2),
                        }
                        if !is_sidebar_collapsed() {
                            span {
                                class: class!(text_lg font_semibold text_blue_600),
                                "Mailiner"
                            }
                        }
                    }
                },
                on_select: move |id: String| {
                    // Check if the ID belongs to an account or a mailbox
                    let is_account = accounts.iter().any(|a| a.id == id);
                    
                    if is_account {
                        mail_controller.select_account(Some(id.clone()));
                        
                        // Select the first mailbox of the account
                        if let Some(account) = mail_controller.get_account(&id) {
                            if !account.mailboxes.is_empty() {
                                mail_controller.select_mailbox(Some(account.mailboxes[0].id.clone()));
                            }
                        }
                    } else {
                        // It's a mailbox
                        mail_controller.select_mailbox(Some(id.clone()));
                    }
                },
                on_toggle_collapse: move |collapsed| {
                    is_sidebar_collapsed.set(collapsed);
                },
            }
            
            // Main content area
            div {
                class: class!(flex_1 flex flex_col overflow_hidden),
                
                // Toolbar
                Toolbar {
                    items: toolbar_items,
                    position: ToolbarPosition::Top,
                    on_item_click: move |id| {
                        if id == "compose" {
                            // Handle compose action
                            println!("Compose clicked");
                        } else if id == "refresh" {
                            // Handle refresh action
                            println!("Refresh clicked");
                        }
                    },
                }
                
                // Content area
                div {
                    class: class!(flex_1 flex overflow_hidden),
                    
                    // Outlet for the nested routes
                    //Outlet::<()> {}
                }
            }
        }
    }
}

/// Create sidebar items from accounts and mailboxes
fn create_sidebar_items(accounts: &Vec<Account>) -> Vec<SidebarItemData> {
    accounts
        .iter()
        .map(|account| {
            // Create account item
            let account_item = SidebarItemData {
                id: account.id.clone(),
                label: account.name.clone(),
                icon: Some(rsx! { Icon { class: class!(h_5 w_5), icon: LdMail } }),
                badge: None,
                children: Some(create_mailbox_items(&account.mailboxes)),
            };
            
            account_item
        })
        .collect()
}

/// Create sidebar items from mailboxes
fn create_mailbox_items(mailboxes: &[Mailbox]) -> Vec<SidebarItemData> {
    mailboxes
        .iter()
        .map(|mailbox| {
            // Choose icon based on mailbox name
            let icon = match mailbox.name.to_lowercase().as_str() {
                "inbox" => rsx! { Icon { class: class!(h_5 w_5), icon: LdInbox } },
                "sent" => rsx! { Icon { class: class!(h_5 w_5), icon: LdSend } },
                "trash" => rsx! { Icon { class: class!(h_5 w_5), icon: LdTrash2 } },
                "archive" => rsx! { Icon { class: class!(h_5 w_5), icon: LdArchive } },
                _ => rsx! { Icon { class: class!(h_5 w_5), icon: LdFolder } },
            };
            
            // Create badge if there are unread messages
            let badge = if mailbox.unread_count > 0 {
                Some(mailbox.unread_count.to_string())
            } else {
                None
            };
            
            // Create mailbox item
            let mailbox_item = SidebarItemData {
                id: mailbox.id.clone(),
                label: mailbox.name.clone(),
                icon: Some(icon),
                badge,
                children: mailbox.children.as_ref().map(|children| create_mailbox_items(children)),
            };
            
            mailbox_item
        })
        .collect()
} 