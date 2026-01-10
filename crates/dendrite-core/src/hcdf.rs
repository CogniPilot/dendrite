//! HCDF (Hardware Configuration Descriptive Format) parsing and serialization
//!
//! HCDF is an XML-based format for describing hardware configurations,
//! extending URDF concepts for CogniPilot systems.

use quick_xml::de::from_str;
use quick_xml::se::to_string;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

use crate::device::Device;

#[derive(Error, Debug)]
pub enum HcdfError {
    #[error("Failed to parse HCDF: {0}")]
    ParseError(String),
    #[error("Failed to serialize HCDF: {0}")]
    SerializeError(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Invalid HCDF structure: {0}")]
    ValidationError(String),
}

/// Pose in 3D space (x, y, z, roll, pitch, yaw)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Pose {
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub z: f64,
    #[serde(default)]
    pub roll: f64,
    #[serde(default)]
    pub pitch: f64,
    #[serde(default)]
    pub yaw: f64,
}

impl Pose {
    pub fn from_array(arr: [f64; 6]) -> Self {
        Self {
            x: arr[0],
            y: arr[1],
            z: arr[2],
            roll: arr[3],
            pitch: arr[4],
            yaw: arr[5],
        }
    }

    pub fn to_array(&self) -> [f64; 6] {
        [self.x, self.y, self.z, self.roll, self.pitch, self.yaw]
    }
}

/// Software running on a device
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Software {
    #[serde(rename = "@name", default)]
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub hash: Option<String>,
    #[serde(default)]
    pub params: Option<String>,
}

/// Discovery information embedded in HCDF
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discovered {
    pub ip: String,
    #[serde(default)]
    pub port: Option<u8>,
    #[serde(default)]
    pub last_seen: Option<String>,
}

/// Network interface configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterface {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@type", default)]
    pub interface_type: Option<String>,
    #[serde(rename = "@ports", default)]
    pub ports: Option<u8>,
    #[serde(default)]
    pub switch: Option<SwitchInfo>,
}

/// Network switch information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchInfo {
    #[serde(rename = "@chip", default)]
    pub chip: Option<String>,
}

/// Network configuration for a device
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Network {
    #[serde(default)]
    pub interface: Vec<NetworkInterface>,
}

/// MCU (Microcontroller) element in HCDF
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mcu {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@hwid", default)]
    pub hwid: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub pose_cg: Option<String>,
    #[serde(default)]
    pub mass: Option<f64>,
    #[serde(default)]
    pub board: Option<String>,
    #[serde(default)]
    pub software: Option<Software>,
    #[serde(default)]
    pub discovered: Option<Discovered>,
    /// Legacy single model reference (deprecated, use visuals instead)
    #[serde(default)]
    pub model: Option<ModelRef>,
    /// Multiple visual elements with individual poses
    #[serde(default)]
    pub visual: Vec<Visual>,
    /// Reference frames for this component
    #[serde(default)]
    pub frame: Vec<Frame>,
    #[serde(default)]
    pub network: Option<Network>,
}

/// Companion computer element in HCDF
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comp {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@role", default)]
    pub role: Option<String>,
    #[serde(rename = "@hwid", default)]
    pub hwid: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub pose_cg: Option<String>,
    #[serde(default)]
    pub mass: Option<f64>,
    #[serde(default)]
    pub board: Option<String>,
    #[serde(default)]
    pub software: Option<Software>,
    #[serde(default)]
    pub discovered: Option<Discovered>,
    /// Legacy single model reference (deprecated, use visuals instead)
    #[serde(default)]
    pub model: Option<ModelRef>,
    /// Multiple visual elements with individual poses
    #[serde(default)]
    pub visual: Vec<Visual>,
    /// Reference frames for this component
    #[serde(default)]
    pub frame: Vec<Frame>,
    #[serde(default)]
    pub network: Option<Network>,
}

