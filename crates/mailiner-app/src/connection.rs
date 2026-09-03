//! Connection state machine and per-account connector manager.
//!
//! Owned solely by `core_loop` (single-task; avoids `Send` issues with `SendWrapper`).
//!
//! # Serial connect policy (v1)
//!
//! `core_loop` fully awaits each event handler before reading the next event, so at most
//! one connect attempt is in flight. Generation counters still guard against stale results
//! if connect is ever made concurrent (or if a disconnect bumps generation mid-attempt).
//! Rapid `SelectAccount` switches are serialized: the second waits for the first to finish
//! (up to [`CONNECT_TIMEOUT_MS`]), then runs with a fresh generation — not a mid-flight cancel.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use dioxus::logger::tracing::{error, info, warn};
use dioxus::prelude::{ReadableExt, WritableExt};
use futures_util::future::{Either, select};
use gloo_timers::future::TimeoutFuture;
use mailiner_core::connector::EmailConnector;
use mailiner_core::error::MailinerError;
use mailiner_core::ids::AccountId;
use mailiner_imap_connector::ImapConnector;

use crate::account_config::AccountConfig;
use crate::account_store::AccountStore;
use crate::context::AppContext;
use crate::mail_cache::MailCache;
use crate::reconnect::{is_session_death, is_session_death_message};
use crate::websocket_stream::{WebSocketStream, WsDeathWatch};

/// Overall connect budget: WS open + TLS + LOGIN (wall clock).
pub const CONNECT_TIMEOUT_MS: u32 = 20_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Idle,
    Connecting,
    Authenticating,
    Ready,
    /// Waiting to retry after the IMAP/WebSocket session died.
    Reconnecting {
        failed_attempts: u32,
        delay_ms: u32,
    },
    Error {
        message: String,
        kind: ConnectErrorKind,
        retryable: bool,
    },
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectErrorKind {
    NetworkOrProxy,
    TlsOrSni,
    Auth,
    Timeout,
    Cancelled,
    Internal,
}

#[derive(Debug, Clone)]
pub struct ConnectError {
    pub kind: ConnectErrorKind,
    /// User-safe message (no secrets).
    pub message: String,
    pub retryable: bool,
}

impl ConnectError {
    pub fn timeout() -> Self {
        Self {
            kind: ConnectErrorKind::Timeout,
            message: "Connection timed out. Check the proxy and IMAP host, then try again.".into(),
            retryable: true,
        }
    }

    pub fn cancelled() -> Self {
        Self {
            kind: ConnectErrorKind::Cancelled,
            message: "Connection cancelled.".into(),
            retryable: false,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: ConnectErrorKind::Internal,
            message: message.into(),
            retryable: true,
        }
    }

    pub fn from_kind(kind: ConnectErrorKind, detail: &str) -> Self {
        let (message, retryable) = match kind {
            ConnectErrorKind::NetworkOrProxy => (
                format!("Could not reach the mail server via the proxy. {detail}"),
                true,
            ),
            ConnectErrorKind::TlsOrSni => (
                format!("Secure connection failed (TLS/SNI). {detail}"),
                true,
            ),
            ConnectErrorKind::Auth => (
                "Authentication failed. Check username and password.".into(),
                true,
            ),
            ConnectErrorKind::Timeout => (
                "Connection timed out. Check the proxy and IMAP host, then try again.".into(),
                true,
            ),
            ConnectErrorKind::Cancelled => ("Connection cancelled.".into(), false),
            ConnectErrorKind::Internal => (format!("Internal error: {detail}"), true),
        };
        Self {
            kind,
            message,
            retryable,
        }
    }

    pub fn to_state(&self) -> ConnectionState {
        ConnectionState::Error {
            message: self.message.clone(),
            kind: self.kind,
            retryable: self.retryable,
        }
    }
}

impl From<ConnectError> for ConnectionState {
    fn from(err: ConnectError) -> Self {
        err.to_state()
    }
}

/// How `ensure_connected` treats other active sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnsureConnectedMode {
    /// Active-only switch: tear down other sessions **before** connecting
    /// (`SelectAccount`, `ConnectExisting`, `Bootstrap`, `Reconnect`).
    Switch,
    /// Trial / first-save connect: never tears down other sessions.
    ///
    /// Callers (e.g. `CommitNewAccount`) must call [`AccountConnectionManager::disconnect_others`]
    /// only after **full** success (connect Ready **and** store upsert + set_active_id).
    /// On connect or store failure the prior active session remains intact.
    KeepActiveUntilReady,
}

