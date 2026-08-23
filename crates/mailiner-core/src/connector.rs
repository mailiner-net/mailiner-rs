use std::collections::HashMap;
use std::fmt::Debug;
use std::ops::Range;
use std::pin::Pin;

use async_trait::async_trait;
use chrono::Utc;
use futures::stream::{self, Stream};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::body::{BodyPart, ContentDisposition};
use crate::error::Result;
use crate::ids::{AccountId, FolderId, MessageId};
use crate::models::{
    Account, Envelope, Folder, MessageContent, MessagePart, PartChunk, PartKind, TransferEncoding,
};

/// Stream of transfer-encoded part chunks (attachment download).
pub type PartStream = Pin<Box<dyn Stream<Item = Result<PartChunk>> + Send>>;

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
    /// Select the folder and return the number of messages (EXISTS).
    async fn open_folder(&self, folder_id: &FolderId) -> Result<usize>;

    // Envelope operations
    async fn list_envelopes(&self, folder_id: &FolderId) -> Result<Vec<Envelope>>;
    /// Fetch envelopes for a UI index range `[start, end)`.
    ///
    /// Indices are newest-first: index 0 is the most recent message in the folder.
    async fn list_envelopes_range(
        &self,
        folder_id: &FolderId,
        range: Range<usize>,
    ) -> Result<Vec<Envelope>>;
    async fn get_envelope(&self, message_id: &MessageId) -> Result<Envelope>;
    /// Set or clear named flags (`is_read`, `is_flagged`, `is_draft`, `is_deleted`, `is_starred`).
    async fn update_envelope_flags(
        &self,
        folder_id: &FolderId,
        message_ids: &[MessageId],
        flags: &[(&str, bool)],
    ) -> Result<()>;

    /// Move messages from `folder_id` to `dest_folder_id` (IMAP MOVE, or COPY+\Deleted).
    async fn move_messages(
        &self,
        folder_id: &FolderId,
        message_ids: &[MessageId],
        dest_folder_id: &FolderId,
    ) -> Result<()>;

    /// Permanently delete messages (STORE \Deleted + EXPUNGE).
    async fn delete_messages(
        &self,
        folder_id: &FolderId,
        message_ids: &[MessageId],
    ) -> Result<()>;

    /// FETCH BODYSTRUCTURE for one message (UID). Selects `folder_id` if needed.
    async fn get_body_structure(
        &self,
        folder_id: &FolderId,
        message_id: &MessageId,
    ) -> Result<BodyPart>;

    /// FETCH one or more `BODY.PEEK[section]` in a single round-trip when possible.
    /// Values are raw transfer-encoded octets.
    async fn fetch_raw_parts(
        &self,
        folder_id: &FolderId,
        message_id: &MessageId,
        sections: &[String],
    ) -> Result<HashMap<String, Vec<u8>>>;

    /// Stream a single part for attachment download.
    async fn stream_raw_part(
        &self,
        folder_id: &FolderId,
        message_id: &MessageId,
        section: &str,
    ) -> Result<PartStream>;
}

