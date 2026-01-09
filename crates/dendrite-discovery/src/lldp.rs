//! LLDP (Link Layer Discovery Protocol) parsing for physical port detection
//!
//! LLDP allows discovery of which physical switch port a device is connected to.

use anyhow::Result;
use std::collections::HashMap;
use std::process::Command;
use tracing::{debug, info, warn};

/// LLDP neighbor information
#[derive(Debug, Clone)]
pub struct LldpNeighbor {
    /// Local interface name
    pub local_interface: String,
    /// Chassis ID of the neighbor
    pub chassis_id: String,
    /// Port ID of the neighbor
    pub port_id: String,
    /// Port description
    pub port_desc: Option<String>,
    /// System name of the neighbor
    pub system_name: Option<String>,
    /// System description
    pub system_desc: Option<String>,
    /// Management addresses
    pub mgmt_addresses: Vec<String>,
}

/// Check if lldpd is running
pub fn is_lldpd_available() -> bool {
    Command::new("lldpcli")
        .arg("show")
        .arg("neighbors")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Get LLDP neighbors from lldpd
pub fn get_lldp_neighbors() -> Result<Vec<LldpNeighbor>> {
    if !is_lldpd_available() {
        debug!("lldpd not available, LLDP discovery disabled");
        return Ok(Vec::new());
    }

    let output = Command::new("lldpcli")
        .args(["show", "neighbors", "-f", "keyvalue"])
        .output()?;

    if !output.status.success() {
        anyhow::bail!(
            "lldpcli failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_lldpcli_keyvalue(&stdout)
}

/// Parse lldpcli keyvalue output format
fn parse_lldpcli_keyvalue(output: &str) -> Result<Vec<LldpNeighbor>> {
    let mut neighbors = Vec::new();
    let mut current: HashMap<String, String> = HashMap::new();
    let mut current_interface = String::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Format: lldp.eth0.chassis.id=...
        if let Some((key, value)) = line.split_once('=') {
            let parts: Vec<&str> = key.split('.').collect();
            if parts.len() >= 3 && parts[0] == "lldp" {
                let interface = parts[1];

                // New interface, save previous neighbor
                if interface != current_interface && !current_interface.is_empty() {
                    if let Some(neighbor) = build_neighbor(&current_interface, &current) {
                        neighbors.push(neighbor);
                    }
                    current.clear();
                }

                current_interface = interface.to_string();

                // Store the rest of the key (e.g., "chassis.id")
                let rest_key = parts[2..].join(".");
                current.insert(rest_key, value.to_string());
            }
        }
    }

    // Don't forget the last neighbor
    if !current_interface.is_empty() {
        if let Some(neighbor) = build_neighbor(&current_interface, &current) {
            neighbors.push(neighbor);
        }
    }

    debug!("Found {} LLDP neighbors", neighbors.len());
    Ok(neighbors)
}

fn build_neighbor(interface: &str, data: &HashMap<String, String>) -> Option<LldpNeighbor> {
    let chassis_id = data.get("chassis.id")?.clone();
    let port_id = data.get("port.id")?.clone();

    Some(LldpNeighbor {
        local_interface: interface.to_string(),
        chassis_id,
        port_id,
        port_desc: data.get("port.descr").cloned(),
        system_name: data.get("chassis.name").cloned(),
        system_desc: data.get("chassis.descr").cloned(),
        mgmt_addresses: data
            .iter()
            .filter(|(k, _)| k.starts_with("chassis.mgmt-ip"))
            .map(|(_, v)| v.clone())
            .collect(),
    })
}

/// Extract port number from port ID (if numeric)
pub fn parse_port_number(port_id: &str) -> Option<u8> {
    // Port ID might be "1", "port1", "eth1", "swp1", etc.
    let digits: String = port_id.chars().filter(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Map MAC address to switch port using LLDP
pub fn find_port_for_mac(neighbors: &[LldpNeighbor], mac: &str) -> Option<u8> {
    // This would require the neighbor to advertise its MAC in chassis ID
    // Common format: chassis ID is MAC address
    let mac_normalized = mac.to_lowercase().replace(':', "").replace('-', "");

    for neighbor in neighbors {
        let chassis_normalized = neighbor
            .chassis_id
            .to_lowercase()
            .replace(':', "")
            .replace('-', "");

        if chassis_normalized == mac_normalized {
            return parse_port_number(&neighbor.port_id);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_port_number() {
        assert_eq!(parse_port_number("1"), Some(1));
        assert_eq!(parse_port_number("port2"), Some(2));
        assert_eq!(parse_port_number("eth3"), Some(3));
        assert_eq!(parse_port_number("swp12"), Some(12));
        assert_eq!(parse_port_number("no-digits"), None);
    }

    #[test]
    fn test_parse_lldpcli_keyvalue() {
        let output = r#"lldp.eth0.chassis.id=aa:bb:cc:dd:ee:ff
lldp.eth0.port.id=1
lldp.eth0.port.descr=Port 1
lldp.eth0.chassis.name=switch1
lldp.eth1.chassis.id=11:22:33:44:55:66
lldp.eth1.port.id=2
"#;

        let neighbors = parse_lldpcli_keyvalue(output).unwrap();
        assert_eq!(neighbors.len(), 2);
        assert_eq!(neighbors[0].local_interface, "eth0");
        assert_eq!(neighbors[0].port_id, "1");
        assert_eq!(neighbors[1].local_interface, "eth1");
        assert_eq!(neighbors[1].port_id, "2");
    }
}
