use wasm_bindgen_test::wasm_bindgen_test;
//use mailiner_rs::corelib::ws_connector;
use mailiner_core::ws_connector::{WSConnector, WSConnectorMessage, WSConnectorEvent};

#[wasm_bindgen_test]
async fn test_websocket() {
    let mut ws = WSConnector::new();
    ws.send(WSConnectorMessage::Connect{url: "ws://localhost:14000/".to_string()});

    loop {
        match ws.receive().await {
            Some(WSConnectorEvent::Connected) => {
                println!("Connected");
                ws.send(WSConnectorMessage::Data{payload: b"Hello".to_vec()});
            },
            Some(WSConnectorEvent::Error{ error} ) => {
                println!("Error: {:?}", error);
            },
            Some(WSConnectorEvent::Data{ payload }) => {
                println!("Message: {:?}", payload);
            },
            None => {
                println!("Socket closed");
                return;
            }
        }
    }
}
