//! REST API handlers

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use dendrite_core::DeviceId;
use dendrite_mcumgr::query_device as mcumgr_query;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info};

use crate::state::AppState;

/// API error response
#[derive(Serialize)]
struct ApiError {
    error: String,
}

impl ApiError {
    fn new(msg: impl Into<String>) -> Self {
        Self { error: msg.into() }
    }
}

/// List all discovered devices
pub async fn list_devices(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let devices = state.devices().await;
    Json(devices)
}

/// Get a specific device by ID
pub async fn get_device(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.get_device(&id).await {
        Some(device) => Json(device).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiError::new("Device not found")),
        )
            .into_response(),
    }
}

/// Query request body
#[derive(Deserialize)]
pub struct QueryRequest {
    /// Force re-query even if device was recently queried
    #[serde(default)]
    force: bool,
}

/// Trigger a query to a specific device
pub async fn query_device(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(_req): Json<QueryRequest>,
) -> impl IntoResponse {
    let device = match state.get_device(&id).await {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiError::new("Device not found")),
            )
                .into_response()
        }
    };

    info!(device = %id, "Manual device query requested");

    match mcumgr_query(device.discovery.ip, device.discovery.port).await {
        Ok(result) => {
            let updated = dendrite_mcumgr::query_result_to_device(
                device.discovery.ip,
                device.discovery.port,
                result,
            );
            Json(updated).into_response()
        }
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiError::new(format!("Query failed: {}", e))),
        )
            .into_response(),
    }
}

/// Get device topology
pub async fn get_topology(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let topology = state.get_topology().await;
    Json(topology.to_graph())
}

/// Get current HCDF document
pub async fn get_hcdf(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let hcdf = state.get_hcdf().await;
    match hcdf.to_xml() {
        Ok(xml) => (
            StatusCode::OK,
            [("content-type", "application/xml")],
            xml,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError::new(format!("Failed to serialize HCDF: {}", e))),
        )
            .into_response(),
    }
}

/// Save HCDF to file
pub async fn save_hcdf(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    match state.save_hcdf().await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"status": "saved"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError::new(format!("Failed to save HCDF: {}", e))),
        )
            .into_response(),
    }
}

/// Trigger a discovery scan
pub async fn trigger_scan(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    info!("Manual scan triggered");

    match state.scanner.scan_once().await {
        Ok(devices) => Json(serde_json::json!({
            "status": "completed",
            "devices_found": devices.len()
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError::new(format!("Scan failed: {}", e))),
        )
            .into_response(),
    }
}

/// Remove a device from the registry
pub async fn remove_device(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    info!(device = %id, "Remove device requested");

    if state.scanner.remove_device(&id).await {
        Json(serde_json::json!({
            "status": "removed",
            "device_id": id
        }))
        .into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(ApiError::new("Device not found")),
        )
            .into_response()
    }
}

/// Get current configuration
pub async fn get_config(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    Json(state.config.clone())
}

/// Network interface info for the UI
#[derive(Serialize)]
pub struct NetworkInterface {
    pub name: String,
    pub ip: String,
    pub subnet: String,
    pub prefix_len: u8,
}

/// List available network interfaces
pub async fn list_interfaces() -> impl IntoResponse {
    use network_interface::{NetworkInterface as NI, NetworkInterfaceConfig};

    let interfaces: Vec<NetworkInterface> = NI::show()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|iface| {
            // Find IPv4 address
            iface.addr.iter().find_map(|addr| {
                if let network_interface::Addr::V4(v4) = addr {
                    let ip = v4.ip;
                    let prefix = v4.netmask.map(|m| {
                        // Count bits in netmask
                        u32::from(m).count_ones() as u8
                    }).unwrap_or(24);

                    // Calculate subnet base address
                    let ip_u32 = u32::from(ip);
                    let mask = if prefix == 0 { 0 } else { !0u32 << (32 - prefix) };
                    let subnet_u32 = ip_u32 & mask;
                    let subnet = std::net::Ipv4Addr::from(subnet_u32);

                    Some(NetworkInterface {
                        name: iface.name.clone(),
                        ip: ip.to_string(),
                        subnet: subnet.to_string(),
                        prefix_len: prefix,
                    })
                } else {
                    None
                }
            })
        })
        .filter(|iface| {
            // Filter out loopback and docker
            !iface.name.starts_with("lo")
                && !iface.name.starts_with("docker")
                && !iface.name.starts_with("br-")
                && !iface.name.starts_with("veth")
                && iface.ip != "127.0.0.1"
        })
        .collect();

    Json(interfaces)
}