/// Classify connector / I/O failures into UI-facing kinds.
pub fn classify_mailiner_error(err: &MailinerError) -> ConnectError {
    let text = err.to_string();
    let kind = match err {
        MailinerError::Auth(_) => ConnectErrorKind::Auth,
        MailinerError::Tls(_) => ConnectErrorKind::TlsOrSni,
        other => {
            let lower = other.to_string().to_ascii_lowercase();
            if lower.contains("auth")
                || lower.contains("login")
                || lower.contains("password")
                || lower.contains("credentials")
            {
                ConnectErrorKind::Auth
            } else if lower.contains("tls")
                || lower.contains("certificate")
                || lower.contains("sni")
                || lower.contains("server name")
            {
                ConnectErrorKind::TlsOrSni
            } else if lower.contains("timeout") {
                ConnectErrorKind::Timeout
            } else {
                ConnectErrorKind::NetworkOrProxy
            }
        }
    };
    // Keep detail short and non-secret for logs/UI.
    let detail = truncate_for_ui(&text, 160);
    ConnectError::from_kind(kind, &detail)
}

pub fn classify_io_error(err: &std::io::Error) -> ConnectError {
    let text = err.to_string();
    let lower = text.to_ascii_lowercase();
    let kind = if lower.contains("tls") || lower.contains("certificate") {
        ConnectErrorKind::TlsOrSni
    } else {
        ConnectErrorKind::NetworkOrProxy
    };
    ConnectError::from_kind(kind, &truncate_for_ui(&text, 160))
}

fn truncate_for_ui(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max).collect();
        format!("{t}…")
    }
}

pub fn set_connection_state(ctx: &mut AppContext, account_id: &AccountId, state: ConnectionState) {
    ctx.connection_states
        .write()
        .insert(account_id.clone(), state);
}

/// Remove a connection-state entry (e.g. ephemeral test `request_id` after UI dismisses).
pub fn clear_connection_state(ctx: &mut AppContext, account_id: &AccountId) {
    ctx.connection_states.write().remove(account_id);
}

/// Per-account connector manager. Owned only by `core_loop`.
pub struct AccountConnectionManager {
    connectors: HashMap<AccountId, ImapConnector<WebSocketStream>>,
    /// Cached configs for reconnect; dropped on delete/disconnect.
    configs: HashMap<AccountId, AccountConfig>,
    /// Ids present only in process memory (not store).
    /// These are the only non-store accounts allowed to appear in the UI account map.
    memory_only: HashSet<AccountId>,
    /// Generation counter for switch debounce / stale result ignore.
    /// See module docs: v1 serializes connects; generation is defensive / future-proof.
    connect_generation: HashMap<AccountId, u64>,
    /// Watchers for WebSocket close/error on live sessions.
    ws_watches: HashMap<AccountId, WsDeathWatch>,
    /// Consecutive failed auto-reconnects (reset on success or manual Retry).
    reconnect_attempts: HashMap<AccountId, u32>,
    /// Transport deaths noted from IMAP command errors (drained by `core_loop`).
    session_deaths: RefCell<HashSet<AccountId>>,
    store: Rc<dyn AccountStore>,
    cache: Rc<dyn MailCache>,
}

impl AccountConnectionManager {
    pub fn new(store: Rc<dyn AccountStore>, cache: Rc<dyn MailCache>) -> Self {
        Self {
            connectors: HashMap::new(),
            configs: HashMap::new(),
            memory_only: HashSet::new(),
            connect_generation: HashMap::new(),
            ws_watches: HashMap::new(),
            reconnect_attempts: HashMap::new(),
            session_deaths: RefCell::new(HashSet::new()),
            store,
            cache,
        }
    }

    pub fn store(&self) -> &Rc<dyn AccountStore> {
        &self.store
    }

    pub fn cache(&self) -> &dyn MailCache {
        self.cache.as_ref()
    }

    pub fn get(&self, account_id: &AccountId) -> Option<&ImapConnector<WebSocketStream>> {
        self.connectors.get(account_id)
    }

    pub fn connector_account_ids(&self) -> Vec<AccountId> {
        self.connectors.keys().cloned().collect()
    }

