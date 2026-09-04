mod auth;
mod bodystructure;
mod compress;
mod fetch_chunk;
mod quota;
mod section_path;
mod sent;
mod sort;
mod structure_cache;
mod sync;
mod tls;
mod watch;

pub use sent::{
    apply_subscriptions, find_drafts_mailbox, find_sent_mailbox, folders_from_listed,
    role_from_name, special_use_from_attrs, ListedMailbox,
};
pub use tls::{add_extra_ca_pems, parse_pem_certificates, root_cert_store};
pub use watch::{MailboxChange, MailboxWatchOutcome};

use std::fmt::Debug;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use anyhow::Result;
use async_imap::types::Flag;
use async_imap::{Client, Session};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::{StreamExt, TryStreamExt};
use imap_proto::types::BodyStructure;
use mail_parser::{Address, HeaderValue, MessageParser};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::ClientConfig;
use tokio_rustls::{client::TlsStream, TlsConnector};
use tracing::info;

use mailiner_core::{
    compile_list_search, compile_unread_sort_extra, is_inbox_mailbox, join_mailbox_path,
    join_search_keys, mailbox_parent_and_leaf, rename_mailbox_path, AccountId, AuthResults,
    BodyPart, EmailAddr, EmailAddress, EmailConnector, Envelope, EnvelopeFlag, Folder,
    FolderCounts, FolderId, FolderListState, Group, MailboxQuota, MailinerError, MessageId,
    MessageListFilter, MessageSort, PartChunk, PartStream, Result as MailinerResult, TextPrefix,
};
use std::collections::HashMap;

use structure_cache::StructureCache;
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

/// How the IMAP byte stream is wrapped before LOGIN.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImapTlsMode {
    /// Implicit TLS (typically port 993).
    #[default]
    Implicit,
    /// STARTTLS after a plaintext greeting (typically port 143).
    StartTls,
    /// No TLS. LOGIN and mail travel in the clear (including through the proxy).
    None,
}

/// Session transport after connect: rustls or leftover plaintext, plus
/// optional RFC 4978 DEFLATE after a successful `COMPRESS DEFLATE`.
#[derive(Debug)]
struct ImapIo<S> {
    inner: Option<ImapIoKind<S>>,
}

#[derive(Debug)]
enum ImapIoKind<S> {
    Tls(Box<TlsStream<S>>),
    Plain(S),
    Deflate(Box<compress::DeflateIo<ImapIoKind<S>>>),
}

impl<S> ImapIo<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn tls(stream: TlsStream<S>) -> Self {
        Self {
            inner: Some(ImapIoKind::Tls(Box::new(stream))),
        }
    }

    fn plain(stream: S) -> Self {
        Self {
            inner: Some(ImapIoKind::Plain(stream)),
        }
    }

    /// Switch the live session stream to raw DEFLATE. Call only after the
    /// tagged `COMPRESS DEFLATE` OK (that exchange is uncompressed).
    fn enable_deflate(&mut self) {
        let kind = match self.inner.take() {
            Some(ImapIoKind::Deflate(d)) => ImapIoKind::Deflate(d),
            Some(raw) => ImapIoKind::Deflate(Box::new(compress::DeflateIo::new(raw))),
            None => return,
        };
        self.inner = Some(kind);
    }

    fn kind_mut(&mut self) -> io::Result<&mut ImapIoKind<S>> {
        self.inner
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "IMAP transport closed"))
    }
}

impl<S> AsyncRead for ImapIoKind<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match &mut *self {
            Self::Tls(s) => Pin::new(&mut **s).poll_read(cx, buf),
            Self::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Self::Deflate(s) => Pin::new(&mut **s).poll_read(cx, buf),
        }
    }
}

impl<S> AsyncWrite for ImapIoKind<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &mut *self {
            Self::Tls(s) => Pin::new(&mut **s).poll_write(cx, buf),
            Self::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Self::Deflate(s) => Pin::new(&mut **s).poll_write(cx, buf),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            Self::Tls(s) => Pin::new(&mut **s).poll_flush(cx),
            Self::Plain(s) => Pin::new(s).poll_flush(cx),
            Self::Deflate(s) => Pin::new(&mut **s).poll_flush(cx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            Self::Tls(s) => Pin::new(&mut **s).poll_shutdown(cx),
            Self::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Self::Deflate(s) => Pin::new(&mut **s).poll_shutdown(cx),
        }
    }
}

impl<S> AsyncRead for ImapIo<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.kind_mut() {
            Ok(kind) => Pin::new(kind).poll_read(cx, buf),
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

impl<S> AsyncWrite for ImapIo<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.kind_mut() {
            Ok(kind) => Pin::new(kind).poll_write(cx, buf),
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.kind_mut() {
            Ok(kind) => Pin::new(kind).poll_flush(cx),
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.kind_mut() {
            Ok(kind) => Pin::new(kind).poll_shutdown(cx),
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

struct ImapClient<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug,
{
    client: Client<TlsStream<S>>,
    session: Option<Session<ImapIo<S>>>,
}

#[derive(Debug)]
enum ImapSession<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug,
{
    Disconnected,
    Unauthenticated(Client<ImapIo<S>>),
    Authenticating,
    Authenticated(Session<ImapIo<S>>),
    /// Session is owned by [`ImapConnector::watch_mailbox`].
    Watching,
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
    tls_mode: ImapTlsMode,
    /// Extra CA PEMs trusted in addition to webpki roots.
    extra_ca_pems: Vec<String>,
    /// Shared so `stream_raw_part` can hold a clone across partial FETCH chunks.
    imap: Arc<Mutex<ImapSession<S>>>,
    /// Mailbox last successfully SELECTed on this session. Chunked FETCH skips
    /// re-SELECT while this still matches the target folder.
    selected_mailbox: Arc<std::sync::Mutex<Option<String>>>,
    /// Side-cache of BODYSTRUCTURE converted to BodyPart, keyed by folder + UID.
    structure_cache: Mutex<StructureCache>,
    /// RFC 5256 SORT advertised after LOGIN.
    has_sort: AtomicBool,
    /// RFC 2087 QUOTA advertised after LOGIN.
    has_quota: AtomicBool,
    /// RFC 2177 IDLE advertised after LOGIN.
    has_idle: AtomicBool,
    /// RFC 4978 `COMPRESS DEFLATE` is active on this session.
    has_compress: AtomicBool,
    /// RFC 7162 CONDSTORE advertised after LOGIN (`QRESYNC` implies this).
    has_condstore: AtomicBool,
    /// RFC 7162 `ENABLE QRESYNC` succeeded on this session.
    has_qresync: AtomicBool,
    /// Last [`prepare_folder_list`] index (UID order). Rebuilt when SELECT EXISTS changes.
    list_index: Mutex<Option<ListIndex>>,
    /// Per-folder UIDVALIDITY + HIGHESTMODSEQ + UID set for incremental SELECT.
    folder_sync: Mutex<HashMap<String, sync::FolderSyncState>>,
}

struct ListIndex {
    folder: String,
    sort: MessageSort,
    filter: MessageListFilter,
    /// Applied mailbox search box (empty = no text criteria).
    search: String,
    /// SELECT EXISTS when this index was built (not the filtered list length).
    exists: usize,
    /// UID order for paging. `None` only if `UID SEARCH ALL` failed (sequence fallback).
    uids: Option<Vec<u32>>,
    total: usize,
    /// Unseen-prefix length for unread-first sort (may be filter-scoped).
    unread: Option<usize>,
    /// Whole-folder `UNSEEN` for the mailbox badge.
    folder_unread: Option<usize>,
    uidvalidity: Option<u32>,
    highestmodseq: Option<u64>,
}

struct FolderListReq<'a> {
    folder_id: &'a str,
    requested: MessageSort,
    filter: MessageListFilter,
    search: &'a str,
}

struct IndexBuild<'a> {
    folder_id: &'a str,
    sort: MessageSort,
    filter: MessageListFilter,
    search: &'a str,
    exists: usize,
    uidvalidity: Option<u32>,
    highestmodseq: Option<u64>,
}

impl<S> ImapConnector<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    /// Create a connector. Password is **not** stored; pass it only to [`EmailConnector::authenticate`].
    ///
    /// Call [`Self::connect`] with the transport, then authenticate. Session
    /// methods on [`EmailConnector`] do not mention the stream type.
    pub fn new(account_id: AccountId, host: String, port: u16, username: String) -> Self {
        Self {
            account_id,
            host,
            port,
            username,
            tls_mode: ImapTlsMode::Implicit,
            extra_ca_pems: Vec::new(),
            imap: Arc::new(Mutex::new(ImapSession::Disconnected)),
            selected_mailbox: Arc::new(std::sync::Mutex::new(None)),
            structure_cache: Mutex::new(StructureCache::new()),
            has_sort: AtomicBool::new(false),
            has_quota: AtomicBool::new(false),
            has_idle: AtomicBool::new(false),
            has_compress: AtomicBool::new(false),
            has_condstore: AtomicBool::new(false),
            has_qresync: AtomicBool::new(false),
            list_index: Mutex::new(None),
            folder_sync: Mutex::new(HashMap::new()),
        }
    }

    /// True when the server advertised `IDLE` after LOGIN.
    pub fn supports_idle(&self) -> bool {
        self.has_idle.load(Ordering::Relaxed)
    }

    /// True when `COMPRESS DEFLATE` is active on this session.
    pub fn supports_compress(&self) -> bool {
        self.has_compress.load(Ordering::Relaxed)
    }

    /// True when the server advertised CONDSTORE or QRESYNC after LOGIN.
    pub fn supports_condstore(&self) -> bool {
        self.has_condstore.load(Ordering::Relaxed)
    }

    /// True when `ENABLE QRESYNC` succeeded on this session.
    pub fn supports_qresync(&self) -> bool {
        self.has_qresync.load(Ordering::Relaxed)
    }

    /// Watch `folder_id` until it changes, `cancel` resolves, or `timeout` resolves.
    ///
    /// Uses IMAP IDLE when advertised; otherwise waits for `timeout` then NOOPs.
    /// The session is restored (IDLE `DONE`) before this future completes so the
    /// caller can run other commands. `timeout` must come from the app (gloo on WASM).
    pub async fn watch_mailbox<C, T>(
        &self,
        folder_id: &FolderId,
        cancel: C,
        timeout: T,
    ) -> MailinerResult<MailboxWatchOutcome>
    where
        C: std::future::Future<Output = ()>,
        T: std::future::Future<Output = ()>,
    {
        let mut imap = self.imap.lock().await;
        let session = match std::mem::replace(&mut *imap, ImapSession::Watching) {
            ImapSession::Authenticated(session) => session,
            other => {
                *imap = other;
                return Err(ImapError::NotAuthenticated.into());
            }
        };
        drop(imap);

        let use_idle = self.has_idle.load(Ordering::Relaxed);
        let finish = watch::run_watch(session, folder_id.as_str(), use_idle, cancel, timeout).await;

        match finish {
            watch::WatchFinish::Ready { session, outcome } => {
                *self.imap.lock().await = ImapSession::Authenticated(session);
                remember_selected(&self.selected_mailbox, folder_id.as_str());
                Ok(outcome)
            }
            watch::WatchFinish::Lost(err) => {
                *self.imap.lock().await = ImapSession::Disconnected;
                clear_selected(&self.selected_mailbox);
                Err(err.into())
            }
        }
    }

    /// Override the default implicit-TLS connect path.
    pub fn with_tls_mode(mut self, tls_mode: ImapTlsMode) -> Self {
        self.tls_mode = tls_mode;
        self
    }

    /// Extra CA PEMs trusted in addition to the webpki root store.
    pub fn with_extra_ca_pems(mut self, extra_ca_pems: Vec<String>) -> Self {
        self.extra_ca_pems = extra_ca_pems;
        self
    }

    /// rustls over the provided byte stream (SNI = `host`). Used after implicit
    /// TLS connect and after STARTTLS.
    pub async fn wrap_tls(&self, stream: S) -> Result<TlsStream<S>, ImapError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let root_store = root_cert_store(&self.extra_ca_pems).map_err(ImapError::Tls)?;
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
        Ok(tls_stream)
    }

    /// Speak greeting + STARTTLS on a plaintext stream. Returns the inner
    /// stream ready for rustls. Does not LOGIN.
    pub async fn starttls_handshake(&self, stream: S) -> Result<S, ImapError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Debug,
    {
        let mut client = Client::new(stream);
        client
            .read_response()
            .await
            .map_err(|e| ImapError::Connection(format!("Failed to read IMAP greeting: {e}")))?
            .ok_or_else(|| {
                ImapError::Connection("IMAP connection closed before greeting".into())
            })?;
        info!(host = %self.host, "IMAP STARTTLS");
        client
            .run_command_and_check_ok("STARTTLS", None)
            .await
            .map_err(|e| ImapError::Tls(format!("IMAP STARTTLS failed: {e}")))?;
        Ok(client.into_inner())
    }

    /// LIST all mailboxes and pick Sent (`\Sent`, else name heuristics).
    pub async fn find_sent_folder(&self) -> Result<Option<String>, ImapError> {
        let listed = self.list_all_mailboxes().await?;
        Ok(find_sent_mailbox(&listed).map(str::to_string))
    }

    /// LIST all mailboxes and pick Drafts (`\Drafts`, else name heuristics).
    pub async fn find_drafts_folder(&self) -> Result<Option<String>, ImapError> {
        let listed = self.list_all_mailboxes().await?;
        Ok(find_drafts_mailbox(&listed).map(str::to_string))
    }

    /// APPEND `rfc822` to `mailbox` with `\Seen`. Does not change the selected folder.
    pub async fn append_rfc822_seen(&self, mailbox: &str, rfc822: &[u8]) -> Result<(), ImapError> {
        self.append_rfc822_flags(mailbox, r"(\Seen)", rfc822)
            .await?;
        Ok(())
    }

