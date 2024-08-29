use futures::{AsyncRead, AsyncWrite};
use futures_rustls::TlsConnector;
use rustls::{ClientConfig, RootCertStore};
use rustls_pki_types::{DnsName, ServerName};
use std::fmt::Debug;
use std::pin::{pin, Pin};
use std::sync::Arc;
use tracing::trace;
use ws_stream_wasm::{WsErr, WsMeta};

/// A meta-trait that combines all the requirements on the underlying stream
trait AsyncStream: AsyncRead + AsyncWrite + Unpin {}

/// Implement this meta trait for any type that satisfies the constraints.
/// This way we can use `Box<dyn AsyncStream>` to hold a pointer to any
/// object that implements `AsyncRead`, `AsyncWrite` and `Unpin``.
impl<T> AsyncStream for T where T: AsyncRead + AsyncWrite + Unpin {}

/// Rrror from the ImapStream
#[derive(Debug)]
pub enum Error {
    WebSocketError(WsErr),
    InvalidDnsNameError,
    IOError(std::io::Error),
}

enum InnerStream {
    Tls(Box<dyn AsyncStream>),
    Plain(Box<dyn AsyncStream>),
}

impl InnerStream {
    fn stream(&mut self) -> &mut Box<dyn AsyncStream> {
        match self {
            Self::Plain(stream) => stream,
            Self::Tls(stream) => stream,
        }
    }
}

/// ImapStream is a high-level stream of IMAP commands and responses between the
/// client and  the IMAP server.
///
/// Internally, the ImapStream takes care of establishing WebSocket connection to
/// our WS<->TCP proxy, and encrypting and decrypting the IMAP data using TLS, if
/// necessary. All of this should be transparent to the user, all they need to know
/// is the name and port of the target IMAP server.
pub struct TransportStream {
    proxy_url: String,
    inner_stream: InnerStream,
}
impl TransportStream {
    /// Establishes secure connection to the IMAP server.
    ///
    /// Internally, this will block until WebSocket connection is established and TLS handshake with
    /// the IMAP server is completed, returning a stream that is fully prepared to handle IMAP communication.
    pub async fn connect_with_tls(
        proxy_url: &str,
        server_name: &str,
        cert_store: RootCertStore,
    ) -> Result<Self, Error> {
        trace!("Connecting to WS proxy at {}", proxy_url);
        let (_ws, ws_stream) = WsMeta::connect(proxy_url, None)
            .await
            .map_err(Error::WebSocketError)?;
        trace!("Connected, preparing TLS layer");
        let config = Arc::new(
            ClientConfig::builder()
                .with_root_certificates(cert_store)
                .with_no_client_auth(),
        );
        let server_name = ServerName::DnsName(
            DnsName::try_from(server_name.to_owned()).map_err(|_| Error::InvalidDnsNameError)?,
        );

        let connector = TlsConnector::from(config);
        trace!("Establishing TLS connection with {:?}", server_name);
        let tls_stream = connector
            .connect(server_name, ws_stream.into_io())
            .await
            .map_err(Error::IOError)?;
        trace!("TLS connection established");

        Ok(Self {
            proxy_url: proxy_url.to_owned(),
            inner_stream: InnerStream::Tls(Box::new(tls_stream)),
        })
    }

    /// Establishes a plain-text connection to the IMAP server.
    pub async fn connect_plain(proxy_url: &str) -> Result<Self, Error> {
        let (_ws, ws_stream) = WsMeta::connect(proxy_url, None)
            .await
            .map_err(Error::WebSocketError)?;

        Ok(Self {
            proxy_url: proxy_url.to_owned(),
            inner_stream: InnerStream::Plain(Box::new(ws_stream.into_io())),
        })
    }
}

impl Debug for TransportStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stream_type = match self.inner_stream {
            InnerStream::Plain(_) => "PlainStream",
            InnerStream::Tls(_) => "TlsStream",
        };
        f.write_fmt(format_args!(
            "ImapStream[proxy={}, stream={}]",
            self.proxy_url, stream_type
        ))
    }
}

impl AsyncRead for TransportStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.get_mut();

        pin!(&mut this.inner_stream.stream()).poll_read(cx, buf)
    }
}

impl AsyncWrite for TransportStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        pin!(&mut this.inner_stream.stream()).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        pin!(&mut this.inner_stream.stream()).poll_write_vectored(cx, bufs)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        pin!(&mut this.inner_stream.stream()).poll_flush(cx)
    }

    fn poll_close(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        pin!(&mut this.inner_stream.stream()).poll_close(cx)
    }
}
