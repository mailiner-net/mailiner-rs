//! Connection state machine and per-account connector manager.
//!
//! Owned solely by `core_loop` (single-task; avoids `Send` issues with `SendWrapper`).

use std::collections::HashMap;
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
use crate::websocket_stream::WebSocketStream;

/// Overall connect budget: WS open + TLS + LOGIN (wall clock).
pub const CONNECT_TIMEOUT_MS: u32 = 20_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Idle,
    Connecting,
    Authenticating,
    Ready,
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

/// Classify connector / I/O failures into UI-facing kinds.
pub fn classify_mailiner_error(err: &MailinerError) -> ConnectError {
    let text = err.to_string();
    let lower = text.to_ascii_lowercase();
    let kind = if lower.contains("auth")
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

/// Per-account connector manager. Owned only by `core_loop`.
pub struct AccountConnectionManager {
    connectors: HashMap<AccountId, ImapConnector<WebSocketStream>>,
    /// Cached configs for reconnect; dropped on delete/disconnect.
    configs: HashMap<AccountId, AccountConfig>,
    /// Generation counter for switch debounce / stale result ignore.
    connect_generation: HashMap<AccountId, u64>,
    store: Rc<dyn AccountStore>,
}

impl AccountConnectionManager {
    pub fn new(store: Rc<dyn AccountStore>) -> Self {
        Self {
            connectors: HashMap::new(),
            configs: HashMap::new(),
            connect_generation: HashMap::new(),
            store,
        }
    }

    pub fn store(&self) -> &Rc<dyn AccountStore> {
        &self.store
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

    /// Cache a config in memory (e.g. dev_default) without writing the store.
    pub fn cache_config(&mut self, config: AccountConfig) {
        self.configs.insert(config.id.clone(), config);
    }

    /// Resolve config: store → manager cache → interim dev_default.
    pub async fn resolve_config(&self, account_id: &AccountId) -> Option<AccountConfig> {
        match self.store.get(account_id).await {
            Ok(Some(cfg)) => return Some(cfg),
            Ok(None) => {}
            Err(e) => {
                warn!("account store get failed for {}: {}", account_id, e);
            }
        }
        if let Some(cfg) = self.configs.get(account_id) {
            return Some(cfg.clone());
        }
        if let Some(cfg) = crate::account_config::dev_default_config()
            && &cfg.id == account_id
        {
            return Some(cfg);
        }
        None
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
    }

    /// Drop connector + cached config; best-effort logout.
    pub async fn disconnect_account(&mut self, account_id: &AccountId, ctx: &mut AppContext) {
        if let Some(connector) = self.connectors.remove(account_id)
            && let Err(e) = connector.disconnect().await
        {
            warn!("disconnect failed for {}: {}", account_id, e);
        }
        self.configs.remove(account_id);
        // Bump generation so any in-flight connect is ignored.
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
        }

        // v1 active-only: tear down other sessions.
        self.disconnect_others(Some(account_id), ctx).await;

        let my_gen = self.bump_generation(account_id);
        self.configs.insert(account_id.clone(), config.clone());
        set_connection_state(ctx, account_id, ConnectionState::Connecting);

        let connect_result = connect_account(config, ctx).await;

        if !self.generation_matches(account_id, my_gen) {
            if let Ok(connector) = connect_result {
                let _ = connector.disconnect().await;
            }
            return Err(ConnectError::cancelled());
        }

        match connect_result {
            Ok(connector) => {
                self.connectors.insert(account_id.clone(), connector);
                set_connection_state(ctx, account_id, ConnectionState::Ready);
                info!("Account {} connected and authenticated", account_id);
                Ok(())
            }
            Err(err) => {
                error!(
                    "Failed to connect account {}: {:?} — {}",
                    account_id, err.kind, err.message
                );
                set_connection_state(ctx, account_id, err.to_state());
                Err(err)
            }
        }
    }

    /// Ephemeral test connect: never persists; always disconnects; uses `request_id` for state.
    pub async fn test_connection(
        &mut self,
        request_id: &AccountId,
        config: &AccountConfig,
        ctx: &mut AppContext,
    ) -> Result<(), ConnectError> {
        // Do not reuse long-lived map entries under real account ids for tests.
        set_connection_state(ctx, request_id, ConnectionState::Connecting);

        let mut test_config = config.clone();
        test_config.id = request_id.clone();

        let result = connect_account(&test_config, ctx).await;

        match result {
            Ok(connector) => {
                set_connection_state(ctx, request_id, ConnectionState::Ready);
                if let Err(e) = connector.disconnect().await {
                    warn!("test connection disconnect: {}", e);
                }
                // Ready is shown briefly; UI may then clear. Leave Ready so UI can show success.
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
) -> Result<ImapConnector<WebSocketStream>, ConnectError> {
    let account_id = config.id.clone();

    let connect_fut = async {
        let url = config
            .proxy
            .websocket_url(&config.imap)
            .map_err(|e| ConnectError::from_kind(ConnectErrorKind::Internal, &e.to_string()))?;

        info!("Opening WebSocket for account {}…", account_id);
        let stream = WebSocketStream::try_new(&url).map_err(|e| classify_io_error(&e))?;

        stream
            .wait_until_open()
            .await
            .map_err(|e| classify_io_error(&e))?;

        let connector = ImapConnector::new(
            account_id.clone(),
            config.imap.host.clone(),
            config.imap.port,
            config.imap.username.clone(),
            config.imap.password.clone(),
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

        Ok(connector)
    };

    let timeout_fut = TimeoutFuture::new(CONNECT_TIMEOUT_MS);

    futures_util::pin_mut!(connect_fut);
    futures_util::pin_mut!(timeout_fut);

    match select(connect_fut, timeout_fut).await {
        Either::Left((result, _)) => result,
        Either::Right((_, _)) => {
            error!(
                "Connect timed out after {}ms for account {}",
                CONNECT_TIMEOUT_MS, account_id
            );
            Err(ConnectError::timeout())
        }
    }
}
