use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use tokio::io::{AsyncRead, AsyncWrite};
use web_sys::wasm_bindgen::prelude::*;
use web_sys::{BinaryType, MessageEvent, WebSocket};

pub struct WebSocketStream {
    web_socket: Option<WebSocket>,
    buf: Vec<u8>,
    wakers: Vec<Waker>,
}

impl WebSocketStream {
    pub fn new(url: &str) -> Self {
        let mut web_socket = WebSocket::new(url).unwrap();
        web_socket.set_binary_type(BinaryType::ArrayBuffer);

        let mut stream = Self {
            web_socket: Some(web_socket),
            buf: Vec::with_capacity(4096),
            wakers: Vec::new(),
        };

        let onopen_cb = Closure::<dyn FnMut(_)>::new(move |e: MessageEvent| {});
        let onmessage_cb = Closure::<dyn FnMut(_)>::new(move |e: MessageEvent| {
            if let Ok(abuf) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                let array = js_sys::Uint8Array::new(&abuf);
                let len = array.byte_length() as usize;

                let data = array.to_vec();

                stream.buf.extend_from_slice(&data);
                for waker in stream.wakers.drain(..) {
                    waker.wake();
                }
            } else {
                panic!("WebSocket message is not an ArrayBuffer");
            }
        });
        let onerror_cb = Closure::<dyn FnMut(_)>::new(move |e: MessageEvent| {
            println!("WebSocket error: {:?}", e);
        });
        let onclose_cb = Closure::<dyn FnMut(_)>::new(move |e: MessageEvent| {
            println!("WebSocket closed: {:?}", e);
            stream.web_socket = None;
        });

        if let Some(web_socket) = stream.web_socket.as_ref() {
            web_socket.set_onopen(Some(onopen_cb.as_ref().unchecked_ref()));
            web_socket.set_onmessage(Some(onmessage_cb.as_ref().unchecked_ref()));
            web_socket.set_onerror(Some(onerror_cb.as_ref().unchecked_ref()));
            web_socket.set_onclose(Some(onclose_cb.as_ref().unchecked_ref()));
        }

        stream
    }
}

impl AsyncRead for WebSocketStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.web_socket.is_none() {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "WebSocket not connected",
            )));
        }

        if self.buf.is_empty() {
            self.wakers.push(cx.waker().clone());
            Poll::Pending
        } else {
            let len = std::cmp::min(self.buf.len(), buf.remaining());
            buf.put_slice(&self.buf[..len]);
            self.buf.drain(..len);
            Poll::Ready(Ok(()))
        }
    }
}

impl AsyncWrite for WebSocketStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if let Some(web_socket) = self.web_socket.as_ref() {
            web_socket.send_with_u8_array(buf);
            Poll::Ready(Ok(buf.len()))
        } else {
            Poll::Ready(Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "WebSocket not connected",
            )))
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Some(web_socket) = self.web_socket.as_ref() {
            // There's no flush for WebSocket, just pretend success
            Poll::Ready(Ok(()))
        } else {
            Poll::Ready(Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "WebSocket not connected",
            )))
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Some(web_socket) = self.web_socket.as_ref() {
            web_socket.close();
            Poll::Ready(Ok(()))
        } else {
            Poll::Ready(Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "WebSocket not connected",
            )))
        }
    }
}
