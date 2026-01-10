//! Device query functions using MCUmgr protocol

use anyhow::Result;
use dendrite_core::{Device, DeviceId, DeviceInfo, DeviceStatus, FirmwareInfo};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::transport::UdpTransportAsync;

/// MCUmgr port
pub const MCUMGR_PORT: u16 = 1337;

/// Default timeout for queries
pub const DEFAULT_TIMEOUT_MS: u64 = 5000;

#[derive(Error, Debug)]
pub enum QueryError {
    #[error("Device not reachable at {0}:{1}")]
    NotReachable(IpAddr, u16),
    #[error("Query failed: {0}")]
    QueryFailed(String),
    #[error("Invalid response: {0}")]
    InvalidResponse(String),
    #[error("Transport error: {0}")]
    TransportError(#[from] anyhow::Error),
}

/// Result of querying a device
#[derive(Debug, Clone)]
pub struct DeviceQueryResult {
    /// Hardware ID (chip unique ID)
    pub hwid: Option<String>,
    /// OS/kernel information
    pub os_info: Option<String>,
    /// App name (e.g., "optical-flow")
    pub app_name: Option<String>,
    /// Board type (e.g., "mr_mcxn_t1")
    pub board: Option<String>,
    /// Processor type
    pub processor: Option<String>,
    /// Bootloader info
    pub bootloader: Option<BootloaderInfo>,
    /// Firmware images
    pub images: Vec<ImageInfo>,
}

#[derive(Debug, Clone)]
pub struct BootloaderInfo {
    pub name: String,
    pub mode: Option<String>,
    pub no_downgrade: bool,
}

#[derive(Debug, Clone)]
pub struct ImageInfo {
    pub slot: u32,
    pub version: String,
    pub hash: String,
    pub bootable: bool,
    pub pending: bool,
    pub confirmed: bool,
    pub active: bool,
}

// MCUmgr request/response structures

#[derive(Serialize)]
struct OsInfoReq<'a> {
    format: &'a str,
}

#[derive(Deserialize)]
struct OsInfoRsp {
    output: String,
    #[serde(default)]
    rc: i32,
}

#[derive(Deserialize)]
struct BootloaderInfoRsp {
    #[serde(default)]
    bootloader: String,
    #[serde(default)]
    mode: Option<i32>,
    #[serde(default)]
    no_downgrade: Option<bool>,
}

#[derive(Deserialize)]
struct ImageStateRsp {
    #[serde(default)]
    images: Vec<ImageEntry>,
}

#[derive(Deserialize)]
struct ImageEntry {
    #[serde(default)]
    image: u32,
    #[serde(default)]
    slot: u32,
    #[serde(default)]
    version: String,
    #[serde(default, with = "hex_bytes")]
    hash: Vec<u8>,
    #[serde(default)]
    bootable: bool,
    #[serde(default)]
    pending: bool,
    #[serde(default)]
    confirmed: bool,
    #[serde(default)]
    active: bool,
}

// Helper for hex encoding/decoding hash bytes
mod hex_bytes {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes: serde_bytes::ByteBuf = Deserialize::deserialize(deserializer)?;
        Ok(bytes.into_vec())
    }
}

/// MCUmgr groups and commands
mod nmp {
    pub const GROUP_DEFAULT: u16 = 0;
    pub const GROUP_IMAGE: u16 = 1;

    pub const ID_OS_INFO: u8 = 7;
    pub const ID_BOOTLOADER_INFO: u8 = 8;
    pub const ID_IMAGE_STATE: u8 = 0;

    pub const OP_READ: u8 = 0;
    pub const OP_WRITE: u8 = 2;
}

/// CogniPilot HCDF MCUmgr group for querying device fragment information
pub mod hcdf_group {
    /// MCUmgr group ID for HCDF queries (CogniPilot custom group)
    pub const GROUP_HCDF: u16 = 100;

    /// Command ID for querying HCDF info (URL + SHA)
    pub const ID_HCDF_INFO: u8 = 0;
}

/// Response from HCDF info query
///
/// Devices that support the HCDF group will return their fragment URL and SHA,
/// allowing the daemon to skip network fetches if the cached version matches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HcdfInfoResponse {
    /// URL to the HCDF fragment file (e.g., "https://hcdf.cognipilot.org/spinali/v1.2.hcdf")
    #[serde(default)]
    pub url: Option<String>,
    /// SHA256 hash of the HCDF content (hex string)
    #[serde(default)]
    pub sha: Option<String>,
}