    pub fn config(&self, account_id: &AccountId) -> Option<&AccountConfig> {
        self.configs.get(account_id)
    }

    /// Ids that are allowed in the UI without being in the store.
    pub fn memory_only_ids(&self) -> &HashSet<AccountId> {
        &self.memory_only
    }

    /// Cache a config in memory without writing the store.
    pub fn cache_config_memory_only(&mut self, config: AccountConfig) {
        self.memory_only.insert(config.id.clone());
        self.configs.insert(config.id.clone(), config);
    }

    /// Cache a config for reconnect after it is (or will be) store-backed.
    pub fn cache_config(&mut self, config: AccountConfig) {
        self.memory_only.remove(&config.id);
        self.configs.insert(config.id.clone(), config);
    }

    /// Resolve config: store → manager cache (no hard-coded / env fallbacks).
    pub async fn resolve_config(&self, account_id: &AccountId) -> Option<AccountConfig> {
        match self.store.get(account_id).await {
            Ok(Some(cfg)) => return Some(cfg),
            Ok(None) => {}
            Err(e) => {
                warn!("account store get failed for {}: {}", account_id, e);
            }
        }
        self.configs.get(account_id).cloned()
    }

    fn bump_generation(&mut self, account_id: &AccountId) -> u64 {
        let entry = self
            .connect_generation
            .entry(account_id.clone())
            .or_insert(0);
        *entry = entry.wrapping_add(1);
        *entry
    }

    fn generation_matches(&self, account_id: &AccountId, expected: u64) -> bool {
        self.connect_generation
            .get(account_id)
            .copied()
            .unwrap_or(0)
            == expected
    }

    pub fn current_generation(&self, account_id: &AccountId) -> u64 {
        self.connect_generation
            .get(account_id)
            .copied()
            .unwrap_or(0)
    }

    pub fn reconnect_attempts(&self, account_id: &AccountId) -> u32 {
        self.reconnect_attempts
            .get(account_id)
            .copied()
            .unwrap_or(0)
    }

    pub fn reset_reconnect_attempts(&mut self, account_id: &AccountId) {
        self.reconnect_attempts.remove(account_id);
    }

    /// Invalidate pending auto-reconnect timers for every account except `keep`.
    ///
    /// Needed when switching accounts: a failed session may have no connector
    /// left, so [`Self::disconnect_others`] would not bump its generation.
    pub fn cancel_pending_reconnects(&mut self, keep: Option<&AccountId>, ctx: &mut AppContext) {
        let ids: Vec<AccountId> = {
            let states = ctx.connection_states.read();
            states
                .iter()
                .filter_map(|(id, state)| {
                    if keep.is_some_and(|k| k == id) {
                        return None;
                    }
                    let pending = matches!(state, ConnectionState::Reconnecting { .. })
                        || self.reconnect_attempts.contains_key(id);
                    pending.then_some(id.clone())
                })
                .collect()
        };
        for id in ids {
            self.bump_generation(&id);
            self.reconnect_attempts.remove(&id);
            let reconnecting = ctx
                .connection_states
                .read()
                .get(&id)
                .is_some_and(|s| matches!(s, ConnectionState::Reconnecting { .. }));
            if reconnecting {
                set_connection_state(ctx, &id, ConnectionState::Disconnected);
            }
        }
    }

    pub fn bump_reconnect_attempts(&mut self, account_id: &AccountId) -> u32 {
        let entry = self
            .reconnect_attempts
            .entry(account_id.clone())
            .or_insert(0);
        *entry = entry.saturating_add(1);
        *entry
    }

    pub fn death_watches(&self) -> Vec<(AccountId, WsDeathWatch)> {
        self.ws_watches
            .iter()
            .map(|(id, watch)| (id.clone(), watch.clone()))
            .collect()
    }

    /// Record a transport death from an IMAP command error (idempotent).
    pub fn note_imap_error(&self, account_id: &AccountId, err: &MailinerError) {
        if is_session_death(err) {
            self.session_deaths.borrow_mut().insert(account_id.clone());
        }
    }

    pub fn note_imap_error_msg(&self, account_id: &AccountId, msg: &str) {
        if is_session_death_message(msg) {
            self.session_deaths.borrow_mut().insert(account_id.clone());
        }
    }

    pub fn take_session_deaths(&self) -> Vec<AccountId> {
        self.session_deaths.borrow_mut().drain().collect()
    }

