use std::sync::Arc;
use std::time::Duration;
use std::io::{Read, Write};

use anyhow::Result;
use async_compat::Compat;
use async_imap::types::{Flag, Mailbox};
use async_imap::{Client, Session};
use async_native_tls::native_tls::TlsConnector;
use async_native_tls::TlsStream;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use tokio::net::TcpStream;
use thiserror::Error;

use mailiner_core::{
    Account, AccountId, EmailAddress, EmailConnector, Envelope, Folder, FolderId,
    MailinerError, MessageContent, MessageId, MessagePart, MessagePartId, Result as MailinerResult,
    MessageStructure,
};

mod bodystructure;
use bodystructure::{parse_bodystructure, BodystructureError};
use tokio::sync::Mutex;

#[derive(Error, Debug)]
pub enum ImapError {
    #[error("Connection error: {0}")]
    Connection(String),
    #[error("Authentication error: {0}")]
    Authentication(String),
    #[error("IMAP error: {0}")]
    Imap(String),
    #[error("Invalid data: {0}")]
    InvalidData(String),
}

impl From<ImapError> for MailinerError {
    fn from(err: ImapError) -> Self {
        match err {
            ImapError::Connection(msg) => MailinerError::Connector(msg),
            ImapError::Authentication(msg) => MailinerError::Connector(msg),
            ImapError::Imap(msg) => MailinerError::Connector(msg),
            ImapError::InvalidData(msg) => MailinerError::InvalidData(msg),
        }
    }
}

impl From<BodystructureError> for MailinerError {
    fn from(err: BodystructureError) -> Self {
        MailinerError::InvalidData(err.to_string())
    }
}

struct ImapClient {
    client: Client<TlsStream<TcpStream>>,
    session: Option<Session<TlsStream<TcpStream>>>,
}

enum ImapSession {
    Disconnected,
    Unauthenticated(Client<TlsStream<TcpStream>>),
    Authenticated(Session<TlsStream<TcpStream>>),
}

pub struct ImapConnector {
    host: String,
    port: u16,
    username: String,
    password: String,
    imap: Mutex<ImapSession>,
}

impl ImapConnector {
    pub fn new(host: String, port: u16, username: String, password: String) -> Self {
        Self {
            host,
            port,
            username,
            password,
            imap: Mutex::new(ImapSession::Disconnected),
        }
    }

    async fn ensure_connected(&mut self) -> Result<(), ImapError> {
        let mut imap = self.imap.lock().await;
        if imap == ImapSession::Disconnected {
            let tls = TlsConnector::new()
                .map_err(|e| ImapError::Connection(format!("Failed to create TLS connector: {}", e)))?;

            let tcp_stream = TcpStream::connect((self.host.as_str(), self.port))
                .await
                .map_err(|e| ImapError::Connection(format!("Failed to connect: {}", e)))?;

            let compat_tcp_stream = Compat::new(tcp_stream);

            let tls_stream = tls
                .connect(self.host.as_str(), compat_tcp_stream)
                .await
                .map_err(|e| ImapError::Connection(format!("Failed to establish TLS: {}", e)))?;

            *imap = ImapSession::Unauthenticated(Client::new(tls_stream));
        }
        Ok(())
    }

    fn parse_email_address(addr: &str) -> EmailAddress {
        if let Some((name, email)) = addr.split_once('<') {
            let name = name.trim().trim_matches('"').to_string();
            let email = email.trim_matches('>').to_string();
            EmailAddress {
                name: if name.is_empty() { None } else { Some(name) },
                email,
            }
        } else {
            EmailAddress {
                name: None,
                email: addr.to_string(),
            }
        }
    }

    fn parse_date(date_str: &str) -> DateTime<Utc> {
        chrono::DateTime::parse_from_rfc2822(date_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now())
    }

    fn parse_flags(flags: &[Flag]) -> (bool, bool, bool, bool, bool) {
        let mut is_read = false;
        let mut is_starred = false;
        let mut is_flagged = false;
        let mut is_draft = false;
        let mut is_deleted = false;

        for flag in flags {
            match flag {
                Flag::Seen => is_read = true,
                Flag::Flagged => is_flagged = true,
                Flag::Draft => is_draft = true,
                Flag::Deleted => is_deleted = true,
                Flag::Custom(name) if name == "\\Starred" => is_starred = true,
                _ => {}
            }
        }

        (is_read, is_starred, is_flagged, is_draft, is_deleted)
    }

    fn parse_folder_hierarchy(name: &str) -> (String, Option<String>) {
        let parts: Vec<&str> = name.split('/').collect();
        if parts.len() > 1 {
            let parent = parts[..parts.len() - 1].join("/");
            let name = parts.last().unwrap().to_string();
            (name, Some(parent))
        } else {
            (name.to_string(), None)
        }
    }

