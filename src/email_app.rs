use crate::components::{
    Button, ButtonSize, ButtonVariant, Sidebar, SidebarItemData, Toolbar, ToolbarItemData,
    ToolbarPosition, ToolbarSize,
};
use crate::kernel::model::{
    Account, AccountId, Folder, FolderId, Message, MessageContent, MessageId,
};
use crate::kernel::repository::MessageSort;
use crate::kernel::service::EmailService;
use crate::AppState;
use dioxus::prelude::*;
use dioxus_free_icons::icons::ld_icons::LdGithub;
use dioxus_free_icons::icons::ld_icons::{LdFolder, LdInbox, LdMail, LdSend, LdTrash2};
use dioxus_free_icons::Icon;
use dioxus_tailwindcss::prelude::*;
use std::sync::Arc;
use tracing::debug;

#[derive(Clone, Debug, PartialEq, Routable)]
pub enum FolderRoute{
    #[route("/#account/:account_id/folder/:folder_id")]
    MessageListView {
        account_id: AccountId,
        folder_id: FolderId,
    },
}

#[derive(Clone, Debug, PartialEq, Routable)]
pub enum MessageRoute {
    #[route("/#account/:account_id/folder/:folder_id/message/:message_id")]
    MessageView {
        account_id: AccountId,
        folder_id: FolderId,
        message_id: MessageId,
    },
}

#[derive(Clone, Debug)]
struct AccountWithFolders {
    account: Account,
    folders: Vec<Folder>,
}

