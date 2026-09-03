use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use dioxus::logger::tracing::{error, info};
use send_wrapper::SendWrapper;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use web_sys::wasm_bindgen::prelude::*;
use web_sys::{BinaryType, CloseEvent, Event, MessageEvent, WebSocket};

/// Browser WebSocket lifecycle as observed by the stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsReadyState {
    Connecting,
    Open,
    Error,
    Closed,
}

#[derive(Debug)]
pub struct WebSocketStreamInner {
    web_socket: SendWrapper<Option<WebSocket>>,
    ready_state: WsReadyState,
    read_buf: Vec<u8>,
    write_buf: Vec<u8>,
    read_wakers: Vec<Waker>,
    write_waiters: Vec<Waker>,
    open_wakers: Vec<Waker>,
    close_wakers: Vec<Waker>,
}

impl WebSocketStreamInner {
    fn try_new(url: &str) -> io::Result<Self> {
        let web_socket = WebSocket::new(url).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Failed to create WebSocket: {e:?}"),
            )
        })?;
        web_socket.set_binary_type(BinaryType::Arraybuffer);

        // JS objects are not Send, but in JavaScript we only have a single thread, so it's safe to wrap them
        // in SendWrapper to make the code that wants Send+Sync happy.
        let web_socket = SendWrapper::new(Some(web_socket));
        Ok(Self {
            web_socket,
            ready_state: WsReadyState::Connecting,
            read_buf: Vec::with_capacity(4096),
            write_buf: Vec::with_capacity(4096),
            read_wakers: Vec::new(),
            write_waiters: Vec::new(),
            open_wakers: Vec::new(),
            close_wakers: Vec::new(),
        })
    }

    fn wake_all(&mut self) {
        for waker in self.read_wakers.drain(..) {
            waker.wake();
        }
        for waker in self.write_waiters.drain(..) {
            waker.wake();
        }
        for waker in self.open_wakers.drain(..) {
            waker.wake();
        }
        for waker in self.close_wakers.drain(..) {
            waker.wake();
        }
    }

    fn fail(&mut self, state: WsReadyState) {
        self.ready_state = state;
        // Drop the socket handle so further I/O fails cleanly.
        // Prefer Option via DerefMut — SendWrapper::take consumes the wrapper.
        if let Some(ws) = std::mem::take(&mut *self.web_socket) {
            let _ = ws.close();
        }
        self.wake_all();
    }

    pub fn on_message(&mut self, e: MessageEvent) {
        if let Ok(abuf) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
            let array = js_sys::Uint8Array::new(&abuf);
            let data = array.to_vec();
            self.read_buf.extend_from_slice(&data);

            for waker in self.read_wakers.drain(..) {
                waker.wake();
            }
        }
    }

    pub fn on_close(&mut self, e: CloseEvent) {
        info!(
            "WebSocket closed: code={} reason={:?}",
            e.code(),
            e.reason()
        );
        // Normal close (1000) or any close still ends the stream — wake waiters with error
        // so connect does not hang in Pending forever.
        if self.ready_state == WsReadyState::Open || self.ready_state == WsReadyState::Connecting {
            self.ready_state = WsReadyState::Closed;
            if let Some(ws) = std::mem::take(&mut *self.web_socket) {
                let _ = ws.close();
            }
            self.wake_all();
        }
    }

    pub fn on_error(&mut self, e: Event) {
        error!("WebSocket error: {:?}", e);
        if self.ready_state == WsReadyState::Connecting || self.ready_state == WsReadyState::Open {
            self.fail(WsReadyState::Error);
        }
    }

    pub fn on_open(&mut self) {
        info!("WebSocket opened");
        self.ready_state = WsReadyState::Open;
        for waker in self.write_waiters.drain(..) {
            waker.wake();
        }
        for waker in self.open_wakers.drain(..) {
            waker.wake();
        }
    }

    fn io_error_for_state(&self) -> io::Error {
        match self.ready_state {
            WsReadyState::Error => io::Error::new(io::ErrorKind::BrokenPipe, "WebSocket error"),
            WsReadyState::Closed => {
                io::Error::new(io::ErrorKind::ConnectionAborted, "WebSocket closed")
            }
            WsReadyState::Connecting | WsReadyState::Open => {
                io::Error::new(io::ErrorKind::NotConnected, "WebSocket not connected")
            }
        }
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
    /// Create a WebSocket stream. Returns an error if the URL is invalid.
    pub fn try_new(url: &str) -> io::Result<Self> {
        let inner = Arc::new(Mutex::new(WebSocketStreamInner::try_new(url)?));

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
        let inner_clone = Arc::clone(&inner);
        let onerror_cb = Closure::<dyn FnMut(Event)>::new(move |e: Event| {
            inner_clone
                .lock()
                .expect("Failed to lock web socket")
                .on_error(e);
        });
        let inner_clone = Arc::clone(&inner);
        let onclose_cb = Closure::<dyn FnMut(CloseEvent)>::new(move |e: CloseEvent| {
            inner_clone
                .lock()
                .expect("Failed to lock web socket")
                .on_close(e);
        });

        {
            let guard = inner.lock().expect("Failed to lock web socket");
            if let Some(web_socket) = guard.web_socket.as_ref() {
                web_socket.set_onopen(Some(onopen_cb.as_ref().unchecked_ref()));
                web_socket.set_onmessage(Some(onmessage_cb.as_ref().unchecked_ref()));
                web_socket.set_onerror(Some(onerror_cb.as_ref().unchecked_ref()));
                web_socket.set_onclose(Some(onclose_cb.as_ref().unchecked_ref()));

                // Handle race where the socket is already open before handlers attach.
                if web_socket.ready_state() == WebSocket::OPEN {
                    drop(guard);
                    inner.lock().expect("Failed to lock web socket").on_open();
                }
            }
        }

        Ok(Self {
            inner,
            onopen_cb: SendWrapper::new(onopen_cb),
            onmessage_cb: SendWrapper::new(onmessage_cb),
            onerror_cb: SendWrapper::new(onerror_cb),
            onclose_cb: SendWrapper::new(onclose_cb),
        })
    }

    /// Current WebSocket readiness.
    pub fn ready_state(&self) -> WsReadyState {
        self.inner
            .lock()
            .expect("Failed to lock web socket")
            .ready_state
    }

    /// Wait until the WebSocket is open, or fail if it errors/closes first.
    pub fn wait_until_open(&self) -> WaitUntilOpen {
        WaitUntilOpen {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Cloneable watcher that completes when the socket errors or closes.
    pub fn death_watch(&self) -> WsDeathWatch {
        WsDeathWatch {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Close the socket and clear JS event handlers (idempotent).
    fn shutdown_socket(&mut self) {
        let mut inner = self.inner.lock().expect("Failed to lock web socket");
        if let Some(ws) = inner.web_socket.as_ref() {
            ws.set_onopen(None);
            ws.set_onmessage(None);
            ws.set_onerror(None);
            ws.set_onclose(None);
        }
        if let Some(ws) = std::mem::take(&mut *inner.web_socket) {
            let _ = ws.close();
        }
        if !matches!(
            inner.ready_state,
            WsReadyState::Closed | WsReadyState::Error
        ) {
            inner.ready_state = WsReadyState::Closed;
        }
        inner.wake_all();
    }
}

impl Drop for WebSocketStream {
    fn drop(&mut self) {
        // Ensure timeout/cancel paths close the browser WebSocket and drop JS callbacks
        // rather than relying on GC of half-open proxy connections.
        self.shutdown_socket();
    }
}

/// Completes when the WebSocket reaches `Error` or `Closed`.
#[derive(Clone)]
pub struct WsDeathWatch {
    inner: Arc<Mutex<WebSocketStreamInner>>,
}

impl Future for WsDeathWatch {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.inner.lock().expect("Failed to lock web socket");
        match inner.ready_state {
            WsReadyState::Error | WsReadyState::Closed => Poll::Ready(()),
            WsReadyState::Connecting | WsReadyState::Open => {
                inner.close_wakers.push(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// Future that completes when the underlying WebSocket reaches `Open`, or fails on error/close.
pub struct WaitUntilOpen {
    inner: Arc<Mutex<WebSocketStreamInner>>,
}

impl Future for WaitUntilOpen {
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.inner.lock().expect("Failed to lock web socket");
        match inner.ready_state {
            WsReadyState::Open => Poll::Ready(Ok(())),
            WsReadyState::Error | WsReadyState::Closed => {
                Poll::Ready(Err(inner.io_error_for_state()))
            }
            WsReadyState::Connecting => {
                inner.open_wakers.push(cx.waker().clone());
                Poll::Pending
            }
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
        if matches!(
            inner.ready_state,
            WsReadyState::Error | WsReadyState::Closed
        ) {
            if inner.read_buf.is_empty() {
                return Poll::Ready(Err(inner.io_error_for_state()));
            }
            // Drain remaining buffered data before surfacing the terminal error.
        } else if inner.web_socket.is_none() {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "WebSocket not connected",
            )));
        }

        if inner.read_buf.is_empty() {
            if matches!(
                inner.ready_state,
                WsReadyState::Error | WsReadyState::Closed
            ) {
                return Poll::Ready(Err(inner.io_error_for_state()));
            }
            inner.read_wakers.push(cx.waker().clone());
            Poll::Pending
        } else {
            let len = std::cmp::min(inner.read_buf.len(), buf.remaining());
            buf.put_slice(&inner.read_buf[..len]);
            inner.read_buf.drain(..len);
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
        let mut inner = self.inner.lock().expect("Failed to lock web socket");
        if matches!(
            inner.ready_state,
            WsReadyState::Error | WsReadyState::Closed
        ) {
            return Poll::Ready(Err(inner.io_error_for_state()));
        }

        if let Some(web_socket) = inner.web_socket.as_ref() {
            if web_socket.ready_state() == WebSocket::OPEN {
                web_socket.send_with_u8_array(buf).map_err(|e| {
                    error!("Failed to send WebSocket message: {:?}", e);
                    io::Error::other("Failed to send WebSocket message")
                })?;
                info!("WebSocket wrote {} bytes", buf.len());
                Poll::Ready(Ok(buf.len()))
            } else if web_socket.ready_state() == WebSocket::CONNECTING
                || inner.ready_state == WsReadyState::Connecting
            {
                inner.write_waiters.push(cx.waker().clone());
                Poll::Pending
            } else {
                Poll::Ready(Err(inner.io_error_for_state()))
            }
        } else {
            Poll::Ready(Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "WebSocket not connected",
            )))
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let inner = self.inner.lock().expect("Failed to lock web socket");
        if matches!(
            inner.ready_state,
            WsReadyState::Error | WsReadyState::Closed
        ) {
            return Poll::Ready(Err(inner.io_error_for_state()));
        }
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

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut inner = self.inner.lock().expect("Failed to lock web socket");
        if let Some(web_socket) = inner.web_socket.as_ref() {
            let _ = web_socket.close();
            inner.ready_state = WsReadyState::Closed;
            Poll::Ready(Ok(()))
        } else if matches!(
            inner.ready_state,
            WsReadyState::Error | WsReadyState::Closed
        ) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Ready(Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "WebSocket not connected",
            )))
        }
    }
}
