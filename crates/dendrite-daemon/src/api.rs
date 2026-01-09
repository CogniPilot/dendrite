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