/// Query a device for all available information
pub async fn query_device(ip: IpAddr, port: u16) -> Result<DeviceQueryResult, QueryError> {
    info!(ip = %ip, port = port, "Querying device");

    let mut transport = UdpTransportAsync::new(&ip.to_string(), port, DEFAULT_TIMEOUT_MS).await?;

    // First check if device is reachable
    if !transport.ping().await.unwrap_or(false) {
        return Err(QueryError::NotReachable(ip, port));
    }

    debug!("Device is reachable, querying info");

    let mut result = DeviceQueryResult {
        hwid: None,
        os_info: None,
        app_name: None,
        board: None,
        processor: None,
        bootloader: None,
        images: Vec::new(),
    };

    // Query hardware ID
    if let Ok(hwid) = query_os_info(&mut transport, "h").await {
        result.hwid = Some(hwid);
    }

    // Query OS info (all fields)
    if let Ok(info) = query_os_info(&mut transport, "a").await {
        result.os_info = Some(info.clone());

        // Parse app name and board from the full os_info string
        // Format: "Zephyr <app> <hash> <version> <date> <arch> <proc> <board/soc/cpu> Zephyr hwid:<id>"
        let parsed = parse_os_info_fields(&info);
        result.app_name = parsed.app_name;
        result.board = parsed.board;
    }

    // Query processor
    if let Ok(proc) = query_os_info(&mut transport, "p").await {
        result.processor = Some(proc);
    }

    // Query bootloader info
    if let Ok(bl) = query_bootloader_info(&mut transport).await {
        result.bootloader = Some(bl);
    }

    // Query image state
    if let Ok(images) = query_image_state(&mut transport).await {
        result.images = images;
    }

    Ok(result)
}

/// Query OS info with specific format
async fn query_os_info(transport: &mut UdpTransportAsync, format: &str) -> Result<String> {
    let req = OsInfoReq { format };
    let body = serde_cbor::to_vec(&req)?;

    let resp_body = transport
        .transceive(nmp::OP_READ, nmp::GROUP_DEFAULT, nmp::ID_OS_INFO, &body)
        .await?;

    let resp: OsInfoRsp = serde_cbor::from_slice(&resp_body)?;
    if resp.rc != 0 {
        anyhow::bail!("OS info query failed with rc={}", resp.rc);
    }

    Ok(resp.output)
}

/// Query bootloader information
async fn query_bootloader_info(transport: &mut UdpTransportAsync) -> Result<BootloaderInfo> {
    let body = serde_cbor::to_vec(&HashMap::<String, String>::new())?;

    let resp_body = transport
        .transceive(
            nmp::OP_READ,
            nmp::GROUP_DEFAULT,
            nmp::ID_BOOTLOADER_INFO,
            &body,
        )
        .await?;

    let resp: BootloaderInfoRsp = serde_cbor::from_slice(&resp_body)?;

    let mode_name = resp.mode.map(|m| match m {
        0 => "Single application".to_string(),
        1 => "Swap using scratch".to_string(),
        2 => "Overwrite (upgrade-only)".to_string(),
        3 => "Swap without scratch".to_string(),
        4 => "Direct XIP without revert".to_string(),
        5 => "Direct XIP with revert".to_string(),
        6 => "RAM loader".to_string(),
        7 => "Firmware loader".to_string(),
        8 => "RAM load with network core".to_string(),
        9 => "Swap using move".to_string(),
        _ => format!("Unknown mode {}", m),
    });

    Ok(BootloaderInfo {
        name: resp.bootloader,
        mode: mode_name,
        no_downgrade: resp.no_downgrade.unwrap_or(false),
    })
}

/// Query image state (firmware slots)
async fn query_image_state(transport: &mut UdpTransportAsync) -> Result<Vec<ImageInfo>> {
    let body = serde_cbor::to_vec(&HashMap::<String, String>::new())?;

    let resp_body = transport
        .transceive(nmp::OP_READ, nmp::GROUP_IMAGE, nmp::ID_IMAGE_STATE, &body)
        .await?;

    let resp: ImageStateRsp = serde_cbor::from_slice(&resp_body)?;

    Ok(resp
        .images
        .into_iter()
        .map(|img| ImageInfo {
            slot: img.slot,
            version: img.version,
            hash: hex_encode(&img.hash),
            bootable: img.bootable,
            pending: img.pending,
            confirmed: img.confirmed,
            active: img.active,
        })
        .collect())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX_CHARS[(b >> 4) as usize] as char);
        s.push(HEX_CHARS[(b & 0xf) as usize] as char);
    }
    s
}

/// Parsed info from the os_info string
struct ParsedOsInfo {
    /// App name (e.g., "optical-flow")
    app_name: Option<String>,
    /// Board name (e.g., "mr_mcxn_t1")
    board: Option<String>,
}

