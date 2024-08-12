use bytes::{BufMut, BytesMut};
use dioxus_logger::tracing::{error, info, warn};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::{
    io::{Error, ErrorKind, Write},
    task::Waker,
};

use wasm_bindgen::prelude::*;
use web_sys::{ErrorEvent, MessageEvent, WebSocket};

use futures_core::Stream;

pub struct WSStream {
    socket: WebSocket,

    buffer: Mutex<BytesMut>,
    stream_waker: Mutex<Option<Waker>>,
}

impl WSStream {
    pub fn new(url: &str) -> std::io::Result<Arc<WSStream>> {
        let socket = WebSocket::new(url).map_err(|err| {
            Error::new(
                ErrorKind::ConnectionAborted,
                format!(
                    "Failed to create a websocket to {}: {}",
                    url,
                    err.as_string().unwrap_or("Unknown error".into())
                ),
            )
        })?;

        let this = Arc::new(Self {
            socket,
            buffer: Mutex::new(BytesMut::with_capacity(1024)),
            stream_waker: Mutex::new(None),
        });

        let this_clone = Arc::clone(&this);
        this.socket
            .set_binary_type(web_sys::BinaryType::Arraybuffer);
        let on_message_cb =
            Closure::<dyn FnMut(_)>::new(move |event: MessageEvent| this_clone.on_message(event));
        this.socket
            .set_onmessage(Some(on_message_cb.as_ref().unchecked_ref()));
        on_message_cb.forget(); // forget the callback to keep it alive

        let this_clone = Arc::clone(&this);
        let on_error_cb =
            Closure::<dyn FnMut(_)>::new(move |event: ErrorEvent| this_clone.on_error(event));
        this.socket
            .set_onerror(Some(on_error_cb.as_ref().unchecked_ref()));
        on_error_cb.forget();

        let this_clone = Arc::clone(&this);
        let on_open_cb = Closure::<dyn FnMut()>::new(move || this_clone.on_open());
        this.socket
            .set_onopen(Some(on_open_cb.as_ref().unchecked_ref()));
        on_open_cb.forget();

        let this_clone = Arc::clone(&this);
        let on_close_cb = Closure::<dyn FnMut()>::new(move || this_clone.on_close());
        this.socket
            .set_onclose(Some(on_close_cb.as_ref().unchecked_ref()));
        on_close_cb.forget();

        Ok(this)
    }

    fn on_open(&self) {
        info!("WebSocket connection established");
    }

    fn on_close(&self) {
        info!("WebSocket connection closed by remote");
    }

    fn on_message(&self, event: MessageEvent) {
        if let Ok(abuf) = event.data().dyn_into::<js_sys::ArrayBuffer>() {
            let array = js_sys::Uint8Array::new(&abuf);
            self.buffer
                .lock()
                .expect("Failed to lock buffer lock")
                .put(array.to_vec().as_ref());
            let mut waker_locked = self
                .stream_waker
                .lock()
                .expect("Failed to lock stream_waker lock");
            if let Some(waker) = waker_locked.as_ref() {
                waker.wake_by_ref();
                *waker_locked = None;
            }
        } else {
            error!("Received WebSocket data in an unexpected format (expected ArrayBuffer)");
        }
    }

    fn on_error(&self, event: ErrorEvent) {
        let msg = event
            .as_string()
            .unwrap_or("Unknown WebSocket error has occurred".into());
        warn!("WebSocket error: {}", msg);
    }
}

impl Write for WSStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.socket.ready_state() {
            WebSocket::CONNECTING => Err(Error::new(
                ErrorKind::WouldBlock,
                "WebSocket is not still in CONNECTING state",
            )),
            WebSocket::CLOSING | WebSocket::CLOSED => Err(Error::new(
                ErrorKind::NotConnected,
                "WebSocket is not connected",
            )),
            WebSocket::OPEN => match self.socket.send_with_u8_array(buf) {
                Ok(_) => Ok(buf.len()),
                Err(err) => Err(Error::new(
                    ErrorKind::BrokenPipe,
                    err.as_string().unwrap_or(
                        "Unknown error occurred while trying to send data to WebSocket".into(),
                    ),
                )),
            },
            _ => Err(Error::new(
                ErrorKind::NotConnected,
                "WebSocket is in invalid state",
            )),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        // No-op, websocket doesn't do any bufferring.
        Ok(())
    }
}

impl Stream for WSStream {
    type Item = Vec<u8>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let mut buffer = self.buffer.lock().expect("Failed to lock buffer");
        if socket_closed(&self.socket) {
            Poll::Ready(None)
        } else if !buffer.is_empty() {
            let result = Poll::Ready(Some(buffer.to_vec()));
            buffer.clear();
            result
        } else {
            let mut stream_waker = self
                .stream_waker
                .lock()
                .expect("Failed to lock stream_waker");
            *stream_waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

fn socket_closed(socket: &WebSocket) -> bool {
    match socket.ready_state() {
        WebSocket::CLOSED | WebSocket::CLOSING => true,
        WebSocket::CONNECTING | WebSocket::OPEN => false,
        _ => {
            error!("Unexpected WebSocket state {}", socket.ready_state());
            true
        }
    }
}
