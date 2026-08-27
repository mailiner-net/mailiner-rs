mod bodystructure;
mod section_path;
mod sent;
mod sort;

pub use sent::{
    find_sent_mailbox, folders_from_listed, role_from_name, special_use_from_attrs, ListedMailbox,
};

use std::fmt::Debug;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_imap::types::Flag;
use async_imap::{Client, Session};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::{StreamExt, TryStreamExt};
use imap_proto::types::BodyStructure;
use mail_parser::{Address, HeaderValue, MessageParser};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::{client::TlsStream, TlsConnector};
use tracing::info;

use mailiner_core::{
    Account, AccountId, BodyPart, EmailAddr, EmailAddress, EmailConnector, Envelope, EnvelopeFlag,
    Folder, FolderCounts, FolderId, FolderListState, Group, MailinerError, MessageId, MessageSort,
    PartChunk, PartStream, Result as MailinerResult,
};
use std::collections::HashMap;

use tokio::sync::Mutex;

#[derive(Error, Debug)]
pub enum ImapError {
    #[error("Connection error: {0}")]
    Connection(String),
    #[error("Authentication error: {0}")]
    Authentication(String),
    #[error("TLS error: {0}")]
    Tls(String),
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
            ImapError::Authentication(msg) => MailinerError::Auth(msg),
            ImapError::Tls(msg) => MailinerError::Tls(msg),
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
    S: AsyncRead + AsyncWrite + Unpin + Debug,
{
    client: Client<TlsStream<S>>,
    session: Option<Session<TlsStream<S>>>,
}

#[derive(Debug)]
enum ImapSession<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug,
{
    Disconnected,
    Unauthenticated(Client<TlsStream<S>>),
    Authenticating,
    Authenticated(Session<TlsStream<S>>),
}

pub struct ImapConnector<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug,
{
    /// App-owned stable account id (not `imap-{username}`).
    account_id: AccountId,
    host: String,
    port: u16,
    username: String,
    /// Shared so `stream_raw_part` can hold a clone across partial FETCH chunks.
    imap: Arc<Mutex<ImapSession<S>>>,
    /// Side-cache of BODYSTRUCTURE converted to BodyPart, keyed by folder + UID.
    structure_cache: Mutex<HashMap<(FolderId, MessageId), BodyPart>>,
    /// RFC 5256 SORT advertised after LOGIN.
    has_sort: AtomicBool,
    /// Last [`prepare_folder_list`] index (UID order). Rebuilt when SELECT EXISTS changes.
    list_index: Mutex<Option<ListIndex>>,
}

struct ListIndex {
    folder: String,
    sort: MessageSort,
    /// UID order for paging. `None` only if `UID SEARCH ALL` failed (sequence fallback).
    uids: Option<Vec<u32>>,
    total: usize,
    /// `UNSEEN` count from SELECT/SEARCH; `None` if it could not be measured.
    unread: Option<usize>,
}

impl<S> ImapConnector<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    /// Create a connector. Password is **not** stored; pass it only to [`EmailConnector::authenticate`].
    pub fn new(account_id: AccountId, host: String, port: u16, username: String) -> Self {
        Self {
            account_id,
            host,
            port,
            username,
            imap: Arc::new(Mutex::new(ImapSession::Disconnected)),
            structure_cache: Mutex::new(HashMap::new()),
            has_sort: AtomicBool::new(false),
            list_index: Mutex::new(None),
        }
    }

    /// LIST all mailboxes and pick Sent (`\Sent`, else name heuristics).
    pub async fn find_sent_folder(&self) -> Result<Option<String>, ImapError> {
        let listed = self.list_all_mailboxes().await?;
        Ok(find_sent_mailbox(&listed).map(str::to_string))
    }

    /// APPEND `rfc822` to `mailbox` with `\Seen`. Does not change the selected folder.
    pub async fn append_rfc822_seen(&self, mailbox: &str, rfc822: &[u8]) -> Result<(), ImapError> {
        let mut imap = self.imap.lock().await;
        let ImapSession::Authenticated(session) = &mut *imap else {
            return Err(ImapError::NotAuthenticated);
        };
        session
            .append(mailbox, Some(r"(\Seen)"), None, rfc822)
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to APPEND to {mailbox}: {e}")))?;
        Ok(())
    }