/// Request to update scan subnet
#[derive(Deserialize)]
pub struct UpdateSubnetRequest {
    pub subnet: String,
    pub prefix_len: u8,
}

/// Update the scan subnet configuration
pub async fn update_subnet(
    State(state): State<Arc<AppState>>,
    Json(req): Json<UpdateSubnetRequest>,
) -> impl IntoResponse {
    use std::net::Ipv4Addr;

    // Parse the subnet
    let subnet: Ipv4Addr = match req.subnet.parse() {
        Ok(ip) => ip,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiError::new("Invalid subnet address")),
            )
                .into_response();
        }
    };

    info!(subnet = %subnet, prefix = req.prefix_len, "Updating scan subnet");

    // Update scanner config
    state.scanner.update_subnet(subnet, req.prefix_len).await;

    Json(serde_json::json!({
        "status": "updated",
        "subnet": subnet.to_string(),
        "prefix_len": req.prefix_len
    }))
    .into_response()
}

/// Request to toggle heartbeat (connection checking)
#[derive(Deserialize)]
pub struct HeartbeatRequest {
    pub enabled: bool,
}

/// Enable or disable heartbeat connection checking
pub async fn set_heartbeat(
    State(state): State<Arc<AppState>>,
    Json(req): Json<HeartbeatRequest>,
) -> impl IntoResponse {
    info!(enabled = req.enabled, "Setting heartbeat checking");
    state.scanner.set_heartbeat_enabled(req.enabled).await;

    Json(serde_json::json!({
        "status": "updated",
        "heartbeat_enabled": req.enabled
    }))
}

/// Get heartbeat status
pub async fn get_heartbeat(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let enabled = state.scanner.is_heartbeat_enabled().await;
    Json(serde_json::json!({
        "heartbeat_enabled": enabled
    }))
}

// ============================================================================
// Device Position API Endpoints
// ============================================================================

/// Request to update device position
#[derive(Deserialize)]
pub struct UpdatePositionRequest {
    /// Position in meters: [x, y, z]
    pub position: [f64; 3],
    /// Optional orientation in radians: [roll, pitch, yaw]
    #[serde(default)]
    pub orientation: Option<[f64; 3]>,
}

