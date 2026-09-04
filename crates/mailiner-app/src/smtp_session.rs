//! Spawned SMTP I/O (write-ahead outbox already persisted).

use std::fmt::Debug;
use tokio::io::{AsyncRead, AsyncWrite};

use dioxus::logger::tracing::info;
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
    Send {
        outbox_id: Option<OutboxId>,
        result: Result<SubmitReceipt, ClassifiedSendError>,
    },
    Test {
        request_id: AccountId,
        result: Result<(), ClassifiedSendError>,
    },
}

pub fn preflight(config: &AccountConfig) -> Result<(), ClassifiedSendError> {
    match &config.smtp {
        None => Err(ClassifiedSendError {
            kind: SendErrorKind::NotConfigured,
            message: "This account has no SMTP settings. Add them in account settings to send."
                .into(),
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
    outbox_id: Option<OutboxId>,
) {
    spawn_fut(async move {
        let result = run_submit(config, request, cancel_rx, timeout_ms).await;
        let _ = event_tx.unbounded_send(CoreEvent::SmtpFinished {
            generation,
            outcome: SmtpOutcome::Send { outbox_id, result },
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
        let (connector, stream, mode) = open_stream(&config).await?;
        let password = smtp_password(&config);
        match mode {
            SmtpTlsMode::Implicit => {
                let tls = connector
                    .wrap_tls(stream)
                    .await
                    .map_err(ClassifiedSendError::from)?;
                connector
                    .submit(tls, &password, &request)
                    .await
                    .map_err(ClassifiedSendError::from)
            }
            SmtpTlsMode::StartTls => connector
                .submit_starttls(stream, &password, &request)
                .await
                .map_err(ClassifiedSendError::from),
            SmtpTlsMode::None => {
                info!(host = %connector.host(), "SMTP plaintext");
                connector
                    .submit(stream, &password, &request)
                    .await
                    .map_err(ClassifiedSendError::from)
            }
        }
    };
    race(work, cancel_rx, timeout_ms).await
}

async fn run_test(
    config: AccountConfig,
    cancel_rx: oneshot::Receiver<()>,
) -> Result<(), ClassifiedSendError> {
    let work = async {
        let (connector, stream, mode) = open_stream(&config).await?;
        let password = smtp_password(&config);
        match mode {
            SmtpTlsMode::Implicit => {
                let tls = connector
                    .wrap_tls(stream)
                    .await
                    .map_err(ClassifiedSendError::from)?;
                connector
                    .test(tls, &password)
                    .await
                    .map_err(ClassifiedSendError::from)
            }
            SmtpTlsMode::StartTls => connector
                .test_starttls(stream, &password)
                .await
                .map_err(ClassifiedSendError::from),
            SmtpTlsMode::None => {
                info!(host = %connector.host(), "SMTP plaintext");
                connector
                    .test(stream, &password)
                    .await
                    .map_err(ClassifiedSendError::from)
            }
        }
    };
    race(work, cancel_rx, CONNECT_TIMEOUT_MS).await
}

async fn open_stream(
    config: &AccountConfig,
) -> Result<
    (
        SmtpConnector,
        impl AsyncRead + AsyncWrite + Unpin + Debug,
        SmtpTlsMode,
    ),
    ClassifiedSendError,
> {
    let smtp = config.smtp.as_ref().ok_or(ClassifiedSendError {
        kind: SendErrorKind::NotConfigured,
        message: "SMTP is not configured.".into(),
    })?;
    let url = config
        .proxy
        .websocket_url_for_smtp(smtp)
        .map_err(|e| ClassifiedSendError {
            kind: SendErrorKind::Internal,
            message: e.to_string(),
        })?;
    let stream = WebSocketStream::try_new(&url).map_err(|e| ClassifiedSendError {
        kind: SendErrorKind::NetworkOrProxy,
        message: e.to_string(),
    })?;
    stream
        .wait_until_open()
        .await
        .map_err(|e| ClassifiedSendError {
            kind: SendErrorKind::NetworkOrProxy,
            message: e.to_string(),
        })?;

    let connector = SmtpConnector::new(
        config.id.clone(),
        smtp.host.clone(),
        smtp.port,
        smtp_username(config),
        ehlo_domain(config),
    )
    .with_extra_ca_pems(config.extra_ca_pems.clone());
    Ok((connector, stream, smtp.tls_mode))
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
    futures_util::pin_mut!(cancel_rx);
    match select(work, select(timeout, cancel_rx)).await {
        Either::Left((result, _)) => result,
        Either::Right((Either::Left((_, work)), _)) => {
            drop(work);
            Err(ClassifiedSendError {
                kind: SendErrorKind::Timeout,
                message: "Sending timed out. Try again or check the proxy and SMTP host.".into(),
            })
        }
        Either::Right((Either::Right((_, work)), _)) => {
            drop(work);
            Err(ClassifiedSendError {
                kind: SendErrorKind::Cancelled,
                message: "Sending was cancelled.".into(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_config::{ImapSettings, ProxySettings, SmtpSettings};
    use chrono::{TimeZone, Utc};

    fn config_with(smtp: Option<SmtpSettings>) -> AccountConfig {
        let ts = Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap();
        AccountConfig {
            id: AccountId::new("acc"),
            display_name: "Work".into(),
            email: "user@example.com".into(),
            identities: Vec::new(),
            signature: None,
            imap: ImapSettings::new(
                "imap.example.com".into(),
                993,
                "user".into(),
                "secret".into(),
                crate::account_config::ImapTlsMode::Implicit,
            ),
            smtp,
            proxy: ProxySettings {
                base_url: "ws://localhost:9400/proxy".into(),
                token: "tok".into(),
                remote_host: None,
                remote_port: None,
            },
            extra_ca_pems: Vec::new(),
            created_at: ts,
            updated_at: ts,
        }
    }

    #[test]
    fn preflight_allows_all_tls_modes() {
        let implicit = config_with(Some(SmtpSettings::new(
            "smtp.example.com".into(),
            465,
            "user".into(),
            None,
            SmtpTlsMode::Implicit,
        )));
        assert!(preflight(&implicit).is_ok());

        let starttls = config_with(Some(SmtpSettings::new(
            "smtp.example.com".into(),
            587,
            "user".into(),
            None,
            SmtpTlsMode::StartTls,
        )));
        assert!(preflight(&starttls).is_ok());

        let plain = config_with(Some(SmtpSettings::new(
            "smtp.example.com".into(),
            25,
            "user".into(),
            None,
            SmtpTlsMode::None,
        )));
        assert!(preflight(&plain).is_ok());
    }

    #[test]
    fn preflight_rejects_missing_smtp() {
        let err = preflight(&config_with(None)).unwrap_err();
        assert_eq!(err.kind, SendErrorKind::NotConfigured);
    }

    #[test]
    fn smtp_proxy_url_uses_remote_override_not_sni_host() {
        let mut smtp = SmtpSettings::new(
            "smtp.example.com".into(),
            465,
            "user".into(),
            None,
            SmtpTlsMode::Implicit,
        );
        smtp.remote_host = Some("smtp-backend.internal".into());
        smtp.remote_port = Some(2525);
        let config = config_with(Some(smtp));
        let smtp = config.smtp.as_ref().unwrap();
        let url = config.proxy.websocket_url_for_smtp(smtp).unwrap();
        assert!(url.contains("remote=smtp-backend.internal:2525"), "{url}");
        assert!(!url.contains("smtp.example.com"));
        assert_eq!(smtp.host, "smtp.example.com");
        assert_eq!(smtp.dial_host(), "smtp-backend.internal");
    }
}
