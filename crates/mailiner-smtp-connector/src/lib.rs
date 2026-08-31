//! Short-lived SMTP submission over a caller-owned stream (plus rustls).

use std::fmt::Debug;
use std::sync::Arc;

use async_smtp::authentication::{Credentials, Mechanism};
use async_smtp::extension::{ClientId, ServerInfo};
use async_smtp::{Envelope, SendableEmail, SmtpClient, SmtpTransport};
use mailiner_core::{AccountId, SendErrorKind, SubmitReceipt, SubmitRequest};
use rustls::{ClientConfig, RootCertStore};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, BufReader};
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::{client::TlsStream, TlsConnector};
use tracing::info;

/// SMTP connector errors (no secrets).
#[derive(Debug, Error)]
pub enum SmtpError {
    #[error("{message}")]
    Classified {
        kind: SendErrorKind,
        message: String,
    },
}

impl SmtpError {
    pub fn kind(&self) -> SendErrorKind {
        match self {
            Self::Classified { kind, .. } => *kind,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Classified { message, .. } => message,
        }
    }

    fn classified(kind: SendErrorKind, message: impl Into<String>) -> Self {
        Self::Classified {
            kind,
            message: message.into(),
        }
    }
}

/// Trust the bundled local-mail CA in debug builds / `local-ca`.
/// See `mailiner-imap-connector` — same CA, same reason (SMTP 465/587).
fn add_local_dev_ca(root_store: &mut RootCertStore) {
    #[cfg(any(debug_assertions, feature = "local-ca"))]
    {
        use rustls_pki_types::pem::PemObject;
        use rustls_pki_types::CertificateDer;

        const PEM: &[u8] = include_bytes!("../../../docker/mail/tls/ca.crt");
        for item in CertificateDer::pem_slice_iter(PEM) {
            match item {
                Ok(cert) => {
                    if let Err(e) = root_store.add(cert) {
                        tracing::warn!("local mail CA not added: {e}");
                    }
                }
                Err(e) => tracing::warn!("local mail CA parse failed: {e}"),
            }
        }
    }
    #[cfg(not(any(debug_assertions, feature = "local-ca")))]
    {
        let _ = root_store;
    }
}

/// One-shot SMTP client. Password is never stored.
pub struct SmtpConnector {
    account_id: AccountId,
    host: String,
    #[allow(dead_code)]
    port: u16,
    username: String,
    hello_name: String,
}

impl SmtpConnector {
    pub fn new(
        account_id: AccountId,
        host: String,
        port: u16,
        username: String,
        hello_name: String,
    ) -> Self {
        Self {
            account_id,
            host,
            port,
            username,
            hello_name,
        }
    }

    pub fn account_id(&self) -> &AccountId {
        &self.account_id
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    /// rustls over the provided byte stream (SNI = `host`). Used after implicit
    /// TLS connect and after STARTTLS.
    pub async fn wrap_tls<S>(&self, stream: S) -> Result<TlsStream<S>, SmtpError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let mut root_store = RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        add_local_dev_ca(&mut root_store);
        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        let tls = TlsConnector::from(Arc::new(config));
        let server_name = ServerName::try_from(self.host.clone()).map_err(|e| {
            SmtpError::classified(
                SendErrorKind::TlsOrSni,
                format!("Invalid SMTP server name: {e}"),
            )
        })?;
        info!(host = %self.host, "SMTP TLS handshake");
        tls.connect(server_name, stream).await.map_err(|e| {
            SmtpError::classified(SendErrorKind::TlsOrSni, format!("SMTP TLS failed: {e}"))
        })
    }

    /// Speak 220 + EHLO + STARTTLS on a plaintext stream. Returns the inner
    /// stream ready for rustls. Does not AUTH.
    pub async fn starttls_handshake<S>(&self, stream: S) -> Result<S, SmtpError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Debug,
    {
        let hello = ClientId::new(self.hello_name.clone());
        let client = SmtpClient::new()
            .hello_name(hello)
            .smtp_utf8(true)
            .pipelining(false);
        let buffered = BufReader::new(stream);
        let transport = SmtpTransport::new(client, buffered)
            .await
            .map_err(map_smtp_connect)?;
        info!(host = %self.host, "SMTP STARTTLS");
        let upgraded = transport.starttls().await.map_err(map_smtp_starttls)?;
        Ok(upgraded.into_inner())
    }

