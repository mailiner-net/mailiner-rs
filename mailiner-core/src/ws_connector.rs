use anyhow::{anyhow, Error};
use futures::channel::mpsc;
use futures_util::{SinkExt, StreamExt};
use js_sys::wasm_bindgen::{prelude::Closure, JsCast};
use tracing::{error, info, trace, warn};
use wasm_bindgen_futures::spawn_local;
use web_sys::{BinaryType, ErrorEvent, MessageEvent, WebSocket};


#[derive(Debug, Clone, PartialEq)]
pub enum WSConnectorCommand {
    Connect { url: String },
    Close,
    Data { payload: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum WSConnectorEvent {
    Connected,
    Error { error: String },
    Data { payload: Vec<u8> },
}

#[derive(Debug)]
pub struct WSConnector {
    cmd_tx: mpsc::UnboundedSender<WSConnectorCommand>,
    event_rx: mpsc::UnboundedReceiver<WSConnectorEvent>,
}

impl WSConnector {
    pub fn new() -> Self {
        let (mut event_tx, event_rx) = mpsc::unbounded::<WSConnectorEvent>();
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded::<WSConnectorCommand>();

        spawn_local(async move {
            let websocket = match connect_websocket(&mut cmd_rx).await {
                Ok(websocket) => websocket,
                Err(err) => {
                    let _ = event_tx.send(WSConnectorEvent::Error {
                       error: err.to_string(),
                    }).await;
                    return;
                }
            };

            let _ = run_websocket_loop(websocket, &mut cmd_rx, &mut event_tx).await;
        });

        Self { cmd_tx, event_rx }
    }

    pub async fn receive(&mut self) -> Option<WSConnectorEvent> {
        self.event_rx.next().await
    }

    pub async fn send(&mut self, message: WSConnectorCommand) -> Result<(), Error> {
        self.cmd_tx
            .send(message)
            .await
            .map_err(|e| anyhow!("Failed to send message: {:?}", e))
    }
}

async fn connect_websocket(
    rx: &mut mpsc::UnboundedReceiver<WSConnectorCommand>,
) -> Result<web_sys::WebSocket, Error> {
    trace!("Waiting for Connect command");
    match rx.next().await {
        Some(WSConnectorCommand::Connect { url }) => {
            trace!("Received WSConnectorCommand::Connect{{url: {}}}", url);
            return WebSocket::new(&url).map_err(|e| {
                anyhow!(e.as_string()
                        .unwrap_or("Unexpect error when creating WebSocket".into()),
                )
            });
        }
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
    rx: &mut mpsc::UnboundedReceiver<WSConnectorCommand>,
    tx: &mut mpsc::UnboundedSender<WSConnectorEvent>,
) -> Result<(), Error> {
    websocket.set_binary_type(BinaryType::Arraybuffer);
    let tx_clone = tx.clone();
    let on_message_cb = Closure::<dyn FnMut(_)>::new(move |e: MessageEvent| {
        trace!("Received MessageEvent from WebSocket");
        if let Ok(abuf) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
            let array = js_sys::Uint8Array::new(&abuf);

            let mut tx_clone = tx_clone.clone();
            spawn_local(async move {
                if let Err(e) = tx_clone
                    .send(WSConnectorEvent::Data {
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
    let ws_clone = websocket.clone();
    let on_error_cb = Closure::<dyn FnMut(_)>::new(move |_: ErrorEvent| {
        let mut tx_clone = tx_clone.clone();
        let ws_clone = ws_clone.clone();
        trace!("Received ErrorEvent from WebSocket");
        spawn_local(async move {
            if let Err(e) = tx_clone
                .send(WSConnectorEvent::Error {
                    error: match ws_clone.ready_state() {
                        web_sys::WebSocket::CLOSED => "An error has occurred: the WebSocket connection is closed".into(),
                        web_sys::WebSocket::CLOSING => "An error has occurred while closing WebSocket connection".into(),
                        web_sys::WebSocket::CONNECTING => "An error has occurred while creating WebSocket connection".into(),
                        web_sys::WebSocket::OPEN => "An error has occurred on the WebSocket connection".into(),
                        _=> "An error has occurred on the WebSocket connection".into()
                    }
                })
                .await
            {
                error!("Failed to send event from WebSocket coroutine: {:?}", e);
            }
        });
    });
    websocket.set_onerror(Some(on_error_cb.as_ref().unchecked_ref()));
    on_error_cb.forget();

    let tx_clone = tx.clone();
    let on_open_cb = Closure::<dyn FnMut()>::new(move || {
        let mut tx_clone = tx_clone.clone();
        info!("WebSocket connection is open");
        spawn_local(async move {
            if let Err(e) = tx_clone.send(WSConnectorEvent::Connected).await {
                error!("Failed to send event from WebSocket coroutine: {:?}", e);
            }
        });
    });
    websocket.set_onopen(Some(on_open_cb.as_ref().unchecked_ref()));
    on_open_cb.forget();

    let on_close_cb = Closure::<dyn FnMut()>::new(move || {
        info!("WebSocket connection has been closed");
    });
    websocket.set_onclose(Some(on_close_cb.as_ref().unchecked_ref()));
    on_close_cb.forget();

    loop {
        trace!("Waiting on next command");
        match rx.next().await {
            Some(WSConnectorCommand::Data { payload }) => {
                trace!(
                    "Received WSConnectorCommand::Data{{payload: {:?}}}",
                    payload
                );
                if let Err(e) = websocket.send_with_u8_array(&payload) {
                    return Err(anyhow!(
                        e.as_string()
                            .unwrap_or("Unexpected error when sending WS message".into()),
                    ));
                }
            }
            Some(WSConnectorCommand::Close) => {
                trace!("Received WSConnectorCommand::Close");
                if let Err(e) = websocket.close() {
                    warn!(
                        "Failed to close WebSocket: {}",
                        e.as_string().unwrap_or("Unknown error".into())
                    );
                }
                return Ok(());
            }
            Some(WSConnectorCommand::Connect { url: _ }) => {
                panic!("Unexpected command Connect when already in connected state");
            }
            None => {
                trace!("Internal stream has been closed");
                // Ignore failure, we are shutting down
                let _ = websocket.close();
                return Ok(())
            }
        }
    }
}

impl Drop for WSConnector {
    fn drop(&mut self) {
        trace!("WebSocket has been dropped");
        let _ = self.cmd_tx.send(WSConnectorCommand::Close);
        self.cmd_tx.close_channel();
        self.event_rx.close();
    }
}
