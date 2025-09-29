use std::fmt::Debug;
use std::ops::Range;

use async_trait::async_trait;
use chrono::Utc;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::error::Result;
use crate::ids::{AccountId, FolderId, MessageId, MessagePartId};
use crate::models::{Account, Envelope, Folder, MessagePart};

#[async_trait]
pub trait EmailConnector<S>: Send + Sync 
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send + Sync,
{
    async fn connect(&self, stream: S) -> Result<()>;
    async fn disconnect(&self) -> Result<()>;

    // Account operations
    async fn authenticate(&self, credentials: &str) -> Result<Account>;

    // Folder operations
    async fn list_folders(&self, account_id: &AccountId) -> Result<Vec<Folder>>;
    async fn create_folder(
        &self,
        account_id: &AccountId,
        name: &str,
        parent_id: Option<&FolderId>,
    ) -> Result<Folder>;
    async fn delete_folder(&self, folder_id: &FolderId) -> Result<()>;

    // Envelope operations
    async fn list_envelopes(&self, folder_id: &FolderId) -> Result<Vec<Envelope>>;
    async fn list_envelopes_range(&self, folder_id: &FolderId, range: Range<usize>) -> Result<Vec<Envelope>>;
    async fn get_envelope(&self, message_id: &MessageId) -> Result<Envelope>;
    async fn update_envelope_flags(
        &self,
        message_id: &MessageId,
        flags: &[(&str, bool)],
    ) -> Result<()>;

    // Message part operations
    async fn get_message_part(
        &self,
        message_id: &MessageId,
        part_id: &MessagePartId,
    ) -> Result<MessagePart>;
}

// Mock implementation for testing
pub struct MockConnector {
    connected: bool,
}

impl MockConnector {
    pub fn new() -> Self {
        Self { connected: false }
    }
}

#[async_trait]
impl<S> EmailConnector<S> for MockConnector 
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send + Sync + 'static,
{
    async fn connect(&self, _stream: S) -> Result<()> {
        Ok(())
    }

    async fn disconnect(&self) -> Result<()> {
        Ok(())
    }

    async fn authenticate(&self, _credentials: &str) -> Result<Account> {
        Ok(Account {
            id: AccountId::new("mock-account-1"),
            name: "Mock Account".to_string(),
            email: "mock@example.com".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }

    async fn list_folders(&self, account_id: &AccountId) -> Result<Vec<Folder>> {
        Ok(vec![
            Folder {
                id: FolderId::new("inbox"),
                account_id: account_id.clone(),
                name: "Inbox".to_string(),
                parent_id: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            Folder {
                id: FolderId::new("sent"),
                account_id: account_id.clone(),
                name: "Sent".to_string(),
                parent_id: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        ])
    }

    async fn create_folder(
        &self,
        account_id: &AccountId,
        name: &str,
        parent_id: Option<&FolderId>,
    ) -> Result<Folder> {
        Ok(Folder {
            id: FolderId::new(format!("folder-{}", name.to_lowercase())),
            account_id: account_id.clone(),
            name: name.to_string(),
            parent_id: parent_id.cloned(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }

    async fn delete_folder(&self, _folder_id: &FolderId) -> Result<()> {
        Ok(())
    }

    async fn list_envelopes(&self, folder_id: &FolderId) -> Result<Vec<Envelope>> {
        // Call the range version with default range
        let mut envelopes = Vec::new();
        for i in 0..100 {
            let message_id = MessageId::new(format!("test-message-{}", i + 1));
            envelopes.push(Envelope {
                id: message_id.clone(),
                account_id: AccountId::new("mock-account-1"),
                folder_id: folder_id.clone(),
                subject: Some(format!("Test Message {}", i + 1)),
                from: Some(crate::models::EmailAddress::List(vec![
                    crate::models::EmailAddr {
                        name: Some(format!("Sender {}", i + 1)),
                        email: Some(format!("sender{}@example.com", i + 1)),
                    },
                ])),
                to: Some(crate::models::EmailAddress::List(vec![
                    crate::models::EmailAddr {
                        name: Some("Test Recipient".to_string()),
                        email: Some("recipient@example.com".to_string()),
                    },
                ])),
                cc: None,
                bcc: None,
                date: Utc::now(),
                is_read: i % 3 == 0,
                is_starred: i % 5 == 0,
                is_flagged: false,
                is_draft: false,
                is_deleted: false,
                has_attachments: i % 2 == 0,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            });
        }
        Ok(envelopes)
    }

    async fn list_envelopes_range(&self, folder_id: &FolderId, range: Range<usize>) -> Result<Vec<Envelope>> {
        let mut envelopes = Vec::new();
        for i in range {
            let message_id = MessageId::new(format!("test-message-{}", i + 1));
            envelopes.push(Envelope {
                id: message_id.clone(),
                account_id: AccountId::new("mock-account-1"),
                folder_id: folder_id.clone(),
                subject: Some(format!("Test Message {}", i + 1)),
                from: Some(crate::models::EmailAddress::List(vec![
                    crate::models::EmailAddr {
                        name: Some(format!("Sender {}", i + 1)),
                        email: Some(format!("sender{}@example.com", i + 1)),
                    },
                ])),
                to: Some(crate::models::EmailAddress::List(vec![
                    crate::models::EmailAddr {
                        name: Some("Test Recipient".to_string()),
                        email: Some("recipient@example.com".to_string()),
                    },
                ])),
                cc: None,
                bcc: None,
                date: Utc::now(),
                is_read: i % 3 == 0,
                is_starred: i % 5 == 0,
                is_flagged: false,
                is_draft: false,
                is_deleted: false,
                has_attachments: i % 2 == 0,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            });
        }
        Ok(envelopes)
    }

    async fn get_envelope(&self, message_id: &MessageId) -> Result<Envelope> {
        Ok(Envelope {
            id: message_id.clone(),
            account_id: AccountId::new("mock-account-1"),
            folder_id: FolderId::new("inbox"),
            subject: Some("Test Message".to_string()),
            from: Some(crate::models::EmailAddress::List(vec![
                crate::models::EmailAddr {
                    name: Some("Test Sender".to_string()),
                    email: Some("sender@example.com".to_string()),
                },
            ])),
            to: Some(crate::models::EmailAddress::List(vec![
                crate::models::EmailAddr {
                    name: Some("Test Recipient".to_string()),
                    email: Some("recipient@example.com".to_string()),
                },
            ])),
            cc: None,
            bcc: None,
            date: Utc::now(),
            is_read: false,
            is_starred: false,
            is_flagged: false,
            is_draft: false,
            is_deleted: false,
            has_attachments: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }

    async fn update_envelope_flags(
        &self,
        _message_id: &MessageId,
        _flags: &[(&str, bool)],
    ) -> Result<()> {
        Ok(())
    }

    async fn get_message_part(
        &self,
        _message_id: &MessageId,
        part_id: &MessagePartId,
    ) -> Result<MessagePart> {
        Ok(MessagePart {
            id: part_id.clone(),
            envelope_id: MessageId::new("test-message-1"),
            content_type: "text/plain".to_string(),
            filename: None,
            size: 100,
            is_attachment: false,
            content: crate::models::MessageContent::Text("This is a test message.".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }
}