/// Reference to a 3D model file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRef {
    #[serde(rename = "@href")]
    pub href: String,
    /// SHA256 hash of the model file for cache validation
    #[serde(rename = "@sha", default)]
    pub sha: Option<String>,
}

/// Visual element - a 3D model with a pose offset
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Visual {
    #[serde(rename = "@name")]
    pub name: String,
    /// Toggle group name for visibility control (e.g., "case")
    #[serde(rename = "@toggle", default)]
    pub toggle: Option<String>,
    /// Pose offset: "x y z roll pitch yaw" (meters, radians)
    #[serde(default)]
    pub pose: Option<String>,
    /// Reference to 3D model
    #[serde(default)]
    pub model: Option<ModelRef>,
}

impl Visual {
    /// Parse the pose string into a Pose struct
    pub fn parse_pose(&self) -> Option<Pose> {
        self.pose.as_ref().and_then(|s| parse_pose_string(s))
    }
}

/// Reference frame - a named coordinate frame with description
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    #[serde(rename = "@name")]
    pub name: String,
    /// Human-readable description of this frame
    #[serde(default)]
    pub description: Option<String>,
    /// Pose offset: "x y z roll pitch yaw" (meters, radians)
    #[serde(default)]
    pub pose: Option<String>,
}

impl Frame {
    /// Parse the pose string into a Pose struct
    pub fn parse_pose(&self) -> Option<Pose> {
        self.pose.as_ref().and_then(|s| parse_pose_string(s))
    }
}

/// Parse a pose string "x y z roll pitch yaw" into a Pose struct
pub fn parse_pose_string(s: &str) -> Option<Pose> {
    let parts: Vec<f64> = s.split_whitespace()
        .filter_map(|p| p.parse().ok())
        .collect();
    if parts.len() == 6 {
        Some(Pose {
            x: parts[0],
            y: parts[1],
            z: parts[2],
            roll: parts[3],
            pitch: parts[4],
            yaw: parts[5],
        })
    } else {
        None
    }
}

/// Wired connection details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wired {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
}

/// Wireless connection details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wireless {
    #[serde(rename = "@name")]
    pub name: String,
}

/// Digital link (wired or wireless)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Digital {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(default)]
    pub wired: Option<Wired>,
    #[serde(default)]
    pub wireless: Option<Wireless>,
}

/// Physical joint types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Physical {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(default)]
    pub fixed: Option<NamedElement>,
    #[serde(default)]
    pub rotational: Option<NamedElement>,
    #[serde(default)]
    pub translational: Option<NamedElement>,
}

/// Generic named element
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedElement {
    #[serde(rename = "@name")]
    pub name: String,
}

/// Link between components (digital or physical)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(default)]
    pub digital: Option<Digital>,
    #[serde(default)]
    pub physical: Option<Physical>,
}

/// Sensor element
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sensor {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(default)]
    pub pose_cg: Option<String>,
    // Sensor types (optical, inertial, rf, chemical, force)
    // Simplified for initial implementation
}

/// Motor/actuator element
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Motor {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(default)]
    pub pose_cg: Option<String>,
    // Motor types (servo, brushless, brushed, etc.)
    // Simplified for initial implementation
}

/// Power source element
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Power {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(default)]
    pub battery: Option<NamedElement>,
    #[serde(default)]
    pub tank: Option<NamedElement>,
}

/// Root HCDF document
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename = "hcdf")]
pub struct Hcdf {
    #[serde(rename = "@version")]
    pub version: String,

    #[serde(default)]
    pub mcu: Vec<Mcu>,

    #[serde(default)]
    pub comp: Vec<Comp>,

    #[serde(default)]
    pub link: Vec<Link>,

    #[serde(default)]
    pub sensor: Vec<Sensor>,

    #[serde(default)]
    pub motor: Vec<Motor>,

    #[serde(default)]
    pub power: Vec<Power>,
}

