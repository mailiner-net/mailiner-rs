use rustls::pki_types::{DnsName, ServerName};
use rustls::{ClientConfig, RootCertStore, StreamOwned as RustlsStream};
use std::io::{ErrorKind, Read, Write};
use std::sync::Arc;

use rustls::client::ClientConnection;

pub struct SslStream<EncryptedStream>
where
    EncryptedStream: Read + Write,
{
    inner_stream: RustlsStream<ClientConnection, EncryptedStream>,
}

impl<EncryptedStream> SslStream<EncryptedStream>
where
    EncryptedStream: Read + Write,
{
    pub fn new(server: &str, encrypted_stream: EncryptedStream) -> Self {
        let cert_store = RootCertStore::empty();
        Self::new_with_cert_store(cert_store, server, encrypted_stream)
    }

    pub fn new_with_cert_store(
        cert_store: RootCertStore,
        server: &str,
        encrypted_stream: EncryptedStream,
    ) -> Self {
        let config = ClientConfig::builder()
            .with_root_certificates(cert_store)
            .with_no_client_auth();

        let name = DnsName::try_from(server.to_owned()).expect("Invalid server name");
        let connection = ClientConnection::new(Arc::new(config), ServerName::DnsName(name))
                .expect("Failed to create SSL ClientConnection");

        Self {
            inner_stream: RustlsStream::new(connection, encrypted_stream)
        }
    }
}

impl<EncryptedStream> Read for SslStream<EncryptedStream>
where
    EncryptedStream: Read + Write
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner_stream.read(buf)
    }
}

impl<EncryptedStream> Write for SslStream<EncryptedStream>
where
    EncryptedStream: Read + Write,
{
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner_stream.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner_stream.flush()
    }
}

#[cfg(test)]
mod testing {
    use super::*;

    use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod, SslStream as OpenSSLStream};
    use rustls_pki_types::CertificateDer;
    use std::net::{TcpListener, TcpStream};
    use std::sync::{mpsc::channel, Arc};
    use std::{
        fs::File,
        io::{Read, Write},
        thread,
    };

    #[tokio::test]
    async fn test_ssl_stream() {
        let (tx, rx) = channel::<u16>();

        thread::spawn(move || {
            let listener = TcpListener::bind("0.0.0.0:0").expect("Failed to bind listener");
            let port = listener.local_addr().unwrap().port();
            tx.send(port).expect("Failed to send port");

            let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls())
                .expect("Failed to create acceptor");
            builder
                .set_private_key_file("../tests/data/private_key.pem", SslFiletype::PEM)
                .expect("Failed to set private key file");
            builder
                .set_certificate_chain_file("../tests/data/certificate_bundle.pem")
                .expect("Failed to set certificate chain file");
            builder
                .check_private_key()
                .expect("Failed to check private key");
            let acceptor = Arc::new(builder.build());

            fn handle_client(stream: OpenSSLStream<TcpStream>) {
                let mut stream = stream;
                let mut buf = [0; 1024];
                stream.read(&mut buf).expect("Failed to read from stream");
                println!("RECEIVED DATA FROM STREAM");
                stream.write(&buf).expect("Failed to write to stream");
                stream.flush().expect("Failed  to flush stream");
            }

            for stream in listener.incoming() {
                let stream = stream.expect("Failed to accept connection");
                let acceptor = Arc::clone(&acceptor);
                thread::spawn(move || {
                    let stream = acceptor
                        .accept(stream)
                        .expect("Failed to accept connection");
                    handle_client(stream);
                });
            }
        });

        let port = rx.recv().expect("Failed to receive port");

        let mut cert_store = RootCertStore::empty();
        let mut cert_data = Vec::<u8>::new();
        File::open("../tests/data/certificate_bundle.der")
            .expect("Failed to open certificate file")
            .read_to_end(&mut cert_data)
            .expect("Failed to read certificate file");
        cert_store
            .add(CertificateDer::from_slice(&cert_data))
            .expect("Failed to add certificate to store");

        let tcp_stream =
            TcpStream::connect(format!("localhost:{}", port)).expect("Failed to connect to server");
        println!("TCP connection established");

        let mut ssl_stream = SslStream::new_with_cert_store(cert_store, "localhost", tcp_stream);
        println!("SSL stream created");
        ssl_stream
            .write("Hello World".to_string().as_bytes())
            .expect("Failed to write to SSL stream");
        println!("Stream written");
        ssl_stream.flush().expect("Failed to flush SSL stream");
        println!("Stream flushed");

        let mut str = String::new();
        println!("Waiting to read from stream");
        ssl_stream
            .read_to_string(&mut str)
            .expect("Failed to read from SSL Stream");
        assert_eq!(str, "Hello World");
    }
}