    async fn list_all_mailboxes(&self) -> Result<Vec<ListedMailbox>, ImapError> {
        let mut imap = self.imap.lock().await;
        let ImapSession::Authenticated(session) = &mut *imap else {
            return Err(ImapError::NotAuthenticated);
        };
        let mut list = session
            .list(Some(""), Some("*"))
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to LIST folders: {e}")))?;
        let mut mailboxes = Vec::new();
        while let Some(result) = list.next().await {
            let mailbox =
                result.map_err(|e| ImapError::Imap(format!("Failed to read LIST row: {e}")))?;
            let (no_select, special_use) = special_use_from_attrs(mailbox.attributes());
            mailboxes.push(ListedMailbox {
                name: mailbox.name().to_string(),
                delimiter: mailbox.delimiter().map(str::to_string),
                no_select,
                special_use,
            });
        }
        Ok(mailboxes)
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
                    .map_err(|e| ImapError::Tls(format!("Invalid server name: {}", e)))?;

                info!("Establishing TLS connection...");
                let tls_stream = tls
                    .connect(server_name, stream)
                    .await
                    .map_err(|e| ImapError::Tls(format!("Failed to establish TLS: {}", e)))?;
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

    fn header_ids(value: &HeaderValue<'_>) -> Vec<String> {
        if let Some(list) = value.as_text_list() {
            list.iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        } else if let Some(text) = value.as_text() {
            let t = text.trim();
            if t.is_empty() {
                Vec::new()
            } else {
                vec![t.to_string()]
            }
        } else {
            Vec::new()
        }
    }

    fn parse_date(date: Option<&mail_parser::DateTime>) -> Result<DateTime<Utc>, ImapError> {
        match date {
            Some(date) => chrono::DateTime::parse_from_rfc3339(&date.to_rfc3339())
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|_| ImapError::InvalidData("Invalid date".to_string())),
            None => Ok(Utc::now()),
        }
    }

