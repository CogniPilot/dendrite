//! Network client for backend communication

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

use crate::app::{AxisAlignData, DeviceData, DeviceRegistry, DeviceStatus, FirmwareCheckState, FirmwareStatusData, FovData, FrameData, GeometryData, PortData, SensorData, VisualData};

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

/// Pending firmware check data from async fetch
#[derive(Resource, Default)]
pub struct PendingFirmwareData(pub Arc<Mutex<Vec<FirmwareCheckResponse>>>);

/// Response from firmware check API
#[derive(Debug, Clone, Deserialize)]
pub struct FirmwareCheckResponse {
    pub device_id: String,
    pub current_version: Option<String>,
    /// MCUboot image hash from the device (what MCUmgr reports)
    pub current_mcuboot_hash: Option<String>,
    pub latest_version: Option<String>,
    /// MCUboot image hash for the latest release (for verification after OTA)
    pub latest_mcuboot_hash: Option<String>,
    pub status: FirmwareStatusJson,
    pub changelog: Option<String>,
}

/// Firmware status JSON from backend (matches serde tag format)
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FirmwareStatusJson {
    UpToDate,
    UpdateAvailable {
        latest_version: String,
        changelog: Option<String>,
    },
    Unknown,
    CheckDisabled,
}

impl From<FirmwareStatusJson> for FirmwareStatusData {
    fn from(json: FirmwareStatusJson) -> Self {
        match json {
            FirmwareStatusJson::UpToDate => FirmwareStatusData::UpToDate,
            FirmwareStatusJson::UpdateAvailable { latest_version, changelog } => {
                FirmwareStatusData::UpdateAvailable { latest_version, changelog }
            }
            FirmwareStatusJson::Unknown => FirmwareStatusData::Unknown,
            FirmwareStatusJson::CheckDisabled => FirmwareStatusData::CheckDisabled,
        }
    }
}

/// Timer for periodic device sync to ensure UI stays in sync even if WebSocket messages are missed
/// This is especially important for WebView environments where WebSocket reliability can vary
#[derive(Resource)]
pub struct PeriodicSyncTimer {
    pub timer: Timer,
}

impl Default for PeriodicSyncTimer {
    fn default() -> Self {
        Self {
            // Sync every 5 seconds
            timer: Timer::from_seconds(5.0, TimerMode::Repeating),
        }
    }
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
            .init_resource::<PendingFirmwareData>()
            .init_resource::<PendingHcdfExport>()
            .init_resource::<PeriodicSyncTimer>()
            .add_message::<ReconnectEvent>()
            .add_systems(Startup, (connect_websocket, fetch_initial_devices, fetch_network_interfaces, fetch_heartbeat_state))
            .add_systems(Update, (process_messages, process_interface_data, process_heartbeat_data, process_firmware_data, handle_reconnect, periodic_device_sync));
    }
}