impl Hcdf {
    /// Create a new empty HCDF document
    pub fn new() -> Self {
        Self {
            version: "1.2".to_string(),
            mcu: Vec::new(),
            comp: Vec::new(),
            link: Vec::new(),
            sensor: Vec::new(),
            motor: Vec::new(),
            power: Vec::new(),
        }
    }

    /// Parse HCDF from XML string
    pub fn from_xml(xml: &str) -> Result<Self, HcdfError> {
        from_str(xml).map_err(|e| HcdfError::ParseError(e.to_string()))
    }

    /// Parse HCDF from file
    pub fn from_file(path: &Path) -> Result<Self, HcdfError> {
        let content = std::fs::read_to_string(path)?;
        Self::from_xml(&content)
    }

    /// Serialize to XML string
    pub fn to_xml(&self) -> Result<String, HcdfError> {
        let xml = to_string(self).map_err(|e| HcdfError::SerializeError(e.to_string()))?;
        Ok(format!("<?xml version='1.0'?>\n{}", xml))
    }

    /// Write to file
    pub fn to_file(&self, path: &Path) -> Result<(), HcdfError> {
        let xml = self.to_xml()?;
        std::fs::write(path, xml)?;
        Ok(())
    }

    /// Find parent device (comp with role="parent")
    pub fn find_parent(&self) -> Option<&Comp> {
        self.comp.iter().find(|c| c.role.as_deref() == Some("parent"))
    }

    /// Get all MCUs as a map by hwid
    pub fn mcus_by_hwid(&self) -> HashMap<String, &Mcu> {
        self.mcu
            .iter()
            .filter_map(|m| m.hwid.as_ref().map(|h| (h.clone(), m)))
            .collect()
    }

    /// Add or update an MCU from a discovered device
    pub fn upsert_device(&mut self, device: &Device, parent_name: Option<&str>) {
        let hwid = device.id.as_str().to_string();

        // Check if MCU already exists
        if let Some(mcu) = self.mcu.iter_mut().find(|m| m.hwid.as_deref() == Some(&hwid)) {
            // Update existing
            mcu.board = device.info.board.clone();
            if let Some(ref name) = device.firmware.name {
                let sw = mcu.software.get_or_insert(Software::default());
                sw.name = name.clone();
                sw.version = device.firmware.version.clone();
                sw.hash = device.firmware.hash.clone();
            }
            mcu.discovered = Some(Discovered {
                ip: device.discovery.ip.to_string(),
                port: device.discovery.switch_port,
                last_seen: Some(device.discovery.last_seen.to_rfc3339()),
            });
        } else {
            // Create new MCU
            let mcu = Mcu {
                name: device.name.clone(),
                hwid: Some(hwid),
                description: None,
                pose_cg: device.pose.map(|p| {
                    format!("{} {} {} {} {} {}", p[0], p[1], p[2], p[3], p[4], p[5])
                }),
                mass: None,
                board: device.info.board.clone(),
                software: device.firmware.name.as_ref().map(|name| Software {
                    name: name.clone(),
                    version: device.firmware.version.clone(),
                    hash: device.firmware.hash.clone(),
                    params: None,
                }),
                discovered: Some(Discovered {
                    ip: device.discovery.ip.to_string(),
                    port: device.discovery.switch_port,
                    last_seen: Some(device.discovery.last_seen.to_rfc3339()),
                }),
                model: device.model_path.as_ref().map(|p| ModelRef { href: p.clone(), sha: None }),
                visual: Vec::new(),
                frame: Vec::new(),
                network: None,
            };
            self.mcu.push(mcu);

            // Add link to parent if specified
            if let (Some(parent), Some(port)) = (parent_name, device.discovery.switch_port) {
                let link = Link {
                    name: format!("{}_to_{}", parent, device.name),
                    digital: Some(Digital {
                        name: "t1_eth".to_string(),
                        wired: Some(Wired {
                            name: "100base-t1".to_string(),
                            from: Some(format!("{}/eth0:{}", parent, port)),
                            to: Some(format!("{}/eth0", device.name)),
                        }),
                        wireless: None,
                    }),
                    physical: None,
                };
                self.link.push(link);
            }
        }
    }

