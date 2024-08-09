use anyhow::{anyhow, Error};
use bytes::{BufMut, BytesMut};
use futures_core::Stream;
use openssl::{
    BIO_free, BIO_new, BIO_s_mem, ERR_get_error, EVP_add_cipher, EVP_aes_128_cbc, EVP_aes_128_gcm, EVP_aes_256_cbc, EVP_aes_256_gcm, EVP_chacha20_poly1305, EVP_sha1, EVP_sha256, EVP_sha384, OPENSSL_init_ssl, SSL_CTX_free, SSL_CTX_new, SSL_ctrl, SSL_free, SSL_new, SSL_set_bio, SSL_set_connect_state, TLSEXT_NAMETYPE_host_name, TLS_client_method, BIO, OPENSSL_INIT_SSL_DEFAULT, SSL, SSL_CTRL_SET_TLSEXT_HOSTNAME, SSL_CTX
};
use std::ffi::CString;
use std::io::{ErrorKind, Read, Write};
use std::ops::{Deref, DerefMut};
use std::ptr;
use std::task::{Poll, Waker};
use std::boxed::Box;

use dioxus_logger::tracing::error;

struct UniquePtr<T> {
    ptr: *mut T,
    deleter: Box<dyn FnMut(* mut T) + 'static>
}

impl<T> UniquePtr<T>
{
    pub fn new(ptr: *mut T, deleter: impl FnMut(* mut T) + 'static) -> Self {
        Self {
            ptr, deleter: Box::new(deleter)
        }
    }
}

impl<T> Deref for UniquePtr<T> {
    type Target = *mut T;
    fn deref(&self) -> &Self::Target {
        return &self.ptr;
    }
}

impl<T> DerefMut for UniquePtr<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        return &mut self.ptr;
    }
}

impl<T> Drop for UniquePtr<T> {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            (self.deleter)(self.ptr);
        }
    }
}

struct Ssl {
    ctx: UniquePtr<SSL_CTX>,
    ssl: UniquePtr<SSL>,
    input_bio: UniquePtr<BIO>,
    output_bio: UniquePtr<BIO>
}

impl Ssl {
    pub fn new() -> Result<Self, Error> {
        unsafe {
            if OPENSSL_init_ssl(OPENSSL_INIT_SSL_DEFAULT, ptr::null) == 0 {
                return Err(anyhow!("Failed to initialize OpenSSL library"));
            }
            EVP_add_cipher(EVP_chacha20_poly1305());
            EVP_add_cipher(EVP_aes_128_gcm());
            EVP_add_cipher(EVP_aes_256_gcm());
            EVP_add_cipher(EVP_aes_128_cbc());
            EVP_add_cipher(EVP_aes_256_cbc());
            EVP_add_cipher(EVP_sha256());
            EVP_add_cipher(EVP_sha384());
            EVP_add_cipher(EVP_sha1());

            let ctx = UniquePtr::<SSL_CTX>::new(SSL_CTX_new(TLS_client_method()), SSL_CTX_free);
            if ctx.is_null() {
                return Err(anyhow!(format!("Failed to create new SSL context: {}", ERR_get_error())));
            }

            let input_bio = UniquePtr::new(BIO_new(BIO_s_mem()), BIO_free);
            let output_bio = UniquePtr::new(BIO_new(BIO_s_mem()), BIO_free);

            let ssl = UniquePtr::<SSL>::new(SSL_new(ctx), SSL_free);
            if ssl.is_null() {
                return Err(anyhow!(format!("Failed to create SSL object: {}", ERR_get_error())));
            }
            SSL_set_bio(*ssl, *input_bio, *output_bio);
            SSL_set_connect_state(*ssl);

            Ok(Ssl {
                ctx,
                ssl,
                input_bio,
                output_bio
            })
        }
    }

    pub async fn connect(&self, hostname: &str) -> Result<(), Error> {
        let hostname_cstr = CString::new(hostname)?;
        unsafe {
            // bindgen refuses to generate bindings for macro SSL_set_tlsext_host_name, this is what the macro
            // really expands to:
            SSL_ctrl(*self.ssl, SSL_CTRL_SET_TLSEXT_HOSTNAME, TLSEXT_NAMETYPE_host_name, hostname_cstr.as_ptr());
        }

        doHandshake().await?;

        Ok(())
    }

    async fn doHandshake() -> Result<(), Error> {
        // TODO
    }
}


pub struct SslStream<CipherStream> {
    ssl_stream: ssl::SslStream<CipherStream>,
    plain_buffer: BytesMut,
    plain_waker: Option<Waker>,
}

