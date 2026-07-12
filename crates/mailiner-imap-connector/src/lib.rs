mod bodystructure;
mod section_path;

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
use tracing::info;

use std::collections::HashMap;
use mailiner_core::{
    Account, AccountId, BodyPart, EmailAddr, EmailAddress, EmailConnector, Envelope, Folder,
    FolderId, Group, MailinerError, MessageId, PartChunk, PartStream, Result as MailinerResult,
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
    S: AsyncRead + AsyncWrite + Unpin + Debug
{
    client: Client<TlsStream<S>>,
    session: Option<Session<TlsStream<S>>>,
}

#[derive(Debug)]
enum ImapSession<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug
{
    Disconnected,
    Unauthenticated(Client<TlsStream<S>>),
    Authenticating,
    Authenticated(Session<TlsStream<S>>),
}

pub struct ImapConnector<S> 
where
    S: AsyncRead + AsyncWrite + Unpin + Debug
{
    host: String,
    port: u16,
    username: String,
    password: String,
    imap: Mutex<ImapSession<S>>,
    /// Side-cache of BODYSTRUCTURE converted to BodyPart, keyed by message UID.
    structure_cache: Mutex<HashMap<MessageId, BodyPart>>,
}

impl<S> ImapConnector<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send
{
    pub fn new(host: String, port: u16, username: String, password: String) -> Self {
        Self {
            host,
            port,
            username,
            password,
            imap: Mutex::new(ImapSession::Disconnected),
            structure_cache: Mutex::new(HashMap::new()),
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

                info!("Establishing TLS connection...");
                let tls_stream = tls.connect(server_name, stream).await.map_err(|e| {
                    ImapError::Connection(format!("Failed to establish TLS: {}", e))
                })?;
                info!("TLS stream established");

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
            Some(bs) => {
                let part = bodystructure::convert_body_structure(bs);
                bodystructure::structure_has_attachments(&part)
            }
            None => false,
        }
    }

    /// Extract raw bytes for a BODY.PEEK section from a FETCH response.
    fn extract_section_bytes(
        fetch: &async_imap::types::Fetch,
        section: &str,
    ) -> Result<Vec<u8>, ImapError> {
        if section.eq_ignore_ascii_case("TEXT") {
            return fetch
                .text()
                .map(|b| b.to_vec())
                .ok_or_else(|| ImapError::InvalidData(format!("missing BODY[{section}]")));
        }
        let path = section_path::parse_section_path(section)?;
        fetch
            .section(&path)
            .map(|b| b.to_vec())
            .ok_or_else(|| ImapError::InvalidData(format!("missing BODY[{section}]")))
    }

    const STREAM_CHUNK: usize = 64 * 1024;
    const MAX_DOWNLOAD: u64 = 100 * 1024 * 1024;

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
                .lsub(Some(""), Some("*"))
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to list folders: {}", e)))?;

            while let Some(result) = list.next().await {
                let mailbox =
                    result.map_err(|e| ImapError::Imap(format!("Failed to get mailbox: {}", e)))?;
                let full_name = mailbox.name().to_string();
                let name_chunked = full_name.split(mailbox.delimiter().unwrap_or("/")).collect::<Vec<&str>>();
                mailboxes.push(Folder {
                    id: FolderId::new(mailbox.name().to_string()),
                    account_id: account_id.clone(),
                    name: name_chunked.last().unwrap_or(&mailbox.name()).to_string(),
                    parent_id: if name_chunked.len() > 1 {
                        Some(FolderId::new(name_chunked[..name_chunked.len() - 1].join(mailbox.delimiter().unwrap_or("/"))))
                    } else {
                        None
                    },
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

    async fn open_folder(&self, folder_id: &FolderId) -> MailinerResult<usize> {
        let mut imap = self.imap.lock().await;
        if let ImapSession::Authenticated(session) = &mut *imap {
            let mailbox = session
                .select(folder_id.as_str())
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to select folder: {}", e)))?;
            Ok(mailbox.exists as usize)
        } else {
            Err(ImapError::NotAuthenticated.into())
        }
    }

    async fn list_envelopes(&self, folder_id: &FolderId) -> MailinerResult<Vec<Envelope>> {
        let total = self.open_folder(folder_id).await?;
        self.list_envelopes_range(folder_id, 0..total).await
    }

    async fn list_envelopes_range(
        &self,
        folder_id: &FolderId,
        range: std::ops::Range<usize>,
    ) -> MailinerResult<Vec<Envelope>> {
        let (envelopes, structures) = {
            let mut imap = self.imap.lock().await;
            let ImapSession::Authenticated(session) = &mut *imap else {
                return Err(ImapError::NotAuthenticated.into());
            };

            let mailbox = session
                .select(folder_id.as_str())
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to select folder: {}", e)))?;
            let total = mailbox.exists as usize;

            if total == 0 || range.start >= total || range.start >= range.end {
                return Ok(Vec::new());
            }

            let end = range.end.min(total);
            let seq_high = total - range.start;
            let seq_low = total - end + 1;
            let sequence_set = format!("{}:{}", seq_low, seq_high);

            let mut fetch = session
                .fetch(&sequence_set, "(UID RFC822.HEADER FLAGS BODYSTRUCTURE)")
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to fetch messages: {}", e)))?;

            let mut envelopes = Vec::new();
            let mut structures: Vec<(MessageId, BodyPart)> = Vec::new();

            while let Some(result) = fetch.next().await {
                let fetch = result
                    .map_err(|e| ImapError::Imap(format!("Failed to fetch message: {}", e)))?;
                let header = fetch
                    .header()
                    .ok_or_else(|| ImapError::InvalidData("No header found".to_string()))?;
                let (is_read, is_starred, is_flagged, is_draft, is_deleted) =
                    Self::parse_flags(fetch.flags());
                let uid = fetch
                    .uid
                    .ok_or_else(|| ImapError::InvalidData("No UID in FETCH response".to_string()))?;

                let parser = MessageParser::new();
                let parsed_headers = parser.parse_headers(header).ok_or::<MailinerError>(
                    ImapError::InvalidData("Failed to parse headers".to_string()).into(),
                )?;

                let mid = MessageId::new(uid.to_string());
                let has_attachments = if let Some(bs) = fetch.bodystructure() {
                    let part = bodystructure::convert_body_structure(bs);
                    let has = bodystructure::structure_has_attachments(&part);
                    structures.push((mid.clone(), part));
                    has
                } else {
                    false
                };

                envelopes.push(Envelope {
                    id: mid,
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
                    has_attachments,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                });
            }

            envelopes.reverse();
            (envelopes, structures)
        }; // imap lock released

        if !structures.is_empty() {
            let mut cache = self.structure_cache.lock().await;
            for (id, part) in structures {
                cache.insert(id, part);
                if cache.len() > 500 {
                    if let Some(k) = cache.keys().next().cloned() {
                        cache.remove(&k);
                    }
                }
            }
        }
        Ok(envelopes)
    }

    async fn get_envelope(&self, message_id: &MessageId) -> MailinerResult<Envelope> {
        let (envelope, structure) = {
            let mut imap = self.imap.lock().await;
            let ImapSession::Authenticated(session) = &mut *imap else {
                return Err(ImapError::NotAuthenticated.into());
            };

            session
                .select("INBOX")
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to select folder: {}", e)))?;

            let mut fetch = session
                .uid_fetch(message_id.as_str(), "(RFC822.HEADER FLAGS BODYSTRUCTURE)")
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

            let parsed_headers = MessageParser::new()
                .parse_headers(header)
                .ok_or(ImapError::InvalidData("Failed to parse headers".to_string()))?;

            let (has_attachments, structure) = if let Some(bs) = fetch.bodystructure() {
                let part = bodystructure::convert_body_structure(bs);
                let has = bodystructure::structure_has_attachments(&part);
                (has, Some(part))
            } else {
                (false, None)
            };

            (
                Envelope {
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
                    has_attachments,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
                structure,
            )
        };

        if let Some(part) = structure {
            self.structure_cache
                .lock()
                .await
                .insert(message_id.clone(), part);
        }
        Ok(envelope)
    }

    async fn update_envelope_flags(
        &self,
        message_id: &MessageId,
        flags: &[(&str, bool)],
    ) -> MailinerResult<()> {
        let mut imap = self.imap.lock().await;
        let ImapSession::Authenticated(session) = &mut *imap else {
            return Err(ImapError::NotAuthenticated.into());
        };

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
                    .uid_store(message_id.as_str(), format!("+FLAGS ({:?})", flag))
                    .await
                    .map_err(|e| ImapError::Imap(format!("Failed to set flag: {}", e)))?
            } else {
                session
                    .uid_store(message_id.as_str(), format!("-FLAGS ({:?})", flag))
                    .await
                    .map_err(|e| ImapError::Imap(format!("Failed to remove flag: {}", e)))?
            };
            let _updates = stream.try_collect::<Vec<_>>().await.map_err(|e| {
                ImapError::Imap(format!("Failed to update envelope flags: {}", e))
            })?;
        }

        Ok(())
    }

    async fn get_body_structure(
        &self,
        folder_id: &FolderId,
        message_id: &MessageId,
    ) -> MailinerResult<BodyPart> {
        {
            let cache = self.structure_cache.lock().await;
            if let Some(part) = cache.get(message_id) {
                return Ok(part.clone());
            }
        }

        let part = {
            let mut imap = self.imap.lock().await;
            let ImapSession::Authenticated(session) = &mut *imap else {
                return Err(ImapError::NotAuthenticated.into());
            };

            session
                .select(folder_id.as_str())
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to select folder: {}", e)))?;

            let mut fetch = session
                .uid_fetch(message_id.as_str(), "(BODYSTRUCTURE)")
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to fetch BODYSTRUCTURE: {}", e)))?;

            let fetch = fetch
                .next()
                .await
                .ok_or_else(|| ImapError::InvalidData("Message not found".to_string()))?
                .map_err(|e| ImapError::Imap(format!("Failed to fetch BODYSTRUCTURE: {}", e)))?;

            let bs = fetch
                .bodystructure()
                .ok_or_else(|| ImapError::InvalidData("No BODYSTRUCTURE".to_string()))?;
            bodystructure::convert_body_structure(bs)
        };

        self.structure_cache
            .lock()
            .await
            .insert(message_id.clone(), part.clone());
        Ok(part)
    }

    async fn fetch_raw_parts(
        &self,
        folder_id: &FolderId,
        message_id: &MessageId,
        sections: &[String],
    ) -> MailinerResult<HashMap<String, Vec<u8>>> {
        if sections.is_empty() {
            return Ok(HashMap::new());
        }

        let query_items: Vec<String> = sections
            .iter()
            .map(|s| format!("BODY.PEEK[{s}]"))
            .collect();
        let query = format!("({})", query_items.join(" "));

        let mut imap = self.imap.lock().await;
        let ImapSession::Authenticated(session) = &mut *imap else {
            return Err(ImapError::NotAuthenticated.into());
        };

        session
            .select(folder_id.as_str())
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to select folder: {}", e)))?;

        let mut fetch = session
            .uid_fetch(message_id.as_str(), &query)
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to fetch parts: {}", e)))?;

        let fetch = fetch
            .next()
            .await
            .ok_or_else(|| ImapError::InvalidData("Message not found".to_string()))?
            .map_err(|e| ImapError::Imap(format!("Failed to fetch parts: {}", e)))?;

        let mut map = HashMap::new();
        for section in sections {
            match Self::extract_section_bytes(&fetch, section) {
                Ok(bytes) => {
                    map.insert(section.clone(), bytes);
                }
                Err(e) => {
                    tracing::warn!(section = %section, error = %e, "section missing in FETCH");
                }
            }
        }
        Ok(map)
    }

    async fn stream_raw_part(
        &self,
        folder_id: &FolderId,
        message_id: &MessageId,
        section: &str,
    ) -> MailinerResult<PartStream> {
        let data = {
            let mut imap = self.imap.lock().await;
            let ImapSession::Authenticated(session) = &mut *imap else {
                return Err(ImapError::NotAuthenticated.into());
            };

            session
                .select(folder_id.as_str())
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to select folder: {}", e)))?;

            let query = format!("(BODY.PEEK[{section}])");
            let mut fetch = session
                .uid_fetch(message_id.as_str(), &query)
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to fetch part: {}", e)))?;

            let fetch = fetch
                .next()
                .await
                .ok_or_else(|| ImapError::InvalidData("Message not found".to_string()))?
                .map_err(|e| ImapError::Imap(format!("Failed to fetch part: {}", e)))?;

            Self::extract_section_bytes(&fetch, section)?
        };

        let total = data.len() as u64;
        if total > Self::MAX_DOWNLOAD {
            return Err(MailinerError::Connector(format!(
                "attachment exceeds download limit ({total} > {})",
                Self::MAX_DOWNLOAD
            )));
        }

        // Yield 64 KiB frames lazily — do not pre-collect every chunk into a second Vec.
        // Note: async-imap still delivers the full BODY.PEEK literal up-front; true
        // progressive IMAP partial-fetch can replace this later behind the same API.
        let chunk_size = Self::STREAM_CHUNK;
        let data = std::sync::Arc::new(data);
        Ok(Box::pin(futures::stream::unfold(
            (data, total, 0usize, chunk_size),
            |(data, total, offset, chunk_size)| async move {
                if offset >= data.len() {
                    return None;
                }
                let end = (offset + chunk_size).min(data.len());
                let chunk = data[offset..end].to_vec();
                Some((
                    Ok(PartChunk {
                        data: chunk,
                        total_hint: Some(total),
                    }),
                    (data, total, end, chunk_size),
                ))
            },
        )))
    }
}