    /// Remove stale devices (not seen within timeout)
    pub fn remove_stale_devices(&mut self, timeout_secs: i64) {
        let now = chrono::Utc::now();
        self.mcu.retain(|mcu| {
            if let Some(ref disc) = mcu.discovered {
                if let Some(ref last_seen) = disc.last_seen {
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(last_seen) {
                        let elapsed = now - dt.with_timezone(&chrono::Utc);
                        return elapsed.num_seconds() <= timeout_secs;
                    }
                }
            }
            true // Keep devices without discovery info
        });
    }
}

impl Default for Hcdf {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_hcdf() {
        let xml = r#"<?xml version='1.0'?>
<hcdf version="1.2">
    <mcu name="spinali-001" hwid="0x12345678">
        <board>spinali</board>
    </mcu>
</hcdf>"#;

        let hcdf = Hcdf::from_xml(xml).unwrap();
        assert_eq!(hcdf.version, "1.2");
        assert_eq!(hcdf.mcu.len(), 1);
        assert_eq!(hcdf.mcu[0].name, "spinali-001");
        assert_eq!(hcdf.mcu[0].hwid, Some("0x12345678".to_string()));
    }

    #[test]
    fn test_serialize_hcdf() {
        let mut hcdf = Hcdf::new();
        hcdf.mcu.push(Mcu {
            name: "test-mcu".to_string(),
            hwid: Some("0xaabbccdd".to_string()),
            description: None,
            pose_cg: None,
            mass: None,
            board: Some("test-board".to_string()),
            software: None,
            discovered: None,
            model: None,
            visual: Vec::new(),
            frame: Vec::new(),
            network: None,
        });

        let xml = hcdf.to_xml().unwrap();
        assert!(xml.contains("test-mcu"));
        assert!(xml.contains("0xaabbccdd"));
    }

    #[test]
    fn test_parse_visual_and_frame() {
        let xml = r#"<?xml version='1.0'?>
<hcdf version="1.2">
    <comp name="sensor-assembly" role="sensor">
        <description>Test sensor</description>
        <visual name="board">
            <pose>0 0 0 0 0 0</pose>
            <model href="models/board.glb" sha="abc123"/>
        </visual>
        <visual name="sensor">
            <pose>0 0 -0.005 3.14159 0 0</pose>
            <model href="models/sensor.glb"/>
        </visual>
        <frame name="sensor_frame">
            <description>Sensor reference frame</description>
            <pose>0 0 -0.005 3.14159 0 0</pose>
        </frame>
    </comp>
</hcdf>"#;

        let hcdf = Hcdf::from_xml(xml).unwrap();
        assert_eq!(hcdf.comp.len(), 1);

        let comp = &hcdf.comp[0];
        assert_eq!(comp.name, "sensor-assembly");
        assert_eq!(comp.description, Some("Test sensor".to_string()));

        // Check visuals
        assert_eq!(comp.visual.len(), 2);
        assert_eq!(comp.visual[0].name, "board");
        assert_eq!(comp.visual[1].name, "sensor");
        assert_eq!(comp.visual[0].model.as_ref().unwrap().sha, Some("abc123".to_string()));
        assert_eq!(comp.visual[1].model.as_ref().unwrap().sha, None);

        // Check frames
        assert_eq!(comp.frame.len(), 1);
        assert_eq!(comp.frame[0].name, "sensor_frame");
        assert_eq!(comp.frame[0].description, Some("Sensor reference frame".to_string()));

        // Check pose parsing
        let pose = comp.visual[1].parse_pose().unwrap();
        assert!((pose.z - (-0.005)).abs() < 0.0001);
        assert!((pose.roll - 3.14159).abs() < 0.0001);
    }
}