    fn envelope_from_fetch(
        account_id: &AccountId,
        folder_id: &FolderId,
        fetch: &async_imap::types::Fetch,
        structures: &mut Vec<(MessageId, BodyPart)>,
    ) -> MailinerResult<Envelope> {
        let header = fetch
            .header()
            .ok_or_else(|| ImapError::InvalidData("No header found".to_string()))?;
        let (is_read, is_starred, is_flagged, is_draft, is_deleted) =
            Self::parse_flags(fetch.flags());
        let uid = fetch
            .uid
            .ok_or_else(|| ImapError::InvalidData("No UID in FETCH response".to_string()))?;
        let parsed_headers = MessageParser::new()
            .parse_headers(header)
            .ok_or::<MailinerError>(
                ImapError::InvalidData("Failed to parse headers".to_string()).into(),
            )?;
        let mid = MessageId::new(folder_id.clone(), uid.to_string());
        let has_attachments = if let Some(bs) = fetch.bodystructure() {
            let part = bodystructure::convert_body_structure(bs);
            let has = bodystructure::structure_has_attachments(&part);
            structures.push((mid.clone(), part));
            has
        } else {
            false
        };
        Ok(Envelope {
            id: mid,
            account_id: account_id.clone(),
            folder_id: folder_id.clone(),
            subject: parsed_headers.subject().map(|s| s.to_string()),
            from: Self::parse_email_address(parsed_headers.from()),
            to: Self::parse_email_address(parsed_headers.to()),
            cc: Self::parse_email_address(parsed_headers.cc()),
            bcc: Self::parse_email_address(parsed_headers.bcc()),
            reply_to: Self::parse_email_address(parsed_headers.reply_to()),
            rfc_message_id: parsed_headers
                .message_id()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            in_reply_to: Self::header_ids(parsed_headers.in_reply_to())
                .into_iter()
                .next(),
            references: Self::header_ids(parsed_headers.references()),
            date: Self::parse_date(parsed_headers.date())?,
            is_read,
            is_starred,
            is_flagged,
            is_draft,
            is_deleted,
            has_attachments,
            size: fetch.size.map(|s| s as u64),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
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

    /// Drop cached BODYSTRUCTURE rows and the list index entries for these UIDs.
    /// Sequence fallback (no UID list) is cleared entirely — EXPUNGE shifts numbers.
    async fn forget_messages(&self, folder_id: &FolderId, message_ids: &[MessageId]) {
        {
            let mut cache = self.structure_cache.lock().await;
            for id in message_ids {
                cache.remove(&(folder_id.clone(), id.clone()));
            }
        }
        let mut slot = self.list_index.lock().await;
        let Some(idx) = slot.as_mut() else {
            return;
        };
        if idx.folder != folder_id.as_str() {
            return;
        }
        if let Some(uids) = idx.uids.as_mut() {
            let gone: std::collections::HashSet<u32> = message_ids
                .iter()
                .filter_map(|id| id.as_uid().parse().ok())
                .collect();
            uids.retain(|u| !gone.contains(u));
            idx.total = uids.len();
        } else {
            *slot = None;
        }
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
    ///
    /// Works for both full and partial (`BODY[sec]<origin>`) responses — the
    /// section path match does not filter on the origin index.
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

    /// One partial `UID FETCH … BODY.PEEK[section]<offset.length>`.
    async fn fetch_partial_chunk(
        session: &mut Session<TlsStream<S>>,
        folder_id: &str,
        message_id: &str,
        section: &str,
        offset: u64,
        length: usize,
    ) -> Result<Vec<u8>, ImapError> {
        session
            .select(folder_id)
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to select folder: {e}")))?;

        // RFC 3501 partial fetch: BODY.PEEK[section]<origin.octet-count>
        let query = format!("(BODY.PEEK[{section}]<{offset}.{length}>)");
        let mut fetch = session
            .uid_fetch(message_id, &query)
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to fetch part: {e}")))?;

        let fetch = match fetch.next().await {
            Some(Ok(f)) => f,
            Some(Err(e)) => {
                return Err(ImapError::Imap(format!("Failed to fetch part: {e}")));
            }
            None => {
                // No FETCH response — treat as end-of-part (empty / past end).
                return Ok(Vec::new());
            }
        };

        // Missing section / NIL body → EOF for this offset.
        match Self::extract_section_bytes(&fetch, section) {
            Ok(bytes) => Ok(bytes),
            Err(_) => Ok(Vec::new()),
        }
    }

    /// Partial FETCH window. Larger = fewer RTTs/SELECTs per download; smaller =
    /// lower peak memory. 512 KiB is a pragmatic default until adaptive sizing.
    const STREAM_CHUNK: usize = 512 * 1024;
    const MAX_DOWNLOAD: u64 = 100 * 1024 * 1024;

    async fn probe_sort(session: &mut Session<TlsStream<S>>, flag: &AtomicBool) {
        match session.capabilities().await {
            Ok(caps) => {
                let has = caps.has_str("SORT");
                flag.store(has, Ordering::Relaxed);
                info!("IMAP SORT capability: {has}");
            }
            Err(e) => {
                tracing::warn!("CAPABILITY failed ({e}); assuming no SORT");
                flag.store(false, Ordering::Relaxed);
            }
        }
    }

    async fn build_list_index(
        session: &mut Session<TlsStream<S>>,
        folder_id: &str,
        requested: MessageSort,
        has_sort: bool,
    ) -> Result<ListIndex, ImapError> {
        let mailbox = session
            .select(folder_id)
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to select folder: {e}")))?;
        let exists = mailbox.exists as usize;
        let sort = sort::apply_sort_or_fallback(requested, has_sort);

        if exists == 0 {
            return Ok(ListIndex {
                folder: folder_id.to_string(),
                sort,
                uids: Some(Vec::new()),
                total: 0,
                unread: Some(0),
            });
        }

        let mut unread = None;
        let uids = match sort {
            MessageSort::Arrival => Some(Self::search_arrival_uids(session).await?),
            MessageSort::Unread => {
                if has_sort {
                    let unseen = sort::uid_sort(session, "REVERSE DATE", "UNSEEN").await?;
                    unread = Some(unseen.len());
                    let seen = sort::uid_sort(session, "REVERSE DATE", "SEEN").await?;
                    let mut all = unseen;
                    all.extend(seen);
                    Some(all)
                } else {
                    let unseen = session
                        .uid_search("UNSEEN")
                        .await
                        .map_err(|e| ImapError::Imap(format!("UID SEARCH UNSEEN: {e}")))?;
                    unread = Some(unseen.len());
                    let seen = session
                        .uid_search("SEEN")
                        .await
                        .map_err(|e| ImapError::Imap(format!("UID SEARCH SEEN: {e}")))?;
                    Some(sort::unread_uid_order(unseen, seen))
                }
            }
            MessageSort::Size | MessageSort::Sender => {
                let (criteria, query) = sort::sort_command(sort).expect("size/sender have SORT");
                match sort::uid_sort(session, criteria, query).await {
                    Ok(uids) => Some(uids),
                    Err(e) => {
                        tracing::warn!("UID SORT {criteria} failed ({e}); falling back to Date");
                        let unread = Self::search_unseen_count(session).await;
                        let uids = Self::search_arrival_uids(session).await.ok();
                        let total = uids.as_ref().map(|u| u.len()).unwrap_or(exists);
                        return Ok(ListIndex {
                            folder: folder_id.to_string(),
                            sort: MessageSort::Arrival,
                            uids,
                            total,
                            unread,
                        });
                    }
                }
            }
        };

        if unread.is_none() {
            unread = Self::search_unseen_count(session).await;
        }

        let total = uids.as_ref().map(|u| u.len()).unwrap_or(exists);
        Ok(ListIndex {
            folder: folder_id.to_string(),
            sort,
            uids,
            total,
            unread,
        })
    }

    async fn search_unseen_count(session: &mut Session<TlsStream<S>>) -> Option<usize> {
        match session.uid_search("UNSEEN").await {
            Ok(set) => Some(set.len()),
            Err(e) => {
                tracing::debug!("UID SEARCH UNSEEN for folder badge: {e}");
                None
            }
        }
    }

    async fn search_arrival_uids(
        session: &mut Session<TlsStream<S>>,
    ) -> Result<Vec<u32>, ImapError> {
        let set = session
            .uid_search("ALL")
            .await
            .map_err(|e| ImapError::Imap(format!("UID SEARCH ALL: {e}")))?;
        Ok(sort::arrival_uid_order(set))
    }
}

fn imap_flag_atom(flag: EnvelopeFlag) -> &'static str {
    match flag {
        EnvelopeFlag::Read => "\\Seen",
        EnvelopeFlag::Flagged => "\\Flagged",
        EnvelopeFlag::Draft => "\\Draft",
        EnvelopeFlag::Deleted => "\\Deleted",
        EnvelopeFlag::Starred => "\\Starred",
    }
}

fn require_folder(folder_id: &FolderId, ids: &[MessageId]) -> Result<(), ImapError> {
    if ids.iter().any(|id| id.folder_id() != folder_id) {
        return Err(ImapError::InvalidData(
            "message id is not in the selected folder".into(),
        ));
    }
    Ok(())
}

fn uid_set(folder_id: &FolderId, ids: &[MessageId]) -> Result<String, ImapError> {
    if ids.is_empty() {
        return Err(ImapError::InvalidData("No message ids".into()));
    }
    require_folder(folder_id, ids)?;
    Ok(ids
        .iter()
        .map(MessageId::as_uid)
        .collect::<Vec<_>>()
        .join(","))
}

fn quote_mailbox(name: &str) -> String {
    format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
}

fn expand_uid_set(folder_id: &FolderId, members: &[imap_proto::UidSetMember]) -> Vec<MessageId> {
    let mut out = Vec::new();
    for member in members {
        match member {
            imap_proto::UidSetMember::Uid(u) => {
                out.push(MessageId::new(folder_id.clone(), u.to_string()))
            }
            imap_proto::UidSetMember::UidRange(range) => {
                for u in *range.start()..=*range.end() {
                    out.push(MessageId::new(folder_id.clone(), u.to_string()));
                }
            }
        }
    }
    out
}

fn copyuid_dest(
    folder_id: &FolderId,
    code: &Option<imap_proto::ResponseCode<'_>>,
) -> Option<Vec<MessageId>> {
    match code {
        Some(imap_proto::ResponseCode::CopyUid(_, _, dest)) => {
            Some(expand_uid_set(folder_id, dest))
        }
        _ => None,
    }
}

/// Run a tagged command and collect destination UIDs from COPYUID.
async fn run_copyuid_command<S>(
    session: &mut Session<TlsStream<S>>,
    dest_folder_id: &FolderId,
    command: &str,
) -> MailinerResult<Vec<MessageId>>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    let tag = session
        .run_command(command)
        .await
        .map_err(|e| ImapError::Imap(format!("Failed to run {command}: {e}")))?;
    let mut dest_uids = Vec::new();
    loop {
        let resp = session
            .read_response()
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to read IMAP response: {e}")))?
            .ok_or_else(|| ImapError::Imap("IMAP connection closed".into()))?;
        match resp.parsed() {
            imap_proto::Response::Data { code, .. } => {
                if let Some(uids) = copyuid_dest(dest_folder_id, code) {
                    dest_uids = uids;
                }
            }
            imap_proto::Response::Done {
                tag: done_tag,
                status,
                code,
                information,
            } if done_tag == &tag => {
                if let Some(uids) = copyuid_dest(dest_folder_id, code) {
                    dest_uids = uids;
                }
                return match status {
                    imap_proto::Status::Ok => Ok(dest_uids),
                    _ => Err(ImapError::Imap(format!(
                        "{command} failed: {}",
                        information.as_deref().unwrap_or("error")
                    ))
                    .into()),
                };
            }
            _ => {}
        }
    }
}

async fn drain_uid_store<S>(
    session: &mut Session<TlsStream<S>>,
    uids: &str,
    query: &str,
) -> MailinerResult<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    let stream = session
        .uid_store(uids, query)
        .await
        .map_err(|e| ImapError::Imap(format!("Failed to store flags: {e}")))?;
    stream
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| ImapError::Imap(format!("Failed to store flags: {e}")))?;
    Ok(())
}

