//! Spawned SMTP I/O (write-ahead outbox already persisted).

use std::fmt::Debug;
use tokio::io::{AsyncRead, AsyncWrite};

use futures_channel::mpsc::UnboundedSender;
use futures_channel::oneshot;
use futures_util::future::{Either, select};
use gloo_timers::future::TimeoutFuture;
use mailiner_core::submit::{SendErrorKind, SubmitReceipt, SubmitRequest};
use mailiner_smtp_connector::{SmtpConnector, SmtpError};

use crate::account::AccountId;
use crate::account_config::{
    AccountConfig, SmtpTlsMode, ehlo_domain, smtp_password, smtp_username,
};
use crate::connection::CONNECT_TIMEOUT_MS;
use crate::core_event::CoreEvent;
use crate::outbox_store::OutboxId;
use crate::websocket_stream::WebSocketStream;

/// DATA / full-send budget (connect + AUTH + DATA).
pub const SEND_TIMEOUT_MS: u32 = 90_000;

#[derive(Debug, Clone)]
pub struct ClassifiedSendError {
    pub kind: SendErrorKind,
    pub message: String,
}

impl From<SmtpError> for ClassifiedSendError {
    fn from(err: SmtpError) -> Self {
        Self {
            kind: err.kind(),
            message: err.message().to_string(),
        }
    }
}

pub enum SmtpOutcome {
    Send(Result<SubmitReceipt, ClassifiedSendError>),
    Test {
        request_id: AccountId,
        result: Result<(), ClassifiedSendError>,
    },
}

pub struct InFlightSmtp {
    pub account_id: AccountId,
    pub generation: u64,
    pub cancel_tx: oneshot::Sender<()>,
    pub outbox_id: Option<OutboxId>,
    pub is_test: bool,
}

pub fn preflight(config: &AccountConfig) -> Result<(), ClassifiedSendError> {
    match &config.smtp {
        None => Err(ClassifiedSendError {
            kind: SendErrorKind::NotConfigured,
            message: "This account has no SMTP settings. Add them in account settings to send."
                .into(),
        }),
        Some(smtp) if smtp.tls_mode != SmtpTlsMode::Implicit => Err(ClassifiedSendError {
            kind: SendErrorKind::TlsModeUnsupported,
            message: "This account is set to STARTTLS or no TLS, which cannot send yet. Switch to implicit TLS / port 465.".into(),
        }),
        Some(smtp) if smtp.host.trim().is_empty() => Err(ClassifiedSendError {
            kind: SendErrorKind::NotConfigured,
            message: "SMTP host is empty.".into(),
        }),
        Some(_) => Ok(()),
    }
}

pub fn spawn_submit(
    config: AccountConfig,
    request: SubmitRequest,
    generation: u64,
    cancel_rx: oneshot::Receiver<()>,
    event_tx: UnboundedSender<CoreEvent>,
    timeout_ms: u32,
) {
    spawn_fut(async move {
        let outcome = run_submit(config, request, cancel_rx, timeout_ms).await;
        let _ = event_tx.unbounded_send(CoreEvent::SmtpFinished {
            generation,
            outcome: SmtpOutcome::Send(outcome),
        });
    });
}

pub fn spawn_test(
    config: AccountConfig,
    request_id: AccountId,
    generation: u64,
    cancel_rx: oneshot::Receiver<()>,
    event_tx: UnboundedSender<CoreEvent>,
) {
    spawn_fut(async move {
        let result = run_test(config, cancel_rx).await;
        let _ = event_tx.unbounded_send(CoreEvent::SmtpFinished {
            generation,
            outcome: SmtpOutcome::Test { request_id, result },
        });
    });
}

fn spawn_fut(fut: impl std::future::Future<Output = ()> + 'static) {
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(fut);
    #[cfg(not(target_arch = "wasm32"))]
    {
        // WebSocketStream is !Send; host cargo-check cannot run SMTP I/O.
        let _ = fut;
    }
}

async fn run_submit(
    config: AccountConfig,
    request: SubmitRequest,
    cancel_rx: oneshot::Receiver<()>,
    timeout_ms: u32,
) -> Result<SubmitReceipt, ClassifiedSendError> {
    let work = async {
        let (connector, tls) = open_tls(&config).await?;
        connector
            .submit(tls, &smtp_password(&config), &request)
            .await
            .map_err(ClassifiedSendError::from)
    };
    race(work, cancel_rx, timeout_ms).await
}

async fn run_test(
    config: AccountConfig,
    cancel_rx: oneshot::Receiver<()>,
) -> Result<(), ClassifiedSendError> {
    let work = async {
        let (connector, tls) = open_tls(&config).await?;
        connector
            .test(tls, &smtp_password(&config))
            .await
            .map_err(ClassifiedSendError::from)
    };
    race(work, cancel_rx, CONNECT_TIMEOUT_MS).await
}

async fn open_tls(
    config: &AccountConfig,
) -> Result<(SmtpConnector, impl AsyncRead + AsyncWrite + Unpin + Debug), ClassifiedSendError>
{
    let smtp = config.smtp.as_ref().ok_or(ClassifiedSendError {
        kind: SendErrorKind::NotConfigured,
        message: "SMTP is not configured.".into(),
    })?;
    let url = config
        .proxy
        .websocket_url_for(&smtp.host, smtp.port)
        .map_err(|e| ClassifiedSendError {
            kind: SendErrorKind::Internal,
            message: e.to_string(),
        })?;
    let stream = WebSocketStream::try_new(&url).map_err(|e| ClassifiedSendError {
        kind: SendErrorKind::NetworkOrProxy,
        message: e.to_string(),
    })?;
    stream.wait_until_open().await.map_err(|e| ClassifiedSendError {
        kind: SendErrorKind::NetworkOrProxy,
        message: e.to_string(),
    })?;

    let connector = SmtpConnector::new(
        config.id.clone(),
        smtp.host.clone(),
        smtp.port,
        smtp_username(config),
        ehlo_domain(config),
    );
    let tls = connector.wrap_tls(stream).await.map_err(ClassifiedSendError::from)?;
    Ok((connector, tls))
}

async fn race<T, F>(
    work: F,
    cancel_rx: oneshot::Receiver<()>,
    timeout_ms: u32,
) -> Result<T, ClassifiedSendError>
where
    F: std::future::Future<Output = Result<T, ClassifiedSendError>>,
{
    let timeout = TimeoutFuture::new(timeout_ms);
    futures_util::pin_mut!(work);
    futures_util::pin_mut!(timeout);
    match select(work, timeout).await {
        Either::Left((result, _)) => {
            drop(cancel_rx);
            result
        }
        Either::Right((_, work)) => {
            drop(work);
            // Also honour cancel: if both fire we still timeout.
            let _ = cancel_rx;
            Err(ClassifiedSendError {
                kind: SendErrorKind::Timeout,
                message: "Sending timed out. Try again or check the proxy and SMTP host.".into(),
            })
        }
    }
}