fn mock_envelopes(folder_id: &FolderId, range: Range<usize>) -> Result<Vec<Envelope>> {
    let total = 100usize;
    let end = range.end.min(total);
    let start = range.start.min(end);
    let mut envelopes = Vec::new();
    for i in start..end {
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

/// Realistic multipart fixture used by MockConnector and UI development.
pub fn mock_multipart_structure() -> BodyPart {
    BodyPart {
        type_: "multipart".into(),
        subtype: "mixed".into(),
        subparts: vec![
            BodyPart {
                type_: "multipart".into(),
                subtype: "alternative".into(),
                subparts: vec![
                    BodyPart {
                        type_: "text".into(),
                        subtype: "plain".into(),
                        encoding: Some("7BIT".into()),
                        size: Some(42),
                        parameters: [("CHARSET".into(), "UTF-8".into())].into(),
                        ..Default::default()
                    },
                    BodyPart {
                        type_: "text".into(),
                        subtype: "html".into(),
                        encoding: Some("QUOTED-PRINTABLE".into()),
                        size: Some(80),
                        parameters: [("CHARSET".into(), "UTF-8".into())].into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            BodyPart {
                type_: "application".into(),
                subtype: "pdf".into(),
                encoding: Some("BASE64".into()),
                size: Some(64),
                disposition: Some(ContentDisposition {
                    type_: "ATTACHMENT".into(),
                    attributes: [("FILENAME".into(), "report.pdf".into())].into(),
                }),
                description: Some("report.pdf".into()),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn mock_section_bytes(section: &str) -> Vec<u8> {
    match section {
        "1.1" => b"Hello plain text body.".to_vec(),
        "1.2" => b"<p>Hello <b>HTML</b> body.</p>".to_vec(),
        "2" => {
            // base64 of "PDFDATA"
            b"UERGRGF0YQ==".to_vec()
        }
        "TEXT" => b"Single part body.".to_vec(),
        _ => format!("mock-section-{}", section).into_bytes(),
    }
}

// Mock implementation for testing
pub struct MockConnector {
    #[allow(dead_code)]
    connected: bool,
}

impl MockConnector {
    pub fn new() -> Self {
        Self { connected: false }
    }
}

impl Default for MockConnector {
    fn default() -> Self {
        Self::new()
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
                role: crate::MailboxRole::Inbox,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            Folder {
                id: FolderId::new("sent"),
                account_id: account_id.clone(),
                name: "Sent".to_string(),
                parent_id: None,
                role: crate::MailboxRole::Sent,
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
            role: crate::MailboxRole::Other,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }

    async fn delete_folder(&self, _folder_id: &FolderId) -> Result<()> {
        Ok(())
    }

    async fn open_folder(&self, _folder_id: &FolderId) -> Result<usize> {
        Ok(100)
    }

    async fn list_envelopes(&self, folder_id: &FolderId) -> Result<Vec<Envelope>> {
        mock_envelopes(folder_id, 0..100)
    }

    async fn list_envelopes_range(
        &self,
        folder_id: &FolderId,
        range: Range<usize>,
    ) -> Result<Vec<Envelope>> {
        mock_envelopes(folder_id, range)
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
        _folder_id: &FolderId,
        _message_ids: &[MessageId],
        _flags: &[(&str, bool)],
    ) -> Result<()> {
        Ok(())
    }

    async fn move_messages(
        &self,
        _folder_id: &FolderId,
        _message_ids: &[MessageId],
        _dest_folder_id: &FolderId,
    ) -> Result<()> {
        Ok(())
    }

    async fn delete_messages(
        &self,
        _folder_id: &FolderId,
        _message_ids: &[MessageId],
    ) -> Result<()> {
        Ok(())
    }

    async fn get_body_structure(
        &self,
        _folder_id: &FolderId,
        _message_id: &MessageId,
    ) -> Result<BodyPart> {
        Ok(mock_multipart_structure())
    }

    async fn fetch_raw_parts(
        &self,
        _folder_id: &FolderId,
        _message_id: &MessageId,
        sections: &[String],
    ) -> Result<HashMap<String, Vec<u8>>> {
        let mut map = HashMap::new();
        for s in sections {
            map.insert(s.clone(), mock_section_bytes(s));
        }
        Ok(map)
    }

    async fn stream_raw_part(
        &self,
        _folder_id: &FolderId,
        _message_id: &MessageId,
        section: &str,
    ) -> Result<PartStream> {
        let data = mock_section_bytes(section);
        let total = data.len() as u64;
        // Re-chunk into 8-byte frames for progress testing.
        let chunks: Vec<Result<PartChunk>> = data
            .chunks(8)
            .map(|c| {
                Ok(PartChunk {
                    data: c.to_vec(),
                    total_hint: Some(total),
                })
            })
            .collect();
        Ok(Box::pin(stream::iter(chunks)))
    }
}

/// Helper: build a minimal single-part text MessagePart for tests/storage.
pub fn mock_text_part(envelope_id: MessageId, part_id: &str, text: &str) -> MessagePart {
    let now = Utc::now();
    MessagePart {
        id: crate::ids::MessagePartId::new(part_id),
        envelope_id,
        path: vec!["TEXT".into()],
        kind: PartKind::TextPlain,
        content_type: "text/plain".into(),
        charset: Some("UTF-8".into()),
        content_id: None,
        description: None,
        filename: None,
        encoding: TransferEncoding::SevenBit,
        original_size: Some(text.len() as u64),
        size: text.len() as u64,
        is_attachment: false,
        is_hidden: false,
        raw_content: None,
        content: MessageContent::Text(text.to_string()),
        created_at: now,
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::LoadedMessage;

    #[test]
    fn mock_structure_has_mixed_root() {
        let s = mock_multipart_structure();
        assert_eq!(s.type_, "multipart");
        assert_eq!(s.subtype, "mixed");
        assert_eq!(s.subparts.len(), 2);
    }

    #[test]
    fn message_part_section_text() {
        let p = mock_text_part(MessageId::new("1"), "p1", "hi");
        assert_eq!(p.section(), "TEXT");
        assert!(p.should_prefetch());
    }

    #[test]
    fn loaded_message_filters() {
        let mut parts = vec![
            mock_text_part(MessageId::new("1"), "a", "hi"),
            mock_text_part(MessageId::new("1"), "b", "att"),
        ];
        parts[1].is_attachment = true;
        parts[1].is_hidden = false;
        let loaded = LoadedMessage {
            envelope_id: MessageId::new("1"),
            folder_id: FolderId::new("inbox"),
            parts,
            structure: None,
        };
        assert_eq!(loaded.attachments().count(), 1);
        assert_eq!(loaded.content_parts().count(), 1);
    }
}