/// Update device position and orientation
///
/// PUT /api/devices/:id/position
pub async fn update_device_position(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdatePositionRequest>,
) -> impl IntoResponse {
    tracing::warn!(device = %id, position = ?req.position, orientation = ?req.orientation, "Updating device position");

    // Build pose array: [x, y, z, roll, pitch, yaw]
    let pose = match req.orientation {
        Some([roll, pitch, yaw]) => [req.position[0], req.position[1], req.position[2], roll, pitch, yaw],
        None => [req.position[0], req.position[1], req.position[2], 0.0, 0.0, 0.0],
    };

    // Get the device from scanner
    let device_id = DeviceId::from_hwid(&id);
    let device = match state.scanner.get_device(&device_id).await {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiError::new("Device not found")),
            )
                .into_response();
        }
    };

    // Update device pose and push back to scanner
    let mut updated_device = device;
    updated_device.pose = Some(pose);
    state.scanner.update_device_silent(updated_device.clone()).await;

    // Update pose_cg in the HCDF MCU element
    {
        let mut hcdf = state.hcdf.write().await;
        let mcu_count = hcdf.mcu.len();
        tracing::warn!(device_id = %id, mcu_count = mcu_count, "Looking for MCU in HCDF");

        // Find MCU by hwid matching device id
        let mut found = false;
        for mcu in &mut hcdf.mcu {
            if let Some(hwid) = &mcu.hwid {
                tracing::warn!(hwid = %hwid, device_id = %id, "Checking MCU hwid");
                if hwid == &id {
                    // Format pose_cg as "x y z roll pitch yaw"
                    mcu.pose_cg = Some(format!(
                        "{} {} {} {} {} {}",
                        pose[0], pose[1], pose[2], pose[3], pose[4], pose[5]
                    ));
                    tracing::warn!(mcu = mcu.name, pose_cg = ?mcu.pose_cg, "Updated MCU pose_cg in HCDF");
                    found = true;
                    break;
                }
            }
        }

        if !found {
            // MCU doesn't exist in HCDF yet - create a minimal entry
            // This ensures position is persisted even before full device discovery completes
            use dendrite_core::hcdf::Mcu;
            let new_mcu = Mcu {
                name: updated_device.name.clone(),
                hwid: Some(id.clone()),
                description: None,
                pose_cg: Some(format!(
                    "{} {} {} {} {} {}",
                    pose[0], pose[1], pose[2], pose[3], pose[4], pose[5]
                )),
                mass: None,
                board: updated_device.info.board.clone(),
                software: None,
                discovered: None,
                model: None,
                visual: Vec::new(),
                frame: Vec::new(),
                network: None,
            };
            hcdf.mcu.push(new_mcu);
            tracing::warn!(device_id = %id, mcu_count = mcu_count, "Created new MCU in HCDF with position");
        }
    }

    // Auto-save HCDF to persist position changes
    if let Err(e) = state.save_hcdf().await {
        tracing::warn!(error = %e, "Failed to auto-save HCDF after position update");
    }

    // Broadcast device update via WebSocket
    state.scanner.broadcast_device_update(updated_device).await;

    Json(serde_json::json!({
        "status": "updated",
        "device_id": id,
        "pose": pose
    }))
    .into_response()
}

// ============================================================================
// Firmware API Endpoints
// ============================================================================

/// Firmware check response
#[derive(Serialize)]
pub struct FirmwareCheckResponse {
    pub device_id: String,
    pub current_version: Option<String>,
    /// MCUboot image hash from the device (what MCUmgr reports)
    pub current_mcuboot_hash: Option<String>,
    pub latest_version: Option<String>,
    /// MCUboot image hash for the latest release (for verification after OTA)
    pub latest_mcuboot_hash: Option<String>,
    pub status: dendrite_core::FirmwareStatus,
    pub changelog: Option<String>,
}

/// Check firmware status for a specific device
///
/// GET /api/firmware/:id/check
pub async fn check_firmware(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // Get device
    let device = match state.get_device(&id).await {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiError::new("Device not found")),
            )
                .into_response()
        }
    };

    // Need board and app name to fetch manifest
    let board = match &device.info.board {
        Some(b) => b.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiError::new("Device has no board info")),
            )
                .into_response()
        }
    };

    let app = match &device.firmware.name {
        Some(a) => a.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiError::new("Device has no app name")),
            )
                .into_response()
        }
    };

    // Get firmware_manifest_uri from HCDF software element
    let firmware_manifest_uri = {
        let hcdf = state.hcdf.read().await;
        hcdf.mcu
            .iter()
            .find(|m| m.hwid.as_deref() == Some(&id))
            .and_then(|m| m.software.as_ref())
            .and_then(|s| s.firmware_manifest_uri.clone())
    };

    info!(device = %id, board = %board, app = %app, uri = ?firmware_manifest_uri, "Checking firmware status");

    // Fetch firmware manifest (requires explicit firmware_manifest_uri)
    let manifest = match state.firmware_fetcher.get_manifest(&board, &app, firmware_manifest_uri.as_deref()).await {
        Ok(Some(m)) => m,
        Ok(None) => {
            return Json(FirmwareCheckResponse {
                device_id: id,
                current_version: device.firmware.version.clone(),
                current_mcuboot_hash: device.firmware.image_hash.clone(),
                latest_version: None,
                latest_mcuboot_hash: None,
                status: dendrite_core::FirmwareStatus::Unknown,
                changelog: None,
            })
            .into_response()
        }
        Err(e) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ApiError::new(format!("Failed to fetch manifest: {}", e))),
            )
                .into_response()
        }
    };

    // Compare versions
    let status = dendrite_core::compare_versions(
        device.firmware.version.as_deref(),
        device.firmware.build_date,
        &manifest,
    );

    Json(FirmwareCheckResponse {
        device_id: id,
        current_version: device.firmware.version.clone(),
        current_mcuboot_hash: device.firmware.image_hash.clone(),
        latest_version: Some(manifest.latest.version.clone()),
        latest_mcuboot_hash: Some(manifest.latest.mcuboot_hash.clone()),
        status,
        changelog: manifest.latest.changelog.clone(),
    })
    .into_response()
}

