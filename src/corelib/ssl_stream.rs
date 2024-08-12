use anyhow::{anyhow, Error};
use bytes::{BufMut, BytesMut};
use dioxus::prelude::spawn;
use futures::stream::SplitSink;
use futures::{Sink, SinkExt};
use futures_util::{Stream, StreamExt};
use openssl_bindings::{
    BIO_ctrl_pending, BIO_free, BIO_new, BIO_read, BIO_s_mem, BIO_write, ERR_error_string,
    ERR_get_error, EVP_add_cipher, EVP_add_digest, EVP_aes_128_cbc, EVP_aes_128_gcm,
    EVP_aes_256_cbc, EVP_aes_256_gcm, EVP_chacha20_poly1305, EVP_sha1, EVP_sha256, EVP_sha384,
    OPENSSL_init_ssl, SSL_CTX_free, SSL_CTX_new, SSL_ctrl, SSL_do_handshake, SSL_free,
    SSL_get_error, SSL_new, SSL_pending, SSL_read, SSL_set_bio, SSL_set_connect_state,
    SSL_shutdown, SSL_write, TLSEXT_NAMETYPE_host_name, TLS_client_method, BIO,
    OPENSSL_INIT_SSL_DEFAULT, SSL, SSL_CTRL_SET_TLSEXT_HOSTNAME, SSL_CTX, SSL_ERROR_WANT_READ,
    SSL_ERROR_WANT_WRITE,
};
use std::ffi::{c_void, CStr, CString};
use std::io::{ErrorKind, Read, Write};
use std::ptr;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

use dioxus_logger::tracing::error;

use crate::utils::UniquePtr;

struct Ssl<EncryptedStream>
where
    EncryptedStream: Stream + Sink<Vec<u8>> + Unpin + 'static,
{
    ctx: UniquePtr<SSL_CTX>,
    ssl: UniquePtr<SSL>,
    input_bio: UniquePtr<BIO, i32>,
    output_bio: UniquePtr<BIO, i32>,
    encrypted_sink: SplitSink<EncryptedStream, Vec<u8>>,

    handshake_done: Mutex<bool>,

    plain_waker: Mutex<Option<Waker>>,
}

