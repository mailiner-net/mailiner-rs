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
    Envelope, EnvelopeFlag, Folder, FolderCounts, FolderListState, MessageContent, MessagePart,
    MessageSort, PartChunk, PartKind, TransferEncoding,
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
    async fn authenticate(&self, credentials: &str) -> Result<()>;

    // Folder operations
    async fn list_folders(&self, account_id: &AccountId) -> Result<Vec<Folder>>;
    /// `STATUS (MESSAGES UNSEEN)` for each id. Missing entries were skipped or failed.
    async fn folder_counts(
        &self,
        folder_ids: &[FolderId],
    ) -> Result<HashMap<FolderId, FolderCounts>>;
    /// SELECT the folder, build the sort index, and return the list length.
    async fn prepare_folder_list(
        &self,
        folder_id: &FolderId,
        sort: MessageSort,
    ) -> Result<FolderListState>;

    /// Fetch envelopes for a UI index range `[start, end)`.
    ///
    /// Indices follow the last [`Self::prepare_folder_list`] sort (default: newest-first arrival).
    async fn list_envelopes_range(
        &self,
        folder_id: &FolderId,
        range: Range<usize>,
    ) -> Result<Vec<Envelope>>;
    /// Set or clear flags. Unknown names cannot be passed (typed).
    async fn update_envelope_flags(
        &self,
        folder_id: &FolderId,
        message_ids: &[MessageId],
        flags: &[(EnvelopeFlag, bool)],
    ) -> Result<()>;
    /// Keep an Unread-first list index in sync after a `\Seen` change.
    ///
    /// Returns `(old_index, new_index)` per id, in call order. Empty when the
    /// current folder is not sorted by unread.
    async fn sync_unread_sort_index(
        &self,
        message_ids: &[MessageId],
        now_read: bool,
    ) -> Result<Vec<(usize, usize)>>;

    /// Move messages from `folder_id` to `dest_folder_id` (IMAP MOVE, or COPY+\Deleted).
    ///
    /// Returns destination UIDs when the server sends `COPYUID` (same order as
    /// `message_ids`). Empty if the server omitted it.
    async fn move_messages(
        &self,
        folder_id: &FolderId,
        message_ids: &[MessageId],
        dest_folder_id: &FolderId,
    ) -> Result<Vec<MessageId>>;

    /// Copy messages from `folder_id` to `dest_folder_id` (IMAP UID COPY).
    ///
    /// Source messages stay; this must not add `\Deleted`. Returns destination
    /// UIDs when the server sends `COPYUID` (same order as `message_ids`).
    /// Empty if the server omitted it.
    async fn copy_messages(
        &self,
        folder_id: &FolderId,
        message_ids: &[MessageId],
        dest_folder_id: &FolderId,
    ) -> Result<Vec<MessageId>>;

    /// Permanently delete messages (STORE \Deleted + EXPUNGE).
    async fn delete_messages(&self, folder_id: &FolderId, message_ids: &[MessageId]) -> Result<()>;

    /// Permanently delete every message in `folder_id`. An already-empty folder is success.
    async fn empty_folder(&self, folder_id: &FolderId) -> Result<()>;

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

    /// FETCH the full RFC 822 message without marking it `\Seen` (`BODY.PEEK[]`).
    async fn fetch_raw_message(
        &self,
        folder_id: &FolderId,
        message_id: &MessageId,
    ) -> Result<Vec<u8>>;
}

