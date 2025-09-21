use std::fmt::Debug;
use std::sync::Arc;

use anyhow::Result;
use async_imap::types::Flag;
use async_imap::{Client, Session};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::{StreamExt, TryStreamExt};
use imap_proto::types::BodyStructure;
use mail_parser::{Address, MessageParser};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::{client::TlsStream, TlsConnector};

use mailiner_core::{
    Account, AccountId, EmailAddr, EmailAddress, EmailConnector, Envelope, Folder, FolderId, Group,
    MailinerError, MessageContent, MessageId, MessagePart, MessagePartId, Result as MailinerResult,
};

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
    #[error("Not authenticated")]
    NotAuthenticated,
}

impl From<ImapError> for MailinerError {
    fn from(err: ImapError) -> Self {
        match err {
            ImapError::Connection(msg) => MailinerError::Connector(msg),
            ImapError::Authentication(msg) => MailinerError::Connector(msg),
            ImapError::NotAuthenticated => {
                MailinerError::Connector("Not authenticated".to_string())
            }
            ImapError::Imap(msg) => MailinerError::Connector(msg),
            ImapError::InvalidData(msg) => MailinerError::InvalidData(msg),
        }
    }
}

struct ImapClient<S>
where
    S: AsyncRead + AsyncWrite + Unpin + std::fmt::Debug
{
    client: Client<TlsStream<S>>,
    session: Option<Session<TlsStream<S>>>,
}

#[derive(Debug)]
enum ImapSession<S>
where
    S: AsyncRead + AsyncWrite + Unpin + std::fmt::Debug
{
    Disconnected,
    Unauthenticated(Client<TlsStream<S>>),
    Authenticating,
    Authenticated(Session<TlsStream<S>>),
}

pub struct ImapConnector<S> 
where
    S: AsyncRead + AsyncWrite + Unpin + std::fmt::Debug
{
    host: String,
    port: u16,
    username: String,
    password: String,
    imap: Mutex<ImapSession<S>>,
}

