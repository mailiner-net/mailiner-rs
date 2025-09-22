use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use send_wrapper::SendWrapper;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use web_sys::wasm_bindgen::prelude::*;
use web_sys::{BinaryType, CloseEvent, Event, MessageEvent, WebSocket};

#[derive(Debug)]
pub struct WebSocketStreamInner {
    web_socket: SendWrapper<Option<WebSocket>>,
    buf: Vec<u8>,
    wakers: Vec<Waker>,
}

impl WebSocketStreamInner {
    pub fn new(url: &str) -> Self {
        let web_socket = WebSocket::new(url).unwrap();
        web_socket.set_binary_type(BinaryType::Arraybuffer);

        // JS objects are not Send, but in JavaScript we only have a single thread, so it's safe to wrap them
        // in SendWrapper to make the code that wants Send+Sync happy.
        let web_socket = SendWrapper::new(Some(web_socket));
        Self {
            web_socket,
            buf: Vec::with_capacity(4096),
            wakers: Vec::new(),
        }
    }

    pub fn on_message(&mut self, e: MessageEvent) {
        if let Ok(abuf) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
            let array = js_sys::Uint8Array::new(&abuf);
            let data = array.to_vec();
            self.buf.extend_from_slice(&data);

            for waker in self.wakers.drain(..) {
                waker.wake();
            }
        }
    }

    pub fn on_close(&mut self, e: CloseEvent) {
        println!("WebSocket closed: {:?}", e);
    }

    pub fn on_open(&mut self) {
        println!("WebSocket opened");
    }
}

#[derive(Debug)]
pub struct WebSocketStream {
    inner: Arc<Mutex<WebSocketStreamInner>>,

    onopen_cb: SendWrapper<Closure<dyn FnMut()>>,
    onmessage_cb: SendWrapper<Closure<dyn FnMut(MessageEvent)>>,
    onerror_cb: SendWrapper<Closure<dyn FnMut(Event)>>,
    onclose_cb: SendWrapper<Closure<dyn FnMut(CloseEvent)>>,
}

impl WebSocketStream {
    pub fn new(url: &str) -> Self {
        let inner = Arc::new(Mutex::new(WebSocketStreamInner::new(url)));

        let inner_clone = Arc::clone(&inner);
        let onopen_cb = Closure::<dyn FnMut()>::new(move || {
            inner_clone
                .lock()
                .expect("Failed to lock web socket")
                .on_open();
        });
        let inner_clone = Arc::clone(&inner);
        let onmessage_cb = Closure::<dyn FnMut(_)>::new(move |e: MessageEvent| {
            inner_clone
                .lock()
                .expect("Failed to lock web socket")
                .on_message(e);
        });
        let onerror_cb = Closure::<dyn FnMut(Event)>::new(move |e: Event| {
            println!("WebSocket error: {:?}", e);
        });
        let inner_clone = Arc::clone(&inner);
        let onclose_cb = Closure::<dyn FnMut(CloseEvent)>::new(move |e: CloseEvent| {
            inner_clone
                .lock()
                .expect("Failed to lock web socket")
                .on_close(e);
        });

        {
            let inner = inner.lock().expect("Failed to lock web socket");
            if let Some(web_socket) = inner.web_socket.as_ref() {
                web_socket.set_onopen(Some(onopen_cb.as_ref().unchecked_ref()));
                web_socket.set_onmessage(Some(onmessage_cb.as_ref().unchecked_ref()));
                web_socket.set_onerror(Some(onerror_cb.as_ref().unchecked_ref()));
                web_socket.set_onclose(Some(onclose_cb.as_ref().unchecked_ref()));
            }
        }

        Self {
            inner,
            onopen_cb: SendWrapper::new(onopen_cb),
            onmessage_cb: SendWrapper::new(onmessage_cb),
            onerror_cb: SendWrapper::new(onerror_cb),
            onclose_cb: SendWrapper::new(onclose_cb),
        }
    }
}

impl AsyncRead for WebSocketStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let mut inner = self.inner.lock().expect("Failed to lock web socket");
        if inner.web_socket.is_none() {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "WebSocket not connected",
            )));
        }

        if inner.buf.is_empty() {
            inner.wakers.push(cx.waker().clone());
            Poll::Pending
        } else {
            let len = std::cmp::min(inner.buf.len(), buf.remaining());
            buf.put_slice(&inner.buf[..len]);
            inner.buf.drain(..len);
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
        let inner = self.inner.lock().expect("Failed to lock web socket");
        if let Some(web_socket) = inner.web_socket.as_ref() {
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
        let inner = self.inner.lock().expect("Failed to lock web socket");
        if inner.web_socket.is_some() {
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
        let inner = self.inner.lock().expect("Failed to lock web socket");
        if let Some(web_socket) = inner.web_socket.as_ref() {
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