    /// APPEND `rfc822` to `mailbox` with `\Draft`. Does not change the selected folder.
    ///
    /// Returns the new UID when the server sends `APPENDUID` (UIDPLUS).
    pub async fn append_rfc822_draft(
        &self,
        mailbox: &str,
        rfc822: &[u8],
    ) -> Result<Option<MessageId>, ImapError> {
        self.append_rfc822_flags(mailbox, r"(\Draft)", rfc822).await
    }

    /// APPEND `rfc822` with no flags (unread). Does not change the selected folder.
    pub async fn append_rfc822(
        &self,
        mailbox: &str,
        rfc822: &[u8],
    ) -> Result<Option<MessageId>, ImapError> {
        self.append_rfc822_flags(mailbox, "()", rfc822).await
    }

    /// APPEND `rfc822` with `flags`. Parses `APPENDUID` when the server sends it.
    async fn append_rfc822_flags(
        &self,
        mailbox: &str,
        flags: &str,
        rfc822: &[u8],
    ) -> Result<Option<MessageId>, ImapError> {
        use tokio::io::AsyncWriteExt;

        let folder_id = FolderId::new(mailbox.to_string());
        let mailbox_q = quote_mailbox(mailbox);
        let mut imap = self.imap.lock().await;
        let ImapSession::Authenticated(session) = &mut *imap else {
            return Err(ImapError::NotAuthenticated);
        };
        let tag = session
            .run_command(&format!("APPEND {mailbox_q} {flags} {{{}}}", rfc822.len()))
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to APPEND to {mailbox}: {e}")))?;
        let Some(res) = session
            .read_response()
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to read APPEND response: {e}")))?
        else {
            return Err(ImapError::Imap(
                "IMAP connection closed during APPEND".into(),
            ));
        };
        if !matches!(res.parsed(), imap_proto::Response::Continue { .. }) {
            return Err(ImapError::Imap(format!(
                "Failed to APPEND to {mailbox}: expected continuation"
            )));
        }
        session
            .as_mut()
            .write_all(rfc822)
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to write APPEND literal: {e}")))?;
        session
            .as_mut()
            .write_all(b"\r\n")
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to finish APPEND literal: {e}")))?;
        // `as_mut()` is the inner TLS/plain stream; flush that, not ImapStream.
        session
            .as_mut()
            .flush()
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to flush APPEND: {e}")))?;

        let mut new_uid = None;
        loop {
            let resp = session
                .read_response()
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to read APPEND response: {e}")))?
                .ok_or_else(|| ImapError::Imap("IMAP connection closed during APPEND".into()))?;
            match resp.parsed() {
                imap_proto::Response::Data { code, .. } => {
                    if let Some(uid) = appenduid_dest(&folder_id, code) {
                        new_uid = Some(uid);
                    }
                }
                imap_proto::Response::Done {
                    tag: done_tag,
                    status,
                    code,
                    information,
                } if done_tag == &tag => {
                    if let Some(uid) = appenduid_dest(&folder_id, code) {
                        new_uid = Some(uid);
                    }
                    return match status {
                        imap_proto::Status::Ok => Ok(new_uid),
                        _ => Err(ImapError::Imap(format!(
                            "APPEND to {mailbox} failed: {}",
                            information.as_deref().unwrap_or("error")
                        ))),
                    };
                }
                _ => {}
            }
        }
    }

    /// Hierarchy delimiter from `LIST "" ""`, else the first listed mailbox.
    async fn hierarchy_delimiter(&self) -> Result<Option<String>, ImapError> {
        {
            let mut imap = self.imap.lock().await;
            let ImapSession::Authenticated(session) = &mut *imap else {
                return Err(ImapError::NotAuthenticated);
            };
            let mut list = session
                .list(Some(""), Some(""))
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to LIST delimiter: {e}")))?;
            let mut delim = None;
            while let Some(result) = list.next().await {
                let mailbox = result
                    .map_err(|e| ImapError::Imap(format!("Failed to read LIST delimiter: {e}")))?;
                if let Some(d) = mailbox.delimiter().filter(|d| !d.is_empty()) {
                    delim = Some(d.to_string());
                }
            }
            if delim.is_some() {
                return Ok(delim);
            }
        }
        let listed = self.list_all_mailboxes().await?;
        Ok(listed
            .iter()
            .find_map(|m| m.delimiter.clone().filter(|d| !d.is_empty())))
    }