/// Check firmware for all devices
///
/// GET /api/firmware/check
pub async fn check_all_firmware(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let devices = state.devices().await;
    let mut results = Vec::new();

    for device in devices {
        let id = device.id.0.clone();

        // Skip devices without board/app info
        let (board, app) = match (&device.info.board, &device.firmware.name) {
            (Some(b), Some(a)) => (b.clone(), a.clone()),
            _ => {
                results.push(FirmwareCheckResponse {
                    device_id: id,
                    current_version: device.firmware.version.clone(),
                    current_mcuboot_hash: device.firmware.image_hash.clone(),
                    latest_version: None,
                    latest_mcuboot_hash: None,
                    status: dendrite_core::FirmwareStatus::Unknown,
                    changelog: None,
                });
                continue;
            }
        };

        // Get firmware_manifest_uri from HCDF software element
        let firmware_manifest_uri = {
            let hcdf = state.hcdf.read().await;
            hcdf.mcu
                .iter()
                .find(|m| m.hwid.as_deref() == Some(&id))
                .and_then(|m| m.software.as_ref())
                .and_then(|s| s.firmware_manifest_uri.clone())
        };

        // Fetch manifest (requires explicit firmware_manifest_uri)
        let (latest_version, latest_mcuboot_hash, status, changelog) =
            match state.firmware_fetcher.get_manifest(&board, &app, firmware_manifest_uri.as_deref()).await {
                Ok(Some(manifest)) => {
                    let status = dendrite_core::compare_versions(
                        device.firmware.version.as_deref(),
                        device.firmware.build_date,
                        &manifest,
                    );
                    (
                        Some(manifest.latest.version.clone()),
                        Some(manifest.latest.mcuboot_hash.clone()),
                        status,
                        manifest.latest.changelog.clone(),
                    )
                }
                _ => (None, None, dendrite_core::FirmwareStatus::Unknown, None),
            };

        results.push(FirmwareCheckResponse {
            device_id: id,
            current_version: device.firmware.version.clone(),
            current_mcuboot_hash: device.firmware.image_hash.clone(),
            latest_version,
            latest_mcuboot_hash,
            status,
            changelog,
        });
    }

    Json(results)
}

// ============================================================================
// OTA (Over-The-Air) Update API Endpoints
// ============================================================================

use crate::ota::UpdateState;

/// OTA update start response
#[derive(Serialize)]
pub struct OtaStartResponse {
    pub device_id: String,
    pub status: String,
}

/// OTA update progress response
#[derive(Serialize)]
pub struct OtaProgressResponse {
    pub device_id: String,
    pub state: Option<UpdateState>,
}

/// Start an OTA firmware update for a device
///
/// POST /api/ota/:id/start
pub async fn start_ota_update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // Get device
    let device = match state.get_device(&id).await {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiError::new("Device not found")),
            )
                .into_response()
        }
    };

    // Need board and app name for firmware fetching
    let board = match &device.info.board {
        Some(b) => b.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiError::new("Device has no board info")),
            )
                .into_response()
        }
    };

    let app = match &device.firmware.name {
        Some(a) => a.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiError::new("Device has no app name")),
            )
                .into_response()
        }
    };

    // Get firmware_manifest_uri from HCDF software element
    let firmware_manifest_uri = {
        let hcdf = state.hcdf.read().await;
        hcdf.mcu
            .iter()
            .find(|m| m.hwid.as_deref() == Some(&id))
            .and_then(|m| m.software.as_ref())
            .and_then(|s| s.firmware_manifest_uri.clone())
    };

    info!(device = %id, board = %board, app = %app, uri = ?firmware_manifest_uri, "Starting OTA update");

    // Start the update (requires explicit firmware_manifest_uri)
    match state
        .ota_service
        .start_update(id.clone(), device.discovery.ip.to_string(), board, app, firmware_manifest_uri)
        .await
    {
        Ok(()) => Json(OtaStartResponse {
            device_id: id,
            status: "started".to_string(),
        })
        .into_response(),
        Err(e) => (
            StatusCode::CONFLICT,
            Json(ApiError::new(format!("Failed to start update: {}", e))),
        )
            .into_response(),
    }
}