impl<CipherStream> SslStream<CipherStream>
where
    CipherStream: Read + Write + Unpin,
{
    pub fn new(cipher_stream: CipherStream) -> std::io::Result<SslStream<CipherStream>> {
        let ssl_ctx = ssl::SslContextBuilder::new(ssl::SslMethod::tls_client())?.build();
        let ssl = ssl::Ssl::new(&ssl_ctx)?;
        let mut ssl_stream = ssl::SslStream::new(ssl, cipher_stream).map_err(|err| {
            Error::new(
                ErrorKind::Other,
                format!("Failed to create SSL stream: {}", err),
            )
        })?;
        ssl_stream.connect().map_err(|err| {
            Error::new(
                ErrorKind::Other,
                format!("Failed to connect to SSL stream: {}", err),
            )
        })?;
        Ok(Self {
            ssl_stream,
            plain_buffer: BytesMut::new(),
            plain_waker: None,
        })
    }

    fn try_read_ssl(&mut self) {
        let mut buffer = [0u8; 4096];
        match self.ssl_stream.ssl_read(&mut buffer) {
            Ok(bytes) => {
                self.plain_buffer
                    .extend_from_slice(buffer[..bytes].as_ref());
                if let Some(waker) = self.plain_waker.as_ref() {
                    waker.wake_by_ref();
                    self.plain_waker = None;
                }
            }
            Err(err) => {
                if err.code() == ssl::ErrorCode::WANT_READ {
                    // This is OK, we'll just try again later once we pushed more data to the SSL context
                } else {
                    match err.ssl_error() {
                        Some(err) => {
                            error!("Error when reading from SSL stream: {}", err);
                        }
                        None => {
                            error!(
                                "Unexpected error from SSL context when reading TLS data: {}",
                                err
                            );
                        }
                    }
                }
            }
        }
    }
}

impl<CipherStream> Write for SslStream<CipherStream>
where
    CipherStream: Read + Write + Unpin,
{
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.ssl_stream.write(buf)?;
        self.try_read_ssl();
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.try_read_ssl();
        Ok(())
    }
}

impl<CipherStream> Stream for SslStream<CipherStream>
where
    CipherStream: Read + Write + Unpin,
{
    type Item = Vec<u8>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.plain_buffer.is_empty() {
            this.plain_waker = Some(cx.waker().clone());
            Poll::Pending
        } else {
            let buffer = this.plain_buffer.to_vec();
            this.plain_buffer.clear();
            Poll::Ready(Some(buffer))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream::StreamExt;
    use openssl::asn1::Asn1Time;
    use openssl::pkey::PKey;
    use openssl::rsa::Rsa;
    use openssl::ssl::{SslAcceptor, SslMethod};
    use openssl::x509::{X509Builder, X509Name, X509NameBuilder, X509};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread::spawn;

    // Create SSL server
    fn run_ssl_server(port_channel: mpsc::Sender<u16>) {
        // Generate RSA keys
        let rsa = Rsa::generate(2048).unwrap();
        let pkey = PKey::from_rsa(rsa).unwrap();

        // Create X509 name
        let mut name_builder = X509NameBuilder::new().unwrap();
        name_builder
            .append_entry_by_text("CN", "localhost")
            .unwrap();
        let name: X509Name = name_builder.build();

        // Create X509 certificate
        let mut x509_builder = X509Builder::new().unwrap();
        x509_builder.set_subject_name(&name).unwrap();
        x509_builder.set_issuer_name(&name).unwrap();
        x509_builder.set_pubkey(&pkey).unwrap();
        x509_builder
            .set_not_before(&Asn1Time::days_from_now(0).unwrap())
            .unwrap();
        x509_builder
            .set_not_after(&Asn1Time::days_from_now(365).unwrap())
            .unwrap();
        x509_builder
            .sign(&pkey, openssl::hash::MessageDigest::sha256())
            .unwrap();
        let x509: X509 = x509_builder.build();

        // Create SSL acceptor
        let mut acceptor_builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
        acceptor_builder.set_private_key(&pkey).unwrap();
        acceptor_builder.set_certificate(&x509).unwrap();
        let acceptor = acceptor_builder.build();

        let tcp_listener = TcpListener::bind("0.0.0.0:0").expect("Failed to bind TCP listener");

        port_channel
            .send(tcp_listener.local_addr().unwrap().port())
            .expect("Failed to send port number to main thread");

        let (stream, _) = tcp_listener.accept().expect("Failed to accept connection");
        let mut ssl_stream = acceptor
            .accept(stream)
            .expect("Failed to accept SSL connection");
        let mut buf = [0u8; 1024];
        let read = ssl_stream.read(&mut buf).expect("Failed to read");
        let result_str = String::from_utf8(buf[..read].to_vec())
            .expect("Received data cannot be deserialized into string");
        let response = result_str.chars().into_iter().rev().collect::<String>();
        let response_data = response.as_bytes();
        ssl_stream
            .write_all(&response_data)
            .expect("Failed to write to SSL stream");
        ssl_stream.flush().expect("Failed to fush SSL stream");
        ssl_stream
            .shutdown()
            .expect("Failed to shutdown SSL stream");
    }

    #[tokio::test]
    async fn test_ssl_stream() {
        let (port_sender, port_receiver) = mpsc::channel();
        let handle = spawn(move || run_ssl_server(port_sender));

        let port = port_receiver
            .recv()
            .expect("Failed to receive port number from server");

        let stream =
            TcpStream::connect(format!("127.0.0.1:{}", port)).expect("Failed to connect to server");
        let mut ssl_stream = SslStream::new(stream).expect("Failed to create SSL stream");
        ssl_stream
            .write_all(b"Hello World")
            .expect("Failed to write to SSL stream");
        ssl_stream.flush().unwrap();

        let result = ssl_stream
            .next()
            .await
            .expect("Failed to read from SSL stream");
        assert_eq!(result, b"dlroW olleH");

        let _ = handle.join().expect("Failed to await handle");
    }
}