    /// Drop cached rows for `folder_id` and any children under the same delimiter.
    async fn forget_folder_tree(&self, folder_id: &FolderId, delimiter: Option<&str>) {
        {
            let mut cache = self.structure_cache.lock().await;
            cache.retain(|(fid, _)| {
                !mailbox_is_self_or_descendant(fid.as_str(), folder_id.as_str(), delimiter)
            });
        }
        let mut slot = self.list_index.lock().await;
        if slot.as_ref().is_some_and(|idx| {
            mailbox_is_self_or_descendant(&idx.folder, folder_id.as_str(), delimiter)
        }) {
            *slot = None;
        }
        self.folder_sync
            .lock()
            .await
            .retain(|name, _| !mailbox_is_self_or_descendant(name, folder_id.as_str(), delimiter));
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
                subscribed: true,
            });
        }
        Ok(mailboxes)
    }

    /// Names declared active via `LSUB`. Empty when the server has no list.
    async fn list_subscribed_names(&self) -> Result<std::collections::HashSet<String>, ImapError> {
        let mut imap = self.imap.lock().await;
        let ImapSession::Authenticated(session) = &mut *imap else {
            return Err(ImapError::NotAuthenticated);
        };
        let mut lsub = session
            .lsub(Some(""), Some("*"))
            .await
            .map_err(|e| ImapError::Imap(format!("Failed to LSUB folders: {e}")))?;
        let mut names = std::collections::HashSet::new();
        while let Some(result) = lsub.next().await {
            let mailbox =
                result.map_err(|e| ImapError::Imap(format!("Failed to read LSUB row: {e}")))?;
            names.insert(mailbox.name().to_string());
        }
        Ok(names)
    }

    /// Consume `stream` (TLS / STARTTLS / plain per [`ImapTlsMode`]) and wait for the greeting.
    pub async fn connect(&self, stream: S) -> MailinerResult<()> {
        self.ensure_connected(stream).await.map_err(Into::into)
    }

    async fn ensure_connected(&self, stream: S) -> Result<(), ImapError> {
        let mut imap = self.imap.lock().await;
        match *imap {
            ImapSession::Disconnected => {
                let io = match self.tls_mode {
                    ImapTlsMode::Implicit => ImapIo::tls(self.wrap_tls(stream).await?),
                    ImapTlsMode::StartTls => {
                        let plain = self.starttls_handshake(stream).await?;
                        ImapIo::tls(self.wrap_tls(plain).await?)
                    }
                    ImapTlsMode::None => {
                        info!(host = %self.host, "IMAP plaintext");
                        ImapIo::plain(stream)
                    }
                };
                *imap = ImapSession::Unauthenticated(Client::new(io));
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
        let flags = parse_flags(fetch.flags());
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
            is_read: flags.is_read,
            is_answered: flags.is_answered,
            is_starred: flags.is_starred,
            is_flagged: flags.is_flagged,
            is_draft: flags.is_draft,
            is_deleted: flags.is_deleted,
            keywords: flags.keywords,
            has_attachments,
            size: fetch.size.map(|s| s as u64),
            snippet: None,
            auth_results: AuthResults::from_header_bytes(header),
        })
    }

    /// Drop cached BODYSTRUCTURE rows and the list index for this folder.
    async fn forget_folder(&self, folder_id: &FolderId) {
        {
            let mut cache = self.structure_cache.lock().await;
            cache.retain(|(fid, _)| fid != folder_id);
        }
        let mut slot = self.list_index.lock().await;
        if slot
            .as_ref()
            .is_some_and(|idx| idx.folder == folder_id.as_str())
        {
            *slot = None;
        }
        self.folder_sync.lock().await.remove(folder_id.as_str());
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
            if let Some(state) = self.folder_sync.lock().await.get_mut(folder_id.as_str()) {
                state.uids.retain(|u| !gone.contains(u));
                state.exists = state.exists.saturating_sub(gone.len());
            }
        } else {
            *slot = None;
            self.folder_sync.lock().await.remove(folder_id.as_str());
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
    /// Empty `section` is the full message (`BODY[]` / `RFC822`).
    fn extract_section_bytes(
        fetch: &async_imap::types::Fetch,
        section: &str,
    ) -> Result<Vec<u8>, ImapError> {
        if section.is_empty() {
            return fetch
                .body()
                .map(|b| b.to_vec())
                .ok_or_else(|| ImapError::InvalidData("missing BODY[]".into()));
        }
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
        session: &mut Session<ImapIo<S>>,
        selected: &std::sync::Mutex<Option<String>>,
        folder_id: &str,
        message_id: &str,
        section: &str,
        offset: u64,
        length: usize,
    ) -> Result<Vec<u8>, ImapError> {
        // Keep the mailbox selected across chunks; only SELECT when needed.
        ensure_mailbox_selected(session, selected, folder_id).await?;

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

    const MAX_DOWNLOAD: u64 = 100 * 1024 * 1024;

    async fn stream_raw_part_chunked(
        &self,
        folder_id: &FolderId,
        message_id: &MessageId,
        section: &str,
        chunk_size: usize,
    ) -> MailinerResult<PartStream>
    where
        S: Sync + 'static,
    {
        self.stream_raw_part_inner(folder_id, message_id, section, Some(chunk_size))
            .await
    }

    async fn stream_raw_part_inner(
        &self,
        folder_id: &FolderId,
        message_id: &MessageId,
        section: &str,
        fixed_chunk: Option<usize>,
    ) -> MailinerResult<PartStream>
    where
        S: Sync + 'static,
    {
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
        let selected = Arc::clone(&self.selected_mailbox);
        let folder_id = folder_id.as_str().to_string();
        let message_id = message_id.as_uid().to_string();
        let section = section.to_string();
        let max_download = Self::MAX_DOWNLOAD;
        let (chunk_size, sizer) = match fixed_chunk {
            Some(n) => (n, None),
            None => {
                let sizer = fetch_chunk::FetchChunkSizer::new();
                (sizer.size(), Some(sizer))
            }
        };

        // Progressive partial FETCH: each poll issues
        //   UID FETCH uid (BODY.PEEK[section]<offset.chunk_size>)
        // so peak memory stays ~one chunk, not the full part. async-imap still
        // buffers each literal fully, but that literal is now only `chunk_size`.
        // Production streams start small and grow/shrink from observed FETCH time.
        // SELECT is issued only when this session is not already on `folder_id`.
        Ok(Box::pin(futures::stream::unfold(
            PartialFetchState {
                imap,
                selected,
                folder_id,
                message_id,
                section,
                offset: 0u64,
                chunk_size,
                sizer,
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
                    if state.offset > state.max_download {
                        return Some((
                            Err(MailinerError::Connector(format!(
                                "attachment exceeds download limit (> {})",
                                state.max_download
                            ))),
                            state,
                        ));
                    }
                    // Exact limit: probe one extra byte so MAX-sized parts succeed
                    // and MAX+1 is still rejected.
                    let probe = {
                        let mut guard = state.imap.lock().await;
                        match &mut *guard {
                            ImapSession::Authenticated(session) => {
                                Self::fetch_partial_chunk(
                                    session,
                                    &state.selected,
                                    &state.folder_id,
                                    &state.message_id,
                                    &state.section,
                                    state.offset,
                                    1,
                                )
                                .await
                            }
                            _ => Err(ImapError::NotAuthenticated),
                        }
                    };
                    return match probe {
                        Ok(bytes) if bytes.is_empty() => None,
                        Ok(_) => Some((
                            Err(MailinerError::Connector(format!(
                                "attachment exceeds download limit (> {})",
                                state.max_download
                            ))),
                            state,
                        )),
                        Err(e) => Some((Err(e.into()), state)),
                    };
                }

                let remaining_cap = (state.max_download - state.offset) as usize;
                let req_len = state.chunk_size.min(remaining_cap);

                let started = fetch_chunk::fetch_now();
                let fetch_result = {
                    let mut guard = state.imap.lock().await;
                    match &mut *guard {
                        ImapSession::Authenticated(session) => {
                            Self::fetch_partial_chunk(
                                session,
                                &state.selected,
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
                        // Short/EOF windows are not a full sample of this size.
                        if bytes.len() == req_len {
                            if let Some(sizer) = state.sizer.as_mut() {
                                let prev = sizer.size();
                                sizer.record(bytes.len(), started.elapsed());
                                let next = sizer.size();
                                if next != prev {
                                    tracing::debug!(
                                        from = prev,
                                        to = next,
                                        bytes = bytes.len(),
                                        "adaptive FETCH chunk"
                                    );
                                }
                                state.chunk_size = next;
                            }
                        }
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

    async fn probe_capabilities(
        session: &mut Session<ImapIo<S>>,
        has_sort: &AtomicBool,
        has_quota: &AtomicBool,
        has_idle: &AtomicBool,
        has_condstore: &AtomicBool,
        has_qresync: &AtomicBool,
    ) -> bool {
        match session.capabilities().await {
            Ok(caps) => {
                let sort = caps.has_str("SORT");
                let quota = caps.has_str("QUOTA");
                let idle = caps.has_str("IDLE");
                let compress = compress::advertises_deflate(&caps);
                let (condstore, qresync) = sync::sync_caps_from(&caps);
                has_sort.store(sort, Ordering::Relaxed);
                has_quota.store(quota, Ordering::Relaxed);
                has_idle.store(idle, Ordering::Relaxed);
                has_condstore.store(condstore, Ordering::Relaxed);
                has_qresync.store(qresync, Ordering::Relaxed);
                info!(
                    "IMAP capabilities: SORT={sort} QUOTA={quota} IDLE={idle} COMPRESS={compress} CONDSTORE={condstore} QRESYNC={qresync}"
                );
                compress
            }
            Err(e) => {
                tracing::warn!(
                    "CAPABILITY failed ({e}); assuming no SORT/QUOTA/IDLE/COMPRESS/CONDSTORE"
                );
                has_sort.store(false, Ordering::Relaxed);
                has_quota.store(false, Ordering::Relaxed);
                has_idle.store(false, Ordering::Relaxed);
                has_condstore.store(false, Ordering::Relaxed);
                has_qresync.store(false, Ordering::Relaxed);
                false
            }
        }
    }

    /// RFC 4978: issue `COMPRESS DEFLATE` when advertised. On `NO`/`BAD`, keep
    /// the uncompressed session (servers may refuse under load).
    async fn maybe_enable_compress(session: &mut Session<ImapIo<S>>, has_compress: &AtomicBool) {
        match session.run_command_and_check_ok("COMPRESS DEFLATE").await {
            Ok(()) => {
                session.as_mut().enable_deflate();
                has_compress.store(true, Ordering::Relaxed);
                info!("IMAP COMPRESS=DEFLATE enabled");
            }
            Err(e) => {
                has_compress.store(false, Ordering::Relaxed);
                tracing::warn!("COMPRESS DEFLATE failed ({e}); continuing uncompressed");
            }
        }
    }

    async fn build_list_index(
        session: &mut Session<ImapIo<S>>,
        selected: &std::sync::Mutex<Option<String>>,
        req: &FolderListReq<'_>,
        has_sort: bool,
        caps: sync::SyncCaps,
        prior: Option<&sync::FolderSyncState>,
    ) -> Result<ListIndex, ImapError> {
        let sort = sort::apply_sort_or_fallback(req.requested, has_sort);
        let mode = select_mode_for(caps, prior, req.folder_id, sort, req.filter, req.search);
        let outcome = match sync::select_sync(session, req.folder_id, mode).await {
            Ok(outcome) => outcome,
            Err(e) if caps.qresync && prior.is_some() => {
                tracing::warn!("QRESYNC SELECT failed ({e}); retrying without QRESYNC");
                let fallback = if caps.condstore {
                    sync::SelectMode::Condstore
                } else {
                    sync::SelectMode::Plain
                };
                sync::select_sync(session, req.folder_id, fallback).await?
            }
            Err(e) => return Err(e),
        };
        remember_selected(selected, req.folder_id);
        let mailbox = &outcome.mailbox;
        let build = IndexBuild {
            folder_id: req.folder_id,
            sort,
            filter: req.filter,
            search: req.search,
            exists: mailbox.exists as usize,
            uidvalidity: mailbox.uid_validity,
            highestmodseq: mailbox.highest_modseq,
        };

        if build.exists == 0 {
            return Ok(ListIndex {
                folder: build.folder_id.to_string(),
                sort: build.sort,
                filter: build.filter,
                search: build.search.to_string(),
                exists: 0,
                uids: Some(Vec::new()),
                total: 0,
                unread: Some(0),
                folder_unread: Some(0),
                uidvalidity: build.uidvalidity,
                highestmodseq: build.highestmodseq,
            });
        }

        if let Some(index) =
            Self::try_incremental_index(session, &build, prior, &outcome, caps).await?
        {
            return Ok(index);
        }

        let compiled = compile_list_search(build.filter, build.search);
        let search_q = compiled.uid_search_query();
        let sort_q = compiled.sort_query();

        let mut unread = None;
        let uids = match build.sort {
            MessageSort::Arrival => Some(Self::search_uids(session, &search_q).await?),
            MessageSort::Date => {
                Some(Self::search_date_uids(session, has_sort, &search_q, sort_q).await?)
            }
            MessageSort::Unread => {
                let extra = compile_unread_sort_extra(build.search);
                let parsed = mailiner_core::MailboxSearch::parse(build.search);
                let flagged = build.filter.flagged || parsed.has_flagged();
                let drop_seen = build.filter.unread || parsed.has_unread();
                let unseen_q = join_search_keys(&[
                    "UNSEEN",
                    if flagged { "FLAGGED" } else { "" },
                    extra.sort_query(),
                ]);
                // Unread filter / is:unread drops the seen group (the list is unseen-only).
                let seen_q = if drop_seen {
                    None
                } else {
                    Some(join_search_keys(&[
                        "SEEN",
                        if flagged { "FLAGGED" } else { "" },
                        extra.sort_query(),
                    ]))
                };
                let unseen_search = if extra.needs_utf8 {
                    format!("CHARSET UTF-8 {unseen_q}")
                } else {
                    unseen_q.clone()
                };
                let seen_search = seen_q.as_ref().map(|q| {
                    if extra.needs_utf8 {
                        format!("CHARSET UTF-8 {q}")
                    } else {
                        q.clone()
                    }
                });
                if has_sort {
                    let unseen = sort::uid_sort(session, "REVERSE DATE", &unseen_q).await?;
                    unread = Some(unseen.len());
                    let mut all = unseen;
                    if let Some(seen_q) = seen_q.as_deref() {
                        let seen = sort::uid_sort(session, "REVERSE DATE", seen_q).await?;
                        all.extend(seen);
                    }
                    Some(all)
                } else {
                    let unseen = session
                        .uid_search(&unseen_search)
                        .await
                        .map_err(|e| ImapError::Imap(format!("UID SEARCH {unseen_search}: {e}")))?;
                    unread = Some(unseen.len());
                    let seen = if let Some(seen_search) = seen_search.as_deref() {
                        session.uid_search(seen_search).await.map_err(|e| {
                            ImapError::Imap(format!("UID SEARCH {seen_search}: {e}"))
                        })?
                    } else {
                        Default::default()
                    };
                    Some(sort::unread_uid_order(unseen, seen))
                }
            }
            MessageSort::Size | MessageSort::Sender => {
                let criteria = sort::sort_criteria(build.sort).expect("size/sender have SORT");
                match sort::uid_sort(session, criteria, sort_q).await {
                    Ok(uids) => Some(uids),
                    Err(e) => {
                        tracing::warn!("UID SORT {criteria} failed ({e}); falling back to Arrival");
                        let folder_unread = Self::search_unseen_count(session).await;
                        let uids = Self::search_uids(session, &search_q).await?;
                        return Ok(ListIndex {
                            folder: build.folder_id.to_string(),
                            sort: MessageSort::Arrival,
                            filter: build.filter,
                            search: build.search.to_string(),
                            exists: build.exists,
                            uids: Some(uids.clone()),
                            total: uids.len(),
                            unread: folder_unread,
                            folder_unread,
                            uidvalidity: build.uidvalidity,
                            highestmodseq: build.highestmodseq,
                        });
                    }
                }
            }
        };

        let folder_unread = Self::search_unseen_count(session).await;
        if unread.is_none() {
            unread = folder_unread;
        }

        let total = uids.as_ref().map(|u| u.len()).unwrap_or(build.exists);
        Ok(ListIndex {
            folder: build.folder_id.to_string(),
            sort: build.sort,
            filter: build.filter,
            search: build.search.to_string(),
            exists: build.exists,
            uids,
            total,
            unread,
            folder_unread,
            uidvalidity: build.uidvalidity,
            highestmodseq: build.highestmodseq,
        })
    }

    async fn try_incremental_index(
        session: &mut Session<ImapIo<S>>,
        build: &IndexBuild<'_>,
        prior: Option<&sync::FolderSyncState>,
        outcome: &sync::SelectOutcome,
        caps: sync::SyncCaps,
    ) -> Result<Option<ListIndex>, ImapError> {
        if !caps.condstore {
            return Ok(None);
        }
        let Some(prior) = prior else {
            return Ok(None);
        };
        let Some(uv) = build.uidvalidity else {
            return Ok(None);
        };
        if prior.uidvalidity != uv {
            return Ok(None);
        }
        if !prior.can_refresh(build.folder_id, build.sort, build.filter, build.search) {
            return Ok(None);
        }

        let mut updates = outcome.flag_updates.clone();
        let vanished = outcome.vanished.clone();
        if !outcome.from_qresync {
            match sync::fetch_changed_since(session, prior.highestmodseq).await {
                Ok(changed) => updates.extend(changed),
                Err(e) => {
                    tracing::warn!("CHANGEDSINCE failed ({e}); falling back to SEARCH ALL");
                    return Ok(None);
                }
            }
        }

        let prior_set: std::collections::HashSet<u32> = prior.uids.iter().copied().collect();
        let vanished_set: std::collections::HashSet<u32> = vanished.iter().copied().collect();
        let new_from_updates: Vec<u32> = updates
            .iter()
            .map(|u| u.uid)
            .filter(|u| !prior_set.contains(u) && !vanished_set.contains(u))
            .collect();
        let vanished_known = prior
            .uids
            .iter()
            .filter(|u| vanished_set.contains(u))
            .count();

        let mut extra_new = Vec::new();
        match sync::exists_gap(
            prior.uids.len(),
            vanished_known,
            new_from_updates.len(),
            build.exists,
        ) {
            sync::ExistsGap::Match => {}
            sync::ExistsGap::MissingExpunge => {
                match sync::search_uid_set(session, &prior.uids).await {
                    Ok(still) => {
                        let gone: Vec<u32> = prior
                            .uids
                            .iter()
                            .copied()
                            .filter(|u| !still.contains(u) && !vanished_set.contains(u))
                            .collect();
                        let mut vanished = vanished;
                        vanished.extend(gone);
                        return Self::finish_incremental(
                            session,
                            build,
                            prior,
                            &vanished,
                            &updates,
                            &[],
                        )
                        .await;
                    }
                    Err(e) => {
                        tracing::warn!("UID SEARCH for expunges failed ({e}); full rebuild");
                        return Ok(None);
                    }
                }
            }
            sync::ExistsGap::MissingNew => {
                let from = prior
                    .uids
                    .iter()
                    .copied()
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1);
                match sync::search_uids_from(session, from).await {
                    Ok(found) => extra_new = found,
                    Err(e) => {
                        tracing::warn!("UID SEARCH for new UIDs failed ({e}); full rebuild");
                        return Ok(None);
                    }
                }
            }
        }

        Self::finish_incremental(session, build, prior, &vanished, &updates, &extra_new).await
    }

    async fn finish_incremental(
        session: &mut Session<ImapIo<S>>,
        build: &IndexBuild<'_>,
        prior: &sync::FolderSyncState,
        vanished: &[u32],
        updates: &[sync::FlagUpdate],
        extra_new: &[u32],
    ) -> Result<Option<ListIndex>, ImapError> {
        let Some(uids) = sync::merge_uid_list(prior, build.exists, vanished, updates, extra_new)
        else {
            return Ok(None);
        };
        let folder_unread = Self::search_unseen_count(session).await;
        let unread = match build.sort {
            MessageSort::Unread => {
                let vanished: std::collections::HashSet<u32> = vanished.iter().copied().collect();
                let mut unread = prior.unread.unwrap_or(0);
                unread = unread.saturating_sub(
                    prior
                        .uids
                        .iter()
                        .take(prior.unread.unwrap_or(0))
                        .filter(|u| vanished.contains(u))
                        .count(),
                );
                Some(unread.min(uids.len()))
            }
            _ => folder_unread,
        };
        Ok(Some(ListIndex {
            folder: build.folder_id.to_string(),
            sort: build.sort,
            filter: build.filter,
            search: build.search.to_string(),
            exists: build.exists,
            total: uids.len(),
            uids: Some(uids),
            unread,
            folder_unread,
            uidvalidity: build.uidvalidity,
            highestmodseq: build.highestmodseq,
        }))
    }

    async fn search_unseen_count(session: &mut Session<ImapIo<S>>) -> Option<usize> {
        match session.uid_search("UNSEEN").await {
            Ok(set) => Some(set.len()),
            Err(e) => {
                tracing::debug!("UID SEARCH UNSEEN for folder badge: {e}");
                None
            }
        }
    }

    async fn search_uids(
        session: &mut Session<ImapIo<S>>,
        query: &str,
    ) -> Result<Vec<u32>, ImapError> {
        let set = session
            .uid_search(query)
            .await
            .map_err(|e| ImapError::Imap(format!("UID SEARCH {query}: {e}")))?;
        Ok(sort::arrival_uid_order(set))
    }

    /// RFC 5322 Date header via `UID SORT REVERSE DATE` when `SORT` is advertised.
    ///
    /// Fallback without `SORT` (or if the command fails): arrival/UID order.
    /// The fetched page is not re-sorted — virtualized indices must stay stable
    /// for the whole mailbox, not just the current window.
    async fn search_date_uids(
        session: &mut Session<ImapIo<S>>,
        has_sort: bool,
        search_q: &str,
        sort_q: &str,
    ) -> Result<Vec<u32>, ImapError> {
        if has_sort {
            let (criteria, _) =
                sort::sort_command(MessageSort::Date).expect("Date has SORT criteria");
            match sort::uid_sort(session, criteria, sort_q).await {
                Ok(uids) => return Ok(uids),
                Err(e) => {
                    tracing::warn!(
                        "UID SORT {criteria} failed ({e}); falling back to arrival/UID order"
                    );
                }
            }
        }
        Self::search_uids(session, search_q).await
    }
}

/// UID set covering every message in the selected mailbox (`UID STORE 1:*`).
const ALL_UIDS: &str = "1:*";

fn imap_flag_atom(flag: EnvelopeFlag) -> &'static str {
    match flag {
        EnvelopeFlag::Read => "\\Seen",
        EnvelopeFlag::Answered => "\\Answered",
        EnvelopeFlag::Flagged => "\\Flagged",
        EnvelopeFlag::Draft => "\\Draft",
        EnvelopeFlag::Deleted => "\\Deleted",
        EnvelopeFlag::Starred => "\\Starred",
        EnvelopeFlag::Keyword(keyword) => keyword.atom(),
    }
}

async fn drop_mismatched_search_uids(
    list_index: &Mutex<Option<ListIndex>>,
    message_ids: &[MessageId],
    flags: &[(EnvelopeFlag, bool)],
) {
    let mut slot = list_index.lock().await;
    let Some(index) = slot.as_mut() else {
        return;
    };
    let parsed = mailiner_core::MailboxSearch::parse(&index.search);
    let drop = flags.iter().any(|(flag, value)| match flag {
        EnvelopeFlag::Read => parsed.drops_on_read_change(index.filter, *value),
        EnvelopeFlag::Flagged => parsed.drops_on_flagged_change(index.filter, *value),
        _ => false,
    });
    if !drop {
        return;
    }
    let Some(uids) = index.uids.as_mut() else {
        return;
    };
    for id in message_ids {
        let Ok(uid) = id.as_uid().parse::<u32>() else {
            continue;
        };
        if let Some(from) = uids.iter().position(|&u| u == uid) {
            uids.remove(from);
            if index.unread.is_some_and(|n| from < n) {
                index.unread = Some(index.unread.unwrap_or(0).saturating_sub(1));
            }
        }
    }
    let total = uids.len();
    index.total = total;
}

struct ParsedFlags {
    is_read: bool,
    is_answered: bool,
    is_starred: bool,
    is_flagged: bool,
    is_draft: bool,
    is_deleted: bool,
    keywords: Vec<String>,
}

pub(crate) fn parse_flags<'a>(flags: impl Iterator<Item = Flag<'a>>) -> ParsedFlags {
    let mut parsed = ParsedFlags {
        is_read: false,
        is_answered: false,
        is_starred: false,
        is_flagged: false,
        is_draft: false,
        is_deleted: false,
        keywords: Vec::new(),
    };

    for flag in flags {
        match flag {
            Flag::Seen => parsed.is_read = true,
            Flag::Answered => parsed.is_answered = true,
            Flag::Flagged => parsed.is_flagged = true,
            Flag::Draft => parsed.is_draft = true,
            Flag::Deleted => parsed.is_deleted = true,
            Flag::Custom(name) if name == "\\Starred" => parsed.is_starred = true,
            Flag::Custom(name)
                if is_imap_keyword(&name)
                    && !parsed
                        .keywords
                        .iter()
                        .any(|existing| existing == name.as_ref()) =>
            {
                parsed.keywords.push(name.into_owned());
            }
            _ => {}
        }
    }

    parsed
}

/// Keywords are atoms that do not start with `\`.
fn is_imap_keyword(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('\\')
}

struct SnippetPlan {
    id: MessageId,
    section: String,
    encoding: String,
    content_type: String,
    charset: Option<String>,
    is_html: bool,
}

fn snippet_plan(id: &MessageId, root: &BodyPart) -> Option<SnippetPlan> {
    let leaf = bodystructure::first_preview_text(root)?;
    Some(SnippetPlan {
        id: id.clone(),
        section: leaf.section,
        encoding: leaf.part.encoding.clone().unwrap_or_else(|| "7BIT".into()),
        content_type: leaf.part.content_type(),
        charset: leaf.part.charset().map(str::to_string),
        is_html: leaf.part.subtype == "html",
    })
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

pub(crate) fn quote_mailbox(name: &str) -> String {
    format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
}

fn select_mode_for<'a>(
    caps: sync::SyncCaps,
    prior: Option<&'a sync::FolderSyncState>,
    folder_id: &str,
    sort: MessageSort,
    filter: MessageListFilter,
    search: &str,
) -> sync::SelectMode<'a> {
    if caps.qresync {
        if let Some(prior) = prior {
            if prior.can_refresh(folder_id, sort, filter, search) {
                return sync::SelectMode::Qresync {
                    uidvalidity: prior.uidvalidity,
                    modseq: prior.highestmodseq,
                    known: &prior.uids,
                };
            }
        }
    }
    if caps.condstore {
        sync::SelectMode::Condstore
    } else {
        sync::SelectMode::Plain
    }
}

fn sync_caps_from_connector<S>(conn: &ImapConnector<S>) -> sync::SyncCaps
where
    S: AsyncRead + AsyncWrite + Unpin + Debug,
{
    sync::SyncCaps {
        condstore: conn.has_condstore.load(Ordering::Relaxed),
        qresync: conn.has_qresync.load(Ordering::Relaxed),
    }
}

fn folder_sync_from_index(index: &ListIndex) -> Option<sync::FolderSyncState> {
    Some(sync::FolderSyncState {
        folder: index.folder.clone(),
        uidvalidity: index.uidvalidity?,
        highestmodseq: index.highestmodseq?,
        sort: index.sort,
        filter: index.filter,
        search: index.search.clone(),
        exists: index.exists,
        uids: index.uids.clone()?,
        unread: index.unread,
        folder_unread: index.folder_unread,
    })
}

async fn remember_folder_sync(
    folder_sync: &Mutex<HashMap<String, sync::FolderSyncState>>,
    index: &ListIndex,
) {
    if let Some(state) = folder_sync_from_index(index) {
        folder_sync.lock().await.insert(state.folder.clone(), state);
    }
}

fn mailbox_is_self_or_descendant(name: &str, ancestor: &str, delimiter: Option<&str>) -> bool {
    if name == ancestor {
        return true;
    }
    match delimiter.filter(|d| !d.is_empty()) {
        Some(d) => name
            .strip_prefix(ancestor)
            .is_some_and(|rest| rest.starts_with(d)),
        None => false,
    }
}

fn remember_selected(selected: &std::sync::Mutex<Option<String>>, folder_id: &str) {
    *selected.lock().unwrap_or_else(|e| e.into_inner()) = Some(folder_id.to_string());
}

fn clear_selected(selected: &std::sync::Mutex<Option<String>>) {
    *selected.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

fn mailbox_is_selected(selected: &std::sync::Mutex<Option<String>>, folder_id: &str) -> bool {
    selected
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_deref()
        == Some(folder_id)
}

/// SELECT `folder_id` and record it as the session's selected mailbox.
async fn select_mailbox<S>(
    session: &mut Session<ImapIo<S>>,
    selected: &std::sync::Mutex<Option<String>>,
    folder_id: &str,
) -> Result<async_imap::types::Mailbox, ImapError>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    let mailbox = session
        .select(folder_id)
        .await
        .map_err(|e| ImapError::Imap(format!("Failed to select folder: {e}")))?;
    remember_selected(selected, folder_id);
    Ok(mailbox)
}

/// SELECT only when `folder_id` is not already the selected mailbox.
async fn ensure_mailbox_selected<S>(
    session: &mut Session<ImapIo<S>>,
    selected: &std::sync::Mutex<Option<String>>,
    folder_id: &str,
) -> Result<(), ImapError>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    if mailbox_is_selected(selected, folder_id) {
        return Ok(());
    }
    select_mailbox(session, selected, folder_id).await?;
    Ok(())
}

/// SELECT INBOX so DELETE/RENAME is not run against the currently selected mailbox.
async fn select_inbox_before_mutate<S>(
    session: &mut Session<ImapIo<S>>,
    selected: &std::sync::Mutex<Option<String>>,
    folder_id: &str,
) -> Result<(), ImapError>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    if is_inbox_mailbox(folder_id) {
        return Ok(());
    }
    session
        .select("INBOX")
        .await
        .map_err(|e| ImapError::Imap(format!("Failed to select INBOX: {e}")))?;
    remember_selected(selected, "INBOX");
    Ok(())
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

fn appenduid_dest(
    folder_id: &FolderId,
    code: &Option<imap_proto::ResponseCode<'_>>,
) -> Option<MessageId> {
    match code {
        Some(imap_proto::ResponseCode::AppendUid(_, dest)) => {
            expand_uid_set(folder_id, dest).into_iter().next()
        }
        _ => None,
    }
}

/// Run a tagged command and collect destination UIDs from COPYUID.
async fn run_copyuid_command<S>(
    session: &mut Session<ImapIo<S>>,
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
    session: &mut Session<ImapIo<S>>,
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

async fn expunge_uids<S>(session: &mut Session<ImapIo<S>>, uids: &str) -> MailinerResult<()>
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

async fn delete_selected_uids<S>(session: &mut Session<ImapIo<S>>, uids: &str) -> MailinerResult<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug + Send,
{
    drain_uid_store(session, uids, "+FLAGS.SILENT (\\Deleted)").await?;
    expunge_uids(session, uids).await
}

#[async_trait]
impl<S> EmailConnector for ImapConnector<S>
where
    // `'static` required so partial-fetch streams can own `Arc<Mutex<ImapSession<S>>>`.
    S: AsyncRead + AsyncWrite + Unpin + std::fmt::Debug + Send + Sync + 'static,
{
    async fn disconnect(&self) -> MailinerResult<()> {
        *self.list_index.lock().await = None;
        self.folder_sync.lock().await.clear();
        clear_selected(&self.selected_mailbox);
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

    async fn authenticate(&self, credentials: &str) -> MailinerResult<()> {
        let mut imap = self.imap.lock().await;
        if let ImapSession::Unauthenticated(_) = &*imap {
            // Temporarily transition to Authenticating state and consume the imap session,
            // that we know is in Unauthenticated state.
            let unauth_imap = std::mem::replace(&mut *imap, ImapSession::Authenticating);
            if let ImapSession::Unauthenticated(mut client) = unauth_imap {
                let choice = match auth::query_preauth_caps(&mut client).await {
                    Ok(caps) => caps.choice(),
                    Err(e) => {
                        tracing::warn!("pre-auth CAPABILITY failed ({e}); falling back to LOGIN");
                        auth::AuthChoice::Login
                    }
                };
                let authenticated = match choice {
                    auth::AuthChoice::Plain => {
                        info!("IMAP AUTHENTICATE PLAIN");
                        let sasl = auth::SaslPlain {
                            username: &self.username,
                            password: credentials,
                        };
                        client.authenticate("PLAIN", sasl).await
                    }
                    auth::AuthChoice::Login => {
                        info!("IMAP LOGIN");
                        client.login(&self.username, credentials).await
                    }
                    auth::AuthChoice::None => {
                        *imap = ImapSession::Unauthenticated(client);
                        return Err(ImapError::Authentication(
                            "Server advertised no supported IMAP auth mechanism (PLAIN/LOGIN)."
                                .into(),
                        )
                        .into());
                    }
                };
                // Transition from the temporary Authenticating state to the Authenticated state.
                *imap = ImapSession::Authenticated(authenticated.map_err(|(e, _)| {
                    ImapError::Authentication(format!("Failed to authenticate: {e}"))
                })?);
                clear_selected(&self.selected_mailbox);
                if let ImapSession::Authenticated(session) = &mut *imap {
                    let try_compress = Self::probe_capabilities(
                        session,
                        &self.has_sort,
                        &self.has_quota,
                        &self.has_idle,
                        &self.has_condstore,
                        &self.has_qresync,
                    )
                    .await;
                    let want_qresync = self.has_qresync.load(Ordering::Relaxed);
                    let want_condstore = self.has_condstore.load(Ordering::Relaxed);
                    if want_qresync || want_condstore {
                        let enabled =
                            sync::enable_sync_extensions(session, want_qresync, want_condstore)
                                .await;
                        self.has_condstore
                            .store(enabled.condstore, Ordering::Relaxed);
                        self.has_qresync.store(enabled.qresync, Ordering::Relaxed);
                    }
                    if try_compress {
                        Self::maybe_enable_compress(session, &self.has_compress).await;
                    } else {
                        self.has_compress.store(false, Ordering::Relaxed);
                    }
                }
            } else {
                return Err(MailinerError::Connector(
                    "IMAP session in invalid state".to_string(),
                ));
            }
            Ok(())
        } else if let ImapSession::Authenticated(_) = &*imap {
            Ok(())
        } else {
            Err(ImapError::Connection("Not connected".to_string()).into())
        }
    }

    async fn list_folders(&self, account_id: &AccountId) -> MailinerResult<Vec<Folder>> {
        // Full LIST so unsubscribed mailboxes stay selectable (manager / show-all).
        let mut listed = self.list_all_mailboxes().await?;
        match self.list_subscribed_names().await {
            Ok(names) if !names.is_empty() => apply_subscriptions(&mut listed, &names),
            Ok(_) => {
                // Empty LSUB: keep the LIST default (all subscribed) so the tree
                // is not blank on servers that never persist subscriptions.
            }
            Err(e) => {
                tracing::warn!("LSUB failed ({e}); showing every LIST folder");
            }
        }
        Ok(folders_from_listed(account_id, &listed))
    }

    async fn set_folder_subscribed(
        &self,
        folder_id: &FolderId,
        subscribed: bool,
    ) -> MailinerResult<()> {
        let mut imap = self.imap.lock().await;
        let ImapSession::Authenticated(session) = &mut *imap else {
            return Err(ImapError::NotAuthenticated.into());
        };
        let name = folder_id.as_str();
        if subscribed {
            session
                .subscribe(name)
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to SUBSCRIBE {name}: {e}")))?;
        } else {
            session
                .unsubscribe(name)
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to UNSUBSCRIBE {name}: {e}")))?;
        }
        Ok(())
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

    async fn folder_quota(&self, folder_id: &FolderId) -> MailinerResult<Option<MailboxQuota>> {
        if !self.has_quota.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let mut imap = self.imap.lock().await;
        let ImapSession::Authenticated(session) = &mut *imap else {
            return Err(ImapError::NotAuthenticated.into());
        };
        match session.get_quota_root(folder_id.as_str()).await {
            Ok((_roots, quotas)) => Ok(quota::storage_quota(&quotas)),
            Err(e) => {
                tracing::debug!("GETQUOTAROOT {} failed: {e}", folder_id.as_str());
                Ok(None)
            }
        }
    }

    async fn prepare_folder_list(
        &self,
        folder_id: &FolderId,
        sort: MessageSort,
        filter: MessageListFilter,
        search: &str,
    ) -> MailinerResult<FolderListState> {
        let has_sort = self.has_sort.load(Ordering::Relaxed);
        let caps = sync_caps_from_connector(self);
        let prior = self
            .folder_sync
            .lock()
            .await
            .get(folder_id.as_str())
            .cloned();
        let mut imap = self.imap.lock().await;
        let ImapSession::Authenticated(session) = &mut *imap else {
            return Err(ImapError::NotAuthenticated.into());
        };
        let index = Self::build_list_index(
            session,
            &self.selected_mailbox,
            &FolderListReq {
                folder_id: folder_id.as_str(),
                requested: sort,
                filter,
                search,
            },
            has_sort,
            caps,
            prior.as_ref(),
        )
        .await?;
        let state = FolderListState {
            total: index.total,
            folder_total: index.exists,
            unread: index.folder_unread,
            sort: index.sort,
            supports_size_sender: has_sort,
        };
        remember_folder_sync(&self.folder_sync, &index).await;
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
            let caps = sync::SyncCaps {
                condstore: self.has_condstore.load(Ordering::Relaxed),
                qresync: self.has_qresync.load(Ordering::Relaxed),
            };
            let mailbox =
                select_mailbox(session, &self.selected_mailbox, folder_id.as_str()).await?;
            let exists = mailbox.exists as usize;
            let mut index_slot = self.list_index.lock().await;
            let stale = index_slot
                .as_ref()
                .is_none_or(|idx| idx.folder != folder_id.as_str() || idx.exists != exists);
            if stale {
                let (requested, filter, search) = index_slot
                    .as_ref()
                    .filter(|idx| idx.folder == folder_id.as_str())
                    .map(|idx| (idx.sort, idx.filter, idx.search.clone()))
                    .unwrap_or((
                        MessageSort::Arrival,
                        MessageListFilter::default(),
                        String::new(),
                    ));
                let prior = self
                    .folder_sync
                    .lock()
                    .await
                    .get(folder_id.as_str())
                    .cloned();
                let index = Self::build_list_index(
                    session,
                    &self.selected_mailbox,
                    &FolderListReq {
                        folder_id: folder_id.as_str(),
                        requested,
                        filter,
                        search: &search,
                    },
                    has_sort,
                    caps,
                    prior.as_ref(),
                )
                .await?;
                remember_folder_sync(&self.folder_sync, &index).await;
                *index_slot = Some(index);
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

        select_mailbox(session, &self.selected_mailbox, folder_id.as_str()).await?;

        for (flag, value) in flags {
            let atom = imap_flag_atom(*flag);
            let query = if *value {
                format!("+FLAGS.SILENT ({atom})")
            } else {
                format!("-FLAGS.SILENT ({atom})")
            };
            drain_uid_store(session, &uids, &query).await?;
        }
        drop(imap);
        drop_mismatched_search_uids(&self.list_index, message_ids, flags).await;
        if let Some(index) = self.list_index.lock().await.as_ref() {
            remember_folder_sync(&self.folder_sync, index).await;
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
        let Some(uids) = index.uids.as_mut() else {
            return Ok(Vec::new());
        };
        let parsed = mailiner_core::MailboxSearch::parse(&index.search);
        let unseen_only = index.filter.unread || parsed.has_unread();
        if unseen_only {
            // Unseen-only SEARCH list: drop read UIDs; insert unread at the unseen prefix.
            for id in message_ids {
                let Ok(uid) = id.as_uid().parse::<u32>() else {
                    continue;
                };
                if now_read {
                    if let Some(from) = uids.iter().position(|&u| u == uid) {
                        uids.remove(from);
                        if index.unread.is_some_and(|n| from < n) {
                            index.unread = Some(index.unread.unwrap_or(0).saturating_sub(1));
                        }
                    }
                } else if !uids.contains(&uid) {
                    let dest_end = index.unread.unwrap_or(0).min(uids.len());
                    let to = uids[..dest_end]
                        .iter()
                        .position(|&u| u < uid)
                        .unwrap_or(dest_end);
                    uids.insert(to, uid);
                    index.unread = Some(index.unread.unwrap_or(0).saturating_add(1));
                }
            }
            index.total = uids.len();
            return Ok(Vec::new());
        }
        if parsed.has_read() && !now_read {
            for id in message_ids {
                let Ok(uid) = id.as_uid().parse::<u32>() else {
                    continue;
                };
                if let Some(from) = uids.iter().position(|&u| u == uid) {
                    uids.remove(from);
                    if index.unread.is_some_and(|n| from < n) {
                        index.unread = Some(index.unread.unwrap_or(0).saturating_sub(1));
                    }
                }
            }
            index.total = uids.len();
            return Ok(Vec::new());
        }
        if index.sort != MessageSort::Unread {
            return Ok(Vec::new());
        }
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

        select_mailbox(session, &self.selected_mailbox, folder_id.as_str()).await?;

        if let Ok(dest_uids) =
            run_copyuid_command(session, dest_folder_id, &format!("UID MOVE {uids} {dest}")).await
        {
            drop(imap);
            self.forget_messages(folder_id, message_ids).await;
            return Ok(dest_uids);
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

    async fn copy_messages(
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

        select_mailbox(session, &self.selected_mailbox, folder_id.as_str()).await?;

        // UID COPY only — originals stay, no `\Deleted`.
        run_copyuid_command(session, dest_folder_id, &format!("UID COPY {uids} {dest}")).await
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

        select_mailbox(session, &self.selected_mailbox, folder_id.as_str()).await?;
        delete_selected_uids(session, &uids).await?;
        drop(imap);
        self.forget_messages(folder_id, message_ids).await;
        Ok(())
    }

    async fn empty_folder(&self, folder_id: &FolderId) -> MailinerResult<()> {
        {
            let mut imap = self.imap.lock().await;
            let ImapSession::Authenticated(session) = &mut *imap else {
                return Err(ImapError::NotAuthenticated.into());
            };

            let mailbox =
                select_mailbox(session, &self.selected_mailbox, folder_id.as_str()).await?;
            // Empty folder is success. `UID STORE 1:*` would be invalid with EXISTS 0.
            if mailbox.exists > 0 {
                delete_selected_uids(session, ALL_UIDS).await?;
            }
        }
        self.forget_folder(folder_id).await;
        Ok(())
    }

    async fn create_folder(
        &self,
        account_id: &AccountId,
        name: &str,
        parent_id: Option<&FolderId>,
    ) -> MailinerResult<Folder> {
        let delim = self.hierarchy_delimiter().await?;
        let delim = delim.as_deref();
        let full_name = join_mailbox_path(parent_id.map(FolderId::as_str), name, delim)?;
        if is_inbox_mailbox(&full_name) {
            return Err(MailinerError::InvalidData(
                "Cannot create a folder named Inbox".into(),
            ));
        }

        {
            let mut imap = self.imap.lock().await;
            let ImapSession::Authenticated(session) = &mut *imap else {
                return Err(ImapError::NotAuthenticated.into());
            };
            session
                .create(&full_name)
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to create folder: {e}")))?;
        }

        let (_, leaf) = mailbox_parent_and_leaf(&full_name, delim);
        let leaf = leaf.to_string();
        let role = role_from_name(&full_name, delim);
        Ok(Folder {
            id: FolderId::new(full_name),
            account_id: account_id.clone(),
            name: leaf,
            parent_id: parent_id.cloned(),
            role,
            selectable: true,
            subscribed: true,
        })
    }

    async fn rename_folder(&self, folder_id: &FolderId, new_name: &str) -> MailinerResult<Folder> {
        if is_inbox_mailbox(folder_id.as_str()) {
            return Err(MailinerError::InvalidData("Cannot rename Inbox".into()));
        }
        let delim = self.hierarchy_delimiter().await?;
        let delim = delim.as_deref();
        let full_name = rename_mailbox_path(folder_id.as_str(), new_name, delim)?;
        if is_inbox_mailbox(&full_name) {
            return Err(MailinerError::InvalidData(
                "Cannot rename a folder to Inbox".into(),
            ));
        }

        {
            let mut imap = self.imap.lock().await;
            let ImapSession::Authenticated(session) = &mut *imap else {
                return Err(ImapError::NotAuthenticated.into());
            };
            select_inbox_before_mutate(session, &self.selected_mailbox, folder_id.as_str()).await?;
            session
                .rename(folder_id.as_str(), &full_name)
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to rename folder: {e}")))?;
        }
        self.forget_folder_tree(folder_id, delim).await;

        let (parent, leaf) = mailbox_parent_and_leaf(&full_name, delim);
        let parent = parent.map(FolderId::new);
        let leaf = leaf.to_string();
        let role = role_from_name(&full_name, delim);
        Ok(Folder {
            id: FolderId::new(full_name),
            account_id: self.account_id.clone(),
            name: leaf,
            parent_id: parent,
            role,
            selectable: true,
            subscribed: true,
        })
    }

    async fn delete_folder(&self, folder_id: &FolderId) -> MailinerResult<()> {
        if is_inbox_mailbox(folder_id.as_str()) {
            return Err(MailinerError::InvalidData("Cannot delete Inbox".into()));
        }
        let delim = self.hierarchy_delimiter().await?;
        {
            let mut imap = self.imap.lock().await;
            let ImapSession::Authenticated(session) = &mut *imap else {
                return Err(ImapError::NotAuthenticated.into());
            };
            select_inbox_before_mutate(session, &self.selected_mailbox, folder_id.as_str()).await?;
            session
                .delete(folder_id.as_str())
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to delete folder: {e}")))?;
        }
        self.forget_folder_tree(folder_id, delim.as_deref()).await;
        Ok(())
    }

    async fn get_body_structure(
        &self,
        folder_id: &FolderId,
        message_id: &MessageId,
    ) -> MailinerResult<BodyPart> {
        require_folder(folder_id, std::slice::from_ref(message_id))?;
        {
            let mut cache = self.structure_cache.lock().await;
            if let Some(part) = cache.get(&(folder_id.clone(), message_id.clone())) {
                return Ok(part.clone());
            }
        }

        let part = {
            let mut imap = self.imap.lock().await;
            let ImapSession::Authenticated(session) = &mut *imap else {
                return Err(ImapError::NotAuthenticated.into());
            };

            select_mailbox(session, &self.selected_mailbox, folder_id.as_str()).await?;

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

        select_mailbox(session, &self.selected_mailbox, folder_id.as_str()).await?;

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

    async fn fetch_text_prefixes(
        &self,
        folder_id: &FolderId,
        message_ids: &[MessageId],
        max_octets: usize,
    ) -> MailinerResult<HashMap<MessageId, TextPrefix>> {
        require_folder(folder_id, message_ids)?;
        if message_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let max_octets = max_octets.clamp(1, 8 * 1024);

        let mut plans = Vec::new();
        let mut missing = Vec::new();
        let mut out = HashMap::new();
        {
            let mut cache = self.structure_cache.lock().await;
            for id in message_ids {
                match cache.get(&(folder_id.clone(), id.clone())) {
                    Some(root) => match snippet_plan(id, root) {
                        Some(plan) => plans.push(plan),
                        None => {
                            out.insert(id.clone(), TextPrefix::empty());
                        }
                    },
                    None => missing.push(id.clone()),
                }
            }
        }
        for id in missing {
            match self.get_body_structure(folder_id, &id).await {
                Ok(root) => match snippet_plan(&id, &root) {
                    Some(plan) => plans.push(plan),
                    None => {
                        out.insert(id, TextPrefix::empty());
                    }
                },
                Err(e) => {
                    tracing::debug!("snippet BODYSTRUCTURE {} failed: {e}", id.as_uid());
                }
            }
        }
        if plans.is_empty() {
            return Ok(out);
        }

        let mut by_section: std::collections::BTreeMap<String, Vec<SnippetPlan>> =
            std::collections::BTreeMap::new();
        for plan in plans {
            by_section
                .entry(plan.section.clone())
                .or_default()
                .push(plan);
        }

        let mut imap = self.imap.lock().await;
        let ImapSession::Authenticated(session) = &mut *imap else {
            return Err(ImapError::NotAuthenticated.into());
        };
        select_mailbox(session, &self.selected_mailbox, folder_id.as_str()).await?;

        for (section, group) in by_section {
            let uids = group
                .iter()
                .map(|p| p.id.as_uid().to_string())
                .collect::<Vec<_>>()
                .join(",");
            let query = format!("(BODY.PEEK[{section}]<0.{max_octets}>)");
            let mut fetch = session
                .uid_fetch(&uids, &query)
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to fetch snippet: {e}")))?;
            let mut by_uid: HashMap<String, SnippetPlan> = group
                .into_iter()
                .map(|p| (p.id.as_uid().to_string(), p))
                .collect();
            while let Some(result) = fetch.next().await {
                let fetch =
                    result.map_err(|e| ImapError::Imap(format!("Failed to fetch snippet: {e}")))?;
                let Some(uid) = fetch.uid else {
                    continue;
                };
                let Some(plan) = by_uid.remove(&uid.to_string()) else {
                    continue;
                };
                let Ok(bytes) = Self::extract_section_bytes(&fetch, &section) else {
                    continue;
                };
                match mailiner_mime::decode_content(
                    &bytes,
                    &plan.encoding,
                    &plan.content_type,
                    plan.charset.as_deref(),
                ) {
                    Ok(mailiner_mime::DecodedContent::Text(text)) => {
                        out.insert(
                            plan.id,
                            TextPrefix {
                                text,
                                is_html: plan.is_html,
                            },
                        );
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::debug!("snippet decode {} failed: {e}", plan.id.as_uid());
                    }
                }
            }
        }
        Ok(out)
    }

    async fn stream_raw_part(
        &self,
        folder_id: &FolderId,
        message_id: &MessageId,
        section: &str,
    ) -> MailinerResult<PartStream> {
        self.stream_raw_part_inner(folder_id, message_id, section, None)
            .await
    }

    async fn fetch_raw_message(
        &self,
        folder_id: &FolderId,
        message_id: &MessageId,
    ) -> MailinerResult<Vec<u8>> {
        require_folder(folder_id, std::slice::from_ref(message_id))?;

        {
            let mut imap = self.imap.lock().await;
            let ImapSession::Authenticated(session) = &mut *imap else {
                return Err(ImapError::NotAuthenticated.into());
            };

            select_mailbox(session, &self.selected_mailbox, folder_id.as_str()).await?;

            let mut fetch = session
                .uid_fetch(message_id.as_uid(), "(RFC822.SIZE)")
                .await
                .map_err(|e| ImapError::Imap(format!("Failed to fetch message size: {e}")))?;

            let fetch = fetch
                .next()
                .await
                .ok_or_else(|| ImapError::InvalidData("Message not found".to_string()))?
                .map_err(|e| ImapError::Imap(format!("Failed to fetch message size: {e}")))?;
            if let Some(size) = fetch.size {
                if u64::from(size) > Self::MAX_DOWNLOAD {
                    return Err(MailinerError::Connector(format!(
                        "message exceeds download limit ({size} > {})",
                        Self::MAX_DOWNLOAD
                    )));
                }
            }
        }

        // BODY.PEEK[] is the full RFC 822 message and does not set \Seen.
        // Partial BODY.PEEK[] so an oversized/stale SIZE cannot materialize the
        // whole literal before the cap is applied.
        let mut stream = self.stream_raw_part(folder_id, message_id, "").await?;
        let mut out = Vec::new();
        while let Some(item) = stream.next().await {
            let chunk = item?;
            let next = (out.len() as u64).saturating_add(chunk.data.len() as u64);
            if next > Self::MAX_DOWNLOAD {
                return Err(MailinerError::Connector(format!(
                    "message exceeds download limit (> {})",
                    Self::MAX_DOWNLOAD
                )));
            }
            out.extend_from_slice(&chunk.data);
        }
        Ok(out)
    }
}

/// State machine for progressive `BODY.PEEK[section]<offset.length>` streaming.
struct PartialFetchState<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Debug,
{
    imap: Arc<Mutex<ImapSession<S>>>,
    selected: Arc<std::sync::Mutex<Option<String>>>,
    folder_id: String,
    message_id: String,
    section: String,
    offset: u64,
    chunk_size: usize,
    sizer: Option<fetch_chunk::FetchChunkSizer>,
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
    use mailiner_core::{EmailConnector, ImapKeyword};

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
    fn raw_header_block_feeds_auth_results() {
        let raw = b"From: Sender <sender@example.com>\r\n\
Subject: Hello\r\n\
Authentication-Results: mx.example.com;\r\n\
\tdkim=pass header.i=@example.com;\r\n\
\tspf=fail smtp.mailfrom=sender@example.com;\r\n\
\tdmarc=pass header.from=example.com\r\n\
ARC-Authentication-Results: i=1; mx.example.com; dkim=fail\r\n\
Received-SPF: pass\r\n\
\r\n";
        let auth = AuthResults::from_header_bytes(raw);
        assert_eq!(auth.spf, Some(mailiner_core::AuthVerdict::Fail));
        assert_eq!(auth.dkim, Some(mailiner_core::AuthVerdict::Pass));
        assert_eq!(auth.dmarc, Some(mailiner_core::AuthVerdict::Pass));
    }

    #[test]
    fn imap_flag_atoms() {
        assert_eq!(imap_flag_atom(EnvelopeFlag::Read), "\\Seen");
        assert_eq!(imap_flag_atom(EnvelopeFlag::Answered), "\\Answered");
        assert_eq!(imap_flag_atom(EnvelopeFlag::Flagged), "\\Flagged");
        assert_eq!(imap_flag_atom(EnvelopeFlag::Deleted), "\\Deleted");
        assert_eq!(imap_flag_atom(EnvelopeFlag::Draft), "\\Draft");
        assert_eq!(imap_flag_atom(EnvelopeFlag::Starred), "\\Starred");
        assert_eq!(
            imap_flag_atom(EnvelopeFlag::Keyword(ImapKeyword::Important)),
            "$Important"
        );
    }

    #[test]
    fn parse_flags_answered() {
        let flags = parse_flags([Flag::Answered, Flag::Seen].into_iter());
        assert!(flags.is_read);
        assert!(flags.is_answered);
        assert!(!flags.is_starred);
        assert!(!flags.is_flagged);
        assert!(!flags.is_draft);
        assert!(!flags.is_deleted);
        assert!(flags.keywords.is_empty());
    }

    #[test]
    fn parse_flags_collects_custom_keywords() {
        let flags = parse_flags(
            [
                Flag::Custom("\\Starred".into()),
                Flag::Custom("$Important".into()),
                Flag::Custom("ProjectX".into()),
                Flag::Custom("$Important".into()),
                Flag::Custom("\\Something".into()),
                Flag::Custom("".into()),
            ]
            .into_iter(),
        );
        assert!(flags.is_starred);
        assert_eq!(
            flags.keywords,
            vec!["$Important".to_string(), "ProjectX".to_string()]
        );
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
    fn uid_copy_command_does_not_delete() {
        let dest = quote_mailbox("Archive");
        let cmd = format!("UID COPY 12,44 {dest}");
        assert_eq!(cmd, r#"UID COPY 12,44 "Archive""#);
        assert!(!cmd.contains("MOVE"));
        assert!(!cmd.contains("Deleted"));
    }

    #[test]
    fn descendant_match_uses_delimiter() {
        assert!(mailbox_is_self_or_descendant(
            "INBOX.Work",
            "INBOX",
            Some(".")
        ));
        assert!(mailbox_is_self_or_descendant("INBOX", "INBOX", Some(".")));
        assert!(!mailbox_is_self_or_descendant("INBOX2", "INBOX", Some(".")));
        assert!(!mailbox_is_self_or_descendant("INBOX.Work", "INBOX", None));
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

    fn connector() -> ImapConnector<tokio::io::DuplexStream> {
        ImapConnector::new(
            AccountId::new("acc"),
            "imap.example.com".into(),
            143,
            "user@example.com".into(),
        )
        .with_tls_mode(ImapTlsMode::StartTls)
    }

    async fn write_all(w: &mut (impl tokio::io::AsyncWriteExt + Unpin), s: &str) {
        tokio::io::AsyncWriteExt::write_all(w, s.as_bytes())
            .await
            .unwrap();
        tokio::io::AsyncWriteExt::flush(w).await.unwrap();
    }

    async fn read_cmd(r: &mut (impl tokio::io::AsyncReadExt + Unpin), buf: &mut Vec<u8>) -> String {
        buf.clear();
        let mut tmp = [0u8; 512];
        loop {
            let n = tokio::io::AsyncReadExt::read(r, &mut tmp).await.unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if buf.windows(2).any(|w| w == b"\r\n") {
                break;
            }
        }
        String::from_utf8_lossy(buf).into_owned()
    }

    #[tokio::test]
    async fn starttls_handshake_issues_command_and_returns_stream() {
        let (client, mut server) = tokio::io::duplex(16 * 1024);
        let conn = connector();

        let server_task = tokio::spawn(async move {
            write_all(&mut server, "* OK IMAP4rev1 ready\r\n").await;
            let mut buf = Vec::new();
            let starttls = read_cmd(&mut server, &mut buf).await;
            assert!(
                starttls.to_ascii_uppercase().contains("STARTTLS"),
                "{starttls}"
            );
            let tag = starttls.split_whitespace().next().unwrap();
            write_all(&mut server, &format!("{tag} OK Begin TLS\r\n")).await;
        });

        let _stream = conn.starttls_handshake(client).await.unwrap();
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn starttls_handshake_does_not_login() {
        let (client, mut server) = tokio::io::duplex(16 * 1024);
        let conn = connector();

        let server_task = tokio::spawn(async move {
            write_all(&mut server, "* OK IMAP4rev1 ready\r\n").await;
            let mut buf = Vec::new();
            let cmd = read_cmd(&mut server, &mut buf).await;
            assert!(
                cmd.to_ascii_uppercase().contains("STARTTLS"),
                "expected STARTTLS, got {cmd}"
            );
            assert!(
                !cmd.to_ascii_uppercase().contains("LOGIN"),
                "LOGIN must not run before TLS wrap: {cmd}"
            );
            let tag = cmd.split_whitespace().next().unwrap();
            write_all(&mut server, &format!("{tag} OK Begin TLS\r\n")).await;
        });

        let _stream = conn.starttls_handshake(client).await.unwrap();
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn starttls_rejected_is_tls_error() {
        let (client, mut server) = tokio::io::duplex(16 * 1024);
        let conn = connector();

        let server_task = tokio::spawn(async move {
            write_all(&mut server, "* OK IMAP4rev1 ready\r\n").await;
            let mut buf = Vec::new();
            let cmd = read_cmd(&mut server, &mut buf).await;
            let tag = cmd.split_whitespace().next().unwrap();
            write_all(&mut server, &format!("{tag} BAD no STARTTLS\r\n")).await;
        });

        let err = conn.starttls_handshake(client).await.unwrap_err();
        assert!(
            matches!(err, ImapError::Tls(_)),
            "expected Tls, got {err:?}"
        );
        assert!(err.to_string().contains("STARTTLS"), "{}", err);
        let _ = server_task.await;
    }

    fn reply_select(tag: &str) -> String {
        format!(
            "* 3 EXISTS\r\n* 0 RECENT\r\n* OK [UNSEEN 1] first unseen\r\n{tag} OK [READ-WRITE] SELECT\r\n"
        )
    }

    fn reply_capability(tag: &str, extra: &str) -> String {
        format!("* CAPABILITY IMAP4rev1{extra}\r\n{tag} OK CAPABILITY\r\n")
    }

    /// Pre-auth CAPABILITY (no AUTH=PLAIN) then IMAP LOGIN. Post-auth CAPABILITY is separate.
    async fn expect_capability_then_login(server: &mut tokio::io::DuplexStream, buf: &mut Vec<u8>) {
        let cap = read_cmd(server, buf).await;
        assert!(
            cap.to_ascii_uppercase().contains("CAPABILITY"),
            "expected pre-auth CAPABILITY, got {cap}"
        );
        let tag = cap.split_whitespace().next().unwrap();
        write_all(server, &reply_capability(tag, "")).await;

        let login = read_cmd(server, buf).await;
        assert!(
            login.to_ascii_uppercase().contains("LOGIN"),
            "expected LOGIN fallback, got {login}"
        );
        assert!(
            !login.to_ascii_uppercase().contains("AUTHENTICATE"),
            "LOGIN fallback must not use AUTHENTICATE: {login}"
        );
        let tag = login.split_whitespace().next().unwrap();
        write_all(server, &format!("{tag} OK logged in\r\n")).await;
    }

    async fn login_plain(
        conn: &ImapConnector<tokio::io::DuplexStream>,
        stream: tokio::io::DuplexStream,
    ) {
        conn.connect(stream).await.unwrap();
        conn.authenticate("secret").await.unwrap();
    }

    #[tokio::test]
    async fn watch_idle_reports_exists() {
        let (client, mut server) = tokio::io::duplex(16 * 1024);
        let conn = connector().with_tls_mode(ImapTlsMode::None);

        let server_task = tokio::spawn(async move {
            write_all(&mut server, "* OK IMAP4rev1 ready\r\n").await;
            let mut buf = Vec::new();
            expect_capability_then_login(&mut server, &mut buf).await;

            let cap = read_cmd(&mut server, &mut buf).await;
            assert!(cap.to_ascii_uppercase().contains("CAPABILITY"), "{cap}");
            let tag = cap.split_whitespace().next().unwrap();
            write_all(&mut server, &reply_capability(tag, " IDLE")).await;

            let select = read_cmd(&mut server, &mut buf).await;
            assert!(select.to_ascii_uppercase().contains("SELECT"), "{select}");
            let tag = select.split_whitespace().next().unwrap();
            write_all(&mut server, &reply_select(tag)).await;

            let idle = read_cmd(&mut server, &mut buf).await;
            assert!(idle.to_ascii_uppercase().contains("IDLE"), "{idle}");
            let idle_tag = idle.split_whitespace().next().unwrap().to_string();
            write_all(&mut server, "+ idling\r\n* 4 EXISTS\r\n").await;

            let done = read_cmd(&mut server, &mut buf).await;
            assert!(done.to_ascii_uppercase().contains("DONE"), "{done}");
            write_all(&mut server, &format!("{idle_tag} OK IDLE terminated\r\n")).await;
        });

        login_plain(&conn, client).await;
        assert!(conn.supports_idle());
        let outcome = conn
            .watch_mailbox(
                &FolderId::new("INBOX"),
                std::future::pending::<()>(),
                std::future::pending::<()>(),
            )
            .await
            .unwrap();
        assert_eq!(
            outcome,
            MailboxWatchOutcome::Changed(MailboxChange::exists())
        );
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn watch_idle_cancel_sends_done() {
        let (client, mut server) = tokio::io::duplex(16 * 1024);
        let conn = connector().with_tls_mode(ImapTlsMode::None);

        let server_task = tokio::spawn(async move {
            write_all(&mut server, "* OK IMAP4rev1 ready\r\n").await;
            let mut buf = Vec::new();
            expect_capability_then_login(&mut server, &mut buf).await;

            let cap = read_cmd(&mut server, &mut buf).await;
            let tag = cap.split_whitespace().next().unwrap();
            write_all(&mut server, &reply_capability(tag, " IDLE")).await;

            let select = read_cmd(&mut server, &mut buf).await;
            let tag = select.split_whitespace().next().unwrap();
            write_all(&mut server, &reply_select(tag)).await;

            let idle = read_cmd(&mut server, &mut buf).await;
            assert!(idle.to_ascii_uppercase().contains("IDLE"), "{idle}");
            let idle_tag = idle.split_whitespace().next().unwrap().to_string();
            write_all(&mut server, "+ idling\r\n").await;

            let done = read_cmd(&mut server, &mut buf).await;
            assert!(done.to_ascii_uppercase().contains("DONE"), "{done}");
            write_all(&mut server, &format!("{idle_tag} OK IDLE terminated\r\n")).await;
        });

        login_plain(&conn, client).await;
        let outcome = conn
            .watch_mailbox(
                &FolderId::new("INBOX"),
                std::future::ready(()),
                std::future::pending::<()>(),
            )
            .await
            .unwrap();
        assert_eq!(outcome, MailboxWatchOutcome::Cancelled);
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn watch_noop_reports_exists() {
        let (client, mut server) = tokio::io::duplex(16 * 1024);
        let conn = connector().with_tls_mode(ImapTlsMode::None);

        let server_task = tokio::spawn(async move {
            write_all(&mut server, "* OK IMAP4rev1 ready\r\n").await;
            let mut buf = Vec::new();
            expect_capability_then_login(&mut server, &mut buf).await;

            let cap = read_cmd(&mut server, &mut buf).await;
            let tag = cap.split_whitespace().next().unwrap();
            write_all(&mut server, &reply_capability(tag, "")).await;

            let select = read_cmd(&mut server, &mut buf).await;
            assert!(select.to_ascii_uppercase().contains("SELECT"), "{select}");
            let tag = select.split_whitespace().next().unwrap();
            write_all(&mut server, &reply_select(tag)).await;

            let noop = read_cmd(&mut server, &mut buf).await;
            assert!(noop.to_ascii_uppercase().contains("NOOP"), "{noop}");
            let tag = noop.split_whitespace().next().unwrap();
            write_all(&mut server, &format!("* 4 EXISTS\r\n{tag} OK NOOP\r\n")).await;
        });

        login_plain(&conn, client).await;
        assert!(!conn.supports_idle());
        let outcome = conn
            .watch_mailbox(
                &FolderId::new("INBOX"),
                std::future::pending::<()>(),
                std::future::ready(()),
            )
            .await
            .unwrap();
        assert_eq!(
            outcome,
            MailboxWatchOutcome::Changed(MailboxChange::exists())
        );
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn watch_noop_quiet_times_out() {
        let (client, mut server) = tokio::io::duplex(16 * 1024);
        let conn = connector().with_tls_mode(ImapTlsMode::None);

        let server_task = tokio::spawn(async move {
            write_all(&mut server, "* OK IMAP4rev1 ready\r\n").await;
            let mut buf = Vec::new();
            expect_capability_then_login(&mut server, &mut buf).await;

            let cap = read_cmd(&mut server, &mut buf).await;
            let tag = cap.split_whitespace().next().unwrap();
            write_all(&mut server, &reply_capability(tag, "")).await;

            let select = read_cmd(&mut server, &mut buf).await;
            let tag = select.split_whitespace().next().unwrap();
            write_all(&mut server, &reply_select(tag)).await;

            let noop = read_cmd(&mut server, &mut buf).await;
            assert!(noop.to_ascii_uppercase().contains("NOOP"), "{noop}");
            let tag = noop.split_whitespace().next().unwrap();
            write_all(&mut server, &format!("{tag} OK NOOP\r\n")).await;
        });

        login_plain(&conn, client).await;
        let outcome = conn
            .watch_mailbox(
                &FolderId::new("INBOX"),
                std::future::pending::<()>(),
                std::future::ready(()),
            )
            .await
            .unwrap();
        assert_eq!(outcome, MailboxWatchOutcome::TimedOut);
        server_task.await.unwrap();
    }

    #[test]
    fn selected_mailbox_tracks_skip_and_switch() {
        let selected = std::sync::Mutex::new(None);
        assert!(!mailbox_is_selected(&selected, "INBOX"));
        remember_selected(&selected, "INBOX");
        assert!(mailbox_is_selected(&selected, "INBOX"));
        assert!(!mailbox_is_selected(&selected, "Sent"));
        remember_selected(&selected, "Sent");
        assert!(mailbox_is_selected(&selected, "Sent"));
        clear_selected(&selected);
        assert!(!mailbox_is_selected(&selected, "Sent"));
    }

    fn classify_imap_cmd(cmd: &str) -> String {
        let upper = cmd.to_ascii_uppercase();
        if upper.contains("SELECT") {
            let name = cmd
                .split_whitespace()
                .last()
                .unwrap_or("")
                .trim_matches('"');
            format!("SELECT {name}")
        } else if upper.contains("UID FETCH") {
            "UID FETCH".into()
        } else if upper.contains("LOGOUT") {
            "LOGOUT".into()
        } else {
            cmd.trim().to_string()
        }
    }

    fn parse_partial_window(cmd: &str) -> (u64, usize) {
        let start = cmd.find('<').expect(cmd);
        let end = cmd[start + 1..].find('>').expect(cmd);
        let inner = &cmd[start + 1..start + 1 + end];
        let (off, len) = inner.split_once('.').expect(cmd);
        (off.parse().expect(cmd), len.parse().expect(cmd))
    }

    async fn write_partial_fetch(
        server: &mut tokio::io::DuplexStream,
        tag: &str,
        section: &str,
        offset: u64,
        data: &[u8],
    ) {
        let header = format!(
            "* 1 FETCH (UID 1 BODY[{section}]<{offset}> {{{}}}\r\n",
            data.len()
        );
        write_all(server, &header).await;
        tokio::io::AsyncWriteExt::write_all(server, data)
            .await
            .unwrap();
        write_all(server, &format!(")\r\n{tag} OK FETCH completed\r\n")).await;
    }

    async fn collect_part(stream: &mut PartStream) -> Vec<u8> {
        let mut out = Vec::new();
        while let Some(item) = stream.next().await {
            out.extend_from_slice(&item.unwrap().data);
        }
        out
    }

    #[tokio::test]
    async fn stream_raw_part_selects_only_when_folder_changes() {
        let (client, mut server) = tokio::io::duplex(16 * 1024);
        let conn = connector().with_tls_mode(ImapTlsMode::None);
        const CHUNK: usize = 4;
        const INBOX_BODY: &[u8] = b"0123456789";
        const SENT_BODY: &[u8] = b"xyz";

        let server_task = tokio::spawn(async move {
            write_all(&mut server, "* OK IMAP4rev1 ready\r\n").await;
            let mut buf = Vec::new();
            expect_capability_then_login(&mut server, &mut buf).await;

            let cap = read_cmd(&mut server, &mut buf).await;
            let tag = cap.split_whitespace().next().unwrap();
            write_all(&mut server, &reply_capability(tag, "")).await;

            let mut cmds = Vec::new();
            let mut selected = String::new();
            loop {
                let cmd = read_cmd(&mut server, &mut buf).await;
                if cmd.is_empty() {
                    break;
                }
                cmds.push(classify_imap_cmd(&cmd));
                let tag = cmd.split_whitespace().next().unwrap();
                let upper = cmd.to_ascii_uppercase();
                if upper.contains("SELECT") {
                    selected = cmd
                        .split_whitespace()
                        .last()
                        .unwrap_or("")
                        .trim_matches('"')
                        .to_string();
                    write_all(&mut server, &reply_select(tag)).await;
                } else if upper.contains("UID FETCH") && upper.contains("BODY.PEEK") {
                    let (offset, len) = parse_partial_window(&cmd);
                    let body = if selected.eq_ignore_ascii_case("INBOX") {
                        INBOX_BODY
                    } else {
                        SENT_BODY
                    };
                    let start = offset as usize;
                    let slice = body.get(start..).unwrap_or(&[]);
                    let slice = &slice[..slice.len().min(len)];
                    write_partial_fetch(&mut server, tag, "1", offset, slice).await;
                } else if upper.contains("LOGOUT") {
                    write_all(&mut server, &format!("{tag} OK logged out\r\n")).await;
                    break;
                } else {
                    panic!("unexpected IMAP command: {cmd}");
                }
            }
            cmds
        });

        login_plain(&conn, client).await;
        let inbox = FolderId::new("INBOX");
        let sent = FolderId::new("Sent");
        let uid = MessageId::new(inbox.clone(), "1");

        let mut stream = conn
            .stream_raw_part_chunked(&inbox, &uid, "1", CHUNK)
            .await
            .unwrap();
        assert_eq!(collect_part(&mut stream).await, INBOX_BODY);

        // Same folder still selected — further chunks must not re-SELECT.
        let mut stream = conn
            .stream_raw_part_chunked(&inbox, &uid, "1", CHUNK)
            .await
            .unwrap();
        assert_eq!(collect_part(&mut stream).await, INBOX_BODY);

        let sent_uid = MessageId::new(sent.clone(), "1");
        let mut stream = conn
            .stream_raw_part_chunked(&sent, &sent_uid, "1", CHUNK)
            .await
            .unwrap();
        assert_eq!(collect_part(&mut stream).await, SENT_BODY);

        EmailConnector::disconnect(&conn).await.unwrap();

        let cmds = server_task.await.unwrap();
        assert_eq!(
            cmds,
            [
                "SELECT INBOX",
                "UID FETCH",
                "UID FETCH",
                "UID FETCH",
                "UID FETCH",
                "UID FETCH",
                "UID FETCH",
                "SELECT Sent",
                "UID FETCH",
                "LOGOUT",
            ]
        );
    }

    #[tokio::test]
    async fn authenticate_uses_sasl_plain_when_advertised() {
        let (client, mut server) = tokio::io::duplex(16 * 1024);
        let conn = connector().with_tls_mode(ImapTlsMode::None);

        let server_task = tokio::spawn(async move {
            write_all(&mut server, "* OK IMAP4rev1 ready\r\n").await;
            let mut buf = Vec::new();
            let cap = read_cmd(&mut server, &mut buf).await;
            assert!(cap.to_ascii_uppercase().contains("CAPABILITY"), "{cap}");
            let tag = cap.split_whitespace().next().unwrap();
            write_all(
                &mut server,
                &reply_capability(tag, " AUTH=PLAIN AUTH=XOAUTH2"),
            )
            .await;

            let auth = read_cmd(&mut server, &mut buf).await;
            let upper = auth.to_ascii_uppercase();
            assert!(upper.contains("AUTHENTICATE PLAIN"), "{auth}");
            assert!(!upper.contains("XOAUTH2"), "{auth}");
            assert!(!upper.contains("LOGIN"), "{auth}");
            write_all(&mut server, "+\r\n").await;

            let payload = read_cmd(&mut server, &mut buf).await;
            let decoded = mailiner_mime::base64_decode(payload.trim().as_bytes()).unwrap();
            assert_eq!(decoded, b"\0user@example.com\0secret");
            let tag = auth.split_whitespace().next().unwrap();
            write_all(&mut server, &format!("{tag} OK logged in\r\n")).await;

            let post = read_cmd(&mut server, &mut buf).await;
            assert!(post.to_ascii_uppercase().contains("CAPABILITY"), "{post}");
            let tag = post.split_whitespace().next().unwrap();
            write_all(&mut server, &reply_capability(tag, "")).await;
        });

        login_plain(&conn, client).await;
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn authenticate_falls_back_to_login_without_plain() {
        let (client, mut server) = tokio::io::duplex(16 * 1024);
        let conn = connector().with_tls_mode(ImapTlsMode::None);

        let server_task = tokio::spawn(async move {
            write_all(&mut server, "* OK IMAP4rev1 ready\r\n").await;
            let mut buf = Vec::new();
            expect_capability_then_login(&mut server, &mut buf).await;
            let post = read_cmd(&mut server, &mut buf).await;
            let tag = post.split_whitespace().next().unwrap();
            write_all(&mut server, &reply_capability(tag, "")).await;
        });

        login_plain(&conn, client).await;
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn authenticate_errors_when_no_plain_and_login_disabled() {
        let (client, mut server) = tokio::io::duplex(16 * 1024);
        let conn = connector().with_tls_mode(ImapTlsMode::None);

        let server_task = tokio::spawn(async move {
            write_all(&mut server, "* OK IMAP4rev1 ready\r\n").await;
            let mut buf = Vec::new();
            let cap = read_cmd(&mut server, &mut buf).await;
            let tag = cap.split_whitespace().next().unwrap();
            write_all(
                &mut server,
                &reply_capability(tag, " LOGINDISABLED AUTH=XOAUTH2"),
            )
            .await;
        });

        conn.connect(client).await.unwrap();
        let err = conn.authenticate("secret").await.unwrap_err();
        match err {
            MailinerError::Auth(msg) => {
                assert!(
                    msg.contains("PLAIN/LOGIN"),
                    "expected no-mechanism Auth, got {msg}"
                );
            }
            other => panic!("expected Auth, got {other:?}"),
        }
        let _ = server_task.await;
    }

    #[tokio::test]
    async fn authenticate_plain_failure_does_not_retry_login() {
        let (client, mut server) = tokio::io::duplex(16 * 1024);
        let conn = connector().with_tls_mode(ImapTlsMode::None);

        let server_task = tokio::spawn(async move {
            write_all(&mut server, "* OK IMAP4rev1 ready\r\n").await;
            let mut buf = Vec::new();
            let cap = read_cmd(&mut server, &mut buf).await;
            let tag = cap.split_whitespace().next().unwrap();
            write_all(&mut server, &reply_capability(tag, " AUTH=PLAIN")).await;

            let auth = read_cmd(&mut server, &mut buf).await;
            assert!(
                auth.to_ascii_uppercase().contains("AUTHENTICATE PLAIN"),
                "{auth}"
            );
            write_all(&mut server, "+\r\n").await;
            let _payload = read_cmd(&mut server, &mut buf).await;
            let tag = auth.split_whitespace().next().unwrap();
            write_all(&mut server, &format!("{tag} NO [AUTHENTICATIONFAILED]\r\n")).await;
        });

        conn.connect(client).await.unwrap();
        let err = conn.authenticate("wrong").await.unwrap_err();
        match err {
            MailinerError::Auth(msg) => {
                assert!(
                    !msg.contains("PLAIN/LOGIN"),
                    "failed PLAIN must not look like missing mechanism: {msg}"
                );
            }
            other => panic!("expected Auth, got {other:?}"),
        }
        let _ = server_task.await;
    }

    #[tokio::test]
    async fn compress_deflate_enabled_when_advertised() {
        let (client, mut server) = tokio::io::duplex(16 * 1024);
        let conn = connector().with_tls_mode(ImapTlsMode::None);

        let server_task = tokio::spawn(async move {
            write_all(&mut server, "* OK IMAP4rev1 ready\r\n").await;
            let mut buf = Vec::new();
            expect_capability_then_login(&mut server, &mut buf).await;

            let post = read_cmd(&mut server, &mut buf).await;
            assert!(post.to_ascii_uppercase().contains("CAPABILITY"), "{post}");
            let tag = post.split_whitespace().next().unwrap();
            write_all(&mut server, &reply_capability(tag, " COMPRESS=DEFLATE")).await;

            let compress = read_cmd(&mut server, &mut buf).await;
            assert!(
                compress.to_ascii_uppercase().contains("COMPRESS DEFLATE"),
                "expected COMPRESS DEFLATE, got {compress}"
            );
            let tag = compress.split_whitespace().next().unwrap();
            write_all(&mut server, &format!("{tag} OK DEFLATE active\r\n")).await;

            let mut codec = crate::compress::DeflateIo::new(server);
            let list = read_cmd(&mut codec, &mut buf).await;
            assert!(
                list.to_ascii_uppercase().contains("LIST"),
                "expected compressed LIST, got {list}"
            );
            let tag = list.split_whitespace().next().unwrap();
            write_all(
                &mut codec,
                &format!("* LIST (\\HasNoChildren) \"/\" INBOX\r\n{tag} OK LIST\r\n"),
            )
            .await;
            let lsub = read_cmd(&mut codec, &mut buf).await;
            assert!(
                lsub.to_ascii_uppercase().contains("LSUB"),
                "expected compressed LSUB, got {lsub}"
            );
            let tag = lsub.split_whitespace().next().unwrap();
            write_all(
                &mut codec,
                &format!("* LSUB (\\HasNoChildren) \"/\" INBOX\r\n{tag} OK LSUB\r\n"),
            )
            .await;
        });

        login_plain(&conn, client).await;
        assert!(conn.supports_compress());
        let folders = conn
            .list_folders(&AccountId::new("acc"))
            .await
            .expect("LIST over COMPRESS=DEFLATE");
        assert!(
            folders.iter().any(|f| f.id.as_str() == "INBOX"),
            "{folders:?}"
        );
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn compress_deflate_rejected_continues_uncompressed() {
        let (client, mut server) = tokio::io::duplex(16 * 1024);
        let conn = connector().with_tls_mode(ImapTlsMode::None);

        let server_task = tokio::spawn(async move {
            write_all(&mut server, "* OK IMAP4rev1 ready\r\n").await;
            let mut buf = Vec::new();
            expect_capability_then_login(&mut server, &mut buf).await;

            let post = read_cmd(&mut server, &mut buf).await;
            let tag = post.split_whitespace().next().unwrap();
            write_all(&mut server, &reply_capability(tag, " COMPRESS=DEFLATE")).await;

            let compress = read_cmd(&mut server, &mut buf).await;
            assert!(
                compress.to_ascii_uppercase().contains("COMPRESS DEFLATE"),
                "{compress}"
            );
            let tag = compress.split_whitespace().next().unwrap();
            write_all(&mut server, &format!("{tag} NO [COMPRESSIONACTIVE]\r\n")).await;

            let logout = read_cmd(&mut server, &mut buf).await;
            assert!(
                logout.to_ascii_uppercase().contains("LOGOUT"),
                "LOGOUT must stay plaintext after COMPRESS NO, got {logout}"
            );
            let tag = logout.split_whitespace().next().unwrap();
            write_all(&mut server, &format!("{tag} OK logged out\r\n")).await;
        });

        login_plain(&conn, client).await;
        assert!(!conn.supports_compress());
        conn.disconnect().await.unwrap();
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn compress_not_sent_when_not_advertised() {
        let (client, mut server) = tokio::io::duplex(16 * 1024);
        let conn = connector().with_tls_mode(ImapTlsMode::None);

        let server_task = tokio::spawn(async move {
            write_all(&mut server, "* OK IMAP4rev1 ready\r\n").await;
            let mut buf = Vec::new();
            expect_capability_then_login(&mut server, &mut buf).await;

            let post = read_cmd(&mut server, &mut buf).await;
            assert!(post.to_ascii_uppercase().contains("CAPABILITY"), "{post}");
            let tag = post.split_whitespace().next().unwrap();
            write_all(&mut server, &reply_capability(tag, " IDLE")).await;

            let logout = read_cmd(&mut server, &mut buf).await;
            assert!(
                logout.to_ascii_uppercase().contains("LOGOUT"),
                "expected LOGOUT (no COMPRESS), got {logout}"
            );
            assert!(
                !logout.to_ascii_uppercase().contains("COMPRESS"),
                "{logout}"
            );
            let tag = logout.split_whitespace().next().unwrap();
            write_all(&mut server, &format!("{tag} OK logged out\r\n")).await;
        });

        login_plain(&conn, client).await;
        assert!(!conn.supports_compress());
        conn.disconnect().await.unwrap();
        server_task.await.unwrap();
    }

    fn reply_select_sync(tag: &str, exists: u32, uidvalidity: u32, highestmodseq: u64) -> String {
        format!(
            "* {exists} EXISTS\r\n* 0 RECENT\r\n* OK [UIDVALIDITY {uidvalidity}] UIDs valid\r\n* OK [HIGHESTMODSEQ {highestmodseq}] Highest\r\n{tag} OK [READ-WRITE] SELECT\r\n"
        )
    }

    fn reply_search(tag: &str, uids: &str) -> String {
        if uids.is_empty() {
            format!("* SEARCH\r\n{tag} OK SEARCH\r\n")
        } else {
            format!("* SEARCH {uids}\r\n{tag} OK SEARCH\r\n")
        }
    }

    async fn prepare_inbox(conn: &ImapConnector<tokio::io::DuplexStream>) {
        conn.prepare_folder_list(
            &FolderId::new("INBOX"),
            MessageSort::Arrival,
            MessageListFilter::default(),
            "",
        )
        .await
        .expect("prepare_folder_list");
    }

    /// Login + two `prepare_folder_list` calls. `second_uidvalidity` overrides the
    /// second SELECT's UIDVALIDITY when set.
    async fn serve_two_prepares(
        mut server: tokio::io::DuplexStream,
        extra_caps: &str,
        first_uidvalidity: u32,
        second_uidvalidity: u32,
    ) -> Vec<String> {
        write_all(&mut server, "* OK IMAP4rev1 ready\r\n").await;
        let mut buf = Vec::new();
        expect_capability_then_login(&mut server, &mut buf).await;

        let post = read_cmd(&mut server, &mut buf).await;
        assert!(post.to_ascii_uppercase().contains("CAPABILITY"), "{post}");
        let tag = post.split_whitespace().next().unwrap();
        write_all(&mut server, &reply_capability(tag, extra_caps)).await;

        let mut cmds = Vec::new();
        let mut selects = 0u32;
        loop {
            let cmd = read_cmd(&mut server, &mut buf).await;
            if cmd.is_empty() {
                break;
            }
            let upper = cmd.to_ascii_uppercase();
            let tag = cmd.split_whitespace().next().unwrap();
            cmds.push(cmd.trim().to_string());
            if upper.contains("ENABLE") {
                write_all(&mut server, &format!("{tag} OK ENABLED\r\n")).await;
            } else if upper.contains("SELECT") {
                selects += 1;
                let uv = if selects == 1 {
                    first_uidvalidity
                } else {
                    second_uidvalidity
                };
                write_all(&mut server, &reply_select_sync(tag, 3, uv, 100)).await;
            } else if upper.contains("UID FETCH") && upper.contains("CHANGEDSINCE") {
                write_all(&mut server, &format!("{tag} OK FETCH completed\r\n")).await;
            } else if upper.contains("UID SEARCH") {
                if upper.contains("UNSEEN") {
                    write_all(&mut server, &reply_search(tag, "1")).await;
                } else {
                    write_all(&mut server, &reply_search(tag, "1 2 3")).await;
                }
            } else if upper.contains("LOGOUT") {
                write_all(&mut server, &format!("{tag} OK logged out\r\n")).await;
                break;
            } else {
                panic!("unexpected IMAP command: {cmd}");
            }
        }
        cmds
    }

    #[tokio::test]
    async fn condstore_not_used_without_capability() {
        let (client, server) = tokio::io::duplex(16 * 1024);
        let conn = connector().with_tls_mode(ImapTlsMode::None);
        let server_task = tokio::spawn(serve_two_prepares(server, "", 1, 1));

        login_plain(&conn, client).await;
        assert!(!conn.supports_condstore());
        assert!(!conn.supports_qresync());
        prepare_inbox(&conn).await;
        prepare_inbox(&conn).await;
        EmailConnector::disconnect(&conn).await.unwrap();

        let cmds = server_task.await.unwrap();
        let joined = cmds.join("\n").to_ascii_uppercase();
        assert!(
            !joined.contains("ENABLE"),
            "ENABLE must not be sent without CONDSTORE: {cmds:?}"
        );
        assert!(
            !joined.contains("CHANGEDSINCE"),
            "CHANGEDSINCE must not be sent without CONDSTORE: {cmds:?}"
        );
        assert!(
            !joined.contains("MODSEQ"),
            "MODSEQ must not be sent without CONDSTORE: {cmds:?}"
        );
        let search_all = cmds
            .iter()
            .filter(|c| {
                let u = c.to_ascii_uppercase();
                u.contains("UID SEARCH") && u.contains("ALL")
            })
            .count();
        assert!(
            search_all >= 2,
            "expected full SEARCH ALL on both opens, got {cmds:?}"
        );
    }

    #[tokio::test]
    async fn condstore_incremental_on_second_select() {
        let (client, server) = tokio::io::duplex(16 * 1024);
        let conn = connector().with_tls_mode(ImapTlsMode::None);
        let server_task = tokio::spawn(serve_two_prepares(server, " CONDSTORE", 1, 1));

        login_plain(&conn, client).await;
        assert!(conn.supports_condstore());
        assert!(!conn.supports_qresync());
        prepare_inbox(&conn).await;
        prepare_inbox(&conn).await;
        EmailConnector::disconnect(&conn).await.unwrap();

        let cmds = server_task.await.unwrap();
        let joined = cmds.join("\n").to_ascii_uppercase();
        assert!(
            joined.contains("ENABLE CONDSTORE"),
            "expected ENABLE CONDSTORE after LOGIN: {cmds:?}"
        );
        let search_all: Vec<_> = cmds
            .iter()
            .filter(|c| {
                let u = c.to_ascii_uppercase();
                u.contains("UID SEARCH") && u.contains("ALL")
            })
            .collect();
        assert_eq!(
            search_all.len(),
            1,
            "SEARCH ALL only on first open, got {cmds:?}"
        );
        let changed: Vec<_> = cmds
            .iter()
            .filter(|c| c.to_ascii_uppercase().contains("CHANGEDSINCE"))
            .collect();
        assert_eq!(
            changed.len(),
            1,
            "CHANGEDSINCE on second open only, got {cmds:?}"
        );
        assert!(
            changed[0].to_ascii_uppercase().contains("CHANGEDSINCE 100"),
            "CHANGEDSINCE should use stored HIGHESTMODSEQ: {changed:?}"
        );
    }

    #[tokio::test]
    async fn condstore_uidvalidity_change_rebuilds() {
        let (client, server) = tokio::io::duplex(16 * 1024);
        let conn = connector().with_tls_mode(ImapTlsMode::None);
        let server_task = tokio::spawn(serve_two_prepares(server, " CONDSTORE", 1, 99));

        login_plain(&conn, client).await;
        prepare_inbox(&conn).await;
        prepare_inbox(&conn).await;
        EmailConnector::disconnect(&conn).await.unwrap();

        let cmds = server_task.await.unwrap();
        let joined = cmds.join("\n").to_ascii_uppercase();
        assert!(
            joined.contains("ENABLE CONDSTORE"),
            "expected ENABLE CONDSTORE: {cmds:?}"
        );
        assert!(
            !joined.contains("CHANGEDSINCE"),
            "UIDVALIDITY change must not CHANGEDSINCE: {cmds:?}"
        );
        let search_all = cmds
            .iter()
            .filter(|c| {
                let u = c.to_ascii_uppercase();
                u.contains("UID SEARCH") && u.contains("ALL")
            })
            .count();
        assert!(
            search_all >= 2,
            "UIDVALIDITY change must SEARCH ALL again, got {cmds:?}"
        );
    }

    #[tokio::test]
    async fn qresync_select_on_second_open() {
        let (client, server) = tokio::io::duplex(16 * 1024);
        let conn = connector().with_tls_mode(ImapTlsMode::None);
        let server_task = tokio::spawn(serve_two_prepares(server, " QRESYNC CONDSTORE", 1, 1));

        login_plain(&conn, client).await;
        assert!(conn.supports_qresync());
        prepare_inbox(&conn).await;
        prepare_inbox(&conn).await;
        EmailConnector::disconnect(&conn).await.unwrap();

        let cmds = server_task.await.unwrap();
        let joined = cmds.join("\n").to_ascii_uppercase();
        assert!(
            joined.contains("ENABLE QRESYNC"),
            "expected ENABLE QRESYNC: {cmds:?}"
        );
        let qresync_select = cmds
            .iter()
            .filter(|c| c.to_ascii_uppercase().contains("QRESYNC"))
            .filter(|c| c.to_ascii_uppercase().contains("SELECT"))
            .count();
        assert_eq!(
            qresync_select, 1,
            "QRESYNC SELECT on second open only, got {cmds:?}"
        );
        assert!(
            !joined.contains("CHANGEDSINCE"),
            "QRESYNC SELECT should not also CHANGEDSINCE: {cmds:?}"
        );
        let search_all = cmds
            .iter()
            .filter(|c| {
                let u = c.to_ascii_uppercase();
                u.contains("UID SEARCH") && u.contains("ALL")
            })
            .count();
        assert_eq!(
            search_all, 1,
            "SEARCH ALL only on first open with QRESYNC, got {cmds:?}"
        );
    }
}