    async fn fetch_message_part(&self, message_id: &MessageId, part_number: &str) -> Result<Vec<u8>, ImapError> {
        let mut imap_client = self.imap.lock().await;
        let session = imap_client.as_mut().unwrap().session.as_mut().unwrap();

        session
            .select("INBOX")
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to select folder: {}", e)))?;

        let mut fetch = session
            .fetch(message_id.as_str(), &format!("(BODY.PEEK[{}])", part_number))
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to fetch message part: {}", e)))?;

        let fetch = fetch
            .next()
            .await
            .ok_or_else(|| ImapError::InvalidData("Message not found".to_string()))?
            .map_err(|e| ImapError::Imap(format!("Failed to fetch message part: {}", e)))?;

        fetch
            .body()
            .ok_or_else(|| ImapError::InvalidData("Message part not found".to_string()))
            .map(|body| body.to_vec())
    }
}

#[async_trait]
impl EmailConnector for ImapConnector {
    async fn connect(&self) -> MailinerResult<()> {
        Ok(())
    }

    async fn disconnect(&self) -> MailinerResult<()> {
        if let Some(session) = &self.session.take() {
            let mut session = session.lock().await;
            session.logout().await.map_err(|e| ImapError::Connection(format!("Failed to logout: {}", e)))?;
        }
        Ok(())
    }

