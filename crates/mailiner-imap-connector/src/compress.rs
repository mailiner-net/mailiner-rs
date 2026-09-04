//! RFC 4978 `COMPRESS DEFLATE` transport.
//!
//! Raw DEFLATE (RFC 1951) with zlib `windowBits` in −8…−15 — no zlib wrapper.
//! `async-compression` uses `flate2`'s `rust_backend` (`miniz_oxide`), which is
//! pure Rust and compiles for `wasm32-unknown-unknown`.
//!
//! Writes are flushed with `Z_SYNC_FLUSH` so the peer can see each complete
//! IMAP command (async-imap flushes after `encode`).

use std::fmt::Debug;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use async_compression::tokio::bufread::DeflateDecoder;
use async_compression::tokio::write::DeflateEncoder;
use async_imap::types::{Capabilities, Capability};
use tokio::io::{AsyncRead, AsyncWrite, BufReader, ReadBuf};

fn atom_is_compress_deflate(name: &str) -> bool {
    name.eq_ignore_ascii_case("COMPRESS=DEFLATE")
}

/// True when post-auth `CAPABILITY` lists `COMPRESS=DEFLATE`.
pub(crate) fn advertises_deflate(caps: &Capabilities) -> bool {
    caps.has_str("COMPRESS=DEFLATE")
        || caps.iter().any(|c| match c {
            Capability::Atom(name) => atom_is_compress_deflate(name),
            _ => false,
        })
}

/// Duplex raw-DEFLATE wrapper (same layout as async-imap's `DeflateStream`).
#[derive(Debug)]
pub(crate) struct DeflateIo<T> {
    inner: DeflateDecoder<BufReader<DeflateEncoder<T>>>,
}

impl<T: AsyncRead + AsyncWrite + Unpin> DeflateIo<T> {
    pub(crate) fn new(stream: T) -> Self {
        let stream = DeflateEncoder::new(stream);
        let stream = BufReader::new(stream);
        Self {
            inner: DeflateDecoder::new(stream),
        }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> AsyncRead for DeflateIo<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> AsyncWrite for DeflateIo<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn compress_deflate_atom_is_case_insensitive() {
        assert!(atom_is_compress_deflate("COMPRESS=DEFLATE"));
        assert!(atom_is_compress_deflate("compress=deflate"));
        assert!(!atom_is_compress_deflate("COMPRESS=GZIP"));
        assert!(!atom_is_compress_deflate("DEFLATE"));
    }

    #[tokio::test]
    async fn deflate_io_roundtrip() {
        let (a, b) = tokio::io::duplex(16 * 1024);
        let mut client = DeflateIo::new(a);
        let mut server = DeflateIo::new(b);

        client.write_all(b"A001 LIST \"\" *\r\n").await.unwrap();
        client.flush().await.unwrap();

        let mut buf = vec![0u8; 64];
        let n = server.read(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"A001 LIST \"\" *\r\n");

        server
            .write_all(b"* LIST () \"/\" INBOX\r\n")
            .await
            .unwrap();
        server.flush().await.unwrap();
        let n = client.read(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"* LIST () \"/\" INBOX\r\n");
    }

    #[tokio::test]
    async fn deflate_io_matches_flate2_raw_sync_flush() {
        use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress, Status};

        let (mut raw, peer) = tokio::io::duplex(16 * 1024);
        let mut wrapped = DeflateIo::new(peer);

        wrapped.write_all(b"A002 NOOP\r\n").await.unwrap();
        wrapped.flush().await.unwrap();

        let mut compressed = vec![0u8; 512];
        let n = raw.read(&mut compressed).await.unwrap();
        assert!(n > 0);
        assert!(
            !compressed[..n].starts_with(b"A002"),
            "wire bytes must be deflated"
        );

        let mut dec = Decompress::new(false);
        let mut plain = vec![0u8; 64];
        let before = dec.total_out();
        let status = dec
            .decompress(&compressed[..n], &mut plain, FlushDecompress::Sync)
            .unwrap();
        let out = (dec.total_out() - before) as usize;
        assert!(matches!(status, Status::Ok | Status::BufError));
        assert_eq!(&plain[..out], b"A002 NOOP\r\n");

        let mut enc = Compress::new(Compression::default(), false);
        let input = b"* OK still here\r\n";
        let mut out = vec![0u8; 128];
        enc.compress(input, &mut out, FlushCompress::Sync).unwrap();
        let m = enc.total_out() as usize;
        raw.write_all(&out[..m]).await.unwrap();
        raw.flush().await.unwrap();

        let mut got = vec![0u8; 64];
        let n = wrapped.read(&mut got).await.unwrap();
        assert_eq!(&got[..n], input);
    }
}
