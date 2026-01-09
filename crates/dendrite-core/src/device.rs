//! Device types for tracking discovered hardware

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use uuid::Uuid;

/// Unique identifier for a device, derived from hardware ID
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceId(pub String);

impl DeviceId {
    /// Create a new DeviceId from a hardware ID string
    pub fn from_hwid(hwid: &str) -> Self {
        Self(hwid.to_string())
    }

    /// Create a new DeviceId from raw bytes (e.g., chip unique ID)
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(hex::encode(bytes))
    }

    /// Generate a temporary ID for devices where hardware ID is unknown
    pub fn temporary() -> Self {
        Self(format!("temp-{}", Uuid::new_v4()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Current status of a device
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceStatus {
    /// Device is online and responding
    Online,
    /// Device was seen but is not currently responding
    Offline,
    /// Device is being queried
    Probing,
    /// Device status is unknown
    Unknown,
}

impl Default for DeviceStatus {
    fn default() -> Self {
        Self::Unknown
    }
}

/// Information about firmware running on a device
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FirmwareInfo {
    /// Name of the firmware/application
    pub name: Option<String>,
    /// Version string (semver)
    pub version: Option<String>,
    /// SHA256 hash of the firmware image
    pub hash: Option<String>,
    /// Whether this image is confirmed (permanent)
    pub confirmed: bool,
    /// Whether this image is pending test
    pub pending: bool,
    /// Image slot number
    pub slot: Option<u32>,
}

/// Network discovery information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryInfo {
    /// IP address of the device
    pub ip: IpAddr,
    /// MCUmgr port (typically 1337)
    pub port: u16,
    /// Physical port on parent switch (if known)
    pub switch_port: Option<u8>,
    /// MAC address (if known)
    pub mac: Option<String>,
    /// When the device was first discovered
    pub first_seen: DateTime<Utc>,
    /// When the device was last seen responding
    pub last_seen: DateTime<Utc>,
    /// How the device was discovered
    pub discovery_method: DiscoveryMethod,
}

/// How a device was discovered
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiscoveryMethod {
    /// Discovered via LLDP
    Lldp,
    /// Discovered via ARP scan
    Arp,
    /// Discovered via MCUmgr port probe
    Probe,
    /// Manually configured
    Manual,
}

/// Complete device information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// OS/kernel name
    pub os_name: Option<String>,
    /// Board/hardware type
    pub board: Option<String>,
    /// Processor architecture
    pub processor: Option<String>,
    /// Bootloader information
    pub bootloader: Option<String>,
    /// MCUboot mode (if applicable)
    pub mcuboot_mode: Option<String>,
}

impl Default for DeviceInfo {
    fn default() -> Self {
        Self {
            os_name: None,
            board: None,
            processor: None,
            bootloader: None,
            mcuboot_mode: None,
        }
    }
}

/// A discovered device in the Dendrite system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    /// Unique device identifier (from hardware ID)
    pub id: DeviceId,
    /// Human-readable name (can be user-assigned)
    pub name: String,
    /// Current device status
    pub status: DeviceStatus,
    /// Network discovery information
    pub discovery: DiscoveryInfo,
    /// Device hardware/software information
    pub info: DeviceInfo,
    /// Firmware information
    pub firmware: FirmwareInfo,
    /// Parent device ID (for topology)
    pub parent_id: Option<DeviceId>,
    /// Path to 3D model file (glTF/GLB)
    pub model_path: Option<String>,
    /// Pose relative to parent (x, y, z, roll, pitch, yaw)
    pub pose: Option<[f64; 6]>,
}

impl Device {
    /// Create a new device with minimal information
    pub fn new(id: DeviceId, name: String, ip: IpAddr, port: u16) -> Self {
        let now = Utc::now();
        Self {
            id,
            name,
            status: DeviceStatus::Unknown,
            discovery: DiscoveryInfo {
                ip,
                port,
                switch_port: None,
                mac: None,
                first_seen: now,
                last_seen: now,
                discovery_method: DiscoveryMethod::Probe,
            },
            info: DeviceInfo::default(),
            firmware: FirmwareInfo::default(),
            parent_id: None,
            model_path: None,
            pose: None,
        }
    }

    /// Update the last seen timestamp
    pub fn touch(&mut self) {
        self.discovery.last_seen = Utc::now();
    }

    /// Check if the device has been seen recently
    pub fn is_stale(&self, timeout_secs: i64) -> bool {
        let elapsed = Utc::now() - self.discovery.last_seen;
        elapsed.num_seconds() > timeout_secs
    }
}

// Need hex for DeviceId::from_bytes
mod hex {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

    pub fn encode(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for &b in bytes {
            s.push(HEX_CHARS[(b >> 4) as usize] as char);
            s.push(HEX_CHARS[(b & 0xf) as usize] as char);
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_device_id_from_hwid() {
        let id = DeviceId::from_hwid("0x12345678");
        assert_eq!(id.as_str(), "0x12345678");
    }

    #[test]
    fn test_device_id_from_bytes() {
        let id = DeviceId::from_bytes(&[0x12, 0x34, 0x56, 0x78]);
        assert_eq!(id.as_str(), "12345678");
    }

    #[test]
    fn test_device_creation() {
        let id = DeviceId::from_hwid("test-001");
        let device = Device::new(
            id.clone(),
            "Test Device".to_string(),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)),
            1337,
        );
        assert_eq!(device.id, id);
        assert_eq!(device.status, DeviceStatus::Unknown);
    }
}
