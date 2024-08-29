use std::{io::Cursor, sync::OnceLock};

use futures::{AsyncReadExt, AsyncWriteExt};
use mailiner_core::transport_stream::TransportStream;
use rustls::RootCertStore;
use rustls_pemfile::Item;
use wasm_bindgen_test::wasm_bindgen_test;
use tracing::info;

const CA_CERT: &[u8; 1318] = include_bytes!("certs/ca-cert.crt");

static INIT_LOGGER: OnceLock<()> = OnceLock::new();

fn init_logger() {
    INIT_LOGGER.get_or_init(|| {
        dioxus_logger::init(tracing::Level::TRACE).expect("Failed to initialize logger");
    });
}

async fn test_stream_operation(mut stream: TransportStream) {

    stream
        .write("Hello World".as_bytes())
        .await
        .expect("Failed to send data");

    let mut buf = [0u8; 1024];
    let bytes = stream
        .read(&mut buf)
        .await
        .expect("Failed to read from socket");

    assert_eq!(&buf[..bytes], b"Hello World".as_slice());
}

#[wasm_bindgen_test]
async fn test_plain_transport_stream() {
    init_logger();

    let stream = TransportStream::connect_plain("ws://127.0.0.1:14000/plain")
        .await
        .expect("Failed to connect to server");
    test_stream_operation(stream).await;
}

#[wasm_bindgen_test]
async fn test_tls_transport_stream() {
    init_logger();

    let mut cert_store = RootCertStore::empty();

    let mut cursor = Cursor::new(&CA_CERT);
    let (added, skipped) = cert_store.add_parsable_certificates(rustls_pemfile::read_all(&mut cursor).filter_map(|item| {
        match item.expect("Failed to load/parse certificate") {
            Item::X509Certificate(cert) => Some(cert),
            _ => None,
        }
    }));
    info!("Added {} certificates, skipped {}", added, skipped);

    let stream = TransportStream::connect_with_tls("ws://127.0.0.1:14000/tls", "test.mailiner.net", cert_store)
        .await
        .expect("Failed to connect to server");
    test_stream_operation(stream).await;
}

#[wasm_bindgen_test]
async fn test_transport_stream_with_nonexistent_proxy() {
    init_logger();

    TransportStream::connect_plain("ws://127.0.0.1:14002")
        .await
        .expect_err("Expected to fail to connect");
}
