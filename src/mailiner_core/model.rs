use std::collections::HashMap;
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

/// Represents an email account
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub email: String,
    pub mailboxes: Vec<Mailbox>,
}

/// Represents a mailbox (folder) in an email account
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mailbox {
    pub id: String,
    pub name: String,
    pub unread_count: usize,
    pub children: Option<Vec<Mailbox>>,
}

/// Represents an email message
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub subject: String,
    pub sender: String,
    pub sender_name: Option<String>,
    pub recipients: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub date: String, // ISO 8601 format
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub is_read: bool,
    pub is_flagged: bool,
    pub attachments: Vec<Attachment>,
}

/// Represents an email attachment
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: usize,
}

/// The main model for the mail client
#[derive(Debug, Clone)]
pub struct MailModel {
    pub accounts: Vec<Account>,
    pub selected_account: Option<String>,
    pub selected_mailbox: Option<String>,
    pub selected_message: Option<String>,
    pub messages: HashMap<String, Vec<Message>>, // mailbox_id -> messages
}

impl MailModel {
    /// Create a new mail model with mock data
    pub fn new() -> Self {
        let accounts = vec![
            Account {
                id: "account1".to_string(),
                name: "Personal".to_string(),
                email: "user@example.com".to_string(),
                mailboxes: vec![
                    Mailbox {
                        id: "inbox1".to_string(),
                        name: "Inbox".to_string(),
                        unread_count: 3,
                        children: None,
                    },
                    Mailbox {
                        id: "sent1".to_string(),
                        name: "Sent".to_string(),
                        unread_count: 0,
                        children: None,
                    },
                    Mailbox {
                        id: "drafts1".to_string(),
                        name: "Drafts".to_string(),
                        unread_count: 0,
                        children: None,
                    },
                    Mailbox {
                        id: "trash1".to_string(),
                        name: "Trash".to_string(),
                        unread_count: 0,
                        children: None,
                    },
                    Mailbox {
                        id: "folders1".to_string(),
                        name: "Folders".to_string(),
                        unread_count: 1,
                        children: Some(vec![
                            Mailbox {
                                id: "work1".to_string(),
                                name: "Work".to_string(),
                                unread_count: 1,
                                children: None,
                            },
                            Mailbox {
                                id: "personal1".to_string(),
                                name: "Personal".to_string(),
                                unread_count: 0,
                                children: None,
                            },
                        ]),
                    },
                ],
            },
            Account {
                id: "account2".to_string(),
                name: "Work".to_string(),
                email: "work@example.com".to_string(),
                mailboxes: vec![
                    Mailbox {
                        id: "inbox2".to_string(),
                        name: "Inbox".to_string(),
                        unread_count: 5,
                        children: None,
                    },
                    Mailbox {
                        id: "sent2".to_string(),
                        name: "Sent".to_string(),
                        unread_count: 0,
                        children: None,
                    },
                    Mailbox {
                        id: "archive2".to_string(),
                        name: "Archive".to_string(),
                        unread_count: 0,
                        children: None,
                    },
                ],
            },
        ];

        // Create mock messages for each mailbox
        let mut messages = HashMap::new();
        
        // Inbox messages for account 1
        messages.insert(
            "inbox1".to_string(),
            vec![
                Message {
                    id: "msg1".to_string(),
                    subject: "Welcome to Mailiner".to_string(),
                    sender: "info@mailiner.app".to_string(),
                    sender_name: Some("Mailiner Team".to_string()),
                    recipients: vec!["user@example.com".to_string()],
                    cc: vec![],
                    bcc: vec![],
                    date: "2023-06-15T10:30:00Z".to_string(),
                    body_text: Some("Welcome to Mailiner, your new email client!".to_string()),
                    body_html: Some("<h1>Welcome to Mailiner</h1><p>Your new email client!</p>".to_string()),
                    is_read: false,
                    is_flagged: true,
                    attachments: vec![],
                },
                Message {
                    id: "msg2".to_string(),
                    subject: "Meeting tomorrow".to_string(),
                    sender: "colleague@example.com".to_string(),
                    sender_name: Some("John Colleague".to_string()),
                    recipients: vec!["user@example.com".to_string()],
                    cc: vec!["manager@example.com".to_string()],
                    bcc: vec![],
                    date: "2023-06-14T15:45:00Z".to_string(),
                    body_text: Some("Let's meet tomorrow at 10 AM to discuss the project.".to_string()),
                    body_html: Some("<p>Let's meet tomorrow at 10 AM to discuss the project.</p>".to_string()),
                    is_read: false,
                    is_flagged: false,
                    attachments: vec![
                        Attachment {
                            id: "att1".to_string(),
                            filename: "agenda.pdf".to_string(),
                            mime_type: "application/pdf".to_string(),
                            size: 1024 * 1024, // 1MB
                        },
                    ],
                },
                Message {
                    id: "msg3".to_string(),
                    subject: "Weekend plans".to_string(),
                    sender: "friend@example.com".to_string(),
                    sender_name: Some("Best Friend".to_string()),
                    recipients: vec!["user@example.com".to_string()],
                    cc: vec![],
                    bcc: vec![],
                    date: "2023-06-13T20:15:00Z".to_string(),
                    body_text: Some("Hey, what are your plans for the weekend? Want to hang out?".to_string()),
                    body_html: Some("<p>Hey, what are your plans for the weekend? Want to hang out?</p>".to_string()),
                    is_read: false,
                    is_flagged: false,
                    attachments: vec![],
                },
            ],
        );
        
        // Work folder messages for account 1
        messages.insert(
            "work1".to_string(),
            vec![
                Message {
                    id: "msg4".to_string(),
                    subject: "Project update".to_string(),
                    sender: "manager@example.com".to_string(),
                    sender_name: Some("Project Manager".to_string()),
                    recipients: vec!["user@example.com".to_string()],
                    cc: vec!["team@example.com".to_string()],
                    bcc: vec![],
                    date: "2023-06-12T09:00:00Z".to_string(),
                    body_text: Some("Here's the latest update on our project. We're making good progress!".to_string()),
                    body_html: Some("<p>Here's the latest update on our project. We're making good progress!</p>".to_string()),
                    is_read: false,
                    is_flagged: true,
                    attachments: vec![],
                },
            ],
        );
        
        // Inbox messages for account 2
        messages.insert(
            "inbox2".to_string(),
            vec![
                Message {
                    id: "msg5".to_string(),
                    subject: "Quarterly report".to_string(),
                    sender: "finance@example.com".to_string(),
                    sender_name: Some("Finance Department".to_string()),
                    recipients: vec!["work@example.com".to_string()],
                    cc: vec![],
                    bcc: vec![],
                    date: "2023-06-15T08:30:00Z".to_string(),
                    body_text: Some("Please find attached the quarterly financial report.".to_string()),
                    body_html: Some("<p>Please find attached the quarterly financial report.</p>".to_string()),
                    is_read: false,
                    is_flagged: true,
                    attachments: vec![
                        Attachment {
                            id: "att2".to_string(),
                            filename: "Q2_2023_Report.xlsx".to_string(),
                            mime_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string(),
                            size: 2 * 1024 * 1024, // 2MB
                        },
                    ],
                },
                Message {
                    id: "msg6".to_string(),
                    subject: "Team lunch next week".to_string(),
                    sender: "hr@example.com".to_string(),
                    sender_name: Some("HR Team".to_string()),
                    recipients: vec!["all-staff@example.com".to_string()],
                    cc: vec![],
                    bcc: vec![],
                    date: "2023-06-14T11:20:00Z".to_string(),
                    body_text: Some("We're organizing a team lunch next Wednesday. Please RSVP by Monday.".to_string()),
                    body_html: Some("<p>We're organizing a team lunch next Wednesday. Please RSVP by Monday.</p>".to_string()),
                    is_read: false,
                    is_flagged: false,
                    attachments: vec![],
                },
                Message {
                    id: "msg7".to_string(),
                    subject: "New client onboarding".to_string(),
                    sender: "sales@example.com".to_string(),
                    sender_name: Some("Sales Team".to_string()),
                    recipients: vec!["work@example.com".to_string()],
                    cc: vec!["manager@example.com".to_string()],
                    bcc: vec![],
                    date: "2023-06-13T14:45:00Z".to_string(),
                    body_text: Some("We have a new client coming onboard next month. Here are the details.".to_string()),
                    body_html: Some("<p>We have a new client coming onboard next month. Here are the details.</p>".to_string()),
                    is_read: false,
                    is_flagged: false,
                    attachments: vec![],
                },
                Message {
                    id: "msg8".to_string(),
                    subject: "Office maintenance".to_string(),
                    sender: "facilities@example.com".to_string(),
                    sender_name: Some("Facilities Management".to_string()),
                    recipients: vec!["all-staff@example.com".to_string()],
                    cc: vec![],
                    bcc: vec![],
                    date: "2023-06-12T16:30:00Z".to_string(),
                    body_text: Some("The office will be closed this Saturday for maintenance. Please ensure you take your belongings with you on Friday.".to_string()),
                    body_html: Some("<p>The office will be closed this Saturday for maintenance. Please ensure you take your belongings with you on Friday.</p>".to_string()),
                    is_read: false,
                    is_flagged: false,
                    attachments: vec![],
                },
                Message {
                    id: "msg9".to_string(),
                    subject: "IT system upgrade".to_string(),
                    sender: "it@example.com".to_string(),
                    sender_name: Some("IT Department".to_string()),
                    recipients: vec!["all-staff@example.com".to_string()],
                    cc: vec![],
                    bcc: vec![],
                    date: "2023-06-11T09:15:00Z".to_string(),
                    body_text: Some("We will be upgrading our systems this weekend. Expect some downtime between 10 PM Saturday and 2 AM Sunday.".to_string()),
                    body_html: Some("<p>We will be upgrading our systems this weekend. Expect some downtime between 10 PM Saturday and 2 AM Sunday.</p>".to_string()),
                    is_read: false,
                    is_flagged: true,
                    attachments: vec![],
                },
            ],
        );

        MailModel {
            accounts,
            selected_account: None,
            selected_mailbox: None,
            selected_message: None,
            messages,
        }
    }