/// Periodically refetch devices from API to ensure sync
/// This helps when WebSocket messages are missed (especially on WebView/mobile)
fn periodic_device_sync(
    time: Res<Time>,
    mut sync_timer: ResMut<PeriodicSyncTimer>,
    daemon_config: Res<DaemonConfig>,
    pending: Res<PendingMessages>,
) {
    sync_timer.timer.tick(time.delta());

    if sync_timer.timer.just_finished() {
        #[cfg(target_arch = "wasm32")]
        {
            refetch_devices(&daemon_config, &pending);
            tracing::debug!("Periodic device sync triggered");
        }
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
    #[serde(rename = "ota_progress")]
    OtaProgress {
        device_id: String,
        state: OtaUpdateState,
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
    /// Ports on this device
    #[serde(default)]
    pub ports: Vec<PortJson>,
    /// Sensors on this device
    #[serde(default)]
    pub sensors: Vec<SensorJson>,
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
    pub toggle: Option<String>,
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

/// Port JSON from the backend
#[derive(Debug, Clone, Deserialize)]
pub struct PortJson {
    pub name: String,
    pub port_type: String,
    #[serde(default)]
    pub pose: Option<[f64; 6]>,
    #[serde(default)]
    pub geometry: Vec<GeometryJson>,
    /// Reference to visual containing the mesh (e.g., "board")
    #[serde(default)]
    pub visual_name: Option<String>,
    /// GLTF mesh node name within the visual (e.g., "port_eth0")
    #[serde(default)]
    pub mesh_name: Option<String>,
}

/// Axis alignment JSON from the backend
#[derive(Debug, Clone, Deserialize)]
pub struct AxisAlignJson {
    pub x: String,
    pub y: String,
    pub z: String,
}

/// Geometry JSON from the backend (tagged enum)
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GeometryJson {
    #[serde(rename = "box")]
    Box { size: [f64; 3] },
    Cylinder { radius: f64, length: f64 },
    Sphere { radius: f64 },
    Cone { radius: f64, length: f64 },
    Frustum { near: f64, far: f64, hfov: f64, vfov: f64 },
    ConicalFrustum { near: f64, far: f64, fov: f64 },
    PyramidalFrustum { near: f64, far: f64, hfov: f64, vfov: f64 },
}

/// FOV JSON from the backend
#[derive(Debug, Clone, Deserialize)]
pub struct FovJson {
    pub name: String,
    #[serde(default)]
    pub color: Option<[f32; 3]>,
    #[serde(default)]
    pub pose: Option<[f64; 6]>,
    #[serde(default)]
    pub geometry: Option<GeometryJson>,
}

/// Sensor JSON from the backend
#[derive(Debug, Clone, Deserialize)]
pub struct SensorJson {
    pub name: String,
    pub category: String,
    pub sensor_type: String,
    #[serde(default)]
    pub driver: Option<String>,
    #[serde(default)]
    pub pose: Option<[f64; 6]>,
    #[serde(default)]
    pub axis_align: Option<AxisAlignJson>,
    #[serde(default)]
    pub geometry: Option<GeometryJson>,
    #[serde(default)]
    pub fovs: Vec<FovJson>,
}

/// Convert GeometryJson to GeometryData
fn convert_geometry(g: GeometryJson) -> GeometryData {
    match g {
        GeometryJson::Box { size } => GeometryData::Box { size },
        GeometryJson::Cylinder { radius, length } => GeometryData::Cylinder { radius, length },
        GeometryJson::Sphere { radius } => GeometryData::Sphere { radius },
        GeometryJson::Cone { radius, length } => GeometryData::Cone { radius, length },
        GeometryJson::Frustum { near, far, hfov, vfov } => GeometryData::Frustum { near, far, hfov, vfov },
        GeometryJson::ConicalFrustum { near, far, fov } => GeometryData::ConicalFrustum { near, far, fov },
        GeometryJson::PyramidalFrustum { near, far, hfov, vfov } => GeometryData::PyramidalFrustum { near, far, hfov, vfov },
    }
}

/// Convert FovJson to FovData
fn convert_fov(f: FovJson) -> FovData {
    FovData {
        name: f.name,
        color: f.color,
        pose: f.pose,
        geometry: f.geometry.map(convert_geometry),
    }
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
            orientation: json.pose.map(|p| [p[3], p[4], p[5]]),
            model_path: json.model_path,
            visuals: json.visuals.into_iter().map(|v| VisualData {
                name: v.name,
                toggle: v.toggle,
                pose: v.pose,
                model_path: v.model_path,
                model_sha: v.model_sha,
            }).collect(),
            frames: json.frames.into_iter().map(|f| FrameData {
                name: f.name,
                description: f.description,
                pose: f.pose,
            }).collect(),
            ports: json.ports.into_iter().map(|p| PortData {
                name: p.name,
                port_type: p.port_type,
                pose: p.pose,
                geometry: p.geometry.into_iter().map(convert_geometry).collect(),
                visual_name: p.visual_name,
                mesh_name: p.mesh_name,
            }).collect(),
            sensors: json.sensors.into_iter().map(|s| SensorData {
                name: s.name,
                category: s.category,
                sensor_type: s.sensor_type,
                driver: s.driver,
                pose: s.pose,
                axis_align: s.axis_align.map(|a| AxisAlignData {
                    x: a.x,
                    y: a.y,
                    z: a.z,
                }),
                geometry: s.geometry.map(convert_geometry),
                fovs: s.fovs.into_iter().map(convert_fov).collect(),
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
    mut ota_state: ResMut<crate::app::OtaState>,
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
                // Update existing device if it exists (e.g., after HCDF import with new position)
                // Otherwise add as new device
                if let Some(existing) = registry.devices.iter_mut().find(|d| d.id == data.id) {
                    *existing = data;
                } else {
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
            WsMessage::OtaProgress { device_id, state } => {
                tracing::info!("OTA progress for {}: {:?}", device_id, state);
                // Store the state, or remove if terminal
                if state.is_terminal() {
                    // Keep terminal states for a while so UI can show completion
                    ota_state.device_updates.insert(device_id, state);
                } else {
                    ota_state.device_updates.insert(device_id, state);
                }
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

/// Process pending firmware check data
fn process_firmware_data(
    pending: Res<PendingFirmwareData>,
    mut firmware_state: ResMut<FirmwareCheckState>,
) {
    if let Ok(mut data) = pending.0.lock() {
        for response in data.drain(..) {
            // Convert JSON status to our enum
            let status: FirmwareStatusData = response.status.into();
            firmware_state.device_status.insert(response.device_id.clone(), status);
            firmware_state.loading.remove(&response.device_id);
        }
    }
}

/// Trigger firmware check for all devices (called from UI)
pub fn check_all_firmware(base_url: &str, pending: &PendingFirmwareData) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen_futures::spawn_local;

        let base_url = base_url.to_string();
        let pending_clone = pending.0.clone();

        spawn_local(async move {
            let url = format!("{}/api/firmware/check", base_url);

            tracing::info!("Checking firmware for all devices");

            match gloo_net::http::Request::get(&url).send().await {
                Ok(response) => {
                    if let Ok(text) = response.text().await {
                        tracing::debug!("Firmware check response: {}", text);
                        if let Ok(results) = serde_json::from_str::<Vec<FirmwareCheckResponse>>(&text) {
                            if let Ok(mut data) = pending_clone.lock() {
                                data.extend(results);
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to check firmware: {:?}", e);
                }
            }
        });
    }
}

// ============================================================================
// OTA Update Functions
// ============================================================================

/// Pending OTA update events from WebSocket
#[derive(Resource, Default)]
pub struct PendingOtaEvents(pub Arc<Mutex<Vec<OtaProgressEvent>>>);

/// OTA progress event from WebSocket
#[derive(Debug, Clone, Deserialize)]
pub struct OtaProgressEvent {
    pub device_id: String,
    pub state: OtaUpdateState,
}

/// OTA update state (mirrors backend UpdateState)
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum OtaUpdateState {
    Downloading { progress: f32 },
    Uploading { progress: f32 },
    Confirming,
    Rebooting,
    Verifying,
    Complete,
    Failed { error: String },
    Cancelled,
}

impl OtaUpdateState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, OtaUpdateState::Complete | OtaUpdateState::Failed { .. } | OtaUpdateState::Cancelled)
    }

    pub fn progress_text(&self) -> String {
        match self {
            OtaUpdateState::Downloading { progress } => format!("Downloading... {:.0}%", progress * 100.0),
            OtaUpdateState::Uploading { progress } => format!("Uploading... {:.0}%", progress * 100.0),
            OtaUpdateState::Confirming => "Confirming image...".to_string(),
            OtaUpdateState::Rebooting => "Rebooting device...".to_string(),
            OtaUpdateState::Verifying => "Verifying update...".to_string(),
            OtaUpdateState::Complete => "Update complete!".to_string(),
            OtaUpdateState::Failed { error } => format!("Failed: {}", error),
            OtaUpdateState::Cancelled => "Cancelled".to_string(),
        }
    }

    pub fn progress_value(&self) -> Option<f32> {
        match self {
            OtaUpdateState::Downloading { progress } => Some(*progress),
            OtaUpdateState::Uploading { progress } => Some(*progress),
            _ => None,
        }
    }
}

/// Start an OTA firmware update for a device (called from UI)
pub fn start_ota_update(device_id: &str, base_url: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen_futures::spawn_local;

        let device_id = device_id.to_string();
        let base_url = base_url.to_string();

        spawn_local(async move {
            let url = format!("{}/api/ota/{}/start", base_url, device_id);

            tracing::info!("Starting OTA update for device: {}", device_id);

            match gloo_net::http::Request::post(&url).send().await {
                Ok(response) => {
                    if response.ok() {
                        tracing::info!("OTA update started for device: {}", device_id);
                    } else {
                        let status = response.status();
                        let text = response.text().await.unwrap_or_default();
                        tracing::error!("Failed to start OTA update: {} - {}", status, text);
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to start OTA update: {:?}", e);
                }
            }
        });
    }
}

/// Cancel an OTA firmware update for a device (called from UI)
pub fn cancel_ota_update(device_id: &str, base_url: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen_futures::spawn_local;

        let device_id = device_id.to_string();
        let base_url = base_url.to_string();

        spawn_local(async move {
            let url = format!("{}/api/ota/{}/cancel", base_url, device_id);

            tracing::info!("Cancelling OTA update for device: {}", device_id);

            match gloo_net::http::Request::post(&url).send().await {
                Ok(response) => {
                    if response.ok() {
                        tracing::info!("OTA update cancelled for device: {}", device_id);
                    } else {
                        let status = response.status();
                        let text = response.text().await.unwrap_or_default();
                        tracing::error!("Failed to cancel OTA update: {} - {}", status, text);
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to cancel OTA update: {:?}", e);
                }
            }
        });
    }
}

// ============================================================================
// Local Firmware Upload (for development images)
// ============================================================================

/// Upload local firmware file to a device (called from UI after file picker)
pub fn upload_local_firmware(device_id: &str, firmware_data: Vec<u8>, base_url: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen_futures::spawn_local;
        use base64::Engine;

        let device_id = device_id.to_string();
        let base_url = base_url.to_string();

        spawn_local(async move {
            let url = format!("{}/api/ota/{}/upload-local", base_url, device_id);

            tracing::info!("Uploading local firmware for device: {} ({} bytes)", device_id, firmware_data.len());

            // Encode firmware as base64
            let firmware_base64 = base64::engine::general_purpose::STANDARD.encode(&firmware_data);
            let body = serde_json::json!({
                "firmware_base64": firmware_base64
            });

            match gloo_net::http::Request::post(&url)
                .header("Content-Type", "application/json")
                .body(body.to_string())
                .unwrap()
                .send()
                .await
            {
                Ok(response) => {
                    if response.ok() {
                        tracing::info!("Local firmware upload started for device: {}", device_id);
                    } else {
                        let status = response.status();
                        let text = response.text().await.unwrap_or_default();
                        tracing::error!("Failed to upload local firmware: {} - {}", status, text);
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to upload local firmware: {:?}", e);
                }
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (device_id, firmware_data, base_url);
        tracing::warn!("Local firmware upload not available in native mode");
    }
}

// ============================================================================
// HCDF Import/Export (for file picker)
// ============================================================================

/// Pending HCDF export data (used when fetching HCDF for file save)
#[derive(Resource, Default)]
pub struct PendingHcdfExport(pub Arc<Mutex<Option<Vec<u8>>>>);

/// Export HCDF (fetch from backend for file save)
pub fn export_hcdf(base_url: &str, pending: &PendingHcdfExport) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen_futures::spawn_local;

        let base_url = base_url.to_string();
        let pending_clone = pending.0.clone();

        spawn_local(async move {
            let url = format!("{}/api/hcdf/export", base_url);

            tracing::info!("Fetching HCDF for export");

            match gloo_net::http::Request::get(&url).send().await {
                Ok(response) => {
                    if response.ok() {
                        if let Ok(text) = response.text().await {
                            // Parse as JSON to extract the XML content
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                if let Some(xml) = json.get("xml").and_then(|v| v.as_str()) {
                                    if let Ok(mut data) = pending_clone.lock() {
                                        *data = Some(xml.as_bytes().to_vec());
                                    }
                                    tracing::info!("HCDF export fetched successfully");
                                }
                            }
                        }
                    } else {
                        tracing::error!("Failed to export HCDF: {}", response.status());
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to export HCDF: {:?}", e);
                }
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (base_url, pending);
        tracing::warn!("HCDF export not available in native mode");
    }
}

/// Import HCDF (send to backend from file picker)
pub fn import_hcdf(xml_content: String, merge: bool, base_url: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen_futures::spawn_local;

        let base_url = base_url.to_string();

        spawn_local(async move {
            let url = format!("{}/api/hcdf/import", base_url);

            tracing::info!("Importing HCDF ({} bytes, merge={})", xml_content.len(), merge);

            let body = serde_json::json!({
                "xml": xml_content,
                "merge": merge
            });

            match gloo_net::http::Request::post(&url)
                .header("Content-Type", "application/json")
                .body(body.to_string())
                .unwrap()
                .send()
                .await
            {
                Ok(response) => {
                    if response.ok() {
                        tracing::info!("HCDF imported successfully");
                    } else {
                        let status = response.status();
                        let text = response.text().await.unwrap_or_default();
                        tracing::error!("Failed to import HCDF: {} - {}", status, text);
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to import HCDF: {:?}", e);
                }
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (xml_content, merge, base_url);
        tracing::warn!("HCDF import not available in native mode");
    }
}

/// Save HCDF to server filesystem (not browser download)
pub fn save_hcdf_to_server(base_url: &str, filename: Option<&str>) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen_futures::spawn_local;

        let base_url = base_url.to_string();
        let filename = filename.map(|s| s.to_string());

        spawn_local(async move {
            let url = format!("{}/api/hcdf/save", base_url);

            tracing::info!("Saving HCDF to server");

            let body = serde_json::json!({
                "filename": filename
            });

            match gloo_net::http::Request::post(&url)
                .header("Content-Type", "application/json")
                .body(body.to_string())
                .unwrap()
                .send()
                .await
            {
                Ok(response) => {
                    if response.ok() {
                        if let Ok(text) = response.text().await {
                            tracing::info!("HCDF saved to server: {}", text);
                        }
                    } else {
                        let status = response.status();
                        let text = response.text().await.unwrap_or_default();
                        tracing::error!("Failed to save HCDF to server: {} - {}", status, text);
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to save HCDF to server: {:?}", e);
                }
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (base_url, filename);
        tracing::warn!("HCDF save to server not available in native mode");
    }
}

/// Update device position and orientation on the backend
/// This syncs position changes to the HCDF so they're persisted on export
pub fn update_device_position(
    device_id: &str,
    position: [f32; 3],
    orientation: Option<[f32; 3]>,
    base_url: &str,
) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen_futures::spawn_local;

        let device_id = device_id.to_string();
        let base_url = base_url.to_string();
        // Convert f32 to f64 for the API
        let position = [position[0] as f64, position[1] as f64, position[2] as f64];
        let orientation = orientation.map(|o| [o[0] as f64, o[1] as f64, o[2] as f64]);

        spawn_local(async move {
            let url = format!("{}/api/devices/{}/position", base_url, device_id);

            let body = serde_json::json!({
                "position": position,
                "orientation": orientation
            });

            tracing::warn!(
                "Updating device {} position to {:?}, orientation {:?}",
                device_id, position, orientation
            );

            match gloo_net::http::Request::put(&url)
                .header("Content-Type", "application/json")
                .body(body.to_string())
                .unwrap()
                .send()
                .await
            {
                Ok(response) => {
                    if response.ok() {
                        tracing::warn!("Device {} position updated successfully", device_id);
                    } else {
                        let status = response.status();
                        let text = response.text().await.unwrap_or_default();
                        tracing::error!(
                            "Failed to update device {} position: {} - {}",
                            device_id, status, text
                        );
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to update device {} position: {:?}", device_id, e);
                }
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (device_id, position, orientation, base_url);
        tracing::warn!("Device position update not available in native mode");
    }
}
