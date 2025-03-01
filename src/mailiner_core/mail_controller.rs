use dioxus::prelude::*;
use crate::mailiner_core::model::{Account, Attachment, MailModel, Mailbox, Message};

/// Controller for mail operations
#[derive(Clone)]
pub struct MailController {
    model: Signal<MailModel>,
}

impl MailController {
    /// Create a new mail controller with mock data
    pub fn new() -> Self {
        let model = use_signal(|| MailModel::new());
        
        MailController { model }
    }
    
    /// Get all accounts (returns a reference to avoid cloning)
    pub fn get_accounts(&self) -> Vec<Account> {
        self.model.read().get_accounts().clone()
    }
    
    /// Get account by ID (returns a reference to avoid cloning)
    pub fn get_account(&self, account_id: &str) -> Option<Account> {
        self.model.read().get_account(account_id).cloned()
    }
    
    /// Get mailbox by ID from a specific account (returns a reference to avoid cloning)
    pub fn get_mailbox(&self, account_id: &str, mailbox_id: &str) -> Option<Mailbox> {
        self.model.read().get_mailbox(account_id, mailbox_id).cloned()
    }
    
    /// Get messages for a specific mailbox (returns references to avoid cloning)
    pub fn get_messages(&self, mailbox_id: &str) -> Vec<Message> {
        self.model.read().get_messages(mailbox_id)
    }
    
    /// Get a specific message by ID (returns a reference to avoid cloning)
    pub fn get_message(&self, message_id: &str) -> Option<Message> {
        self.model.read().get_message(message_id)
    }
    
    /// Mark a message as read
    pub fn mark_as_read(&mut self, message_id: &str, read: bool) {
        self.model.write().mark_as_read(message_id, read);
    }
    
    /// Flag or unflag a message
    pub fn toggle_flag(&mut self, message_id: &str) {
        self.model.write().toggle_flag(message_id);
    }
    
    /// Select an account
    pub fn select_account(&mut self, account_id: Option<String>) {
        self.model.write().select_account(account_id);
    }
    
    /// Select a mailbox
    pub fn select_mailbox(&mut self, mailbox_id: Option<String>) {
        self.model.write().select_mailbox(mailbox_id);
    }
    
    /// Select a message
    pub fn select_message(&mut self, message_id: Option<String>) {
        self.model.write().select_message(message_id);
    }
    
    /// Get the currently selected account ID
    pub fn get_selected_account_id(&self) -> Option<String> {
        self.model.read().selected_account.clone()
    }
    
    /// Get the currently selected mailbox ID
    pub fn get_selected_mailbox_id(&self) -> Option<String> {
        self.model.read().selected_mailbox.clone()
    }
    
    /// Get the currently selected message ID
    pub fn get_selected_message_id(&self) -> Option<String> {
        self.model.read().selected_message.clone()
    }
    
    /// Get the currently selected account (returns a reference to avoid cloning)
    pub fn get_selected_account(&self) -> Option<Account> {
        let model = self.model.read();
        if let Some(account_id) = &model.selected_account {
            model.get_account(account_id).cloned()
        } else {
            None
        }
    }
    
    /// Get the currently selected mailbox (returns a reference to avoid cloning)
    pub fn get_selected_mailbox(&self) -> Option<Mailbox> {
        let model = self.model.read();
        if let Some(account_id) = &model.selected_account {
            if let Some(mailbox_id) = &model.selected_mailbox {
                model.get_mailbox(account_id, mailbox_id).cloned()
            } else {
                None
            }
        } else {
            None
        }
    }
    
    /// Get the currently selected message (returns a reference to avoid cloning)
    pub fn get_selected_message(&self) -> Option<Message> {
        let model = self.model.read();
        if let Some(message_id) = &model.selected_message {
            model.get_message(message_id)
        } else {
            None
        }
    }
    
    /// Get messages for the currently selected mailbox (returns references to avoid cloning)
    pub fn get_selected_mailbox_messages(&self) -> Vec<Message> {
        let model = self.model.read();
        if let Some(mailbox_id) = &model.selected_mailbox {
            model.get_messages(mailbox_id)
        } else {
            Vec::new()
        }
    }
} 