/// Get OTA update progress for a device
///
/// GET /api/ota/:id/progress
pub async fn get_ota_progress(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let update_state = state.ota_service.get_state(&id).await;

    Json(OtaProgressResponse {
        device_id: id,
        state: update_state,
    })
}

/// Get all active OTA updates
///
/// GET /api/ota
pub async fn get_all_ota_updates(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let updates = state.ota_service.get_all_updates().await;

    let responses: Vec<OtaProgressResponse> = updates
        .into_iter()
        .map(|(device_id, update_state)| OtaProgressResponse {
            device_id,
            state: Some(update_state),
        })
        .collect();

    Json(responses)
}

/// Cancel an OTA update for a device
///
/// POST /api/ota/:id/cancel
pub async fn cancel_ota_update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    info!(device = %id, "Cancelling OTA update");

    match state.ota_service.cancel_update(&id).await {
        Ok(()) => Json(serde_json::json!({
            "device_id": id,
            "status": "cancelled"
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError::new(format!("Failed to cancel update: {}", e))),
        )
            .into_response(),
    }
}

/// Request body for local firmware upload
#[derive(Deserialize)]
pub struct LocalFirmwareUpload {
    /// Base64-encoded firmware binary
    pub firmware_base64: String,
}

/// Upload local firmware binary to a device (for development use)
///
/// POST /api/ota/:id/upload-local
///
/// This allows uploading a local firmware binary directly without going through
/// the firmware repository. Useful for development and testing.
pub async fn upload_local_firmware(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<LocalFirmwareUpload>,
) -> impl IntoResponse {
    // Get device
    let device = match state.get_device(&id).await {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiError::new("Device not found")),
            )
                .into_response()
        }
    };

    // Decode base64 firmware
    use base64::Engine;
    let firmware_data = match base64::engine::general_purpose::STANDARD.decode(&req.firmware_base64) {
        Ok(data) => data,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiError::new(format!("Invalid base64 firmware data: {}", e))),
            )
                .into_response()
        }
    };

    info!(
        device = %id,
        size = firmware_data.len(),
        "Starting local firmware upload"
    );

    // Start the upload
    match state
        .ota_service
        .upload_local_firmware(id.clone(), device.discovery.ip.to_string(), firmware_data)
        .await
    {
        Ok(()) => Json(OtaStartResponse {
            device_id: id,
            status: "started".to_string(),
        })
        .into_response(),
        Err(e) => (
            StatusCode::CONFLICT,
            Json(ApiError::new(format!("Failed to start local upload: {}", e))),
        )
            .into_response(),
    }
}

// ============================================================================
// HCDF Import/Export API Endpoints
// ============================================================================

/// Export the current HCDF as XML
///
/// GET /api/hcdf/export
pub async fn export_hcdf(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let hcdf = state.hcdf.read().await;

    match hcdf.to_xml() {
        Ok(xml) => (
            StatusCode::OK,
            Json(serde_json::json!({ "xml": xml })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError::new(format!("Failed to export HCDF: {}", e))),
        )
            .into_response(),
    }
}

/// Request body for HCDF import
#[derive(Deserialize)]
pub struct HcdfImportRequest {
    /// HCDF XML content
    pub xml: String,
    /// Whether to merge with existing HCDF (true) or replace (false)
    #[serde(default)]
    pub merge: bool,
}

