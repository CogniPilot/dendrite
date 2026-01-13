//! Discovery scanner that combines all discovery methods

use anyhow::Result;
use dendrite_core::device::DiscoveryMethod;
use dendrite_core::{Device, DeviceId, DeviceStatus};
use dendrite_mcumgr::{query_result_to_device, MCUMGR_PORT};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tokio::time::{interval, Duration};
use tracing::{debug, info, warn};

use crate::arp::{get_arp_table, scan_subnet};
use crate::lldp::{get_lldp_neighbors, LldpNeighbor};
use crate::probe::{probe_hosts, query_hosts};

/// Scanner configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScannerConfig {
    /// Subnet to scan (e.g., "192.168.186.0")
    pub subnet: Ipv4Addr,
    /// Subnet prefix length (e.g., 24 for /24)
    pub prefix_len: u8,
    /// MCUmgr port
    pub mcumgr_port: u16,
    /// Full scan interval in seconds (discovers new devices)
    pub interval_secs: u64,
    /// Heartbeat interval in seconds (lightweight status check)
    pub heartbeat_interval_secs: u64,
    /// Whether heartbeat checking is enabled (sends ARP/ping to check connectivity)
    pub heartbeat_enabled: bool,
    /// Use LLDP for port detection
    pub use_lldp: bool,
    /// Use ARP scanning
    pub use_arp: bool,
    /// Parent device configuration
    pub parent: Option<ParentConfig>,
    /// Manual device overrides
    pub overrides: Vec<DeviceOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParentConfig {
    pub name: String,
    pub board: String,
    pub ports: u8,
    pub ip: Option<Ipv4Addr>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceOverride {
    pub hwid: String,
    pub name: Option<String>,
    pub port: Option<u8>,
    pub model_path: Option<String>,
}

impl Default for ScannerConfig {
    fn default() -> Self {
        Self {
            subnet: Ipv4Addr::new(192, 168, 186, 0),
            prefix_len: 24,
            mcumgr_port: MCUMGR_PORT,
            interval_secs: 60,          // Full scan every 60 seconds
            heartbeat_interval_secs: 2, // Lightweight ARP/ping check every 2 seconds
            heartbeat_enabled: false,   // Disabled by default (no network traffic until user enables)
            use_lldp: true,
            use_arp: true,
            parent: None,
            overrides: Vec::new(),
        }
    }
}

/// Discovery event for real-time updates
#[derive(Debug, Clone)]
pub enum DiscoveryEvent {
    /// New device discovered
    DeviceDiscovered(Device),
    /// Device went offline
    DeviceOffline(DeviceId),
    /// Device information updated
    DeviceUpdated(Device),
    /// Device removed from registry
    DeviceRemoved(DeviceId),
    /// Scan started
    ScanStarted,
    /// Scan completed
    ScanCompleted { found: usize, total: usize },
}

/// Discovery scanner service
pub struct DiscoveryScanner {
    config: Arc<RwLock<ScannerConfig>>,
    devices: Arc<RwLock<HashMap<String, Device>>>,
    event_tx: broadcast::Sender<DiscoveryEvent>,
}

impl DiscoveryScanner {
    /// Create a new scanner with the given configuration
    pub fn new(config: ScannerConfig) -> Self {
        let (event_tx, _) = broadcast::channel(100);
        Self {
            config: Arc::new(RwLock::new(config)),
            devices: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
        }
    }

    /// Update the scan subnet at runtime
    pub async fn update_subnet(&self, subnet: Ipv4Addr, prefix_len: u8) {
        let mut config = self.config.write().await;
        config.subnet = subnet;
        config.prefix_len = prefix_len;
        info!(subnet = %subnet, prefix = prefix_len, "Scan subnet updated");
    }

    /// Get current config
    pub async fn get_config(&self) -> ScannerConfig {
        self.config.read().await.clone()
    }

    /// Enable or disable heartbeat checking (ARP/ping connectivity checks)
    pub async fn set_heartbeat_enabled(&self, enabled: bool) {
        let mut config = self.config.write().await;
        config.heartbeat_enabled = enabled;
        info!(enabled = enabled, "Heartbeat checking {}", if enabled { "enabled" } else { "disabled" });
    }

    /// Check if heartbeat is enabled
    pub async fn is_heartbeat_enabled(&self) -> bool {
        self.config.read().await.heartbeat_enabled
    }

    /// Subscribe to discovery events
    pub fn subscribe(&self) -> broadcast::Receiver<DiscoveryEvent> {
        self.event_tx.subscribe()
    }

    /// Get current device list
    pub async fn devices(&self) -> Vec<Device> {
        self.devices.read().await.values().cloned().collect()
    }

    /// Get a specific device
    pub async fn get_device(&self, id: &DeviceId) -> Option<Device> {
        self.devices.read().await.get(&id.0).cloned()
    }

    /// Run a single discovery scan
    pub async fn scan_once(&self) -> Result<Vec<Device>> {
        let _ = self.event_tx.send(DiscoveryEvent::ScanStarted);

        // Get a snapshot of config for this scan
        let config = self.config.read().await.clone();

        info!(
            subnet = %config.subnet,
            prefix = config.prefix_len,
            "Starting discovery scan"
        );

        // Step 1: Get list of potential hosts
        let mut candidates: Vec<Ipv4Addr> = Vec::new();

        if config.use_arp {
            // Check ARP table first (instant)
            if let Ok(entries) = get_arp_table() {
                for entry in entries {
                    if is_in_subnet(entry.ip, config.subnet, config.prefix_len) {
                        candidates.push(entry.ip);
                    }
                }
            }

            // Also do active scan for hosts not in ARP table
            if let Ok(hosts) = scan_subnet(config.subnet, config.prefix_len).await {
                for host in hosts {
                    if !candidates.contains(&host) {
                        candidates.push(host);
                    }
                }
            }
        } else {
            // Just scan the subnet
            candidates = scan_subnet(config.subnet, config.prefix_len).await?;
        }

        debug!("Found {} candidate hosts", candidates.len());

        // Step 2: Probe for MCUmgr devices
        let mcumgr_hosts = probe_hosts(&candidates, config.mcumgr_port).await;

        debug!("Found {} MCUmgr devices", mcumgr_hosts.len());

        // Step 3: Query device information
        let query_results = query_hosts(&mcumgr_hosts, config.mcumgr_port).await;

        // Step 4: Get LLDP info for port mapping
        let lldp_neighbors = if config.use_lldp {
            get_lldp_neighbors().unwrap_or_default()
        } else {
            Vec::new()
        };

        // Step 5: Build/update device registry
        let mut discovered = Vec::new();
        let mut devices = self.devices.write().await;
        let existing_ids: Vec<String> = devices.keys().cloned().collect();

        for (ip, result) in query_results {
            let mut device =
                query_result_to_device(IpAddr::V4(ip), config.mcumgr_port, result);

            // Apply LLDP port mapping
            if let Some(mac) = get_mac_for_ip(ip) {
                device.discovery.mac = Some(mac.clone());
                if let Some(port) = find_port_for_mac(&lldp_neighbors, &mac) {
                    device.discovery.switch_port = Some(port);
                }
            }

            // Apply overrides
            if let Some(override_cfg) = config
                .overrides
                .iter()
                .find(|o| o.hwid == device.id.0)
            {
                if let Some(ref name) = override_cfg.name {
                    device.name = name.clone();
                }
                if let Some(port) = override_cfg.port {
                    device.discovery.switch_port = Some(port);
                }
                if let Some(ref model) = override_cfg.model_path {
                    device.model_path = Some(model.clone());
                }
            }

            // Set parent ID if configured
            if let Some(ref parent) = config.parent {
                device.parent_id = Some(DeviceId::from_hwid(&parent.name));
            }

            // Check for IP address conflicts - find any existing device with same IP
            let device_ip = device.discovery.ip;
            let conflicting_id = devices.iter()
                .find(|(id, d)| d.discovery.ip == device_ip && *id != &device.id.0)
                .map(|(id, _)| id.clone());

            if let Some(old_id) = conflicting_id {
                let new_has_real_id = !device.id.0.starts_with("temp-");
                let old_has_temp_id = old_id.starts_with("temp-");

                if new_has_real_id && old_has_temp_id {
                    // New device has real hwid, old had temp - remove old entry
                    debug!(
                        old_id = %old_id,
                        new_id = %device.id,
                        ip = %device_ip,
                        "Replacing temp device ID with real hardware ID"
                    );
                    devices.remove(&old_id);
                    let _ = self.event_tx.send(DiscoveryEvent::DeviceOffline(DeviceId::from_hwid(&old_id)));
                } else if !new_has_real_id && !old_has_temp_id {
                    // New device has temp ID but old has real ID - skip the temp one
                    debug!(
                        old_id = %old_id,
                        temp_id = %device.id,
                        ip = %device_ip,
                        "Ignoring temp ID, device already registered with real hardware ID"
                    );
                    // Update the existing device instead
                    if let Some(existing) = devices.get_mut(&old_id) {
                        existing.status = DeviceStatus::Online;
                        let _ = self.event_tx.send(DiscoveryEvent::DeviceUpdated(existing.clone()));
                        discovered.push(existing.clone());
                    }
                    continue;
                } else if new_has_real_id && !old_has_temp_id && device.id.0 != old_id {
                    // Both have real IDs but different - IP conflict warning
                    tracing::warn!(
                        old_id = %old_id,
                        new_id = %device.id,
                        ip = %device_ip,
                        "IP address conflict: two different devices claim same IP"
                    );
                }
            }

            // Check if new or updated
            let is_new = !devices.contains_key(&device.id.0);
            devices.insert(device.id.0.clone(), device.clone());

            if is_new {
                let _ = self.event_tx.send(DiscoveryEvent::DeviceDiscovered(device.clone()));
            } else {
                let _ = self.event_tx.send(DiscoveryEvent::DeviceUpdated(device.clone()));
            }

            discovered.push(device);
        }

        // Mark missing devices as offline
        for id in existing_ids {
            if !discovered.iter().any(|d| d.id.0 == id) {
                if let Some(device) = devices.get_mut(&id) {
                    if device.status == DeviceStatus::Online {
                        device.status = DeviceStatus::Offline;
                        let _ = self
                            .event_tx
                            .send(DiscoveryEvent::DeviceOffline(device.id.clone()));
                    }
                }
            }
        }

        let total = devices.len();
        let _ = self.event_tx.send(DiscoveryEvent::ScanCompleted {
            found: discovered.len(),
            total,
        });

        info!(
            "Scan complete: {} devices found, {} total tracked",
            discovered.len(),
            total
        );

        Ok(discovered)
    }

    /// Lightweight heartbeat check for known devices
    /// Checks if IPs are still reachable and marks devices online/offline accordingly
    pub async fn heartbeat(&self) -> Result<()> {
        let devices = self.devices.read().await;

        // Collect all known devices (both online and offline) with their IPs
        let device_ips: Vec<(String, Ipv4Addr, DeviceStatus)> = devices
            .values()
            .filter_map(|d| {
                if let IpAddr::V4(ip) = d.discovery.ip {
                    Some((d.id.0.clone(), ip, d.status.clone()))
                } else {
                    None
                }
            })
            .collect();

        if device_ips.is_empty() {
            return Ok(());
        }

        let online_count = device_ips.iter().filter(|(_, _, s)| *s == DeviceStatus::Online).count();
        let offline_count = device_ips.iter().filter(|(_, _, s)| *s == DeviceStatus::Offline).count();

        drop(devices); // Release read lock before async operation

        info!(online = online_count, offline = offline_count, "Heartbeat check");

        // Ping all known device IPs
        let all_ips: Vec<Ipv4Addr> = device_ips.iter().map(|(_, ip, _)| *ip).collect();
        let reachable = ping_hosts(&all_ips).await;
        let reachable_set: std::collections::HashSet<_> = reachable.into_iter().collect();

        // Update device statuses
        let mut devices = self.devices.write().await;
        for (id, ip, old_status) in device_ips {
            let is_reachable = reachable_set.contains(&ip);

            if let Some(device) = devices.get_mut(&id) {
                match (old_status, is_reachable) {
                    (DeviceStatus::Online, false) => {
                        // Was online, now unreachable -> mark offline
                        info!(device = %id, ip = %ip, "Device went offline");
                        device.status = DeviceStatus::Offline;
                        let _ = self.event_tx.send(DiscoveryEvent::DeviceOffline(device.id.clone()));
                    }
                    (DeviceStatus::Offline, true) => {
                        // Was offline, now reachable -> mark online
                        info!(device = %id, ip = %ip, "Device came back online");
                        device.status = DeviceStatus::Online;
                        let _ = self.event_tx.send(DiscoveryEvent::DeviceUpdated(device.clone()));
                    }
                    _ => {
                        // No change
                    }
                }
            }
        }

        Ok(())
    }

    /// Run continuous discovery in background
    /// Only runs heartbeat checks - full MCUmgr scans are manual only
    pub async fn run(&self) -> Result<()> {
        use tokio::time::interval;

        // Do initial full scan on startup
        info!("Running initial MCUmgr discovery scan");
        if let Err(e) = self.scan_once().await {
            warn!(error = %e, "Initial discovery scan failed");
        }

        // Use a fixed 2-second interval, but check config each time to see if heartbeat is enabled
        let mut heartbeat_interval = interval(Duration::from_secs(2));

        info!("Heartbeat scheduler started (MCUmgr scans are manual only)");

        loop {
            heartbeat_interval.tick().await;

            // Check if heartbeat is enabled (config may have changed at runtime)
            let config = self.config.read().await;
            if !config.heartbeat_enabled {
                // Heartbeat is disabled, skip this iteration
                continue;
            }
            drop(config);

            debug!("Running heartbeat check");
            if let Err(e) = self.heartbeat().await {
                warn!(error = %e, "Heartbeat check failed");
            }
        }
    }

    /// Manually add a device (sends DeviceDiscovered event)
    pub async fn add_device(&self, device: Device) {
        let mut devices = self.devices.write().await;
        devices.insert(device.id.0.clone(), device.clone());
        let _ = self.event_tx.send(DiscoveryEvent::DeviceDiscovered(device));
    }

    /// Update a device in the registry without sending events
    /// Used for internal updates like fragment matching
    pub async fn update_device_silent(&self, device: Device) {
        let mut devices = self.devices.write().await;
        devices.insert(device.id.0.clone(), device);
    }

    /// Remove a device by ID string, returns true if device was found and removed
    pub async fn remove_device(&self, id: &str) -> bool {
        let mut devices = self.devices.write().await;
        if let Some(device) = devices.remove(id) {
            info!(device = %id, "Device removed from registry");
            let _ = self.event_tx.send(DiscoveryEvent::DeviceRemoved(device.id.clone()));
            true
        } else {
            false
        }
    }
}

/// Check if IP is in subnet
fn is_in_subnet(ip: Ipv4Addr, subnet: Ipv4Addr, prefix_len: u8) -> bool {
    let ip_u32 = u32::from(ip);
    let subnet_u32 = u32::from(subnet);
    let mask = if prefix_len >= 32 {
        0xFFFFFFFF
    } else {
        !((1u32 << (32 - prefix_len)) - 1)
    };
    (ip_u32 & mask) == (subnet_u32 & mask)
}

/// Get MAC address for an IP from ARP table
fn get_mac_for_ip(ip: Ipv4Addr) -> Option<String> {
    if let Ok(entries) = get_arp_table() {
        for entry in entries {
            if entry.ip == ip {
                return Some(entry.mac);
            }
        }
    }
    None
}

/// Find switch port for a MAC address using LLDP
fn find_port_for_mac(neighbors: &[LldpNeighbor], mac: &str) -> Option<u8> {
    crate::lldp::find_port_for_mac(neighbors, mac)
}

/// Ping multiple hosts in parallel, return list of reachable IPs
async fn ping_hosts(hosts: &[Ipv4Addr]) -> Vec<Ipv4Addr> {
    use tokio::task::JoinSet;

    let mut tasks = JoinSet::new();

    for &ip in hosts {
        tasks.spawn(async move {
            let result = tokio::process::Command::new("ping")
                .args(["-c", "1", "-W", "1", &ip.to_string()])
                .output()
                .await;

            match result {
                Ok(output) if output.status.success() => Some(ip),
                _ => None,
            }
        });
    }

    let mut reachable = Vec::new();
    while let Some(result) = tasks.join_next().await {
        if let Ok(Some(ip)) = result {
            reachable.push(ip);
        }
    }

    reachable
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_in_subnet() {
        let subnet = Ipv4Addr::new(192, 168, 186, 0);
        assert!(is_in_subnet(Ipv4Addr::new(192, 168, 186, 1), subnet, 24));
        assert!(is_in_subnet(Ipv4Addr::new(192, 168, 186, 255), subnet, 24));
        assert!(!is_in_subnet(Ipv4Addr::new(192, 168, 187, 1), subnet, 24));
        assert!(!is_in_subnet(Ipv4Addr::new(10, 0, 0, 1), subnet, 24));
    }
}