async fn expunge_uids<S>(session: &mut Session<TlsStream<S>>, uids: &str) -> MailinerResult<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    let uidplus = match session.uid_expunge(uids).await {
        Ok(stream) => Some(
            stream
                .try_collect::<Vec<_>>()
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to expunge: {e}"))),
        ),
        Err(_) => None,
    };
    if let Some(result) = uidplus {
        result?;
        return Ok(());
    }

    let stream = session
        .expunge()
        .await
        .map_err(|e| ImapError::Imap(format!("Failed to expunge: {e}")))?;
    stream
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| ImapError::Imap(format!("Failed to expunge: {e}")))?;
    Ok(())
}

async fn delete_selected_uids<S>(
    session: &mut Session<TlsStream<S>>,
    uids: &str,
) -> MailinerResult<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    drain_uid_store(session, uids, "+FLAGS.SILENT (\\Deleted)").await?;
    expunge_uids(session, uids).await
}

#[async_trait]
impl<S> EmailConnector<S> for ImapConnector<S>
where
    // `'static` required so partial-fetch streams can own `Arc<Mutex<ImapSession<S>>>`.
    S: AsyncRead + AsyncWrite + Unpin + std::fmt::Debug + Send + Sync + 'static,
{
    async fn connect(&self, stream: S) -> MailinerResult<()> {
        self.ensure_connected(stream).await.map_err(|e| e.into())
    }

    async fn disconnect(&self) -> MailinerResult<()> {
        *self.list_index.lock().await = None;
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
                if let ImapSession::Authenticated(session) = &mut *imap {
                    Self::probe_sort(session, &self.has_sort).await;
                }
            } else {
                return Err(MailinerError::Connector(
                    "IMAP session in invalid state".to_string(),
                ));
            }
            Ok(Account {
                id: self.account_id.clone(),
                name: self.username.clone(),
                email: self.username.clone(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
        } else if let ImapSession::Authenticated(_) = &*imap {
            Ok(Account {
                id: self.account_id.clone(),
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
        // Full LIST (not LSUB): unsubscribed mailboxes are still selectable.
        let listed = self.list_all_mailboxes().await?;
        Ok(folders_from_listed(account_id, &listed))
    }

    async fn folder_counts(
        &self,
        folder_ids: &[FolderId],
    ) -> MailinerResult<HashMap<FolderId, FolderCounts>> {
        let mut out = HashMap::new();
        for id in folder_ids {
            let mut imap = self.imap.lock().await;
            let ImapSession::Authenticated(session) = &mut *imap else {
                return Err(ImapError::NotAuthenticated.into());
            };
            let result = session.status(id.as_str(), "(MESSAGES UNSEEN)").await;
            drop(imap);
            match result {
                Ok(mbox) => {
                    let Some(unseen) = mbox.unseen else {
                        tracing::debug!("STATUS {} omitted UNSEEN; skipping count", id.as_str());
                        continue;
                    };
                    out.insert(
                        id.clone(),
                        FolderCounts {
                            total_messages: u64::from(mbox.exists),
                            unread_messages: u64::from(unseen),
                        },
                    );
                }
                Err(e) => {
                    tracing::debug!("STATUS {} failed: {e}", id.as_str());
                }
            }
        }
        Ok(out)
    }

    async fn prepare_folder_list(
        &self,
        folder_id: &FolderId,
        sort: MessageSort,
    ) -> MailinerResult<FolderListState> {
        let has_sort = self.has_sort.load(Ordering::Relaxed);
        let mut imap = self.imap.lock().await;
        let ImapSession::Authenticated(session) = &mut *imap else {
            return Err(ImapError::NotAuthenticated.into());
        };
        let index = Self::build_list_index(session, folder_id.as_str(), sort, has_sort).await?;
        let state = FolderListState {
            total: index.total,
            unread: index.unread,
            sort: index.sort,
            supports_size_sender: has_sort,
        };
        *self.list_index.lock().await = Some(index);
        Ok(state)
    }

    async fn list_envelopes_range(
        &self,
        folder_id: &FolderId,
        range: std::ops::Range<usize>,
    ) -> MailinerResult<Vec<Envelope>> {
        let query = "(UID BODY.PEEK[HEADER] RFC822.SIZE FLAGS BODYSTRUCTURE)";
        let (envelopes, structures) = {
            let mut imap = self.imap.lock().await;
            let ImapSession::Authenticated(session) = &mut *imap else {
                return Err(ImapError::NotAuthenticated.into());
            };

            let has_sort = self.has_sort.load(Ordering::Relaxed);
            let mailbox = session
                .select(folder_id.as_str())
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to select folder: {e}")))?;
            let exists = mailbox.exists as usize;
            let mut index_slot = self.list_index.lock().await;
            let stale = index_slot
                .as_ref()
                .is_none_or(|idx| idx.folder != folder_id.as_str() || idx.total != exists);
            if stale {
                let requested = index_slot
                    .as_ref()
                    .filter(|idx| idx.folder == folder_id.as_str())
                    .map(|idx| idx.sort)
                    .unwrap_or(MessageSort::Arrival);
                *index_slot = Some(
                    Self::build_list_index(session, folder_id.as_str(), requested, has_sort)
                        .await?,
                );
            }
            let index = index_slot.as_ref().expect("index just set");
            let total = index.total;
            if total == 0 || range.start >= total || range.start >= range.end {
                return Ok(Vec::new());
            }
            let end = range.end.min(total);

            let (fetch_set, uid_order) = if let Some(uids) = &index.uids {
                let slice = &uids[range.start..end];
                let set = slice
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                (set, Some(slice.to_vec()))
            } else {
                let seq_high = total - range.start;
                let seq_low = total - end + 1;
                (format!("{seq_low}:{seq_high}"), None)
            };

            let mut envelopes = Vec::new();
            let mut structures: Vec<(MessageId, BodyPart)> = Vec::new();

            if uid_order.is_some() {
                let mut fetch = session
                    .uid_fetch(&fetch_set, query)
                    .await
                    .map_err(|e| ImapError::Imap(format!("Failed to fetch messages: {e}")))?;
                while let Some(result) = fetch.next().await {
                    let fetch = result
                        .map_err(|e| ImapError::Imap(format!("Failed to fetch message: {e}")))?;
                    envelopes.push(Self::envelope_from_fetch(
                        &self.account_id,
                        folder_id,
                        &fetch,
                        &mut structures,
                    )?);
                }
            } else {
                let mut fetch = session
                    .fetch(&fetch_set, query)
                    .await
                    .map_err(|e| ImapError::Imap(format!("Failed to fetch messages: {e}")))?;
                while let Some(result) = fetch.next().await {
                    let fetch = result
                        .map_err(|e| ImapError::Imap(format!("Failed to fetch message: {e}")))?;
                    envelopes.push(Self::envelope_from_fetch(
                        &self.account_id,
                        folder_id,
                        &fetch,
                        &mut structures,
                    )?);
                }
            }

            if let Some(order) = uid_order {
                let mut by_uid: HashMap<u32, Envelope> = envelopes
                    .into_iter()
                    .filter_map(|e| e.id.as_uid().parse::<u32>().ok().map(|u| (u, e)))
                    .collect();
                envelopes = order
                    .into_iter()
                    .filter_map(|u| by_uid.remove(&u))
                    .collect();
            } else {
                envelopes.reverse();
            }
            (envelopes, structures)
        };

        if !structures.is_empty() {
            let mut cache = self.structure_cache.lock().await;
            for (id, part) in structures {
                cache.insert((folder_id.clone(), id), part);
                if cache.len() > 500 {
                    if let Some(k) = cache.keys().next().cloned() {
                        cache.remove(&k);
                    }
                }
            }
        }
        Ok(envelopes)
    }

    async fn update_envelope_flags(
        &self,
        folder_id: &FolderId,
        message_ids: &[MessageId],
        flags: &[(EnvelopeFlag, bool)],
    ) -> MailinerResult<()> {
        if message_ids.is_empty() || flags.is_empty() {
            return Ok(());
        }
        let uids = uid_set(folder_id, message_ids)?;
        let mut imap = self.imap.lock().await;
        let ImapSession::Authenticated(session) = &mut *imap else {
            return Err(ImapError::NotAuthenticated.into());
        };

        session
            .select(folder_id.as_str())
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to select folder: {e}")))?;

        for (flag, value) in flags {
            let atom = imap_flag_atom(*flag);
            let query = if *value {
                format!("+FLAGS.SILENT ({atom})")
            } else {
                format!("-FLAGS.SILENT ({atom})")
            };
            drain_uid_store(session, &uids, &query).await?;
        }

        Ok(())
    }

    async fn sync_unread_sort_index(
        &self,
        message_ids: &[MessageId],
        now_read: bool,
    ) -> MailinerResult<Vec<(usize, usize)>> {
        let mut slot = self.list_index.lock().await;
        let Some(index) = slot.as_mut() else {
            return Ok(Vec::new());
        };
        if index.sort != MessageSort::Unread {
            return Ok(Vec::new());
        }
        let Some(uids) = index.uids.as_mut() else {
            return Ok(Vec::new());
        };
        let mut unread = index.unread.unwrap_or(0);
        let mut moves = Vec::new();
        for id in message_ids {
            let Ok(uid) = id.as_uid().parse::<u32>() else {
                continue;
            };
            if let Some(mv) = sort::move_uid_for_seen_flag(uids, &mut unread, uid, now_read) {
                moves.push(mv);
            }
        }
        index.unread = Some(unread);
        Ok(moves)
    }

    async fn move_messages(
        &self,
        folder_id: &FolderId,
        message_ids: &[MessageId],
        dest_folder_id: &FolderId,
    ) -> MailinerResult<Vec<MessageId>> {
        if message_ids.is_empty() || folder_id == dest_folder_id {
            return Ok(message_ids.to_vec());
        }
        let uids = uid_set(folder_id, message_ids)?;
        let dest = quote_mailbox(dest_folder_id.as_str());
        let mut imap = self.imap.lock().await;
        let ImapSession::Authenticated(session) = &mut *imap else {
            return Err(ImapError::NotAuthenticated.into());
        };

        session
            .select(folder_id.as_str())
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to select folder: {e}")))?;

        match run_copyuid_command(session, dest_folder_id, &format!("UID MOVE {uids} {dest}")).await
        {
            Ok(dest_uids) => {
                drop(imap);
                self.forget_messages(folder_id, message_ids).await;
                return Ok(dest_uids);
            }
            Err(_) => {}
        }

        // RFC 6851 fallback: COPY + \Deleted + EXPUNGE.
        let dest_uids =
            run_copyuid_command(session, dest_folder_id, &format!("UID COPY {uids} {dest}"))
                .await?;
        if let Err(e) = delete_selected_uids(session, &uids).await {
            return Err(MailinerError::PartialMove {
                message: e.to_string(),
                dest_ids: dest_uids,
            });
        }
        drop(imap);
        self.forget_messages(folder_id, message_ids).await;
        Ok(dest_uids)
    }

    async fn delete_messages(
        &self,
        folder_id: &FolderId,
        message_ids: &[MessageId],
    ) -> MailinerResult<()> {
        if message_ids.is_empty() {
            return Ok(());
        }
        let uids = uid_set(folder_id, message_ids)?;
        let mut imap = self.imap.lock().await;
        let ImapSession::Authenticated(session) = &mut *imap else {
            return Err(ImapError::NotAuthenticated.into());
        };

        session
            .select(folder_id.as_str())
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to select folder: {e}")))?;
        delete_selected_uids(session, &uids).await?;
        drop(imap);
        self.forget_messages(folder_id, message_ids).await;
        Ok(())
    }

    async fn get_body_structure(
        &self,
        folder_id: &FolderId,
        message_id: &MessageId,
    ) -> MailinerResult<BodyPart> {
        require_folder(folder_id, std::slice::from_ref(message_id))?;
        {
            let cache = self.structure_cache.lock().await;
            if let Some(part) = cache.get(&(folder_id.clone(), message_id.clone())) {
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
                .uid_fetch(message_id.as_uid(), "(BODYSTRUCTURE)")
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
            .insert((folder_id.clone(), message_id.clone()), part.clone());
        Ok(part)
    }

    async fn fetch_raw_parts(
        &self,
        folder_id: &FolderId,
        message_id: &MessageId,
        sections: &[String],
    ) -> MailinerResult<HashMap<String, Vec<u8>>> {
        require_folder(folder_id, std::slice::from_ref(message_id))?;
        if sections.is_empty() {
            return Ok(HashMap::new());
        }

        let query_items: Vec<String> = sections.iter().map(|s| format!("BODY.PEEK[{s}]")).collect();
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
            .uid_fetch(message_id.as_uid(), &query)
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to fetch parts: {}", e)))?;

        let fetch = fetch
            .next()
            .await
            .ok_or_else(|| ImapError::InvalidData("Message not found".to_string()))?
            .map_err(|e| ImapError::Imap(format!("Failed to fetch parts: {}", e)))?;

        let mut map = HashMap::new();
        for section in sections {
            let bytes = Self::extract_section_bytes(&fetch, section)?;
            map.insert(section.clone(), bytes);
        }
        Ok(map)
    }

    async fn stream_raw_part(
        &self,
        folder_id: &FolderId,
        message_id: &MessageId,
        section: &str,
    ) -> MailinerResult<PartStream> {
        require_folder(folder_id, std::slice::from_ref(message_id))?;
        // Fail fast if not authenticated (before returning a stream that would error later).
        {
            let imap = self.imap.lock().await;
            if !matches!(*imap, ImapSession::Authenticated(_)) {
                return Err(ImapError::NotAuthenticated.into());
            }
        }

        let total_hint = self
            .structure_cache
            .lock()
            .await
            .get(&(folder_id.clone(), message_id.clone()))
            .and_then(|root| part_size_from_structure(root, section));

        if let Some(total) = total_hint {
            if total > Self::MAX_DOWNLOAD {
                return Err(MailinerError::Connector(format!(
                    "attachment exceeds download limit ({total} > {})",
                    Self::MAX_DOWNLOAD
                )));
            }
        }

        let imap = Arc::clone(&self.imap);
        let folder_id = folder_id.as_str().to_string();
        let message_id = message_id.as_uid().to_string();
        let section = section.to_string();
        let chunk_size = Self::STREAM_CHUNK;
        let max_download = Self::MAX_DOWNLOAD;

        // Progressive partial FETCH: each poll issues
        //   UID FETCH uid (BODY.PEEK[section]<offset.chunk_size>)
        // so peak memory stays ~one chunk, not the full part. async-imap still
        // buffers each literal fully, but that literal is now only `chunk_size`.
        Ok(Box::pin(futures::stream::unfold(
            PartialFetchState {
                imap,
                folder_id,
                message_id,
                section,
                offset: 0u64,
                chunk_size,
                max_download,
                total_hint,
                done: false,
            },
            |mut state| async move {
                if state.done {
                    return None;
                }

                if state.offset >= state.max_download {
                    state.done = true;
                    return Some((
                        Err(MailinerError::Connector(format!(
                            "attachment exceeds download limit (> {})",
                            state.max_download
                        ))),
                        state,
                    ));
                }

                let remaining_cap = (state.max_download - state.offset) as usize;
                let req_len = state.chunk_size.min(remaining_cap);

                let fetch_result = {
                    let mut guard = state.imap.lock().await;
                    match &mut *guard {
                        ImapSession::Authenticated(session) => {
                            Self::fetch_partial_chunk(
                                session,
                                &state.folder_id,
                                &state.message_id,
                                &state.section,
                                state.offset,
                                req_len,
                            )
                            .await
                        }
                        _ => Err(ImapError::NotAuthenticated),
                    }
                };

                match fetch_result {
                    Ok(bytes) if bytes.is_empty() => None,
                    Ok(bytes) => {
                        let n = bytes.len() as u64;
                        state.offset = state.offset.saturating_add(n);
                        // Cap without relying solely on BODYSTRUCTURE size.
                        if state.offset > state.max_download {
                            state.done = true;
                            return Some((
                                Err(MailinerError::Connector(format!(
                                    "attachment exceeds download limit (> {})",
                                    state.max_download
                                ))),
                                state,
                            ));
                        }
                        // Short read ⇒ last chunk.
                        if bytes.len() < req_len {
                            state.done = true;
                        }
                        Some((
                            Ok(PartChunk {
                                data: bytes,
                                total_hint: state.total_hint,
                            }),
                            state,
                        ))
                    }
                    Err(e) => {
                        state.done = true;
                        Some((Err(e.into()), state))
                    }
                }
            },
        )))
    }
}

/// State machine for progressive `BODY.PEEK[section]<offset.length>` streaming.
struct PartialFetchState<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug,
{
    imap: Arc<Mutex<ImapSession<S>>>,
    folder_id: String,
    message_id: String,
    section: String,
    offset: u64,
    chunk_size: usize,
    max_download: u64,
    total_hint: Option<u64>,
    done: bool,
}

/// Look up BODYSTRUCTURE-reported octets for a section path (e.g. `"1.2"`).
fn part_size_from_structure(root: &BodyPart, section: &str) -> Option<u64> {
    if section.eq_ignore_ascii_case("TEXT") {
        return root.size;
    }
    let mut node = root;
    for seg in section.split('.') {
        let n: usize = seg.parse().ok()?;
        if n == 0 {
            return None;
        }
        node = node.subparts.get(n - 1)?;
    }
    node.size
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(size: u64) -> BodyPart {
        BodyPart {
            type_: "application".into(),
            subtype: "pdf".into(),
            size: Some(size),
            ..Default::default()
        }
    }

    #[test]
    fn part_size_text_uses_root() {
        let root = BodyPart {
            type_: "text".into(),
            subtype: "plain".into(),
            size: Some(42),
            ..Default::default()
        };
        assert_eq!(part_size_from_structure(&root, "TEXT"), Some(42));
    }

    #[test]
    fn part_size_nested_section() {
        let root = BodyPart {
            type_: "multipart".into(),
            subtype: "mixed".into(),
            subparts: vec![
                BodyPart {
                    type_: "text".into(),
                    subtype: "plain".into(),
                    size: Some(10),
                    ..Default::default()
                },
                leaf(99_000),
            ],
            ..Default::default()
        };
        assert_eq!(part_size_from_structure(&root, "2"), Some(99_000));
        assert_eq!(part_size_from_structure(&root, "1"), Some(10));
        assert_eq!(part_size_from_structure(&root, "3"), None);
    }

    #[test]
    fn imap_flag_atoms() {
        assert_eq!(imap_flag_atom(EnvelopeFlag::Read), "\\Seen");
        assert_eq!(imap_flag_atom(EnvelopeFlag::Flagged), "\\Flagged");
        assert_eq!(imap_flag_atom(EnvelopeFlag::Deleted), "\\Deleted");
        assert_eq!(imap_flag_atom(EnvelopeFlag::Starred), "\\Starred");
    }

    #[test]
    fn uid_set_joins() {
        let folder = FolderId::new("INBOX");
        let ids = [
            MessageId::new(folder.clone(), "12"),
            MessageId::new(folder.clone(), "44"),
        ];
        assert_eq!(uid_set(&folder, &ids).unwrap(), "12,44");
        assert!(uid_set(&folder, &[]).is_err());
        let mixed = [
            MessageId::new(FolderId::new("INBOX"), "12"),
            MessageId::new(FolderId::new("Sent"), "44"),
        ];
        assert!(uid_set(&FolderId::new("INBOX"), &mixed).is_err());
    }

    #[test]
    fn quote_mailbox_escapes() {
        assert_eq!(quote_mailbox("Trash"), "\"Trash\"");
        assert_eq!(quote_mailbox("Deleted Items"), "\"Deleted Items\"");
        assert_eq!(quote_mailbox(r#"foo"bar"#), r#""foo\"bar""#);
    }

    #[test]
    fn expand_uid_range() {
        let folder = FolderId::new("Sent");
        let ids = expand_uid_set(
            &folder,
            &[
                imap_proto::UidSetMember::Uid(12),
                imap_proto::UidSetMember::UidRange(20..=22),
            ],
        );
        let raw: Vec<_> = ids.iter().map(MessageId::as_uid).collect();
        assert_eq!(raw, ["12", "20", "21", "22"]);
        assert!(ids.iter().all(|id| id.folder_id() == &folder));
    }
}