/// Import HCDF from XML
///
/// POST /api/hcdf/import
pub async fn import_hcdf(
    State(state): State<Arc<AppState>>,
    Json(req): Json<HcdfImportRequest>,
) -> impl IntoResponse {
    use dendrite_core::{Hcdf, Device, DeviceId, DeviceStatus, DeviceInfo, FirmwareInfo, parse_pose_string};
    use dendrite_core::device::{DiscoveryInfo, DiscoveryMethod, DeviceVisual, DeviceFrame};
    use chrono::{DateTime, Utc};
    use std::net::IpAddr;

    // Parse the incoming HCDF
    let imported_hcdf = match Hcdf::from_xml(&req.xml) {
        Ok(h) => h,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiError::new(format!("Invalid HCDF XML: {}", e))),
            )
                .into_response()
        }
    };

    // Collect MCUs and Comps to convert to devices
    let mcus_to_import: Vec<_> = imported_hcdf.mcu.clone();
    let comps_to_import: Vec<_> = imported_hcdf.comp.clone();
    let mcu_count = mcus_to_import.len();
    let comp_count = comps_to_import.len();

    // Update HCDF state - always merge to preserve existing devices
    {
        let mut hcdf = state.hcdf.write().await;

        // Merge MCUs by hwid (update if exists, add if new)
        for mcu in &mcus_to_import {
            if let Some(hwid) = &mcu.hwid {
                if let Some(existing) = hcdf.mcu.iter_mut().find(|m| m.hwid.as_deref() == Some(hwid)) {
                    // Update existing MCU
                    *existing = mcu.clone();
                    debug!("Updated existing MCU '{}' (hwid: {})", mcu.name, hwid);
                } else {
                    // Add new MCU
                    hcdf.mcu.push(mcu.clone());
                    debug!("Added new MCU '{}' (hwid: {})", mcu.name, hwid);
                }
            } else {
                // MCU without hwid - add by name match or append
                if let Some(existing) = hcdf.mcu.iter_mut().find(|m| m.name == mcu.name && m.hwid.is_none()) {
                    *existing = mcu.clone();
                    debug!("Updated existing MCU '{}' (no hwid)", mcu.name);
                } else {
                    hcdf.mcu.push(mcu.clone());
                    debug!("Added new MCU '{}' (no hwid)", mcu.name);
                }
            }
        }

        // Merge Comps by hwid or name
        for comp in &comps_to_import {
            let comp_key = comp.hwid.as_ref()
                .map(|h| format!("hwid:{}", h))
                .unwrap_or_else(|| format!("name:{}", comp.name));

            let existing = if let Some(hwid) = &comp.hwid {
                hcdf.comp.iter_mut().find(|c| c.hwid.as_deref() == Some(hwid))
            } else {
                hcdf.comp.iter_mut().find(|c| c.name == comp.name && c.hwid.is_none())
            };

            if let Some(existing) = existing {
                *existing = comp.clone();
                debug!("Updated existing comp '{}'", comp_key);
            } else {
                hcdf.comp.push(comp.clone());
                debug!("Added new comp '{}'", comp_key);
            }
        }

        // Merge links, sensors, motors, power from imported HCDF
        for link in &imported_hcdf.link {
            if !hcdf.link.iter().any(|l| l.name == link.name) {
                hcdf.link.push(link.clone());
            }
        }
        for sensor in &imported_hcdf.sensor {
            if !hcdf.sensor.iter().any(|s| s.name == sensor.name) {
                hcdf.sensor.push(sensor.clone());
            }
        }
        for motor in &imported_hcdf.motor {
            if !hcdf.motor.iter().any(|m| m.name == motor.name) {
                hcdf.motor.push(motor.clone());
            }
        }
        for power in &imported_hcdf.power {
            if !hcdf.power.iter().any(|p| p.name == power.name) {
                hcdf.power.push(power.clone());
            }
        }

        info!("Merged HCDF data ({} MCUs, {} Comps imported, now {} MCUs, {} Comps total)",
              mcu_count, comp_count, hcdf.mcu.len(), hcdf.comp.len());
    }

    let mut devices_imported = 0;

    // Convert MCUs to Devices and add to scanner (which broadcasts events)
    for mcu in mcus_to_import {
        // Need hwid to create device ID
        let device_id = match &mcu.hwid {
            Some(hwid) => DeviceId::from_hwid(hwid),
            None => {
                info!("Skipping MCU '{}' - no hwid", mcu.name);
                continue;
            }
        };

        // Parse IP from discovered info
        let (ip, port, last_seen) = match &mcu.discovered {
            Some(disc) => {
                let ip: IpAddr = match disc.ip.parse() {
                    Ok(ip) => ip,
                    Err(_) => {
                        info!("Skipping MCU '{}' - invalid IP '{}'", mcu.name, disc.ip);
                        continue;
                    }
                };
                let port: u16 = disc.port.map(|p| p as u16).unwrap_or(1337);
                let last_seen = disc.last_seen
                    .as_ref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(Utc::now);
                (ip, port, last_seen)
            }
            None => {
                info!("Skipping MCU '{}' - no discovery info", mcu.name);
                continue;
            }
        };

        // Build firmware info from software element
        let firmware = mcu.software.as_ref().map(|sw| FirmwareInfo {
            name: Some(sw.name.clone()),
            version: sw.version.clone(),
            build_date: None,
            image_hash: sw.hash.clone(),
            confirmed: true,
            pending: false,
            slot: None,
        }).unwrap_or_default();

        // Build device info
        let info = DeviceInfo {
            os_name: Some("Zephyr".to_string()),
            board: mcu.board.clone(),
            processor: None,
            bootloader: None,
            mcuboot_mode: None,
        };

        // Parse pose from pose_cg string
        let pose: Option<[f64; 6]> = mcu.pose_cg.as_ref().and_then(|s| {
            parse_pose_string(s).map(|p| [p.x, p.y, p.z, p.roll, p.pitch, p.yaw])
        });

        // Create device
        let mut device = Device {
            id: device_id,
            name: mcu.name.clone(),
            status: DeviceStatus::Unknown, // Will be checked by heartbeat
            discovery: DiscoveryInfo {
                ip,
                port,
                switch_port: mcu.discovered.as_ref().and_then(|d| d.port),
                mac: None,
                first_seen: last_seen,
                last_seen,
                discovery_method: DiscoveryMethod::Manual,
            },
            info,
            firmware,
            firmware_status: Default::default(),
            firmware_manifest_uri: mcu.software.as_ref().and_then(|s| s.firmware_manifest_uri.clone()),
            parent_id: None,
            model_path: mcu.model.as_ref().map(|m| m.href.clone()),
            pose,
            visuals: Vec::new(),
            frames: Vec::new(),
            ports: Vec::new(),
            sensors: Vec::new(),
        };

        // Let AppState apply fragment matching and other enrichment
        device = state.update_device(&device).await;

        // Add to scanner (this broadcasts DeviceDiscovered event to WebSocket clients)
        state.scanner.add_device(device).await;
        info!("Imported device '{}' from HCDF MCU", mcu.name);
        devices_imported += 1;
    }

    // Convert Comps with visuals to "scene objects" (devices with placeholder network info)
    for comp in comps_to_import {
        // Skip comps without visuals - nothing to render
        if comp.visual.is_empty() {
            debug!("Skipping comp '{}' - no visuals", comp.name);
            continue;
        }

        // Create a synthetic device ID from comp name (or hwid if present)
        let device_id = comp.hwid.as_ref()
            .map(|h| DeviceId::from_hwid(h))
            .unwrap_or_else(|| DeviceId::from_hwid(&format!("comp-{}", comp.name)));

        let now = Utc::now();

        // Convert comp visuals to device visuals
        // Model paths in HCDF are relative to hcdf.cognipilot.org root
        let visuals: Vec<DeviceVisual> = comp.visual.iter().map(|v| {
            let model_path = v.model.as_ref().map(|m| {
                // If the model href is relative, prepend the HCDF CDN base URL
                if m.href.starts_with("http://") || m.href.starts_with("https://") {
                    m.href.clone()
                } else {
                    format!("https://hcdf.cognipilot.org/{}", m.href.trim_start_matches("./"))
                }
            });
            DeviceVisual {
                name: v.name.clone(),
                toggle: v.toggle.clone(),
                pose: v.pose.as_ref().and_then(|p| parse_pose_string(p)).map(|p| [p.x, p.y, p.z, p.roll, p.pitch, p.yaw]),
                model_path,
                model_sha: v.model.as_ref().and_then(|m| m.sha.clone()),
            }
        }).collect();

        // Convert comp frames to device frames
        let frames: Vec<DeviceFrame> = comp.frame.iter().map(|f| {
            DeviceFrame {
                name: f.name.clone(),
                description: f.description.clone(),
                pose: f.pose.as_ref().and_then(|p| parse_pose_string(p)).map(|p| [p.x, p.y, p.z, p.roll, p.pitch, p.yaw]),
            }
        }).collect();

        // Parse pose from pose_cg string
        let pose: Option<[f64; 6]> = comp.pose_cg.as_ref().and_then(|s| {
            parse_pose_string(s).map(|p| [p.x, p.y, p.z, p.roll, p.pitch, p.yaw])
        });

        // Create device (scene object) with placeholder network info
        let device = Device {
            id: device_id,
            name: comp.name.clone(),
            status: DeviceStatus::Offline, // Static scene object - use Offline so it can be deleted
            discovery: DiscoveryInfo {
                ip: "127.0.0.1".parse().unwrap(), // Placeholder - not a real device
                port: 0,
                switch_port: None,
                mac: None,
                first_seen: now,
                last_seen: now,
                discovery_method: DiscoveryMethod::Manual,
            },
            info: DeviceInfo {
                os_name: None,
                board: comp.board.clone(),
                processor: None,
                bootloader: None,
                mcuboot_mode: None,
            },
            firmware: FirmwareInfo::default(),
            firmware_status: Default::default(),
            firmware_manifest_uri: None,
            parent_id: None,
            model_path: comp.model.as_ref().map(|m| m.href.clone()),
            pose,
            visuals,
            frames,
            ports: Vec::new(), // TODO: Convert comp.port if needed
            sensors: Vec::new(), // TODO: Convert comp.sensor if needed
        };

        // Add to scanner (this broadcasts DeviceDiscovered event to WebSocket clients)
        state.scanner.add_device(device).await;
        info!("Imported scene object '{}' from HCDF comp ({} visuals)", comp.name, comp.visual.len());
        devices_imported += 1;
    }

    Json(serde_json::json!({
        "status": "imported",
        "merge": req.merge,
        "mcu_count": mcu_count,
        "comp_count": comp_count,
        "devices_imported": devices_imported
    }))
    .into_response()
}

