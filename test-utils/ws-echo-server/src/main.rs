use env_logger::Env;
use log::{info, warn};
use openssl::ssl::{Ssl, SslAcceptor, SslFiletype, SslMethod};
use std::path::Path;
use std::pin::pin;
use std::{io::Error, net::SocketAddr};
use stream_ws::{WsByteStream, WsErrorKind, WsMessageHandle, WsMessageKind};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_openssl::SslStream;

use axum::extract::{ws::Message, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::{routing::get, Router};

const LOCALHOST: [u8; 4] = [127, 0, 0, 1];

async fn plain_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_failed_upgrade(move |e| {
        panic!("WebSocket upgrade failed: {}", e);
    })
    .on_upgrade(move |mut socket| async move {
        loop {
            match socket.recv().await {
                Some(Ok(msg)) => {
                    let data = msg.into_data();
                    info!("Received {:?}", data);
                    if data.len() > 0 {
                        let _ = socket.send(Message::Binary(data)).await;
                    }
                }
                Some(Err(e)) => warn!("Error receiving WS message: {:?}", e),
                None => return,
            };
        }
    })
}

type AxumWsByteStream<S> = WsByteStream<S, Message, axum::Error, AxumWsMessageHandler>;

struct AxumWsMessageHandler;

impl WsMessageHandle<Message, axum::Error> for AxumWsMessageHandler {
    fn message_into_kind(msg: Message) -> stream_ws::WsMessageKind {
        match msg {
            Message::Binary(msg) => WsMessageKind::Bytes(msg),
            Message::Close(_) => WsMessageKind::Close,
            _ => WsMessageKind::Other,
        }
    }

    fn message_from_bytes<T: Into<Vec<u8>>>(bytes: T) -> Message {
        Message::Binary(bytes.into())
    }

    fn error_into_kind(e: axum::Error) -> stream_ws::WsErrorKind {
        WsErrorKind::Other(Box::new(e))
    }
}

async fn tls_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_failed_upgrade(move |e| {
        panic!("WebSocket upgrade failed: {}", e);
    })
    .on_upgrade(move |socket| async move {
        let mut acceptor_builder = SslAcceptor::mozilla_intermediate(SslMethod::tls_server())
            .expect("Failed to create acceptor");
        let path = Path::new(&format!(
            "{}/../../tests/certs/domain-bundle.pem",
            env!("CARGO_MANIFEST_DIR")
        ))
        .canonicalize()
        .expect("Failed to process path");
        acceptor_builder
            .set_private_key_file(path.display().to_string(), SslFiletype::PEM)
            .expect("Failed to set private key");
        let path = Path::new(&format!(
            "{}/../../tests/certs/domain.crt",
            env!("CARGO_MANIFEST_DIR")
        ))
        .canonicalize()
        .expect("Failed to process path");
        acceptor_builder
            .set_certificate_chain_file(path.display().to_string())
            .expect("Failed to set certificate chain");
        acceptor_builder
            .check_private_key()
            .expect("Failed to check private key");
        let acceptor = acceptor_builder.build();

        let ws_stream = AxumWsByteStream::new(socket);

        let ssl = Ssl::new(acceptor.context()).expect("Failed to create SSL object");
        let mut ssl_stream =
            pin!(SslStream::new(ssl, ws_stream).expect("Failed to create SSL stream"));

        ssl_stream
            .as_mut()
            .accept()
            .await
            .expect("Failed to accept SSL connection");

        loop {
            let mut buf = [0u8; 1024];
            let size = ssl_stream
                .read(&mut buf)
                .await
                .expect("Failed to read from SSL stream");
            if size == 0 {
                break;
            }
            ssl_stream
                .write(&buf[..size])
                .await
                .expect("Failed to write to SSL stream");
            ssl_stream
                .flush()
                .await
                .expect("Failed to flush SSL stream");
        }
    })
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init_from_env(Env::default().filter_or("LOG_LEVEL", "debug"));
    openssl::init();

    let router = Router::new()
        .route("/plain", get(plain_handler))
        .route("/tls", get(tls_handler));

    axum_server::bind(SocketAddr::from((LOCALHOST, 14000)))
        .serve(router.clone().into_make_service())
        .await?;
    Ok(())
}