impl<S> ImapConnector<S>
where
    S: AsyncRead + AsyncWrite + Unpin + std::fmt::Debug + Send + Sync,
{
    pub fn new(host: String, port: u16, username: String, password: String) -> Self {
        Self {
            host,
            port,
            username,
            password,
            imap: Mutex::new(ImapSession::Disconnected),
        }
    }

    async fn ensure_connected(&self, stream: S) -> Result<(), ImapError> {
        let mut imap = self.imap.lock().await;
        match *imap {
            ImapSession::Disconnected => {
                let root_store = RootCertStore {
                    roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
                };
                let config = ClientConfig::builder()
                    .with_root_certificates(root_store)
                    .with_no_client_auth();
                let tls = TlsConnector::from(Arc::new(config));
                let server_name = ServerName::try_from(self.host.clone())
                    .map_err(|e| ImapError::Connection(format!("Invalid server name: {}", e)))?;

                let tls_stream = tls.connect(server_name, stream).await.map_err(|e| {
                    ImapError::Connection(format!("Failed to establish TLS: {}", e))
                })?;

                *imap = ImapSession::Unauthenticated(Client::new(tls_stream));
            }
            _ => {
                // Already connected
            }
        }
        Ok(())
    }

    fn parse_email_address<'a>(addr: Option<&Address<'a>>) -> Option<EmailAddress> {
        addr.map(|addr| match addr {
            Address::Group(groups) => EmailAddress::Group(
                groups
                    .iter()
                    .map(|group| Group {
                        name: group.name.as_ref().map(|s| s.to_string()),
                        members: group
                            .addresses
                            .iter()
                            .map(|addr| EmailAddr {
                                name: addr.name.as_ref().map(|s| s.to_string()),
                                email: addr.address.as_ref().map(|s| s.to_string()),
                            })
                            .collect(),
                    })
                    .collect(),
            ),
            Address::List(list) => EmailAddress::List(
                list.iter()
                    .map(|addr| EmailAddr {
                        name: addr.name.as_ref().map(|s| s.to_string()),
                        email: addr.address.as_ref().map(|s| s.to_string()),
                    })
                    .collect(),
            ),
        })
    }

    fn parse_date(date: Option<&mail_parser::DateTime>) -> Result<DateTime<Utc>, ImapError> {
        match date {
            Some(date) => chrono::DateTime::parse_from_rfc3339(&date.to_rfc3339())
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|_| ImapError::InvalidData("Invalid date".to_string())),
            None => Ok(Utc::now()),
        }
    }

    fn parse_flags<'a>(flags: impl Iterator<Item = Flag<'a>>) -> (bool, bool, bool, bool, bool) {
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

    fn has_attachments(bodystructure: Option<&BodyStructure<'_>>) -> bool {
        match bodystructure {
            Some(BodyStructure::Basic { common, .. }) => common
                .disposition
                .as_ref()
                .map(|d| d.ty == "attachment")
                .unwrap_or(false),
            Some(BodyStructure::Text { common, .. }) => common
                .disposition
                .as_ref()
                .map(|d| d.ty == "attachment")
                .unwrap_or(false),
            Some(BodyStructure::Message { common, .. }) => common
                .disposition
                .as_ref()
                .map(|d| d.ty == "attachment")
                .unwrap_or(false),
            Some(BodyStructure::Multipart { common, bodies, .. }) => {
                common
                    .disposition
                    .as_ref()
                    .map(|d| d.ty == "attachment")
                    .unwrap_or(false)
                    || bodies.iter().any(|b| Self::has_attachments(Some(b)))
            }
            None => false,
        }
    }

    async fn fetch_message_part(
        &self,
        message_id: &MessageId,
        part_number: &MessagePartId,
    ) -> Result<Vec<u8>, ImapError> {
        let mut imap_client = self.imap.lock().await;
        if let ImapSession::Authenticated(session) = &mut *imap_client {
            session
                .select("INBOX")
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to select folder: {}", e)))?;

            let mut fetch = session
                .fetch(
                    message_id.as_str(),
                    &format!("(BODY.PEEK[{}])", part_number.as_str()),
                )
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
        } else {
            Err(ImapError::NotAuthenticated.into())
        }
    }
}

