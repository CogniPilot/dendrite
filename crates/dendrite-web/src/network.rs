//! Network client for backend communication

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

use crate::app::{DeviceData, DeviceRegistry, DeviceStatus, FrameData, VisualData};

pub struct NetworkPlugin;

/// Resource storing the daemon connection configuration
#[derive(Resource, Clone)]
pub struct DaemonConfig {
    /// HTTP(S) base URL for REST API (e.g., "http://192.168.1.100:8080")
    pub http_url: String,
    /// WebSocket URL (e.g., "ws://192.168.1.100:8080/ws")
    pub ws_url: String,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            http_url: String::new(),
            ws_url: String::new(),
        }
    }
}

impl DaemonConfig {
    /// Create config from URL query parameters or same-origin fallback
    #[cfg(target_arch = "wasm32")]
    pub fn from_browser() -> Self {
        let window = web_sys::window().expect("no window");
        let location = window.location();

        // Check for ?daemon= query parameter
        if let Ok(search) = location.search() {
            if let Some(daemon_param) = Self::parse_query_param(&search, "daemon") {
                tracing::info!("Using daemon from URL parameter: {}", daemon_param);
                return Self::from_daemon_address(&daemon_param);
            }
        }

        // Fall back to same-origin
        let host = location.host().unwrap_or_else(|_| "localhost:8080".to_string());
        let is_https = location.protocol().unwrap_or_default() == "https:";

        Self {
            http_url: format!("{}://{}", if is_https { "https" } else { "http" }, host),
            ws_url: format!("{}://{}/ws", if is_https { "wss" } else { "ws" }, host),
        }
    }

    /// Create config from a daemon address (host:port)
    pub fn from_daemon_address(addr: &str) -> Self {
        // If no protocol specified, default to http/ws
        let (http_url, ws_url) = if addr.starts_with("https://") || addr.starts_with("http://") {
            let http = addr.to_string();
            let ws = addr.replace("https://", "wss://").replace("http://", "ws://");
            (http, format!("{}/ws", ws))
        } else {
            // Assume plain address like "192.168.1.100:8080"
            (format!("http://{}", addr), format!("ws://{}/ws", addr))
        };

        Self {
            http_url,
            ws_url,
        }
    }