    /// Drop a dead connector without LOGOUT (transport is already gone).
    pub fn drop_dead_connector(&mut self, account_id: &AccountId) {
        self.connectors.remove(account_id);
        self.ws_watches.remove(account_id);
        self.session_deaths.borrow_mut().remove(account_id);
        self.bump_generation(account_id);
    }

    /// Remove a death watch without bumping generation (avoids a closed-watch busy loop).
    pub fn remove_ws_watch(&mut self, account_id: &AccountId) {
        self.ws_watches.remove(account_id);
    }

    /// Disconnect all accounts except optionally `keep`.
    pub async fn disconnect_others(&mut self, keep: Option<&AccountId>, ctx: &mut AppContext) {
        let ids: Vec<AccountId> = self
            .connectors
            .keys()
            .filter(|id| keep.is_none_or(|k| k != *id))
            .cloned()
            .collect();
        for id in ids {
            self.disconnect_account(&id, ctx).await;
        }
        self.cancel_pending_reconnects(keep, ctx);
    }

    /// Drop connector + cached config; best-effort logout.
    pub async fn disconnect_account(&mut self, account_id: &AccountId, ctx: &mut AppContext) {
        if let Some(connector) = self.connectors.remove(account_id)
            && let Err(e) = connector.disconnect().await
        {
            warn!("disconnect failed for {}: {}", account_id, e);
        }
        self.configs.remove(account_id);
        self.memory_only.remove(account_id);
        self.ws_watches.remove(account_id);
        self.reconnect_attempts.remove(account_id);
        self.session_deaths.borrow_mut().remove(account_id);
        // Bump generation so any in-flight connect / auto-reconnect is ignored.
        self.bump_generation(account_id);
        set_connection_state(ctx, account_id, ConnectionState::Disconnected);
    }

    /// Connect + authenticate with 20s timeout. Does not list folders.
    ///
    /// On success, connector is stored under `config.id` and state is `Ready`.
    /// Generation is checked after the await so stale results are dropped.
    pub async fn ensure_connected(
        &mut self,
        config: &AccountConfig,
        ctx: &mut AppContext,
        mode: EnsureConnectedMode,
    ) -> Result<(), ConnectError> {
        let account_id = &config.id;

        if self.connectors.contains_key(account_id) {
            let already_ready = ctx
                .connection_states
                .read()
                .get(account_id)
                .is_some_and(|s| matches!(s, ConnectionState::Ready));
            if already_ready {
                return Ok(());
            }
            // Connector present but not Ready — drop and reconnect.
            if let Some(connector) = self.connectors.remove(account_id) {
                let _ = connector.disconnect().await;
            }
            self.ws_watches.remove(account_id);
        }

        match mode {
            EnsureConnectedMode::Switch => {
                // Intentional account switch: tear down other sessions first.
                self.disconnect_others(Some(account_id), ctx).await;
            }
            EnsureConnectedMode::KeepActiveUntilReady => {
                // Trial / first-save: leave existing active session alone until Ready.
            }
        }

        let my_gen = self.bump_generation(account_id);
        // Cache for the attempt; may be dropped on failure if not store-backed (see below).
        self.configs.insert(account_id.clone(), config.clone());
        set_connection_state(ctx, account_id, ConnectionState::Connecting);

        let connect_result = connect_account(config, ctx).await;

        if !self.generation_matches(account_id, my_gen) {
            if let Ok((connector, _)) = connect_result {
                let _ = connector.disconnect().await;
            }
            // Stale attempt: only surface Cancelled if nothing newer has overwritten state.
            // Newer ensure_connected always overwrites Connecting/Authenticating; if we are
            // still the latest for this id we would have matched generation. Leave state
            // alone when mismatched (newer owner owns the signal).
            // Still return cancelled to the caller of the abandoned attempt.
            return Err(ConnectError::cancelled());
        }

        match connect_result {
            Ok((connector, watch)) => {
                // KeepActiveUntilReady deliberately does **not** disconnect others here.
                // Callers switch active-only only after full commit (store writes) succeed.
                self.connectors.insert(account_id.clone(), connector);
                self.ws_watches.insert(account_id.clone(), watch);
                self.reconnect_attempts.remove(account_id);
                set_connection_state(ctx, account_id, ConnectionState::Ready);
                info!("Account {} connected and authenticated", account_id);
                Ok(())
            }
            Err(err) => {
                error!(
                    "Failed to connect account {}: {:?} — {}",
                    account_id, err.kind, err.message
                );
                // Drop unpersisted secrets from the manager cache (failed CommitNewAccount).
                self.drop_config_if_not_persisted(account_id).await;
                set_connection_state(ctx, account_id, err.to_state());
                Err(err)
            }
        }
    }