impl<'a, EncryptedStream> Ssl<EncryptedStream>
where
    EncryptedStream: Stream<Item = Vec<u8>> + Sink<Vec<u8>> + Unpin,
{
    pub fn new(encrypted_stream: EncryptedStream) -> Result<Arc<Ssl<EncryptedStream>>, Error> {
        let (sink, mut stream) = encrypted_stream.split();
        let ssl = unsafe {
            if OPENSSL_init_ssl(OPENSSL_INIT_SSL_DEFAULT as u64, ptr::null()) == 0 {
                return Err(anyhow!("Failed to initialize OpenSSL library"));
            }
            EVP_add_cipher(EVP_chacha20_poly1305());
            EVP_add_cipher(EVP_aes_128_gcm());
            EVP_add_cipher(EVP_aes_256_gcm());
            EVP_add_cipher(EVP_aes_128_cbc());
            EVP_add_cipher(EVP_aes_256_cbc());
            EVP_add_digest(EVP_sha256());
            EVP_add_digest(EVP_sha384());
            EVP_add_digest(EVP_sha1());

            let ctx = UniquePtr::new(SSL_CTX_new(TLS_client_method()), SSL_CTX_free);
            if ctx.is_null() {
                return Err(anyhow!(format!(
                    "Failed to create new SSL context: {}",
                    ERR_get_error()
                )));
            }

            let input_bio = UniquePtr::new(BIO_new(BIO_s_mem()), BIO_free);
            let output_bio = UniquePtr::new(BIO_new(BIO_s_mem()), BIO_free);

            let ssl = UniquePtr::new(SSL_new(*ctx), SSL_free);
            if ssl.is_null() {
                return Err(anyhow!(format!(
                    "Failed to create SSL object: {}",
                    ERR_get_error()
                )));
            }
            SSL_set_bio(*ssl, *input_bio, *output_bio);
            SSL_set_connect_state(*ssl);

            Arc::new(Ssl {
                ctx,
                ssl,
                input_bio,
                output_bio,
                encrypted_sink: sink,
                handshake_done: Mutex::new(false),
                plain_waker: Mutex::new(None),
            })
        };

        let ssl_clone = Arc::clone(&ssl);
        spawn(async move {
            loop {
                let data = stream.next().await;
                match data {
                    Some(data) => {
                        ssl_clone.write_tls_data(&data).await.unwrap();
                    }
                    None => break,
                }
            }
        });

        Ok(ssl)
    }

    pub async fn connect(&mut self, hostname: &str) -> Result<(), Error> {
        let hostname_cstr = CString::new(hostname)?;
        unsafe {
            // bindgen refuses to generate bindings for macro SSL_set_tlsext_host_name, this is what the macro
            // really expands to:
            SSL_ctrl(
                *self.ssl,
                SSL_CTRL_SET_TLSEXT_HOSTNAME as i32,
                TLSEXT_NAMETYPE_host_name as i64,
                hostname_cstr.as_ptr() as *mut c_void,
            );
        }

        self.do_handshake().await?;

        Ok(())
    }

    async fn do_handshake(&self) -> Result<(), Error> {
        let res = unsafe { SSL_do_handshake(*self.ssl) };

        self.send_tls_data().await?;

        if res == 1 {
            *self
                .handshake_done
                .lock()
                .expect("Failed to lock handshake_done") = true;
            Ok(())
        } else {
            let ssl_err = unsafe { SSL_get_error(*self.ssl, res) } as u32;
            if ssl_err == SSL_ERROR_WANT_READ || ssl_err == SSL_ERROR_WANT_WRITE {
                // Waiting for more data
                Ok(())
            } else {
                unsafe {
                    let str = CStr::from_ptr(ERR_error_string(ssl_err as u64, ptr::null_mut()));
                    Err(anyhow!(format!(
                        "Failed to perform SSL handshake: {}",
                        str.to_string_lossy()
                    )))
                }
            }
        }
    }

    async fn write_plain_data(&self, data: &[u8]) -> Result<usize, Error> {
        let mut written = 0usize;
        while written < data.len() {
            let res = unsafe {
                SSL_write(
                    *self.ssl,
                    data[written..].as_ptr() as *const c_void,
                    (data.len() - written) as i32,
                )
            };
            if res <= 0 {
                let ssl_err = unsafe { SSL_get_error(*self.ssl, res) } as u32;
                if ssl_err == SSL_ERROR_WANT_WRITE {
                    self.send_tls_data().await?; // flush data
                    continue; // retry
                }
                unsafe {
                    let str = CStr::from_ptr(ERR_error_string(ssl_err as u64, ptr::null_mut()));
                    return Err(anyhow!(format!(
                        "Failed to write data to SSL stream: {}",
                        str.to_string_lossy()
                    )));
                }
            }
            written += res as usize;
        }

        // flush
        self.send_tls_data().await?;

        Ok(written)
    }

    async fn write_tls_data(&self, data: &[u8]) -> Result<(), Error> {
        let mut written = 0usize;
        while written < data.len() {
            let res = unsafe {
                BIO_write(
                    *self.input_bio,
                    data[written..].as_ptr() as *const c_void,
                    (data.len() - written) as i32,
                )
            };
            written += res as usize;
        }

        if !*self
            .handshake_done
            .lock()
            .expect("Failed to lock handshake_done")
        {
            self.do_handshake().await?;
        }

        if *self
            .handshake_done
            .lock()
            .expect("Failed to lock handshake_done")
        {
            if let Some(waker) = &*self.plain_waker.lock().expect("Failed to lock plain_waker") {
                waker.wake_by_ref();
            }
        }

        Ok(())
    }

    fn read_plain_data(&self) -> Result<BytesMut, Error> {
        let mut buffer = BytesMut::new();
        let mut read_buffer = [0u8; 4096];
        loop {
            let res = unsafe {
                SSL_read(
                    *self.ssl,
                    read_buffer.as_mut_ptr() as *mut c_void,
                    read_buffer.len() as i32,
                )
            };
            if res <= 0 {
                let ssl_err = unsafe { SSL_get_error(*self.ssl, res) } as u32;
                if ssl_err != SSL_ERROR_WANT_READ && ssl_err != SSL_ERROR_WANT_WRITE {
                    unsafe {
                        let str = CStr::from_ptr(ERR_error_string(ssl_err as u64, ptr::null_mut()));
                        return Err(anyhow!(format!(
                            "Failed to read data from SSL stream: {}",
                            str.to_string_lossy()
                        )));
                    }
                }
                return Ok(buffer);
            } else {
                buffer.put(&read_buffer[..res as usize]);
            }
        }
    }

    async fn send_tls_data(&self) -> Result<(), Error> {
        let mut read_buffer = [0u8; 4096];
        while unsafe { BIO_ctrl_pending(*self.output_bio) } > 0 {
            let read = unsafe {
                BIO_read(
                    *self.output_bio,
                    read_buffer.as_mut_ptr() as *mut c_void,
                    read_buffer.len() as i32,
                )
            };
            if read > 0 {
                self.encrypted_sink
                    .send(read_buffer[..read as usize].to_vec())
                    .await
                    //.map_err(|e| anyhow::Error::msg(format!("Failed to send data into encrypted stream: {:?}", e))?;
            }
        }

        Ok(())
    }

    async fn close(&self) -> Result<(), Error> {
        unsafe {
            SSL_shutdown(*self.ssl);
        }
        self.send_tls_data().await
    }
}

impl<EncryptedStream> Stream for Ssl<EncryptedStream>
where
    EncryptedStream: Stream<Item = Vec<u8>> + Sink<Vec<u8>> + Unpin,
{
    type Item = BytesMut;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        if unsafe { SSL_pending(*self.ssl) } > 0 {
            let data = self.read_plain_data();
            if let Ok(data) = data {
                Poll::Ready(Some(data))
            } else {
                Poll::Ready(None)
            }
        } else {
            *self.plain_waker.lock().expect("Failed to lock plain_waker") =
                Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

pub struct SslStream<EncryptedStream>
where
    EncryptedStream: Stream<Item = Vec<u8>> + Sink<Vec<u8>> + Unpin + 'static,
{
    ssl: Arc<Ssl<EncryptedStream>>,
    plain_buffer: BytesMut,
    plain_waker: Option<Waker>,
}

impl<EncryptedStream> SslStream<EncryptedStream>
where
    EncryptedStream: Stream<Item = Vec<u8>> + Sink<Vec<u8>> + Unpin + 'static,
{
    pub fn new(encrypted_stream: EncryptedStream) -> Result<SslStream<EncryptedStream>, Error> {
        Ok(Self {
            ssl: Ssl::new(encrypted_stream)?,
            plain_buffer: BytesMut::new(),
            plain_waker: None,
        })
    }
}

impl<CipherStream> Write for SslStream<CipherStream>
where
    CipherStream: Stream<Item = Vec<u8>> + Sink<Vec<u8>> + Unpin,
{
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.ssl
            .write_plain_data(buf)
            .map_err(|e| std::io::Error::new(ErrorKind::Other, e.to_string()))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        //self.try_read_ssl();
        Ok(())
    }
}

impl<CipherStream> Stream for SslStream<CipherStream>
where
    CipherStream: Stream<Item = Vec<u8>> + Sink<Vec<u8>> + Unpin,
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