/// Request body for HCDF save to server
#[derive(Deserialize)]
pub struct HcdfSaveRequest {
    /// Optional filename (without path) - defaults to "dendrite_config.hcdf"
    #[serde(default)]
    pub filename: Option<String>,
}

/// Save HCDF to server filesystem
///
/// POST /api/hcdf/save
pub async fn save_hcdf_to_server(
    State(state): State<Arc<AppState>>,
    Json(req): Json<HcdfSaveRequest>,
) -> impl IntoResponse {
    use std::path::PathBuf;
    use tokio::fs;

    let hcdf = state.hcdf.read().await;

    let xml = match hcdf.to_xml() {
        Ok(xml) => xml,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError::new(format!("Failed to serialize HCDF: {}", e))),
            )
                .into_response()
        }
    };

    // Determine save path - use config hcdf_path directory or default to current dir
    let filename = req.filename.unwrap_or_else(|| "dendrite_config.hcdf".to_string());

    // Sanitize filename - only allow alphanumeric, underscore, hyphen, and .hcdf extension
    let sanitized_filename = filename
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
        .collect::<String>();

    // Ensure .hcdf extension
    let sanitized_filename = if sanitized_filename.ends_with(".hcdf") {
        sanitized_filename
    } else {
        format!("{}.hcdf", sanitized_filename)
    };

    // Save to the configured hcdf path directory, or current directory if not set
    let hcdf_path = PathBuf::from(&state.config.hcdf.path);
    let parent = hcdf_path.parent().unwrap_or(std::path::Path::new("."));
    let save_path = parent.join(&sanitized_filename);

    info!("Saving HCDF to server: {:?}", save_path);

    match fs::write(&save_path, &xml).await {
        Ok(()) => {
            info!("HCDF saved successfully to {:?}", save_path);
            Json(serde_json::json!({
                "status": "saved",
                "path": save_path.to_string_lossy(),
                "size": xml.len()
            }))
            .into_response()
        }
        Err(e) => {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError::new(format!("Failed to write HCDF file: {}", e))),
            )
                .into_response()
        }
    }
}