    /// Remove `configs` entry when the account is not in the store and not memory-only
    /// (e.g. failed first-save whose id was only used for the attempt).
    async fn drop_config_if_not_persisted(&mut self, account_id: &AccountId) {
        if self.memory_only.contains(account_id) {
            return;
        }
        let in_store = match self.store.get(account_id).await {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(e) => {
                warn!(
                    "store get failed while cleaning config for {}: {}",
                    account_id, e
                );
                // Be conservative: keep cache if store is unavailable.
                true
            }
        };
        if !in_store {
            self.configs.remove(account_id);
        }
    }

    /// Ephemeral test connect: never persists; always disconnects the trial stream;
    /// uses `request_id` for state.
    ///
    /// On success leaves `connection_states[request_id] = Ready` so the UI can show
    /// “Connection successful”. The ephemeral connector is dropped (not installed in
    /// the long-lived map). **UI owns cleanup:** call [`clear_connection_state`] when
    /// the user dismisses the success indicator so ephemeral keys do not accumulate.
    pub async fn test_connection(
        &mut self,
        request_id: &AccountId,
        config: &AccountConfig,
        ctx: &mut AppContext,
    ) -> Result<(), ConnectError> {
        // Do not reuse long-lived map entries under real account ids for tests.
        // Does not touch other active connectors.
        set_connection_state(ctx, request_id, ConnectionState::Connecting);

        let mut test_config = config.clone();
        test_config.id = request_id.clone();

        let result = connect_account(&test_config, ctx).await;

        match result {
            Ok((connector, _watch)) => {
                if let Err(e) = connector.disconnect().await {
                    warn!("test connection disconnect: {}", e);
                }
                // Leave Ready for UI observation; UI clears via clear_connection_state.
                set_connection_state(ctx, request_id, ConnectionState::Ready);
                Ok(())
            }
            Err(err) => {
                set_connection_state(ctx, request_id, err.to_state());
                Err(err)
            }
        }
    }
}

/// WS + TLS + LOGIN, raced against [`CONNECT_TIMEOUT_MS`].
async fn connect_account(
    config: &AccountConfig,
    ctx: &mut AppContext,
) -> Result<(ImapConnector<WebSocketStream>, WsDeathWatch), ConnectError> {
    let account_id = config.id.clone();

    let connect_fut = async {
        let url = config
            .proxy
            .websocket_url(&config.imap)
            .map_err(|e| ConnectError::from_kind(ConnectErrorKind::Internal, &e.to_string()))?;

        info!("Opening WebSocket for account {}…", account_id);
        let stream = WebSocketStream::try_new(&url).map_err(|e| classify_io_error(&e))?;
        let watch = stream.death_watch();

        stream
            .wait_until_open()
            .await
            .map_err(|e| classify_io_error(&e))?;

        // Password is not stored on the connector — only passed to authenticate.
        let connector = ImapConnector::new(
            account_id.clone(),
            config.imap.host.clone(),
            config.imap.port,
            config.imap.username.clone(),
        );

        info!("TLS + IMAP greeting for account {}…", account_id);
        connector
            .connect(stream)
            .await
            .map_err(|e| classify_mailiner_error(&e))?;

        set_connection_state(ctx, &account_id, ConnectionState::Authenticating);
        info!("Authenticating account {}…", account_id);
        connector
            .authenticate(config.imap.password.as_str())
            .await
            .map_err(|e| classify_mailiner_error(&e))?;

        Ok((connector, watch))
    };

    let timeout_fut = TimeoutFuture::new(CONNECT_TIMEOUT_MS);

    futures_util::pin_mut!(connect_fut);
    futures_util::pin_mut!(timeout_fut);

    match select(connect_fut, timeout_fut).await {
        Either::Left((result, _)) => result,
        Either::Right((_, _)) => {
            // Dropping connect_fut drops WebSocketStream; Drop closes the socket.
            error!(
                "Connect timed out after {}ms for account {}",
                CONNECT_TIMEOUT_MS, account_id
            );
            Err(ConnectError::timeout())
        }
    }
}