    /// EHLO + AUTH + MAIL/RCPT/DATA + QUIT on an already-secure stream.
    pub async fn submit<S>(
        &self,
        stream: S,
        password: &str,
        request: &SubmitRequest,
    ) -> Result<SubmitReceipt, SmtpError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Debug,
    {
        self.submit_on(stream, password, request, true).await
    }

    /// Plaintext stream: EHLO + STARTTLS + rustls, then AUTH + DATA. Never AUTH
    /// before the TLS wrap.
    pub async fn submit_starttls<S>(
        &self,
        stream: S,
        password: &str,
        request: &SubmitRequest,
    ) -> Result<SubmitReceipt, SmtpError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Debug,
    {
        let plain = self.starttls_handshake(stream).await?;
        let tls = self.wrap_tls(plain).await?;
        self.submit_on(tls, password, request, false).await
    }

    /// EHLO + AUTH + QUIT (no MAIL/DATA) on an already-secure stream.
    pub async fn test<S>(&self, stream: S, password: &str) -> Result<(), SmtpError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Debug,
    {
        self.test_on(stream, password, true).await
    }

    /// Plaintext stream: EHLO + STARTTLS + rustls, then AUTH + QUIT. Never AUTH
    /// before the TLS wrap.
    pub async fn test_starttls<S>(&self, stream: S, password: &str) -> Result<(), SmtpError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Debug,
    {
        let plain = self.starttls_handshake(stream).await?;
        let tls = self.wrap_tls(plain).await?;
        self.test_on(tls, password, false).await
    }

    async fn submit_on<S>(
        &self,
        stream: S,
        password: &str,
        request: &SubmitRequest,
        expect_greeting: bool,
    ) -> Result<SubmitReceipt, SmtpError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Debug,
    {
        let mut transport = self
            .open_and_auth(stream, password, expect_greeting)
            .await?;
        let envelope = build_envelope(request)?;
        let email = SendableEmail::new(envelope, request.rfc822.clone());
        let response = transport.send(email).await.map_err(map_smtp_send)?;
        let reply = response.message.first().map(|s| truncate_smtp_reply(s));
        let _ = transport.quit().await;
        Ok(SubmitReceipt {
            message_id: request.message_id.clone(),
            server_reply: reply,
        })
    }

    async fn test_on<S>(
        &self,
        stream: S,
        password: &str,
        expect_greeting: bool,
    ) -> Result<(), SmtpError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Debug,
    {
        let mut transport = self
            .open_and_auth(stream, password, expect_greeting)
            .await?;
        let _ = transport.quit().await;
        Ok(())
    }

    async fn open_and_auth<S>(
        &self,
        stream: S,
        password: &str,
        expect_greeting: bool,
    ) -> Result<SmtpTransport<BufReader<S>>, SmtpError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Debug,
    {
        let hello = ClientId::new(self.hello_name.clone());
        let mut client = SmtpClient::new()
            .hello_name(hello.clone())
            .smtp_utf8(true)
            .pipelining(false);
        if !expect_greeting {
            client = client.without_greeting();
        }
        let buffered = BufReader::new(stream);
        let mut transport = SmtpTransport::new(client, buffered)
            .await
            .map_err(map_smtp_connect)?;

        // Second EHLO so we can parse AUTH without relying on private server_info.
        let ehlo = transport
            .get_mut()
            .ehlo(hello)
            .await
            .map_err(map_smtp_connect)?;
        let info = ServerInfo::from_response(&ehlo).map_err(|e| {
            SmtpError::classified(SendErrorKind::Internal, format!("EHLO parse failed: {e}"))
        })?;

        let mechanism = if info.supports_auth_mechanism(Mechanism::Plain) {
            Mechanism::Plain
        } else if info.supports_auth_mechanism(Mechanism::Login) {
            Mechanism::Login
        } else {
            return Err(SmtpError::classified(
                SendErrorKind::Auth,
                "Server advertised no supported AUTH mechanism (PLAIN/LOGIN).",
            ));
        };

        let creds = Credentials::new(self.username.clone(), password.to_string());
        transport
            .auth(mechanism, &creds)
            .await
            .map_err(map_smtp_auth)?;
        Ok(transport)
    }
}