/// Parse board and app name from the full os_info string
/// Example: "Zephyr optical-flow 4ad28d86da70 4.3.0-rc1 Sun Jan  4 02:34:48 2026 arm cortex-m33 mr_mcxn_t1/mcxn947/cpu0 Zephyr hwid:..."
fn parse_os_info_fields(os_info: &str) -> ParsedOsInfo {
    let mut result = ParsedOsInfo {
        app_name: None,
        board: None,
    };

    debug!(os_info = %os_info, "Parsing os_info string");

    let parts: Vec<&str> = os_info.split_whitespace().collect();

    // App name is the second word after "Zephyr"
    // Format: "Zephyr <app_name> <hash> <version> ..."
    if parts.len() > 1 && parts[0] == "Zephyr" {
        result.app_name = Some(parts[1].to_string());
        debug!(app_name = %parts[1], "Parsed app name");
    }

    // Board identifier contains "/" and looks like "board/soc" or "board/soc/cpu"
    // We want just the first part (the board name)
    for part in &parts {
        if part.starts_with("hwid:") {
            continue;
        }
        if part.contains('/') {
            // Verify it's not a version number
            if !part.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                // Extract just the board name (before first /)
                if let Some(board_name) = part.split('/').next() {
                    result.board = Some(board_name.to_string());
                    debug!(full_board = %part, board_name = %board_name, "Parsed board name");
                }
                break;
            }
        }
    }

    result
}

/// Probe an IP address to check if it has an MCUmgr device
pub async fn probe_device(ip: IpAddr, port: u16, timeout_ms: u64) -> bool {
    match UdpTransportAsync::new(&ip.to_string(), port, timeout_ms).await {
        Ok(mut transport) => transport.ping().await.unwrap_or(false),
        Err(_) => false,
    }
}

/// Query HCDF info from a device (URL and SHA of its fragment)
///
/// This queries the CogniPilot custom MCUmgr group (100) to get the device's
/// HCDF fragment URL and content hash. If the device doesn't support this group,
/// None is returned.
///
/// # Arguments
/// * `ip` - Device IP address
/// * `port` - MCUmgr port (usually 1337)
///
/// # Returns
/// * `Ok(Some(response))` - Device returned HCDF info
/// * `Ok(None)` - Device doesn't support HCDF group or returned empty response
/// * `Err(e)` - Transport or parse error
pub async fn query_hcdf_info(ip: IpAddr, port: u16) -> Result<Option<HcdfInfoResponse>, QueryError> {
    debug!(ip = %ip, port = port, "Querying HCDF info");

    let mut transport = UdpTransportAsync::new(&ip.to_string(), port, DEFAULT_TIMEOUT_MS).await?;

    // Send empty request body
    let body = serde_cbor::to_vec(&HashMap::<String, String>::new())
        .map_err(|e| QueryError::QueryFailed(e.to_string()))?;

    match transport
        .transceive(
            nmp::OP_READ,
            hcdf_group::GROUP_HCDF,
            hcdf_group::ID_HCDF_INFO,
            &body,
        )
        .await
    {
        Ok(resp_body) => {
            let resp: HcdfInfoResponse = serde_cbor::from_slice(&resp_body)
                .map_err(|e| QueryError::InvalidResponse(e.to_string()))?;

            // Return None if both fields are empty
            if resp.url.is_none() && resp.sha.is_none() {
                return Ok(None);
            }

            debug!(url = ?resp.url, sha = ?resp.sha, "Got HCDF info");
            Ok(Some(resp))
        }
        Err(e) => {
            // If the device doesn't support the group, it will return an error
            // This is expected behavior, not a failure
            debug!(error = %e, "HCDF group not supported");
            Ok(None)
        }
    }
}

/// Convert query result to Device struct
pub fn query_result_to_device(
    ip: IpAddr,
    port: u16,
    result: DeviceQueryResult,
) -> Device {
    let id = result
        .hwid
        .as_ref()
        .map(|h| DeviceId::from_hwid(h))
        .unwrap_or_else(DeviceId::temporary);

    // Use app name as the device name, falling back to board or IP
    let name = result
        .app_name
        .clone()
        .or_else(|| result.board.clone())
        .unwrap_or_else(|| format!("device-{}", ip));

    let mut device = Device::new(id, name, ip, port);
    device.status = DeviceStatus::Online;

    device.info = DeviceInfo {
        os_name: result.os_info,
        board: result.board,
        processor: result.processor,
        bootloader: result.bootloader.as_ref().map(|b| b.name.clone()),
        mcuboot_mode: result.bootloader.as_ref().and_then(|b| b.mode.clone()),
    };

    // Get active firmware info (prefer active, fall back to slot 0 or first image)
    let active_image = result
        .images
        .iter()
        .find(|i| i.active)
        .or_else(|| result.images.iter().find(|i| i.slot == 0))
        .or_else(|| result.images.first());

    if let Some(img) = active_image {
        device.firmware = FirmwareInfo {
            name: result.app_name.clone(),
            version: Some(img.version.clone()),
            hash: Some(img.hash.clone()),
            confirmed: img.confirmed,
            pending: img.pending,
            slot: Some(img.slot),
        };
    }

    device
}
