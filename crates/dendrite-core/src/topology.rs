//! Topology graph for parent/child device relationships

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::device::{Device, DeviceId};
use crate::hcdf::Hcdf;

/// A node in the topology graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyNode {
    /// Device ID
    pub id: DeviceId,
    /// Device name
    pub name: String,
    /// Board type
    pub board: Option<String>,
    /// Is this the parent/root node
    pub is_parent: bool,
    /// Physical port on parent (if applicable)
    pub port: Option<u8>,
    /// Children device IDs
    pub children: Vec<DeviceId>,
    /// Position for visualization (auto-arranged if not specified)
    pub position: Option<[f64; 3]>,
}

/// Device topology representing the parent/child network structure
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Topology {
    /// All nodes indexed by device ID
    nodes: HashMap<String, TopologyNode>,
    /// Root/parent device ID
    root: Option<DeviceId>,
}

impl Topology {
    /// Create a new empty topology
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            root: None,
        }
    }

    /// Build topology from HCDF document
    pub fn from_hcdf(hcdf: &Hcdf) -> Self {
        let mut topology = Self::new();

        // Find and add parent device
        if let Some(parent) = hcdf.find_parent() {
            let parent_id = parent
                .hwid
                .clone()
                .map(DeviceId)
                .unwrap_or_else(|| DeviceId(parent.name.clone()));

            topology.add_node(TopologyNode {
                id: parent_id.clone(),
                name: parent.name.clone(),
                board: parent.board.clone(),
                is_parent: true,
                port: None,
                children: Vec::new(),
                position: Some([0.0, 0.0, 0.0]),
            });
            topology.root = Some(parent_id);
        }

        // Add MCU devices
        for mcu in &hcdf.mcu {
            let device_id = mcu
                .hwid
                .clone()
                .map(DeviceId)
                .unwrap_or_else(|| DeviceId(mcu.name.clone()));

            let port = mcu.discovered.as_ref().and_then(|d| d.port);

            topology.add_node(TopologyNode {
                id: device_id.clone(),
                name: mcu.name.clone(),
                board: mcu.board.clone(),
                is_parent: false,
                port,
                children: Vec::new(),
                position: None,
            });

            // Link to parent
            if let Some(root_id) = topology.root.clone() {
                topology.add_child(&root_id, &device_id);
            }
        }

        topology.auto_arrange();
        topology
    }

    /// Build topology from device registry
    pub fn from_devices(devices: &[Device], parent_id: Option<&DeviceId>) -> Self {
        let mut topology = Self::new();

        // Add parent if specified
        if let Some(pid) = parent_id {
            if let Some(parent) = devices.iter().find(|d| &d.id == pid) {
                topology.add_node(TopologyNode {
                    id: parent.id.clone(),
                    name: parent.name.clone(),
                    board: parent.info.board.clone(),
                    is_parent: true,
                    port: None,
                    children: Vec::new(),
                    position: Some([0.0, 0.0, 0.0]),
                });
                topology.root = Some(parent.id.clone());
            }
        }

        // Add all other devices
        for device in devices {
            if Some(&device.id) == parent_id {
                continue; // Skip parent, already added
            }

            topology.add_node(TopologyNode {
                id: device.id.clone(),
                name: device.name.clone(),
                board: device.info.board.clone(),
                is_parent: false,
                port: device.discovery.switch_port,
                children: Vec::new(),
                position: device.pose.map(|p| [p[0], p[1], p[2]]),
            });

            // Link to parent
            if let Some(pid) = device.parent_id.clone() {
                topology.add_child(&pid, &device.id);
            } else if let Some(root_id) = topology.root.clone() {
                topology.add_child(&root_id, &device.id);
            }
        }

        topology.auto_arrange();
        topology
    }

    /// Add a node to the topology
    pub fn add_node(&mut self, node: TopologyNode) {
        self.nodes.insert(node.id.0.clone(), node);
    }

    /// Add a child relationship
    pub fn add_child(&mut self, parent_id: &DeviceId, child_id: &DeviceId) {
        if let Some(parent) = self.nodes.get_mut(&parent_id.0) {
            if !parent.children.contains(child_id) {
                parent.children.push(child_id.clone());
            }
        }
    }

    /// Remove a node from the topology
    pub fn remove_node(&mut self, id: &DeviceId) {
        self.nodes.remove(&id.0);
        // Remove from parent's children list
        for node in self.nodes.values_mut() {
            node.children.retain(|c| c != id);
        }
    }

    /// Get a node by ID
    pub fn get_node(&self, id: &DeviceId) -> Option<&TopologyNode> {
        self.nodes.get(&id.0)
    }

    /// Get all nodes
    pub fn nodes(&self) -> impl Iterator<Item = &TopologyNode> {
        self.nodes.values()
    }

    /// Get the root/parent node
    pub fn root(&self) -> Option<&TopologyNode> {
        self.root.as_ref().and_then(|id| self.nodes.get(&id.0))
    }

    /// Get children of a node
    pub fn children(&self, id: &DeviceId) -> Vec<&TopologyNode> {
        self.nodes
            .get(&id.0)
            .map(|node| {
                node.children
                    .iter()
                    .filter_map(|cid| self.nodes.get(&cid.0))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Auto-arrange nodes in a radial pattern around parent
    pub fn auto_arrange(&mut self) {
        if let Some(ref root_id) = self.root.clone() {
            let children: Vec<_> = self
                .nodes
                .get(&root_id.0)
                .map(|n| n.children.clone())
                .unwrap_or_default();

            let count = children.len();
            if count == 0 {
                return;
            }

            let radius = 0.15; // 15cm radius around parent
            for (i, child_id) in children.iter().enumerate() {
                if let Some(node) = self.nodes.get_mut(&child_id.0) {
                    if node.position.is_none() {
                        // Arrange in a circle
                        let angle = (i as f64) * 2.0 * std::f64::consts::PI / (count as f64);
                        node.position = Some([
                            radius * angle.cos(),
                            radius * angle.sin(),
                            0.0,
                        ]);
                    }
                }
            }
        }
    }

    /// Get topology as JSON-serializable structure
    pub fn to_graph(&self) -> TopologyGraph {
        TopologyGraph {
            nodes: self.nodes.values().cloned().collect(),
            root: self.root.clone(),
        }
    }
}

/// Serializable topology graph for API responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyGraph {
    pub nodes: Vec<TopologyNode>,
    pub root: Option<DeviceId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{Device, DiscoveryInfo, DiscoveryMethod};
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn test_topology_from_devices() {
        let parent_id = DeviceId::from_hwid("parent-001");
        let child_id = DeviceId::from_hwid("child-001");

        let now = chrono::Utc::now();
        let parent = Device {
            id: parent_id.clone(),
            name: "navq95".to_string(),
            status: crate::device::DeviceStatus::Online,
            discovery: DiscoveryInfo {
                ip: IpAddr::V4(Ipv4Addr::new(192, 168, 186, 1)),
                port: 1337,
                switch_port: None,
                mac: None,
                first_seen: now,
                last_seen: now,
                discovery_method: DiscoveryMethod::Manual,
            },
            info: Default::default(),
            firmware: Default::default(),
            parent_id: None,
            model_path: None,
            pose: None,
            visuals: Vec::new(),
            frames: Vec::new(),
        };

        let mut child = Device::new(
            child_id.clone(),
            "spinali-001".to_string(),
            IpAddr::V4(Ipv4Addr::new(192, 168, 186, 10)),
            1337,
        );
        child.parent_id = Some(parent_id.clone());
        child.discovery.switch_port = Some(2);

        let devices = vec![parent, child];
        let topology = Topology::from_devices(&devices, Some(&parent_id));

        assert!(topology.root().is_some());
        assert_eq!(topology.root().unwrap().name, "navq95");

        let children = topology.children(&parent_id);
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "spinali-001");
        assert_eq!(children[0].port, Some(2));
    }
}