    /// Get all accounts
    pub fn get_accounts(&self) -> &Vec<Account> {
        &self.accounts
    }

    /// Get account by ID
    pub fn get_account(&self, account_id: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == account_id)
    }

    /// Get mailbox by ID from a specific account
    pub fn get_mailbox(&self, account_id: &str, mailbox_id: &str) -> Option<&Mailbox> {
        if let Some(account) = self.get_account(account_id) {
            Self::find_mailbox_in_tree(&account.mailboxes, mailbox_id)
        } else {
            None
        }
    }

    /// Recursively find a mailbox in a tree of mailboxes
    fn find_mailbox_in_tree<'a>(mailboxes: &'a [Mailbox], mailbox_id: &str) -> Option<&'a Mailbox> {
        for mailbox in mailboxes {
            if mailbox.id == mailbox_id {
                return Some(mailbox);
            }
            
            if let Some(children) = &mailbox.children {
                if let Some(found) = Self::find_mailbox_in_tree(children, mailbox_id) {
                    return Some(found);
                }
            }
        }
        
        None
    }

    /// Get messages for a specific mailbox (clones the messages)
    pub fn get_messages(&self, mailbox_id: &str) -> Vec<Message> {
        self.messages.get(mailbox_id).cloned().unwrap_or_default()
    }

    /// Get messages for a specific mailbox (returns references to avoid cloning)
    pub fn get_messages_ref<'a>(&'a self, mailbox_id: &str) -> Vec<&'a Message> {
        if let Some(messages) = self.messages.get(mailbox_id) {
            messages.iter().collect()
        } else {
            Vec::new()
        }
    }

    /// Get a specific message by ID (clones the message)
    pub fn get_message(&self, message_id: &str) -> Option<Message> {
        for messages in self.messages.values() {
            if let Some(message) = messages.iter().find(|m| m.id == message_id) {
                return Some(message.clone());
            }
        }
        None
    }

    /// Get a specific message by ID (returns a reference to avoid cloning)
    pub fn get_message_ref<'a>(&'a self, message_id: &str) -> Option<&'a Message> {
        for messages in self.messages.values() {
            if let Some(message) = messages.iter().find(|m| m.id == message_id) {
                return Some(message);
            }
        }
        None
    }

    /// Mark a message as read
    pub fn mark_as_read(&mut self, message_id: &str, read: bool) {
        for messages in self.messages.values_mut() {
            for message in messages.iter_mut() {
                if message.id == message_id {
                    message.is_read = read;
                    return;
                }
            }
        }
    }

    /// Flag or unflag a message
    pub fn toggle_flag(&mut self, message_id: &str) {
        for messages in self.messages.values_mut() {
            for message in messages.iter_mut() {
                if message.id == message_id {
                    message.is_flagged = !message.is_flagged;
                    return;
                }
            }
        }
    }

    /// Select an account
    pub fn select_account(&mut self, account_id: Option<String>) {
        self.selected_account = account_id;
        // Reset mailbox and message selection when changing accounts
        self.selected_mailbox = None;
        self.selected_message = None;
    }

    /// Select a mailbox
    pub fn select_mailbox(&mut self, mailbox_id: Option<String>) {
        self.selected_mailbox = mailbox_id;
        // Reset message selection when changing mailboxes
        self.selected_message = None;
    }

    /// Select a message
    pub fn select_message(&mut self, message_id: Option<String>) {
        self.selected_message = message_id.clone();
        
        // Mark the message as read when selected
        if let Some(id) = &message_id {
            self.mark_as_read(id, true);
        }
    }
} 