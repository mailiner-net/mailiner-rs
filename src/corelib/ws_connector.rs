use anyhow::{anyhow, Error};
use dioxus::prelude::*;
use dioxus_logger::tracing::{error, info, warn};
use futures::channel::mpsc;
use futures_util::{SinkExt, StreamExt};
use js_sys::wasm_bindgen::{prelude::Closure, JsCast};
use wasm_bindgen_futures::spawn_local;
use web_sys::{BinaryType, ErrorEvent, MessageEvent, WebSocket};

pub enum WSConnectorMessage {
    Connect { url: String },
    Close,
    Data { payload: Vec<u8> },
}

pub enum WSConnectorEvents {
    Connected,
    Error { error: Error },
    Data { payload: Vec<u8> },
}

pub struct WSConnector {
    coro_handle: Coroutine<WSConnectorMessage>,
    event_rx: UnboundedReceiver<WSConnectorEvents>,
}

impl WSConnector {
    fn new() -> Self {
        let (mut event_tx, event_rx) = mpsc::unbounded::<WSConnectorEvents>();
        let coro_handle =
            use_coroutine(|mut rx: UnboundedReceiver<WSConnectorMessage>| async move {
                let websocket = match connect_websocket(&mut rx).await {
                    Ok(websocket) => websocket,
                    Err(err) => {
                        let _ = event_tx.send(WSConnectorEvents::Error { error: err }).await;
                        return;
                    }
                };

                let _ = run_websocket_loop(websocket, &mut rx, &mut event_tx).await;
            });

        Self {
            coro_handle,
            event_rx,
        }
    }
}

async fn connect_websocket(
    rx: &mut UnboundedReceiver<WSConnectorMessage>,
) -> Result<web_sys::WebSocket, Error> {
    match rx.next().await {
        Some(WSConnectorMessage::Connect { url }) => WebSocket::new(&url).map_err(|e| {
            anyhow::Error::msg(
                e.as_string()
                    .unwrap_or("Unexpect error when creating WebSocket".into()),
            )
        }),
        Some(_) => {
            panic!("Invalid command received while in pre-connect state!")
        }
        None => {
            info!("Message stream to WS coroutine has been closed!");
            Err(anyhow!("Message Stream to WS coroutine was closed!"))
        }
    }
}

async fn run_websocket_loop(
    websocket: web_sys::WebSocket,
    rx: &mut UnboundedReceiver<WSConnectorMessage>,
    tx: &mut UnboundedSender<WSConnectorEvents>,
) -> Result<(), Error> {
    websocket.set_binary_type(BinaryType::Arraybuffer);
    let tx_clone = tx.clone();
    let on_message_cb = Closure::<dyn FnMut(_)>::new(move |e: MessageEvent| {
        if let Ok(abuf) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
            let array = js_sys::Uint8Array::new(&abuf);

            let mut tx_clone = tx_clone.clone();
            spawn_local(async move {
                if let Err(e) = tx_clone
                    .send(WSConnectorEvents::Data {
                        payload: array.to_vec(),
                    })
                    .await
                {
                    error!("Failed to send event from WebSocket coroutine: {:?}", e);
                }
            });
        } else {
            error!("Received WebSocket message does not contain ArrayBuffer data!");
        }
    });
    websocket.set_onmessage(Some(on_message_cb.as_ref().unchecked_ref()));
    on_message_cb.forget();

    let tx_clone = tx.clone();
    let on_error_cb = Closure::<dyn FnMut(_)>::new(move |e: ErrorEvent| {
        let mut tx_clone = tx_clone.clone();
        spawn_local(async move {
            if let Err(e) = tx_clone
                .send(WSConnectorEvents::Error {
                    error: anyhow::Error::msg(
                        e.as_string()
                            .unwrap_or("Unknown error from WebSocket".into()),
                    ),
                })
                .await
            {
                error!("Failed to send event from WebSocket coroutine: {:?}", e);
            }
        });
    });
    websocket.set_onerror(Some(on_error_cb.as_ref().unchecked_ref()));
    on_error_cb.forget();

    let on_open_cb = Closure::<dyn FnMut()>::new(move || {
        info!("WebSocket connection is open");
    });
    websocket.set_onopen(Some(on_open_cb.as_ref().unchecked_ref()));
    on_open_cb.forget();

    let on_close_cb = Closure::<dyn FnMut()>::new(move || {
        info!("WebSocket connection has been closed");
    });
    websocket.set_onclose(Some(on_close_cb.as_ref().unchecked_ref()));
    on_close_cb.forget();

    loop {
        match rx.next().await {
            Some(WSConnectorMessage::Data { payload }) => {
                if let Err(e) = websocket.send_with_u8_array(&payload) {
                    return Err(Error::msg(
                        e.as_string()
                            .unwrap_or("Unexpected error when sending WS message".into()),
                    ));
                }
            }
            Some(WSConnectorMessage::Close) => {
                if let Err(e) = websocket.close() {
                    warn!(
                        "Failed to close WebSocket: {}",
                        e.as_string().unwrap_or("Unknown error".into())
                    );
                }
                return Ok(());
            }
            Some(WSConnectorMessage::Connect { url: _ }) => {
                panic!("Unexpected command Connect when already in connected state");
            }
            None => {
                info!("WebSocket stream closed unexpectedly");
                return Err(anyhow!("WebSocket stream closed unexpectedly"));
            }
        }
    }
}