pub fn EmailApp() -> Element {
    // Get the app state
    let app_state = use_context::<AppState>();

    // Local state for UI components
    let mut is_sidebar_collapsed = use_signal(|| false);
    /*
    let folders = use_signal::<Option<Vec<Folder>>>(|| None);
    let mut selected_folder = use_signal::<Option<FolderId>>(|| None);
    let selected_message_content = use_signal::<Option<(Message, MessageContent)>>(|| None);
    let page = use_signal(|| 0usize);
    let page_size = 20;
    */

    // Function to load accounts and their folders
    let all = use_resource(move || {
        let service = Arc::clone(&app_state.email_service);
        async move {
            let accounts = service.list_accounts().await.unwrap();
            let mut accounts_with_folders = vec![];
            for account in accounts {
                let folders = service.list_folders(&account.id).await.unwrap_or_default();
                accounts_with_folders.push(AccountWithFolders {
                    account: account.clone(),
                    folders,
                });
            }

            accounts_with_folders
        }
    });
    /*
    let messages = use_resource(move || {
        let service = Arc::clone(&app_state.email_service);
        async move {
            if let Some(folder_id) = &*selected_folder.read() {
                service
                    .list_messages_paginated(
                        &selected_folder,
                        page.read().unwrap(),
                        page_size,
                        MessageSort::DateDesc,
                    )
                    .await
            } else {
                Ok(vec![])
            }
        }
    });
    */

    let sidebar_items = all
        .as_ref()
        .map(|accounts_with_folders| {
            debug!("accounts_with_folders: {:?}", accounts_with_folders);
            accounts_with_folders
                .iter()
                .map(|account_with_folders: &AccountWithFolders| {
                    let account_id = account_with_folders.account.id.clone();
                    SidebarItemData {
                        id: account_id.into(),
                        label: account_with_folders.account.name.clone(),
                        icon: None,
                        badge: None,
                        children: Some(
                            account_with_folders
                                .folders
                                .iter()
                                .map(|folder| SidebarItemData {
                                    id: folder.id.as_string(),
                                    label: folder.name.clone(),
                                    icon: None,
                                    badge: None,
                                    children: None,
                                })
                                .collect(),
                        ),
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec![]);

    /*
    // Handler for sidebar item selection
    let on_sidebar_select = move |id: String| {
        if let Some(accts) = accounts() {
            // Check if it's an account ID
            if let Some(account) = accts.iter().find(|a| a.id.0 == id) {
                load_folders((account.id.clone(), ()));
                return;
            }
        }

        if let Some(folder_list) = folders() {
            // Check if it's a folder ID
            if let Some(folder) = folder_list.iter().find(|f| f.id.0 == id) {
                page.set(0);
                selected_folder.set(Some(folder.id.clone()));
            }
        }
    };
    */

    /*
    // Toolbar items for the message view
    let toolbar_items = vec![
        ToolbarItemData::Button {
            id: "compose".to_string(),
            icon: Some(rsx! {
                Icon {
                    icon: LdSend,
                    class: class!(h_4 w_4),
                }
            }),
            label: Some("Compose".to_string()),
            on_click: None,
        },
        ToolbarItemData::Button {
            id: "refresh".to_string(),
            icon: Some(rsx! {
                Icon {
                    icon: LdGithub, // Using a placeholder icon
                    class: class!(h_4 w_4),
                }
            }),
            label: Some("Refresh".to_string()),
            on_click: None,
        },
    ];
    */

    rsx! {
        div {
            class: class!(flex h_screen overflow_hidden bg_white),

            // Email Sidebar - Folders
            Sidebar {
                items: sidebar_items,
                selected: app_state.selected_folder.read().as_ref().map(|f| f.0.clone()),
                collapsed: is_sidebar_collapsed(),
                on_select: move |id: String| {
                    router().replace(EmailRoute::Folder { account_id: id.clone().into(), folder_id: id.into() });
                },
                on_toggle_collapse: move |state| is_sidebar_collapsed.set(state),
                header: rsx! {
                    div {
                        class: class!(flex items_center justify_between),
                        h2 {
                            class: class!(text_lg font_semibold text_gray_800),
                            if !is_sidebar_collapsed() {
                                "Mailiner"
                            }
                        }
                    }
                },
                footer: rsx! {
                    div {
                        class: class!(flex items_center justify_between),
                        "Mailiner v0.1.0"
                    }
                }
            }
            /*
            Sidebar {
                items: sidebar_items,
                selected: app_state.selected_folder.read().as_ref().map(|f| f.0.clone()),
                collapsed: is_sidebar_collapsed(),
                on_select: Some(on_sidebar_select),
                on_toggle_collapse: Some(|state| is_sidebar_collapsed.set(state)),
                header: Some(rsx! {
                    div {
                        class: class!(flex items_center justify_between),
                        h2 {
                            class: class!(text_lg font_semibold text_gray_800),
                            if !is_sidebar_collapsed() {
                                "Mailiner"
                            }
                        }
                    }
                }),
                footer: Some(rsx! {
                    if !is_sidebar_collapsed() {
                        div {
                            class: class!(flex items_center justify_between),
                            "Version 0.1.0"
                        }
                    }
                })
            }
            */

            /*
            // Main content area
            div {
                class: class!(flex flex_col flex_1 overflow_hidden),

                // Top toolbar
                Toolbar {
                    items: toolbar_items,
                    position: ToolbarPosition::Top,
                    size: ToolbarSize::Medium,
                }

                // Main content with messages and message view
                div {
                    class: class!(flex flex_1 overflow_hidden),

                    // Message list panel
                    div {
                        class: class!(w_80 border_r border_gray_200 overflow_y_auto),

                        if app_state.is_loading() {
                            div {
                                class: class!(flex items_center justify_center h_full),
                                "Loading messages..."
                            }
                        } else {
                            match &*messages.read_unchecked() {
                                Some(Ok(msgs)) => rsx! {
                                    if msgs.is_empty() {
                                        div {
                                            class: class!(flex items_center justify_center h_full text_gray_500),
                                            "No messages in this folder"
                                        }
                                    } else {
                                        ul {
                                            class: class!(divide_y divide_gray_200),
                                            msgs.iter().map(|message| {
                                                let message_id = message.id.clone();
                                                let is_selected = app_state.selected_message.read().as_ref().map_or(false, |id| id.0 == message.id.0);
                                                let is_unread = !message.flags.read;

                                                let bg_class = if is_selected {
                                                    class!(bg_blue_50)
                                                } else if is_unread {
                                                    class!(bg_gray_50)
                                                } else {
                                                    class!(bg_white)
                                                };

                                                let font_weight = if is_unread {
                                                    class!(font_semibold)
                                                } else {
                                                    class!(font_normal)
                                                };

                                                rsx! {
                                                    li {
                                                        class: format!("{} {}", class!(cursor_pointer hover(bg_gray_50) px_4 py_3), bg_class),
                                                        onclick: move |_| load_message_content(message_id.clone()),
                                                        div {
                                                            class: class!(flex flex_col),
                                                            div {
                                                                class: format!("{} {}", class!(text_sm mb_1 truncate), font_weight),
                                                                "{message.subject}"
                                                            }
                                                            div {
                                                                class: class!(text_xs text_gray_600),
                                                                "{message.sender}"
                                                            }
                                                            div {
                                                                class: class!(text_xs text_gray_500 mt_1),
                                                                "{message.date.format('%d %b %H:%M')}"
                                                            }
                                                        }
                                                    }
                                                }
                                            })
                                        }

                                        // Pagination controls
                                        div {
                                            class: class!(flex justify_between p_4 border_t border_gray_200),
                                            Button {
                                                variant: ButtonVariant::Secondary,
                                                size: ButtonSize::Small,
                                                disabled: page() == 0,
                                                onclick: move |_| {
                                                    if let Some(folder_id) = &*app_state.selected_folder.read() {
                                                        let new_page = if page() > 0 { page() - 1 } else { 0 };
                                                        page.set(new_page);
                                                        load_messages((folder_id.clone(), new_page));
                                                    }
                                                },
                                                "Previous"
                                            }
                                            Button {
                                                variant: ButtonVariant::Secondary,
                                                size: ButtonSize::Small,
                                                disabled: messages().as_ref().map_or(true, |m| m.len() < page_size),
                                                onclick: move |_| {
                                                    if let Some(folder_id) = &*app_state.selected_folder.read() {
                                                        let new_page = page() + 1;
                                                        page.set(new_page);
                                                        load_messages((folder_id.clone(), new_page));
                                                    }
                                                },
                                                "Next"
                                            }
                                        }
                                    }
                                },
                                Some(Err(e)) => rsx! {
                                    div {
                                        class: class!(flex items_center justify_center h_full text_red_500),
                                        "Failed to load messages: {e}"
                                    }
                                },
                                None => rsx! {
                                    div {
                                        class: class!(flex items_center justify_center h_full text_gray_500),
                                        "Loading messages..."
                                    }
                                }
                            }
                        }
                    }

                    // Message view panel
                    div {
                        class: class!(flex_1 overflow_y_auto p_6 bg_gray_50),

                        if app_state.is_loading() {
                            div {
                                class: class!(flex items_center justify_center h_full),
                                "Loading message content..."
                            }
                        } else if let Some((message, content)) = selected_message_content() {
                            div {
                                class: class!(bg_white rounded_lg shadow_sm p_6 max_w_4xl mx_auto),

                                // Message header
                                div {
                                    class: class!(mb_6 pb_4 border_b border_gray_200),
                                    h1 {
                                        class: class!(text_xl font_semibold mb_2),
                                        "{message.subject}"
                                    }
                                    div {
                                        class: class!(text_sm text_gray_600 mb_1),
                                        "From: {message.sender}"
                                    }
                                    div {
                                        class: class!(text_sm text_gray_600 mb_1),
                                        "To: {message.recipients.join(\", \")}"
                                    }
                                    div {
                                        class: class!(text_sm text_gray_400),
                                        "{message.date.format(\"%d %b %Y %H:%M\")}"
                                    }
                                }

                                // Message body
                                div {
                                    class: class!(message_body),
                                    if let Some(html) = &content.html_body {
                                        div {
                                            dangerous_inner_html: html
                                        }
                                    } else if let Some(text) = &content.text_body {
                                        pre {
                                            class: class!(whitespace_pre_wrap font_sans text_sm),
                                            "{text}"
                                        }
                                    } else {
                                        p {
                                            class: class!(text_gray_500 italic),
                                            "No message content"
                                        }
                                    }
                                }

                                // Attachments
                                if !content.attachments.is_empty() {
                                    div {
                                        class: class!(mt_6 pt_4 border_t border_gray_200),
                                        h3 {
                                            class: class!(text_md font_medium mb_2),
                                            "Attachments ({content.attachments.len()})"
                                        }
                                        div {
                                            class: class!(flex flex_wrap gap_2),
                                            content.attachments.iter().map(|attachment| {
                                                rsx! {
                                                    div {
                                                        class: class!(flex items_center gap_2 px_3 py_2 bg_gray_100 rounded_md text_sm),
                                                        span {
                                                            "{attachment.name}"
                                                        }
                                                        span {
                                                            class: class!(text_gray_500 text_xs),
                                                            "({format_size(attachment.size)})"
                                                        }
                                                    }
                                                }
                                            })
                                        }
                                    }
                                }
                            }
                        } else {
                            div {
                                class: class!(flex items_center justify_center h_full text_gray_500),
                                "Select a message to view its content"
                            }
                        }
                    }
                }

                // Status bar
                div {
                    class: class!(h_8 border_t border_gray_200 bg_gray_50 px_4 flex items_center justify_between text_xs text_gray_500),

                    div {
                        class: class!(flex items_center gap_2),
                        if let Some(error) = app_state.error() {
                            div {
                                class: class!(text_red_500),
                                "{error}"
                            }
                        } else if app_state.is_loading() {
                            "Loading..."
                        } else if let Some(folders) = folders() {
                            let total_messages: usize = folders.iter().map(|f| f.total_messages).sum();
                            let unread_messages: usize = folders.iter().map(|f| f.unread_count).sum();
                            "Total messages: {total_messages} | Unread: {unread_messages}"
                        } else {
                            "Ready"
                        }
                    }

                    div {
                        "Mailiner v0.1.0"
                    }
                }
            }
            */
        }
    }
}

// Helper function to format file sizes
fn format_size(size: usize) -> String {
    if size < 1024 {
        format!("{}B", size)
    } else if size < 1024 * 1024 {
        format!("{:.1}KB", size as f64 / 1024.0)
    } else if size < 1024 * 1024 * 1024 {
        format!("{:.1}MB", size as f64 / 1024.0 / 1024.0)
    } else {
        format!("{:.1}GB", size as f64 / 1024.0 / 1024.0 / 1024.0)
    }
}
