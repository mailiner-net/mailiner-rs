use rustls::pki_types::{DnsName, ServerName};
use rustls::{ClientConfig, RootCertStore};
use std::io::{ErrorKind, Read, Write};
use std::sync::Arc;

use rustls::client::ClientConnection;

struct SslStream<EncryptedStream>
where
    EncryptedStream: Read + Write,
{
    connection: ClientConnection,
    encrypted_stream: EncryptedStream,
}

impl<EncryptedStream> SslStream<EncryptedStream>
where
    EncryptedStream: Read + Write,
{
    pub fn new(server: &str, encrypted_stream: EncryptedStream) -> Self {
        let cert_store = RootCertStore::empty();
        Self::new_with_cert_store(cert_store, server, encrypted_stream)
    }

    pub fn new_with_cert_store(cert_store: RootCertStore, server: &str, encrypted_stream: EncryptedStream) -> Self {
        let config = ClientConfig::builder()
            .with_root_certificates(cert_store)
            .with_no_client_auth();

        let name = DnsName::try_from(server.to_owned()).expect("Invalid server name");
        Self {
            connection: ClientConnection::new(Arc::new(config), ServerName::DnsName(name))
                .expect("Failed to create SSL ClientConnection"),
            encrypted_stream,
        }
    }

    fn try_read_ssl(&mut self) -> std::io::Result<()> {
        self.connection.read_tls(&mut self.encrypted_stream)?;
        Ok(())
    }

    fn try_write_ssl(&mut self) -> std::io::Result<()> {
        self.connection.write_tls(&mut self.encrypted_stream)?;
        Ok(())
    }
}

impl<EncryptedStream> Read for SslStream<EncryptedStream>
where
    EncryptedStream: Read + Write,
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.try_read_ssl()?;

        self.connection
            .reader()
            .read(buf)
            .map_err(|e| std::io::Error::new(ErrorKind::Other, e))
    }
}

impl<EncryptedStream> Write for SslStream<EncryptedStream>
where
    EncryptedStream: Read + Write,
{
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let res = self.connection.writer().write(buf)?;

        self.try_write_ssl()?;

        Ok(res)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.connection.writer().flush()?;
        self.try_write_ssl()?;
        Ok(())
    }
}

#[cfg(test)]
mod testing {
    use super::*;

    use std::{fs::File, io::{Read, Write}, thread};
    use std::sync::{Arc, mpsc::channel};
    use rustls_pki_types::CertificateDer;
    use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod, SslStream as OpenSSLStream};
    use std::net::{TcpListener, TcpStream};

    #[tokio::test]
    async fn test_ssl_stream() {
        let (tx, rx) = channel::<u16>();

        thread::spawn(move || {
            let listener = TcpListener::bind("0.0.0.0:0").expect("Failed to bind listener");
            let port = listener.local_addr().unwrap().port();
            tx.send(port).expect("Failed to send port");

            let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).expect("Failed to create acceptor");
            builder.set_private_key_file("tests/data/private_key.pem", SslFiletype::PEM).expect("Failed to set private key file");
            builder.set_certificate_chain_file("tests/data/certificate_bundle.pem").expect("Failed to set certificate chain file");
            builder.check_private_key().expect("Failed to check private key");
            let acceptor = Arc::new(builder.build());

            fn handle_client(stream: OpenSSLStream<TcpStream>) {
                let mut stream = stream;
                let mut buf = [0; 1024];
                stream.read(&mut buf).expect("Failed to read from stream");
                stream.write(&buf).expect("Failed to write to stream");
            }

            for stream in listener.incoming() {
                let stream = stream.expect("Failed to accept connection");
                let acceptor = Arc::clone(&acceptor);
                thread::spawn(move || {
                    let stream = acceptor.accept(stream).expect("Failed to accept connection");
                    handle_client(stream);
                });
            }
        });

        let port = rx.recv().expect("Failed to receive port");

        let mut cert_store = RootCertStore::empty();
        let mut cert_data = Vec::<u8>::new();
        File::open("tests/data/certificate.pem")
            .expect("Failed to open certificate file")
            .read_to_end(&mut cert_data)
            .expect("Failed to read certificate file");
        cert_store.add(CertificateDer::from_slice(&cert_data)).expect("Failed to add certificate to store");

        let tcp_stream = TcpStream::connect(format!("localhost:{}", port)).expect("Failed to connect to server");

        let mut ssl_stream = SslStream::new_with_cert_store(cert_store, "localhost", tcp_stream);
        ssl_stream.write(b"Hello World").expect("Failed to write to SSL stream");

        let mut buf = Vec::<u8>::new();
        ssl_stream.read_to_end(&mut buf).expect("Failed to read from SSL stream");
        assert_eq!(buf, b"Hello World");
    }

}