    /// Parse a query parameter from a search string
    fn parse_query_param(search: &str, param: &str) -> Option<String> {
        let search = search.trim_start_matches('?');
        for pair in search.split('&') {
            let mut parts = pair.splitn(2, '=');
            if let (Some(key), Some(value)) = (parts.next(), parts.next()) {
                if key == param {
                    // URL decode the value
                    return Some(value.replace("%3A", ":").replace("%2F", "/"));
                }
            }
        }
        None
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_browser() -> Self {
        Self::default()
    }
}

/// Network interface info from the server
#[derive(Debug, Clone, Deserialize, Default)]
pub struct NetworkInterfaceInfo {
    pub name: String,
    pub ip: String,
    pub subnet: String,
    pub prefix_len: u8,
}

/// Resource storing available network interfaces
#[derive(Resource, Default)]
pub struct NetworkInterfaces {
    pub interfaces: Vec<NetworkInterfaceInfo>,
    pub selected_index: Option<usize>,
    pub loading: bool,
    pub scan_in_progress: bool,
}

/// Resource storing heartbeat (connection checking) state
#[derive(Resource)]
pub struct HeartbeatState {
    /// Whether connection checking is enabled
    pub enabled: bool,
    /// Whether we're waiting for initial state from server
    pub loading: bool,
}

impl Default for HeartbeatState {
    fn default() -> Self {
        Self {
            enabled: false, // Default to off (no network traffic)
            loading: true,  // Loading initial state
        }
    }
}

/// Request to update subnet (used by trigger_scan_on_interface)
#[derive(Serialize)]
#[allow(dead_code)]
struct UpdateSubnetRequest {
    subnet: String,
    prefix_len: u8,
}

impl Plugin for NetworkPlugin {
    fn build(&self, app: &mut App) {
        // Initialize daemon config from browser URL
        let daemon_config = DaemonConfig::from_browser();

        app.insert_resource(daemon_config)
            .init_resource::<WebSocketConnection>()
            .init_resource::<PendingMessages>()
            .init_resource::<NetworkInterfaces>()
            .init_resource::<PendingInterfaceData>()
            .init_resource::<HeartbeatState>()
            .init_resource::<PendingHeartbeatData>()
            .add_message::<ReconnectEvent>()
            .add_systems(Startup, (connect_websocket, fetch_initial_devices, fetch_network_interfaces, fetch_heartbeat_state))
            .add_systems(Update, (process_messages, process_interface_data, process_heartbeat_data, handle_reconnect));
    }
}

/// Handle reconnection events
fn handle_reconnect(
    mut events: MessageReader<ReconnectEvent>,
    mut daemon_config: ResMut<DaemonConfig>,
    mut connection: ResMut<WebSocketConnection>,
    pending: Res<PendingMessages>,
    pending_interfaces: Res<PendingInterfaceData>,
    mut registry: ResMut<crate::app::DeviceRegistry>,
) {
    for event in events.read() {
        tracing::info!("Reconnecting to daemon: {}", event.daemon_address);

        // Update daemon config
        *daemon_config = DaemonConfig::from_daemon_address(&event.daemon_address);

        // Clear existing state
        registry.devices.clear();
        registry.connected = false;
        connection.connected = false;

        // Clear pending messages
        if let Ok(mut queue) = pending.0.lock() {
            queue.clear();
        }

        // Clear pending interfaces
        if let Ok(mut data) = pending_interfaces.0.lock() {
            *data = None;
        }

        // Reconnect WebSocket and fetch data
        #[cfg(target_arch = "wasm32")]
        {
            reconnect_websocket(&daemon_config, &pending, &mut connection);
            refetch_devices(&daemon_config, &pending);
            refetch_interfaces(&daemon_config, &pending_interfaces);
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn reconnect_websocket(
    daemon_config: &DaemonConfig,
    pending: &PendingMessages,
    connection: &mut WebSocketConnection,
) {
    use wasm_bindgen::prelude::*;
    use web_sys::{MessageEvent, WebSocket};

    let ws_url = daemon_config.ws_url.clone();
    tracing::info!("Reconnecting WebSocket to: {}", ws_url);

    match WebSocket::new(&ws_url) {
        Ok(ws) => {
            ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

            let onopen = Closure::wrap(Box::new(move |_| {
                tracing::info!("WebSocket reconnected");
            }) as Box<dyn FnMut(JsValue)>);
            ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
            onopen.forget();

            let pending_clone = pending.0.clone();
            let onmessage = Closure::wrap(Box::new(move |e: MessageEvent| {
                if let Ok(text) = e.data().dyn_into::<js_sys::JsString>() {
                    let text: String = text.into();
                    if let Ok(msg) = serde_json::from_str::<WsMessage>(&text) {
                        if let Ok(mut queue) = pending_clone.lock() {
                            queue.push(msg);
                        }
                    }
                }
            }) as Box<dyn FnMut(MessageEvent)>);
            ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
            onmessage.forget();

            connection.connected = true;
        }
        Err(e) => {
            tracing::error!("Failed to reconnect WebSocket: {:?}", e);
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn refetch_devices(daemon_config: &DaemonConfig, pending: &PendingMessages) {
    use wasm_bindgen_futures::spawn_local;

    let pending_clone = pending.0.clone();
    let base_url = daemon_config.http_url.clone();

    spawn_local(async move {
        let url = format!("{}/api/devices", base_url);
        tracing::info!("Refetching devices from: {}", url);

        match gloo_net::http::Request::get(&url).send().await {
            Ok(response) => {
                if let Ok(text) = response.text().await {
                    if let Ok(devices) = serde_json::from_str::<Vec<DeviceJson>>(&text) {
                        if let Ok(mut queue) = pending_clone.lock() {
                            for device in devices {
                                queue.push(WsMessage::DeviceDiscovered(device));
                            }
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to refetch devices: {:?}", e);
            }
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn refetch_interfaces(daemon_config: &DaemonConfig, pending: &PendingInterfaceData) {
    use wasm_bindgen_futures::spawn_local;

    let pending_clone = pending.0.clone();
    let base_url = daemon_config.http_url.clone();

    spawn_local(async move {
        let url = format!("{}/api/interfaces", base_url);
        tracing::info!("Refetching interfaces from: {}", url);

        match gloo_net::http::Request::get(&url).send().await {
            Ok(response) => {
                if let Ok(text) = response.text().await {
                    if let Ok(interfaces) = serde_json::from_str::<Vec<NetworkInterfaceInfo>>(&text) {
                        if let Ok(mut data) = pending_clone.lock() {
                            *data = Some(interfaces);
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to refetch interfaces: {:?}", e);
            }
        }
    });
}

/// Pending interface data from async fetch
#[derive(Resource, Default)]
pub struct PendingInterfaceData(pub Arc<Mutex<Option<Vec<NetworkInterfaceInfo>>>>);

/// Pending heartbeat data from async fetch
#[derive(Resource, Default)]
pub struct PendingHeartbeatData(pub Arc<Mutex<Option<bool>>>);

/// Shared message queue between WebSocket callback and Bevy
#[derive(Resource, Default, Clone)]
pub struct PendingMessages(pub Arc<Mutex<Vec<WsMessage>>>);

/// WebSocket connection state
#[derive(Resource, Default)]
pub struct WebSocketConnection {
    pub connected: bool,
}

/// Message to trigger reconnection with new daemon config
#[derive(Message)]
pub struct ReconnectEvent {
    pub daemon_address: String,
}

/// Messages from the server
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum WsMessage {
    #[serde(rename = "device_discovered")]
    DeviceDiscovered(DeviceJson),
    #[serde(rename = "device_offline")]
    DeviceOffline { id: String },
    #[serde(rename = "device_updated")]
    DeviceUpdated(DeviceJson),
    #[serde(rename = "device_removed")]
    DeviceRemoved { id: String },
    #[serde(rename = "scan_started")]
    ScanStarted,
    #[serde(rename = "scan_completed")]
    ScanCompleted {
        #[allow(dead_code)]
        found: usize,
        #[allow(dead_code)]
        total: usize,
    },
    #[serde(rename = "pong")]
    Pong,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceJson {
    pub id: IdJson,
    pub name: String,
    pub status: String,
    pub discovery: DiscoveryJson,
    pub info: InfoJson,
    pub firmware: FirmwareJson,
    pub model_path: Option<String>,
    pub pose: Option<[f64; 6]>,
    /// Composite visuals with individual poses
    #[serde(default)]
    pub visuals: Vec<VisualJson>,
    /// Reference frames for this device
    #[serde(default)]
    pub frames: Vec<FrameJson>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IdJson(pub String);

#[derive(Debug, Clone, Deserialize)]
pub struct DiscoveryJson {
    pub ip: String,
    #[allow(dead_code)]
    pub port: u16,
    pub switch_port: Option<u8>,
    pub last_seen: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InfoJson {
    pub board: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FirmwareJson {
    pub version: Option<String>,
}

/// Visual element JSON from the backend
#[derive(Debug, Clone, Deserialize)]
pub struct VisualJson {
    pub name: String,
    #[serde(default)]
    pub pose: Option<[f64; 6]>,
    #[serde(default)]
    pub model_path: Option<String>,
    #[serde(default)]
    pub model_sha: Option<String>,
}

/// Reference frame JSON from the backend
#[derive(Debug, Clone, Deserialize)]
pub struct FrameJson {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub pose: Option<[f64; 6]>,
}

impl From<DeviceJson> for DeviceData {
    fn from(json: DeviceJson) -> Self {
        DeviceData {
            id: json.id.0,
            name: json.name,
            board: json.info.board,
            ip: json.discovery.ip,
            port: json.discovery.switch_port,
            status: match json.status.as_str() {
                "online" => DeviceStatus::Online,
                "offline" => DeviceStatus::Offline,
                _ => DeviceStatus::Unknown,
            },
            version: json.firmware.version,
            position: json.pose.map(|p| [p[0], p[1], p[2]]),
            model_path: json.model_path,
            visuals: json.visuals.into_iter().map(|v| VisualData {
                name: v.name,
                pose: v.pose,
                model_path: v.model_path,
                model_sha: v.model_sha,
            }).collect(),
            frames: json.frames.into_iter().map(|f| FrameData {
                name: f.name,
                description: f.description,
                pose: f.pose,
            }).collect(),
            last_seen: json.discovery.last_seen,
        }
    }
}

fn connect_websocket(
    mut connection: ResMut<WebSocketConnection>,
    pending: Res<PendingMessages>,
    daemon_config: Res<DaemonConfig>,
) {
    // In WASM, we use web_sys WebSocket
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::prelude::*;
        use web_sys::{MessageEvent, WebSocket};

        let ws_url = daemon_config.ws_url.clone();
        tracing::info!("Connecting to WebSocket: {}", ws_url);

        match WebSocket::new(&ws_url) {
            Ok(ws) => {
                ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

                let onopen = Closure::wrap(Box::new(move |_| {
                    tracing::info!("WebSocket connected");
                }) as Box<dyn FnMut(JsValue)>);
                ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
                onopen.forget();

                // Clone pending for the callback
                let pending_clone = pending.0.clone();
                let onmessage = Closure::wrap(Box::new(move |e: MessageEvent| {
                    if let Ok(text) = e.data().dyn_into::<js_sys::JsString>() {
                        let text: String = text.into();
                        tracing::debug!("WS message: {}", text);
                        if let Ok(msg) = serde_json::from_str::<WsMessage>(&text) {
                            if let Ok(mut queue) = pending_clone.lock() {
                                queue.push(msg);
                            }
                        }
                    }
                }) as Box<dyn FnMut(MessageEvent)>);
                ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
                onmessage.forget();

                connection.connected = true;
            }
            Err(e) => {
                tracing::error!("Failed to create WebSocket: {:?}", e);
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        tracing::info!("WebSocket not available in native mode");
    }
}

/// Fetch devices from REST API on startup
fn fetch_initial_devices(pending: Res<PendingMessages>, daemon_config: Res<DaemonConfig>) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen_futures::spawn_local;

        let pending_clone = pending.0.clone();
        let base_url = daemon_config.http_url.clone();

        spawn_local(async move {
            let url = format!("{}/api/devices", base_url);

            tracing::info!("Fetching devices from: {}", url);

            match gloo_net::http::Request::get(&url).send().await {
                Ok(response) => {
                    if let Ok(text) = response.text().await {
                        tracing::debug!("Devices response: {}", text);
                        if let Ok(devices) = serde_json::from_str::<Vec<DeviceJson>>(&text) {
                            if let Ok(mut queue) = pending_clone.lock() {
                                for device in devices {
                                    queue.push(WsMessage::DeviceDiscovered(device));
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to fetch devices: {:?}", e);
                }
            }
        });
    }
}

fn process_messages(
    connection: Res<WebSocketConnection>,
    pending: Res<PendingMessages>,
    mut registry: ResMut<DeviceRegistry>,
) {
    // Process queued messages from the shared queue
    let messages = {
        if let Ok(mut queue) = pending.0.lock() {
            std::mem::take(&mut *queue)
        } else {
            Vec::new()
        }
    };

    for msg in messages {
        match msg {
            WsMessage::DeviceDiscovered(device) => {
                let data: DeviceData = device.into();
                tracing::info!("Device discovered: {} - {}", data.id, data.name);
                if !registry.devices.iter().any(|d| d.id == data.id) {
                    registry.devices.push(data);
                }
            }
            WsMessage::DeviceUpdated(device) => {
                let data: DeviceData = device.into();
                if let Some(existing) = registry.devices.iter_mut().find(|d| d.id == data.id) {
                    *existing = data;
                }
            }
            WsMessage::DeviceOffline { id } => {
                if let Some(device) = registry.devices.iter_mut().find(|d| d.id == id) {
                    device.status = DeviceStatus::Offline;
                }
            }
            WsMessage::DeviceRemoved { id } => {
                registry.devices.retain(|d| d.id != id);
            }
            _ => {}
        }
    }

    registry.connected = connection.connected;
}

/// Fetch network interfaces from backend
fn fetch_network_interfaces(pending: Res<PendingInterfaceData>, daemon_config: Res<DaemonConfig>) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen_futures::spawn_local;

        let pending_clone = pending.0.clone();
        let base_url = daemon_config.http_url.clone();

        spawn_local(async move {
            let url = format!("{}/api/interfaces", base_url);

            tracing::info!("Fetching network interfaces from: {}", url);

            match gloo_net::http::Request::get(&url).send().await {
                Ok(response) => {
                    if let Ok(text) = response.text().await {
                        tracing::debug!("Interfaces response: {}", text);
                        if let Ok(interfaces) = serde_json::from_str::<Vec<NetworkInterfaceInfo>>(&text) {
                            if let Ok(mut data) = pending_clone.lock() {
                                *data = Some(interfaces);
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to fetch interfaces: {:?}", e);
                }
            }
        });
    }
}

/// Process pending interface data
fn process_interface_data(
    pending: Res<PendingInterfaceData>,
    mut interfaces: ResMut<NetworkInterfaces>,
) {
    if let Ok(mut data) = pending.0.lock() {
        if let Some(fetched) = data.take() {
            interfaces.interfaces = fetched;
            interfaces.loading = false;
        }
    }
}

/// Fetch initial heartbeat state from backend
fn fetch_heartbeat_state(pending: Res<PendingHeartbeatData>, daemon_config: Res<DaemonConfig>) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen_futures::spawn_local;

        let pending_clone = pending.0.clone();
        let base_url = daemon_config.http_url.clone();

        spawn_local(async move {
            let url = format!("{}/api/heartbeat", base_url);

            tracing::info!("Fetching heartbeat state from: {}", url);

            match gloo_net::http::Request::get(&url).send().await {
                Ok(response) => {
                    if let Ok(text) = response.text().await {
                        tracing::debug!("Heartbeat response: {}", text);
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                            if let Some(enabled) = json.get("heartbeat_enabled").and_then(|v| v.as_bool()) {
                                if let Ok(mut data) = pending_clone.lock() {
                                    *data = Some(enabled);
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to fetch heartbeat state: {:?}", e);
                }
            }
        });
    }
}

/// Process pending heartbeat data
fn process_heartbeat_data(
    pending: Res<PendingHeartbeatData>,
    mut heartbeat_state: ResMut<HeartbeatState>,
) {
    if let Ok(mut data) = pending.0.lock() {
        if let Some(enabled) = data.take() {
            heartbeat_state.enabled = enabled;
            heartbeat_state.loading = false;
        }
    }
}

/// Toggle heartbeat checking (called from UI)
pub fn toggle_heartbeat(enabled: bool, base_url: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen_futures::spawn_local;

        let base_url = base_url.to_string();

        spawn_local(async move {
            let url = format!("{}/api/heartbeat", base_url);
            let body = serde_json::json!({ "enabled": enabled });

            tracing::info!("Setting heartbeat to: {}", enabled);

            match gloo_net::http::Request::post(&url)
                .header("Content-Type", "application/json")
                .body(body.to_string())
                .unwrap()
                .send()
                .await
            {
                Ok(_) => {
                    tracing::info!("Heartbeat set to: {}", enabled);
                }
                Err(e) => {
                    tracing::error!("Failed to set heartbeat: {:?}", e);
                }
            }
        });
    }
}

/// Trigger a scan on the selected interface (called from UI)
pub fn trigger_scan_on_interface(subnet: &str, prefix_len: u8, base_url: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen_futures::spawn_local;

        let subnet = subnet.to_string();
        let base_url = base_url.to_string();

        spawn_local(async move {
            // First update the subnet
            let update_url = format!("{}/api/subnet", base_url);
            let body = serde_json::json!({
                "subnet": subnet,
                "prefix_len": prefix_len
            });

            tracing::info!("Updating subnet to: {}/{}", subnet, prefix_len);

            match gloo_net::http::Request::post(&update_url)
                .header("Content-Type", "application/json")
                .body(body.to_string())
                .unwrap()
                .send()
                .await
            {
                Ok(_) => {
                    tracing::info!("Subnet updated, triggering scan");

                    // Now trigger a scan
                    let scan_url = format!("{}/api/scan", base_url);
                    match gloo_net::http::Request::post(&scan_url).send().await {
                        Ok(_) => {
                            tracing::info!("Scan triggered successfully");
                        }
                        Err(e) => {
                            tracing::error!("Failed to trigger scan: {:?}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to update subnet: {:?}", e);
                }
            }
        });
    }
}

/// Remove a device from the backend (called from UI)
pub fn remove_device(device_id: &str, base_url: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen_futures::spawn_local;

        let device_id = device_id.to_string();
        let base_url = base_url.to_string();

        spawn_local(async move {
            let url = format!("{}/api/devices/{}", base_url, device_id);

            tracing::info!("Removing device: {}", device_id);

            match gloo_net::http::Request::delete(&url).send().await {
                Ok(response) => {
                    if response.ok() {
                        tracing::info!("Device removed successfully: {}", device_id);
                    } else {
                        tracing::error!("Failed to remove device: {}", response.status());
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to remove device: {:?}", e);
                }
            }
        });
    }
}
