//! WebSocket handler for real-time updates

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use dendrite_discovery::DiscoveryEvent;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::ota::{OtaEvent, UpdateState};
use crate::state::AppState;

/// WebSocket message types
#[derive(Serialize)]
#[serde(tag = "type", content = "data")]
enum WsMessage {
    #[serde(rename = "device_discovered")]
    DeviceDiscovered(dendrite_core::Device),
    #[serde(rename = "device_offline")]
    DeviceOffline { id: String },
    #[serde(rename = "device_updated")]
    DeviceUpdated(dendrite_core::Device),
    #[serde(rename = "device_removed")]
    DeviceRemoved { id: String },
    #[serde(rename = "scan_started")]
    ScanStarted,
    #[serde(rename = "scan_completed")]
    ScanCompleted { found: usize, total: usize },
    #[serde(rename = "ota_progress")]
    OtaProgress { device_id: String, state: UpdateState },
    #[serde(rename = "pong")]
    Pong,
}

/// WebSocket upgrade handler
pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut discovery_events = state.subscribe();
    let mut ota_events = state.ota_service.subscribe();

    info!("WebSocket client connected");

    // Send current device list on connect
    let devices = state.devices().await;
    for device in devices {
        let msg = WsMessage::DeviceDiscovered(device);
        if let Ok(json) = serde_json::to_string(&msg) {
            if sender.send(Message::Text(json.into())).await.is_err() {
                return;
            }
        }
    }

    // Handle incoming messages and forward events
    loop {
        tokio::select! {
            // Forward discovery events to client
            event = discovery_events.recv() => {
                match event {
                    Ok(event) => {
                        let msg = match event {
                            DiscoveryEvent::DeviceDiscovered(device) => {
                                WsMessage::DeviceDiscovered(device)
                            }
                            DiscoveryEvent::DeviceOffline(id) => {
                                WsMessage::DeviceOffline { id: id.0 }
                            }
                            DiscoveryEvent::DeviceUpdated(device) => {
                                WsMessage::DeviceUpdated(device)
                            }
                            DiscoveryEvent::DeviceRemoved(id) => {
                                WsMessage::DeviceRemoved { id: id.0 }
                            }
                            DiscoveryEvent::ScanStarted => WsMessage::ScanStarted,
                            DiscoveryEvent::ScanCompleted { found, total } => {
                                WsMessage::ScanCompleted { found, total }
                            }
                        };

                        if let Ok(json) = serde_json::to_string(&msg) {
                            if sender.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        debug!(error = %e, "Discovery event channel error");
                        break;
                    }
                }
            }

            // Forward OTA events to client
            event = ota_events.recv() => {
                match event {
                    Ok(OtaEvent { device_id, state }) => {
                        let msg = WsMessage::OtaProgress { device_id, state };
                        if let Ok(json) = serde_json::to_string(&msg) {
                            if sender.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        debug!(skipped = n, "OTA event channel lagged");
                        // Continue - lagging is not fatal
                    }
                    Err(e) => {
                        debug!(error = %e, "OTA event channel error");
                        // OTA channel closed is not fatal - continue with discovery events
                    }
                }
            }

            // Handle incoming messages from client
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Ping(data))) => {
                        if sender.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        // Handle ping/pong for keepalive
                        if text.as_str() == "ping" {
                            let pong = serde_json::to_string(&WsMessage::Pong).unwrap();
                            if sender.send(Message::Text(pong.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        break;
                    }
                    Some(Err(e)) => {
                        warn!(error = %e, "WebSocket error");
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    info!("WebSocket client disconnected");
}