fn mock_envelopes(folder_id: &FolderId, range: Range<usize>) -> Result<Vec<Envelope>> {
    let total = 100usize;
    let end = range.end.min(total);
    let start = range.start.min(end);
    let mut envelopes = Vec::new();
    // Arrival: index 0 is newest (`test-message-100`).
    for i in start..end {
        let n = total - i;
        let message_id = MessageId::new(folder_id.clone(), format!("test-message-{n}"));
        envelopes.push(Envelope {
            id: message_id.clone(),
            account_id: AccountId::new("mock-account-1"),
            folder_id: folder_id.clone(),
            subject: Some(format!("Test Message {n}")),
            from: Some(crate::models::EmailAddress::List(vec![
                crate::models::EmailAddr {
                    name: Some(format!("Sender {n}")),
                    email: Some(format!("sender{n}@example.com")),
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
            reply_to: None,
            rfc_message_id: None,
            in_reply_to: None,
            references: Vec::new(),
            date: Utc::now(),
            is_read: n.is_multiple_of(3),
            is_answered: n.is_multiple_of(7),
            is_starred: n.is_multiple_of(5),
            is_flagged: false,
            is_draft: false,
            is_deleted: false,
            has_attachments: n.is_multiple_of(2),
            size: Some(1_000 + ((n * 37) % 97) as u64 * 100),
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
                        encoding: Some("7BIT".into()),
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

/// Small RFC 822 fixture for MockConnector / UI development.
pub fn mock_rfc822() -> &'static [u8] {
    b"From: sender@example.com\r\n\
To: recipient@example.com\r\n\
Subject: Test Message\r\n\
Date: Wed, 01 Jan 2020 00:00:00 +0000\r\n\
Message-ID: <mock@example.com>\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=us-ascii\r\n\
\r\n\
Hello from MockConnector.\r\n"
}

fn mock_section_bytes(section: &str) -> Vec<u8> {
    match section {
        "" => mock_rfc822().to_vec(),
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

/// Loader / UI fixture. Not a faithful IMAP session (no sort index, no dest UIDs).
pub struct MockConnector;

impl MockConnector {
    pub fn new() -> Self {
        Self
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

    async fn authenticate(&self, _credentials: &str) -> Result<()> {
        Ok(())
    }

    async fn list_folders(&self, account_id: &AccountId) -> Result<Vec<Folder>> {
        Ok(vec![
            Folder {
                id: FolderId::new("inbox"),
                account_id: account_id.clone(),
                name: "Inbox".to_string(),
                parent_id: None,
                role: crate::MailboxRole::Inbox,
                selectable: true,
            },
            Folder {
                id: FolderId::new("sent"),
                account_id: account_id.clone(),
                name: "Sent".to_string(),
                parent_id: None,
                role: crate::MailboxRole::Sent,
                selectable: true,
            },
        ])
    }

    async fn folder_counts(
        &self,
        folder_ids: &[FolderId],
    ) -> Result<HashMap<FolderId, FolderCounts>> {
        let mut out = HashMap::new();
        for id in folder_ids {
            let unread = if id.as_str().eq_ignore_ascii_case("inbox") {
                3
            } else {
                0
            };
            out.insert(
                id.clone(),
                FolderCounts {
                    total_messages: 100,
                    unread_messages: unread,
                },
            );
        }
        Ok(out)
    }

    async fn prepare_folder_list(
        &self,
        _folder_id: &FolderId,
        sort: MessageSort,
    ) -> Result<FolderListState> {
        Ok(FolderListState {
            total: 100,
            unread: Some(3),
            sort,
            supports_size_sender: false,
        })
    }

    async fn list_envelopes_range(
        &self,
        folder_id: &FolderId,
        range: Range<usize>,
    ) -> Result<Vec<Envelope>> {
        mock_envelopes(folder_id, range)
    }

    async fn update_envelope_flags(
        &self,
        _folder_id: &FolderId,
        _message_ids: &[MessageId],
        _flags: &[(EnvelopeFlag, bool)],
    ) -> Result<()> {
        Ok(())
    }

    async fn sync_unread_sort_index(
        &self,
        _message_ids: &[MessageId],
        _now_read: bool,
    ) -> Result<Vec<(usize, usize)>> {
        Ok(Vec::new())
    }

    async fn move_messages(
        &self,
        _folder_id: &FolderId,
        message_ids: &[MessageId],
        _dest_folder_id: &FolderId,
    ) -> Result<Vec<MessageId>> {
        let _ = message_ids;
        Ok(Vec::new())
    }

    async fn copy_messages(
        &self,
        _folder_id: &FolderId,
        message_ids: &[MessageId],
        _dest_folder_id: &FolderId,
    ) -> Result<Vec<MessageId>> {
        let _ = message_ids;
        Ok(Vec::new())
    }

    async fn delete_messages(
        &self,
        _folder_id: &FolderId,
        _message_ids: &[MessageId],
    ) -> Result<()> {
        Ok(())
    }

    async fn empty_folder(&self, _folder_id: &FolderId) -> Result<()> {
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

    async fn fetch_raw_message(
        &self,
        _folder_id: &FolderId,
        _message_id: &MessageId,
    ) -> Result<Vec<u8>> {
        Ok(mock_rfc822().to_vec())
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
        content: MessageContent::Text(text.to_string()),
        created_at: now,
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::LoadedMessage;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

    #[derive(Debug)]
    struct NoopStream;

    impl AsyncRead for NoopStream {
        fn poll_read(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for NoopStream {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[test]
    fn mock_empty_folder_succeeds() {
        let connector = MockConnector::new();
        let folder = FolderId::new("trash");
        let result = futures::executor::block_on(EmailConnector::<NoopStream>::empty_folder(
            &connector, &folder,
        ));
        assert!(result.is_ok());
    }

    #[test]
    fn mock_copy_messages_keeps_source() {
        let connector = MockConnector::new();
        let from = FolderId::new("inbox");
        let to = FolderId::new("archive");
        let ids = [MessageId::new(from.clone(), "1")];
        let dest = futures::executor::block_on(EmailConnector::<NoopStream>::copy_messages(
            &connector, &from, &ids, &to,
        ))
        .expect("copy");
        assert!(dest.is_empty());
    }

    #[test]
    fn mock_fetch_raw_message_is_rfc822() {
        let connector = MockConnector::new();
        let folder = FolderId::new("inbox");
        let id = MessageId::new(folder.clone(), "1");
        let bytes = futures::executor::block_on(EmailConnector::<NoopStream>::fetch_raw_message(
            &connector, &folder, &id,
        ))
        .unwrap();
        assert_eq!(bytes, mock_rfc822());
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("Subject: Test Message"));
        assert!(text.contains("\r\n\r\n"));
        assert!(text.contains("Hello from MockConnector."));
    }

    #[test]
    fn mock_structure_has_mixed_root() {
        let s = mock_multipart_structure();
        assert_eq!(s.type_, "multipart");
        assert_eq!(s.subtype, "mixed");
        assert_eq!(s.subparts.len(), 2);
    }

    #[test]
    fn message_part_section_text() {
        let p = mock_text_part(MessageId::new(FolderId::new("inbox"), "1"), "p1", "hi");
        assert_eq!(p.section(), "TEXT");
        assert!(p.should_prefetch());
    }

    #[test]
    fn loaded_message_filters() {
        let mut parts = vec![
            mock_text_part(MessageId::new(FolderId::new("inbox"), "1"), "a", "hi"),
            mock_text_part(MessageId::new(FolderId::new("inbox"), "1"), "b", "att"),
        ];
        parts[1].is_attachment = true;
        parts[1].is_hidden = false;
        let loaded = LoadedMessage {
            envelope_id: MessageId::new(FolderId::new("inbox"), "1"),
            folder_id: FolderId::new("inbox"),
            parts,
        };
        assert_eq!(loaded.attachments().count(), 1);
        assert_eq!(loaded.content_parts().count(), 1);
    }
}