#[async_trait]
impl<S> EmailConnector<S> for ImapConnector<S>
where
    S: AsyncRead + AsyncWrite + Unpin + std::fmt::Debug + Send + Sync,
{
    async fn connect(&self, stream: S) -> MailinerResult<()>
    {
        self.ensure_connected(stream).await.map_err(|e| e.into())
    }

    async fn disconnect(&self) -> MailinerResult<()> {
        let mut imap = self.imap.lock().await;
        if let ImapSession::Authenticated(session) = &mut *imap {
            session
                .logout()
                .await
                .map_err(|e| ImapError::Connection(format!("Failed to logout: {}", e)))?;
        }
        *imap = ImapSession::Disconnected;
        Ok(())
    }

    async fn authenticate(&self, credentials: &str) -> MailinerResult<Account> {
        let mut imap = self.imap.lock().await;
        if let ImapSession::Unauthenticated(_) = &*imap {
            // Temporarily transition to Authenticating state and consume the imap session,
            // that we know is in Unauthenticated state.
            let unauth_imap = std::mem::replace(&mut *imap, ImapSession::Authenticating);
            if let ImapSession::Unauthenticated(client) = unauth_imap {
                let authenticated = client.login(&self.username, credentials).await;
                // Transition from the temporary Authenticating state to the Authenticated state.
                *imap = ImapSession::Authenticated(authenticated.map_err(|(e, _)| {
                    ImapError::Authentication(format!("Failed to login: {}", e))
                })?);
            } else {
                return Err(MailinerError::Connector(
                    "IMAP session in invalid state".to_string(),
                ));
            }
            Ok(Account {
                id: AccountId::new(format!("imap-{}", self.username)),
                name: self.username.clone(),
                email: self.username.clone(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
        } else if let ImapSession::Authenticated(_) = &*imap {
            Ok(Account {
                id: AccountId::new(format!("imap-{}", self.username)),
                name: self.username.clone(),
                email: self.username.clone(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
        } else {
            Err(ImapError::Connection("Not connected".to_string()).into())
        }
    }

    async fn list_folders(&self, account_id: &AccountId) -> MailinerResult<Vec<Folder>> {
        let mut imap = self.imap.lock().await;
        if let ImapSession::Authenticated(session) = &mut *imap {
            let mut mailboxes = Vec::new();
            let mut list = session
                .list(Some(""), Some("*"))
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to list folders: {}", e)))?;

            while let Some(result) = list.next().await {
                let mailbox =
                    result.map_err(|e| ImapError::Imap(format!("Failed to get mailbox: {}", e)))?;
                let name = mailbox.name().to_string();
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
        } else {
            Err(ImapError::NotAuthenticated.into())
        }
    }

    async fn create_folder(
        &self,
        account_id: &AccountId,
        name: &str,
        parent_id: Option<&FolderId>,
    ) -> MailinerResult<Folder> {
        let mut imap = self.imap.lock().await;
        if let ImapSession::Authenticated(session) = &mut *imap {
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
        } else {
            Err(ImapError::NotAuthenticated.into())
        }
    }

    async fn delete_folder(&self, folder_id: &FolderId) -> MailinerResult<()> {
        let mut imap = self.imap.lock().await;
        if let ImapSession::Authenticated(session) = &mut *imap {
            session
                .delete(folder_id.as_str())
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to delete folder: {}", e)))?;
            Ok(())
        } else {
            Err(ImapError::NotAuthenticated.into())
        }
    }

    async fn list_envelopes(&self, folder_id: &FolderId) -> MailinerResult<Vec<Envelope>> {
        let mut imap = self.imap.lock().await;
        if let ImapSession::Authenticated(session) = &mut *imap {
            session
                .select(folder_id.as_str())
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to select folder: {}", e)))?;

            let mut envelopes = Vec::new();
            let mut fetch = session
                .uid_fetch("1:*", "(RFC822.HEADER FLAGS BODYSTRUCTURE)")
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to fetch messages: {}", e)))?;

            while let Some(result) = fetch.next().await {
                let fetch = result
                    .map_err(|e| ImapError::Imap(format!("Failed to fetch message: {}", e)))?;
                let header = fetch
                    .header()
                    .ok_or_else(|| ImapError::InvalidData("No header found".to_string()))?;
                let (is_read, is_starred, is_flagged, is_draft, is_deleted) =
                    Self::parse_flags(fetch.flags());
                assert!(fetch.uid.is_some());

                let parser = MessageParser::new();
                let parsed_headers = parser.parse_headers(header).ok_or::<MailinerError>(
                    ImapError::InvalidData("Failed to parse headers".to_string()).into(),
                )?;

                envelopes.push(Envelope {
                    id: MessageId::new(fetch.uid.unwrap().to_string()),
                    account_id: AccountId::new(self.username.clone()),
                    folder_id: folder_id.clone(),
                    subject: parsed_headers.subject().map(|s| s.to_string()),
                    from: Self::parse_email_address(parsed_headers.from()),
                    to: Self::parse_email_address(parsed_headers.to()),
                    cc: Self::parse_email_address(parsed_headers.cc()),
                    bcc: Self::parse_email_address(parsed_headers.bcc()),
                    date: Self::parse_date(parsed_headers.date())?,
                    is_read,
                    is_starred,
                    is_flagged,
                    is_draft,
                    is_deleted,
                    has_attachments: Self::has_attachments(fetch.bodystructure()),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                });
            }

            Ok(envelopes)
        } else {
            Err(ImapError::NotAuthenticated.into())
        }
    }

    async fn get_envelope(&self, message_id: &MessageId) -> MailinerResult<Envelope> {
        let mut imap = self.imap.lock().await;
        if let ImapSession::Authenticated(session) = &mut *imap {
            session
                .select("INBOX")
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to select folder: {}", e)))?;

            let mut fetch = session
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

            let (is_read, is_starred, is_flagged, is_draft, is_deleted) =
                Self::parse_flags(fetch.flags());

            let parsed_headers =
                MessageParser::new()
                    .parse_headers(header)
                    .ok_or(ImapError::InvalidData(
                        "Failed to parse headers".to_string(),
                    ))?;

            Ok(Envelope {
                id: message_id.clone(),
                account_id: AccountId::new(self.username.clone()),
                folder_id: FolderId::new("INBOX"),
                subject: parsed_headers.subject().map(|s| s.to_string()),
                from: Self::parse_email_address(parsed_headers.from()),
                to: Self::parse_email_address(parsed_headers.to()),
                cc: Self::parse_email_address(parsed_headers.cc()),
                bcc: Self::parse_email_address(parsed_headers.bcc()),
                date: Self::parse_date(parsed_headers.date())?,
                is_read,
                is_starred,
                is_flagged,
                is_draft,
                is_deleted,
                has_attachments: Self::has_attachments(fetch.bodystructure()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
        } else {
            Err(ImapError::NotAuthenticated.into())
        }
    }

    async fn update_envelope_flags(
        &self,
        message_id: &MessageId,
        flags: &[(&str, bool)],
    ) -> MailinerResult<()> {
        let mut imap = self.imap.lock().await;
        if let ImapSession::Authenticated(session) = &mut *imap {
            session
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
                    _ => {
                        return Err(ImapError::InvalidData(format!("Unknown flag: {}", flag)).into())
                    }
                };

                let stream = if *value {
                    session
                        .store(message_id.as_str(), format!("+FLAGS ({:?})", flag))
                        .await
                        .map_err(|e| ImapError::Imap(format!("Failed to set flag: {}", e)))?
                } else {
                    session
                        .store(message_id.as_str(), format!("-FLAGS ({:?})", flag))
                        .await
                        .map_err(|e| ImapError::Imap(format!("Failed to remove flag: {}", e)))?
                };
                let _updates = stream.try_collect::<Vec<_>>().await.map_err(|e| {
                    ImapError::Imap(format!("Failed to update envelope flags: {}", e))
                })?;
            }

            Ok(())
        } else {
            Err(ImapError::NotAuthenticated.into())
        }
    }

    async fn get_message_part(
        &self,
        message_id: &MessageId,
        part_id: &MessagePartId,
    ) -> MailinerResult<MessagePart> {
        let mut imap = self.imap.lock().await;
        if let ImapSession::Authenticated(session) = &mut *imap {
            session
                .select("INBOX")
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to select folder: {}", e)))?;

            let mut fetch = session
                .fetch(message_id.as_str(), "(RFC822.HEADER FLAGS BODYSTRUCTURE)")
                .await
                .map_err(|e| {
                    ImapError::Imap(format!("Failed to fetch message structure: {}", e))
                })?;

            let fetch = fetch
                .next()
                .await
                .ok_or_else(|| ImapError::InvalidData("Message not found".to_string()))?
                .map_err(|e| {
                    ImapError::Imap(format!("Failed to fetch message structure: {}", e))
                })?;

            // Fetch the actual content
            let content = self
                .fetch_message_part(&message_id, part_id)
                .await?;

            // TODO: Parse the content
            Ok(MessagePart {
                id: part_id.clone(),
                envelope_id: message_id.clone(),
                content_type: "text/plain".to_string(),
                filename: None,
                size: content.len() as u64,
                is_attachment: false,
                content: MessageContent::Text(String::from_utf8_lossy(&content).to_string()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
        } else {
            Err(ImapError::NotAuthenticated.into())
        }
    }
}