fn build_envelope(request: &SubmitRequest) -> Result<Envelope, SmtpError> {
    let from = async_smtp::EmailAddress::new(request.mail_from.clone()).map_err(|e| {
        SmtpError::classified(SendErrorKind::Internal, format!("Invalid MAIL FROM: {e}"))
    })?;
    let mut to = Vec::new();
    for rcpt in &request.rcpt_to {
        to.push(async_smtp::EmailAddress::new(rcpt.clone()).map_err(|e| {
            SmtpError::classified(SendErrorKind::Internal, format!("Invalid RCPT TO: {e}"))
        })?);
    }
    Envelope::new(Some(from), to).map_err(|e| {
        SmtpError::classified(SendErrorKind::Internal, format!("Invalid envelope: {e}"))
    })
}

fn map_smtp_connect(err: async_smtp::error::Error) -> SmtpError {
    SmtpError::classified(SendErrorKind::NetworkOrProxy, err.to_string())
}

fn map_smtp_starttls(err: async_smtp::error::Error) -> SmtpError {
    let text = err.to_string();
    if text
        .to_ascii_lowercase()
        .contains("does not support starttls")
    {
        return SmtpError::classified(
            SendErrorKind::TlsOrSni,
            "SMTP server did not advertise STARTTLS.",
        );
    }
    SmtpError::classified(SendErrorKind::NetworkOrProxy, text)
}

fn map_smtp_auth(_err: async_smtp::error::Error) -> SmtpError {
    SmtpError::classified(SendErrorKind::Auth, "SMTP authentication failed.")
}

fn smtp_response_text(resp: &async_smtp::response::Response) -> String {
    if resp.message.is_empty() {
        resp.code.to_string()
    } else {
        format!("{} {}", resp.code, resp.message.join("; "))
    }
}

fn classify_permanent(resp: &async_smtp::response::Response) -> SendErrorKind {
    let text = resp.message.join(" ").to_ascii_lowercase();
    if resp.has_code(552) || text.contains("message too large") {
        SendErrorKind::MessageTooLarge
    } else if resp.has_code(550) || resp.has_code(551) || resp.has_code(553) {
        SendErrorKind::RecipientRejected
    } else {
        SendErrorKind::Permanent
    }
}

fn map_smtp_send(err: async_smtp::error::Error) -> SmtpError {
    use async_smtp::error::Error::*;
    match err {
        Transient(resp) => {
            SmtpError::classified(SendErrorKind::Transient, smtp_response_text(&resp))
        }
        Permanent(resp) => {
            let kind = classify_permanent(&resp);
            SmtpError::classified(kind, smtp_response_text(&resp))
        }
        Io(e) => SmtpError::classified(SendErrorKind::NetworkOrProxy, e.to_string()),
        other => {
            let text = other.to_string();
            if text.starts_with("timeout:") {
                SmtpError::classified(SendErrorKind::Timeout, text)
            } else {
                SmtpError::classified(SendErrorKind::Permanent, text)
            }
        }
    }
}

