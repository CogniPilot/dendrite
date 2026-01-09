//! ARP-based network scanning for device discovery

use anyhow::Result;
use std::net::{IpAddr, Ipv4Addr};
use std::process::Command;
use std::str::FromStr;
use tracing::{debug, trace, warn};

/// ARP table entry
#[derive(Debug, Clone)]
pub struct ArpEntry {
    pub ip: Ipv4Addr,
    pub mac: String,
    pub interface: String,
    pub state: ArpState,
}

/// ARP entry state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArpState {
    Reachable,
    Stale,
    Delay,
    Probe,
    Failed,
    Incomplete,
    Permanent,
    Unknown,
}

/// Get current ARP table entries
pub fn get_arp_table() -> Result<Vec<ArpEntry>> {
    // Use `ip neigh` command on Linux
    let output = Command::new("ip")
        .args(["neigh", "show"])
        .output()?;

    if !output.status.success() {
        anyhow::bail!("Failed to get ARP table: {}", String::from_utf8_lossy(&output.stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();

    for line in stdout.lines() {
        if let Some(entry) = parse_ip_neigh_line(line) {
            entries.push(entry);
        }
    }

    debug!("Found {} ARP entries", entries.len());
    Ok(entries)
}

/// Parse a line from `ip neigh show` output
fn parse_ip_neigh_line(line: &str) -> Option<ArpEntry> {
    // Format: "192.168.1.1 dev eth0 lladdr aa:bb:cc:dd:ee:ff REACHABLE"
    let parts: Vec<&str> = line.split_whitespace().collect();

    if parts.len() < 4 {
        return None;
    }

    let ip = Ipv4Addr::from_str(parts[0]).ok()?;

    // Find "dev" and "lladdr" positions
    let dev_idx = parts.iter().position(|&p| p == "dev")?;
    let lladdr_idx = parts.iter().position(|&p| p == "lladdr");

    if dev_idx + 1 >= parts.len() {
        return None;
    }

    let interface = parts[dev_idx + 1].to_string();

    // MAC might not be present for INCOMPLETE entries
    let mac = lladdr_idx
        .and_then(|idx| parts.get(idx + 1))
        .map(|s| s.to_string())
        .unwrap_or_default();

    // Parse state from last part
    let state = parts.last().map(|s| parse_arp_state(s)).unwrap_or(ArpState::Unknown);

    Some(ArpEntry { ip, mac, interface, state })
}

/// Parse ARP state string
fn parse_arp_state(s: &str) -> ArpState {
    match s.to_uppercase().as_str() {
        "REACHABLE" => ArpState::Reachable,
        "STALE" => ArpState::Stale,
        "DELAY" => ArpState::Delay,
        "PROBE" => ArpState::Probe,
        "FAILED" => ArpState::Failed,
        "INCOMPLETE" => ArpState::Incomplete,
        "PERMANENT" => ArpState::Permanent,
        _ => ArpState::Unknown,
    }
}

/// Scan a subnet for reachable hosts using ping
pub async fn scan_subnet(subnet: Ipv4Addr, prefix_len: u8) -> Result<Vec<Ipv4Addr>> {
    let subnet_u32 = u32::from(subnet);
    let mask = if prefix_len >= 32 {
        0xFFFFFFFF
    } else {
        !((1u32 << (32 - prefix_len)) - 1)
    };
    let network = subnet_u32 & mask;
    let broadcast = network | !mask;

    let mut hosts = Vec::new();

    // Skip network and broadcast addresses
    for host in (network + 1)..broadcast {
        hosts.push(Ipv4Addr::from(host));
    }

    debug!(
        "Scanning {} hosts in {}/{}",
        hosts.len(),
        subnet,
        prefix_len
    );

    // Use fping if available (much faster), otherwise fall back to sequential ping
    if is_fping_available() {
        scan_with_fping(&hosts).await
    } else {
        scan_with_ping(&hosts).await
    }
}

fn is_fping_available() -> bool {
    Command::new("which")
        .arg("fping")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

async fn scan_with_fping(hosts: &[Ipv4Addr]) -> Result<Vec<Ipv4Addr>> {
    let host_list: Vec<String> = hosts.iter().map(|h| h.to_string()).collect();

    let output = Command::new("fping")
        .args(["-a", "-q", "-r", "1", "-t", "100"])
        .args(&host_list)
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut reachable = Vec::new();

    for line in stdout.lines() {
        if let Ok(ip) = Ipv4Addr::from_str(line.trim()) {
            reachable.push(ip);
        }
    }

    debug!("fping found {} reachable hosts", reachable.len());
    Ok(reachable)
}

async fn scan_with_ping(hosts: &[Ipv4Addr]) -> Result<Vec<Ipv4Addr>> {
    use tokio::task::JoinSet;

    let mut tasks = JoinSet::new();

    for &host in hosts {
        tasks.spawn(async move {
            let result = tokio::process::Command::new("ping")
                .args(["-c", "1", "-W", "1", &host.to_string()])
                .output()
                .await;

            match result {
                Ok(output) if output.status.success() => Some(host),
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

    debug!("ping found {} reachable hosts", reachable.len());
    Ok(reachable)
}

/// Get hosts from ARP table for a specific interface
pub fn get_hosts_on_interface(interface: &str) -> Result<Vec<Ipv4Addr>> {
    let entries = get_arp_table()?;
    Ok(entries
        .into_iter()
        .filter(|e| e.interface == interface)
        .map(|e| e.ip)
        .collect())
}

/// Check if an IP is still reachable (lightweight check)
/// Uses ARP cache first (only REACHABLE/DELAY states), falls back to ping
pub async fn is_host_reachable(ip: Ipv4Addr) -> bool {
    // First check ARP cache (instant, no network traffic)
    // Only consider REACHABLE, DELAY, or PERMANENT as "alive"
    if let Ok(entries) = get_arp_table() {
        for entry in &entries {
            if entry.ip == ip {
                match entry.state {
                    ArpState::Reachable | ArpState::Delay | ArpState::Permanent => {
                        trace!(ip = %ip, mac = %entry.mac, state = ?entry.state, "Found reachable in ARP cache");
                        return true;
                    }
                    _ => {
                        // STALE, FAILED, etc. - need to ping to confirm
                        trace!(ip = %ip, state = ?entry.state, "ARP entry not reachable, will ping");
                    }
                }
            }
        }
    }

    // Not in ARP cache or not reachable, do a quick ping
    let result = tokio::process::Command::new("ping")
        .args(["-c", "1", "-W", "1", &ip.to_string()])
        .output()
        .await;

    match result {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

/// Batch check multiple IPs for reachability (lightweight)
/// Returns list of IPs that are still reachable
pub async fn check_hosts_reachable(hosts: &[Ipv4Addr]) -> Vec<Ipv4Addr> {
    use tokio::task::JoinSet;

    // First, get current ARP table - only consider active states
    let arp_entries = get_arp_table().unwrap_or_default();
    let active_arp_ips: std::collections::HashSet<Ipv4Addr> = arp_entries
        .into_iter()
        .filter(|e| matches!(e.state, ArpState::Reachable | ArpState::Delay | ArpState::Permanent))
        .map(|e| e.ip)
        .collect();

    let mut reachable = Vec::new();
    let mut need_ping = Vec::new();

    // Separate hosts into those actively reachable in ARP and those that need ping
    for &ip in hosts {
        if active_arp_ips.contains(&ip) {
            reachable.push(ip);
        } else {
            need_ping.push(ip);
        }
    }

    debug!(
        arp_hits = reachable.len(),
        need_ping = need_ping.len(),
        "ARP cache check (active states only)"
    );

    // For those not in ARP cache, do parallel pings
    if !need_ping.is_empty() {
        let mut tasks = JoinSet::new();

        for ip in need_ping {
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

        while let Some(result) = tasks.join_next().await {
            if let Ok(Some(ip)) = result {
                reachable.push(ip);
            }
        }
    }

    reachable
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ip_neigh_line_reachable() {
        let line = "192.168.1.100 dev eth0 lladdr aa:bb:cc:dd:ee:ff REACHABLE";
        let entry = parse_ip_neigh_line(line).unwrap();
        assert_eq!(entry.ip, Ipv4Addr::new(192, 168, 1, 100));
        assert_eq!(entry.mac, "aa:bb:cc:dd:ee:ff");
        assert_eq!(entry.interface, "eth0");
        assert_eq!(entry.state, ArpState::Reachable);
    }

    #[test]
    fn test_parse_ip_neigh_line_stale() {
        let line = "192.168.1.100 dev eth0 lladdr aa:bb:cc:dd:ee:ff STALE";
        let entry = parse_ip_neigh_line(line).unwrap();
        assert_eq!(entry.state, ArpState::Stale);
    }

    #[test]
    fn test_parse_incomplete_line() {
        let line = "192.168.1.100 dev eth0 INCOMPLETE";
        let entry = parse_ip_neigh_line(line).unwrap();
        assert_eq!(entry.ip, Ipv4Addr::new(192, 168, 1, 100));
        assert_eq!(entry.mac, ""); // No MAC for incomplete
        assert_eq!(entry.state, ArpState::Incomplete);
    }

    #[test]
    fn test_parse_too_short() {
        let line = "192.168.1.100 dev";
        assert!(parse_ip_neigh_line(line).is_none());
    }
}