    async fn authenticate(&self, credentials: &str) -> MailinerResult<Account> {
        let mut imap = self.imap.lock().await;
        let client = &imap.as_mut().unwrap().client;

       client
            .login(&self.username, credentials)
            .await
            .map_err(|(e, _)| ImapError::Authentication(format!("Failed to login: {}", e)))?;

        Ok(Account {
            id: AccountId::new(format!("imap-{}", self.username)),
            name: self.username.clone(),
            email: self.username.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }

    async fn list_folders(&self, account_id: &AccountId) -> MailinerResult<Vec<Folder>> {
        let session = self.session.as_ref().unwrap();
        let mut mailboxes = Vec::new();
        let mut list = session
            .list(Some(""), Some("*"))
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to list folders: {}", e)))?;

        while let Some(result) = list.next().await {
            let mailbox = result.map_err(|e| ImapError::Imap(format!("Failed to get mailbox: {}", e)))?;
            let name = mailbox.name().to_string();
            let attributes = mailbox.attributes().iter().map(|a| a.to_string()).collect();
            mailboxes.push(Folder {
                id: FolderId::new(name.clone()),
                account_id: account_id.clone(),
                name,
                parent_id: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            });
        }

        Ok(mailboxes)
    }

    async fn create_folder(&self, account_id: &AccountId, name: &str, parent_id: Option<&FolderId>) -> MailinerResult<Folder> {
        let session = self.session.as_ref().unwrap();
        let full_name = if let Some(parent) = parent_id {
            format!("{}/{}", parent.as_str(), name)
        } else {
            name.to_string()
        };

        session
            .create(&full_name)
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to create folder: {}", e)))?;

        Ok(Folder {
            id: FolderId::new(full_name),
            account_id: account_id.clone(),
            name: name.to_string(),
            parent_id: parent_id.cloned(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }

    async fn delete_folder(&self, folder_id: &FolderId) -> MailinerResult<()> {
        let session = self.session.as_ref().unwrap();
        session
            .delete(folder_id.as_str())
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to delete folder: {}", e)))?;

        Ok(())
    }

    async fn list_envelopes(&self, folder_id: &FolderId) -> MailinerResult<Vec<Envelope>> {
        let session = self.session.as_ref().unwrap();
        let mut imap_session = session.clone();
        imap_session
            .select(folder_id.as_str())
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to select folder: {}", e)))?;

        let mut envelopes = Vec::new();
        let mut fetch = imap_session
            .fetch("1:*", "(RFC822.HEADER FLAGS BODYSTRUCTURE)")
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to fetch messages: {}", e)))?;

        while let Some(result) = fetch.next().await {
            let fetch = result.map_err(|e| ImapError::Imap(format!("Failed to fetch message: {}", e)))?;
            let header = fetch.header().ok_or_else(|| ImapError::InvalidData("No header found".to_string()))?;
            let (is_read, is_starred, is_flagged, is_draft, is_deleted) = Self::parse_flags(&fetch.flags);
            let bodystructure = parse_bodystructure(fetch.bodystructure.unwrap_or_default())?;
            let message_structure = bodystructure.to_message_structure(&MessageId::new(fetch.message_sequence_number.to_string()));

            envelopes.push(Envelope {
                id: MessageId::new(fetch.message_sequence_number.to_string()),
                account_id: AccountId::new(self.username.clone()),
                folder_id: folder_id.clone(),
                subject: header.subject.unwrap_or_default().to_string(),
                from: Self::parse_email_address(header.from.unwrap_or_default()),
                to: header
                    .to
                    .unwrap_or_default()
                    .split(',')
                    .map(Self::parse_email_address)
                    .collect(),
                cc: header
                    .cc
                    .unwrap_or_default()
                    .split(',')
                    .map(Self::parse_email_address)
                    .collect(),
                bcc: vec![],
                date: Self::parse_date(header.date.unwrap_or_default()),
                received_at: Utc::now(),
                is_read,
                is_starred,
                is_flagged,
                is_draft,
                is_deleted,
                has_attachments: bodystructure.parts.iter().any(|p| p.is_attachment()),
                message_structure,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            });
        }

        Ok(envelopes)
    }

    async fn get_envelope(&self, message_id: &MessageId) -> MailinerResult<Envelope> {
        let session = self.session.as_ref().unwrap();
        let mut imap_session = session.clone();
        imap_session
            .select("INBOX")
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to select folder: {}", e)))?;

        let mut fetch = imap_session
            .fetch(message_id.as_str(), "(RFC822.HEADER FLAGS BODYSTRUCTURE)")
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to fetch message: {}", e)))?;

        let fetch = fetch
            .next()
            .await
            .ok_or_else(|| ImapError::InvalidData("Message not found".to_string()))?
            .map_err(|e| ImapError::Imap(format!("Failed to fetch message: {}", e)))?;

        let header = fetch
            .header()
            .ok_or_else(|| ImapError::InvalidData("Invalid message header".to_string()))?;

        let (is_read, is_starred, is_flagged, is_draft, is_deleted) = Self::parse_flags(&fetch.flags);
        let bodystructure = parse_bodystructure(fetch.bodystructure.unwrap_or_default())?;
        let message_structure = bodystructure.to_message_structure(message_id);

        Ok(Envelope {
            id: message_id.clone(),
            account_id: AccountId::new(self.username.clone()),
            folder_id: FolderId::new("INBOX"),
            subject: header.subject.unwrap_or_default().to_string(),
            from: Self::parse_email_address(header.from.unwrap_or_default()),
            to: header
                .to
                .unwrap_or_default()
                .split(',')
                .map(Self::parse_email_address)
                .collect(),
            cc: header
                .cc
                .unwrap_or_default()
                .split(',')
                .map(Self::parse_email_address)
                .collect(),
            bcc: vec![],
            date: Self::parse_date(header.date.unwrap_or_default()),
            received_at: Utc::now(),
            is_read,
            is_starred,
            is_flagged,
            is_draft,
            is_deleted,
            has_attachments: bodystructure.parts.iter().any(|p| p.is_attachment()),
            message_structure,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }

    async fn update_envelope_flags(&self, message_id: &MessageId, flags: &[(&str, bool)]) -> MailinerResult<()> {
        let session = self.session.as_ref().unwrap();
        let mut imap_session = session.clone();
        imap_session
            .select("INBOX")
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to select folder: {}", e)))?;

        for (flag, value) in flags {
            let flag = match *flag {
                "is_read" => Flag::Seen,
                "is_flagged" => Flag::Flagged,
                "is_draft" => Flag::Draft,
                "is_deleted" => Flag::Deleted,
                "is_starred" => Flag::Custom("\\Starred".into()),
                _ => return Err(ImapError::InvalidData(format!("Unknown flag: {}", flag)).into()),
            };

            if *value {
                imap_session
                    .store(message_id.as_str(), format!("+FLAGS ({:?})", flag))
                    .await
                    .map_err(|e| ImapError::Imap(format!("Failed to set flag: {}", e)))?;
            } else {
                imap_session
                    .store(message_id.as_str(), format!("-FLAGS ({:?})", flag))
                    .await
                    .map_err(|e| ImapError::Imap(format!("Failed to remove flag: {}", e)))?;
            }
        }

        Ok(())
    }

    async fn get_message_part(&self, part_id: &MessagePartId) -> MailinerResult<MessagePart> {
        let session = self.session.as_ref().unwrap();
        let mut imap_session = session.clone();
        imap_session
            .select("INBOX")
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to select folder: {}", e)))?;

        // Extract message ID and part number from part ID (format: "message_id-part_number")
        let (message_id, part_number) = part_id
            .as_str()
            .split_once('-')
            .ok_or_else(|| ImapError::InvalidData("Invalid part ID".to_string()))?;

        let mut fetch = imap_session
            .fetch(message_id, "(RFC822.HEADER FLAGS BODYSTRUCTURE)")
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to fetch message structure: {}", e)))?;

        let fetch = fetch
            .next()
            .await
            .ok_or_else(|| ImapError::InvalidData("Message not found".to_string()))?
            .map_err(|e| ImapError::Imap(format!("Failed to fetch message structure: {}", e)))?;

        let bodystructure = parse_bodystructure(fetch.bodystructure.unwrap_or_default())?;
        let message_part = bodystructure.to_message_part(&MessageId::new(message_id), part_number);

        // Fetch the actual content
        let content = Self::fetch_message_part(&MessageId::new(message_id), part_number).await?;

        Ok(MessagePart {
            content: if message_part.content_type.starts_with("text/") {
                MessageContent::Text(String::from_utf8_lossy(&content).to_string())
            } else {
                MessageContent::Binary(content)
            },
            ..message_part
        })
    }
} 