fn truncate_smtp_reply(s: &str) -> String {
    const MAX: usize = 200;
    if s.chars().count() <= MAX {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(MAX).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};

    fn connector() -> SmtpConnector {
        SmtpConnector::new(
            AccountId::new("acc"),
            "smtp.example.com".into(),
            465,
            "user@example.com".into(),
            "example.com".into(),
        )
    }

    fn request() -> SubmitRequest {
        SubmitRequest {
            mail_from: "me@example.com".into(),
            rcpt_to: vec!["you@example.com".into()],
            rfc822: b"From: me@example.com\r\nTo: you@example.com\r\nSubject: Hi\r\n\r\nHello\r\n"
                .to_vec(),
            message_id: "<id@example.com>".into(),
        }
    }

    async fn write_all(w: &mut (impl AsyncWriteExt + Unpin), s: &str) {
        w.write_all(s.as_bytes()).await.unwrap();
        w.flush().await.unwrap();
    }

    async fn read_cmd(r: &mut (impl AsyncReadExt + Unpin), buf: &mut Vec<u8>) -> String {
        buf.clear();
        let mut tmp = [0u8; 512];
        loop {
            let n = r.read(&mut tmp).await.unwrap();
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
    async fn submit_plain_auth_and_data() {
        let (client, mut server) = duplex(64 * 1024);
        let conn = connector();
        let req = request();

        let server_task = tokio::spawn(async move {
            write_all(&mut server, "220 smtp.example.com ESMTP\r\n").await;
            let mut buf = Vec::new();
            // first EHLO (SmtpTransport::new)
            let _ = read_cmd(&mut server, &mut buf).await;
            write_all(
                &mut server,
                "250-smtp.example.com\r\n250 AUTH PLAIN LOGIN\r\n",
            )
            .await;
            // second EHLO
            let _ = read_cmd(&mut server, &mut buf).await;
            write_all(
                &mut server,
                "250-smtp.example.com\r\n250 AUTH PLAIN LOGIN\r\n",
            )
            .await;
            // AUTH
            let auth = read_cmd(&mut server, &mut buf).await;
            assert!(auth.to_ascii_uppercase().contains("AUTH"), "{auth}");
            write_all(&mut server, "235 2.7.0 Authentication successful\r\n").await;
            // MAIL
            let mail = read_cmd(&mut server, &mut buf).await;
            assert!(mail.to_ascii_uppercase().contains("MAIL FROM"), "{mail}");
            write_all(&mut server, "250 2.1.0 OK\r\n").await;
            // RCPT
            let rcpt = read_cmd(&mut server, &mut buf).await;
            assert!(rcpt.to_ascii_uppercase().contains("RCPT TO"), "{rcpt}");
            write_all(&mut server, "250 2.1.5 OK\r\n").await;
            // DATA
            let data = read_cmd(&mut server, &mut buf).await;
            assert!(data.to_ascii_uppercase().contains("DATA"), "{data}");
            write_all(&mut server, "354 End data with <CR><LF>.<CR><LF>\r\n").await;
            // message until .\r\n
            let mut body = Vec::new();
            let mut tmp = [0u8; 1024];
            loop {
                let n = server.read(&mut tmp).await.unwrap();
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&tmp[..n]);
                if body.windows(5).any(|w| w == b"\r\n.\r\n") {
                    break;
                }
            }
            write_all(&mut server, "250 2.0.0 OK queued\r\n").await;
            let quit = read_cmd(&mut server, &mut buf).await;
            assert!(quit.to_ascii_uppercase().contains("QUIT"), "{quit}");
            write_all(&mut server, "221 2.0.0 Bye\r\n").await;
        });

        let receipt = conn.submit(client, "secret", &req).await.unwrap();
        assert_eq!(receipt.message_id, "<id@example.com>");
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn no_auth_mechanism_is_auth_error() {
        let (client, mut server) = duplex(16 * 1024);
        let conn = connector();

        let server_task = tokio::spawn(async move {
            write_all(&mut server, "220 smtp.example.com ESMTP\r\n").await;
            let mut buf = Vec::new();
            let _ = read_cmd(&mut server, &mut buf).await;
            write_all(&mut server, "250-smtp.example.com\r\n250 SIZE 10000\r\n").await;
            let _ = read_cmd(&mut server, &mut buf).await;
            write_all(&mut server, "250-smtp.example.com\r\n250 SIZE 10000\r\n").await;
        });

        let err = conn.test(client, "secret").await.unwrap_err();
        assert_eq!(err.kind(), SendErrorKind::Auth);
        let _ = server_task.await;
    }

    #[tokio::test]
    async fn starttls_handshake_issues_command_and_returns_stream() {
        let (client, mut server) = duplex(16 * 1024);
        let conn = connector();

        let server_task = tokio::spawn(async move {
            write_all(&mut server, "220 smtp.example.com ESMTP\r\n").await;
            let mut buf = Vec::new();
            let ehlo = read_cmd(&mut server, &mut buf).await;
            assert!(ehlo.to_ascii_uppercase().contains("EHLO"), "{ehlo}");
            write_all(
                &mut server,
                "250-smtp.example.com\r\n250-STARTTLS\r\n250 AUTH PLAIN\r\n",
            )
            .await;
            let starttls = read_cmd(&mut server, &mut buf).await;
            assert!(
                starttls.to_ascii_uppercase().contains("STARTTLS"),
                "{starttls}"
            );
            write_all(&mut server, "220 2.0.0 Ready to start TLS\r\n").await;
        });

        let _stream = conn.starttls_handshake(client).await.unwrap();
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn starttls_not_advertised_is_tls_error() {
        let (client, mut server) = duplex(16 * 1024);
        let conn = connector();

        let server_task = tokio::spawn(async move {
            write_all(&mut server, "220 smtp.example.com ESMTP\r\n").await;
            let mut buf = Vec::new();
            let _ = read_cmd(&mut server, &mut buf).await;
            write_all(
                &mut server,
                "250-smtp.example.com\r\n250 AUTH PLAIN LOGIN\r\n",
            )
            .await;
        });

        let err = conn.starttls_handshake(client).await.unwrap_err();
        assert_eq!(err.kind(), SendErrorKind::TlsOrSni);
        assert!(
            err.message().contains("did not advertise STARTTLS"),
            "{}",
            err.message()
        );
        let _ = server_task.await;
    }

    #[tokio::test]
    async fn starttls_handshake_does_not_auth() {
        let (client, mut server) = duplex(16 * 1024);
        let conn = connector();

        let server_task = tokio::spawn(async move {
            write_all(&mut server, "220 smtp.example.com ESMTP\r\n").await;
            let mut buf = Vec::new();
            let _ = read_cmd(&mut server, &mut buf).await;
            write_all(
                &mut server,
                "250-smtp.example.com\r\n250-STARTTLS\r\n250 AUTH PLAIN\r\n",
            )
            .await;
            let second = read_cmd(&mut server, &mut buf).await;
            assert!(
                second.to_ascii_uppercase().contains("STARTTLS"),
                "expected STARTTLS, got {second}"
            );
            assert!(
                !second.to_ascii_uppercase().contains("AUTH"),
                "AUTH must not run before TLS wrap: {second}"
            );
            write_all(&mut server, "220 2.0.0 Ready to start TLS\r\n").await;
        });

        let _stream = conn.starttls_handshake(client).await.unwrap();
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn auth_without_greeting_starts_with_ehlo() {
        let (client, mut server) = duplex(16 * 1024);
        let conn = connector();

        let server_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            // No 220 — post-STARTTLS SmtpClient::without_greeting.
            let ehlo = read_cmd(&mut server, &mut buf).await;
            assert!(ehlo.to_ascii_uppercase().contains("EHLO"), "{ehlo}");
            write_all(
                &mut server,
                "250-smtp.example.com\r\n250 AUTH PLAIN LOGIN\r\n",
            )
            .await;
            let _ = read_cmd(&mut server, &mut buf).await;
            write_all(
                &mut server,
                "250-smtp.example.com\r\n250 AUTH PLAIN LOGIN\r\n",
            )
            .await;
            let auth = read_cmd(&mut server, &mut buf).await;
            assert!(auth.to_ascii_uppercase().contains("AUTH"), "{auth}");
            write_all(&mut server, "235 2.7.0 Authentication successful\r\n").await;
            let quit = read_cmd(&mut server, &mut buf).await;
            assert!(quit.to_ascii_uppercase().contains("QUIT"), "{quit}");
            write_all(&mut server, "221 2.0.0 Bye\r\n").await;
        });

        conn.test_on(client, "secret", false).await.unwrap();
        server_task.await.unwrap();
    }
}
