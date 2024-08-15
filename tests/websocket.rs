use mailiner_core::ws_connector::{WSConnector, WSConnectorCommand, WSConnectorEvent};
use wasm_bindgen_test::wasm_bindgen_test;
use std::sync::Once;

static INIT_LOGGER: Once = Once::new();

fn init_logger() {
    INIT_LOGGER.call_once(|| {
        dioxus_logger::init(dioxus_logger::tracing::Level::TRACE).expect("Failed to initialize logger");
    });
}


#[wasm_bindgen_test]
async fn test_websocket_connects() {
    init_logger();

    let mut ws = WSConnector::new();
    ws.send(WSConnectorCommand::Connect {
        url: "ws://localhost:14000/".to_string(),
    })
    .await
    .expect("Failed to send Conect command");

    match ws.receive().await {
        Some(WSConnectorEvent::Connected) => {}
        _ => panic!("Expected Connected event"),
    }
}

#[wasm_bindgen_test]
async fn test_websocket_handles_connect_failure() {
    init_logger();

    let mut ws = WSConnector::new();
    ws.send(WSConnectorCommand::Connect {
        url: "ws://localhost:14001/".to_string(),
    })
    .await
    .expect("Failed to send Connect command");

    match ws.receive().await {
        Some(WSConnectorEvent::Error { error }) => {
            assert_eq!(
                error.to_string(),
                "An error has occurred: the WebSocket connection is closed"
            );
        }
        _ => panic!("Expected Error event"),
    }
}

#[wasm_bindgen_test]
async fn test_websocket_closes_on_drop() {
    init_logger();

    {
        let mut ws = WSConnector::new();
        ws.send(WSConnectorCommand::Connect {
            url: "ws://localhost:14000/".to_string(),
        })
        .await
        .expect("Failed to send Connect command");

        match ws.receive().await {
            Some(WSConnectorEvent::Connected) => {}
            _ => panic!("Expected Connected event"),
        }
    }
}
