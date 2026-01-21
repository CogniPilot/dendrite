//! HCDF (Hardware Configuration Descriptive Format) parsing and serialization
//!
//! HCDF is an XML-based format for describing hardware configurations,
//! extending URDF concepts for CogniPilot systems.
//!
//! # Port Schema
//!
//! Ports represent physical connection interfaces on components. There are two
//! ways to visualize ports:
//!
//! ## Mode 1: Mesh Reference
//! Reference an existing mesh node within a visual's GLTF model:
//! ```xml
//! <port name="ETH0" type="ethernet" visual="board" mesh="ETH0">
//!   <capabilities>
//!     <speed unit="Mbps">1000</speed>
//!   </capabilities>
//! </port>
//! ```
//!
//! ## Mode 2: Fallback Visual
//! When no mesh is available, define inline geometry with pose:
//! ```xml
//! <port name="CAN0" type="CAN">
//!   <capabilities>
//!     <bitrate unit="bps">500000</bitrate>
//!     <protocol>CAN-FD</protocol>
//!   </capabilities>
//!   <fallback_visual>
//!     <pose>-0.0225 -0.0155 -0.0085 0 0 0</pose>
//!     <geometry>
//!       <box><size>0.005 0.004 0.003</size></box>
//!     </geometry>
//!   </fallback_visual>
//! </port>
//! ```
//!
//! The `<fallback_visual>` element follows URDF/SDF conventions with pose and
//! geometry as siblings. The `<capabilities>` element contains type-specific
//! properties like speed (ethernet), bitrate (CAN), baud (serial), and protocol.
//!
//! ## Port Power Capabilities
//!
//! Power capabilities can be added to any port type (POWER, Ethernet with PoE/PoDL,
//! USB with PD, CAN with power pins, etc.):
//!
//! ```xml
//! <port name="pwr_in" type="POWER" visual="main_board" mesh="pwr">
//!   <capabilities>
//!     <voltage unit="V" min="7" max="28">12</voltage>
//!     <current unit="A" max="3"/>
//!     <power unit="W" max="36"/>
//!     <capacity unit="Wh">55.5</capacity>  <!-- for batteries -->
//!     <connector>XT30</connector>
//!   </capabilities>
//! </port>
//! ```
//!
//! Data ports with power delivery (e.g., Ethernet with PoDL):
//! ```xml
//! <port name="eth_podl" type="ethernet" visual="board" mesh="eth">
//!   <capabilities>
//!     <speed unit="Mbps">1000</speed>
//!     <standard>1000BASE-T1</standard>
//!     <protocol>PoDL</protocol>
//!     <voltage unit="V" min="12" max="48">24</voltage>
//!     <power unit="W" max="50"/>
//!   </capabilities>
//! </port>
//! ```
//!
//! # Antenna Schema
//!
//! Antennas represent wireless interfaces and follow the same pattern as ports.
//! Capabilities are organized into:
//! - `band`: RF frequency bands (e.g., "2.4 GHz", "5 GHz", "L1", "L2")
//! - `standard`: PHY/MAC layer specs (e.g., "802.11ax", "802.15.4", "Bluetooth 5.4")
//! - `protocol`: Higher-layer protocols (e.g., "Thread", "6LoWPAN", "Matter")
//!
//! ## Mode 1: Mesh Reference
//! ```xml
//! <antenna name="GNSS0" type="gnss" visual="board" mesh="GNSS_ANT">
//!   <capabilities>
//!     <band>L1</band>
//!     <band>L2</band>
//!     <band>L5</band>
//!     <gain unit="dBi">3.5</gain>
//!     <polarization>RHCP</polarization>
//!   </capabilities>
//! </antenna>
//! ```
//!
//! ## Tri-Radio Example (Wi-Fi 6 / Bluetooth 5.4 / 802.15.4)
//! ```xml
//! <antenna name="mlan0" type="wifi" visual="board" mesh="ant0">
//!   <capabilities>
//!     <band>2.4 GHz</band>
//!     <band>5 GHz</band>
//!     <gain unit="dBi">2.0</gain>
//!     <standard>802.11ax</standard>
//!     <protocol>WPA3</protocol>
//!   </capabilities>
//! </antenna>
//!
//! <antenna name="wpan0" type="802.15.4" visual="board" mesh="ant1">
//!   <capabilities>
//!     <band>2.4 GHz</band>
//!     <gain unit="dBi">2.0</gain>
//!     <standard>Bluetooth 5.4</standard>
//!     <standard>802.15.4</standard>
//!     <protocol>Thread</protocol>
//!     <protocol>6LoWPAN</protocol>
//!     <protocol>Matter</protocol>
//!   </capabilities>
//! </antenna>
//! ```
//!
//! ## Mode 2: Fallback Visual
//! ```xml
//! <antenna name="WIFI0" type="wifi">
//!   <capabilities>
//!     <band>2.4 GHz</band>
//!     <standard>802.11n</standard>
//!   </capabilities>
//!   <fallback_visual>
//!     <pose>0.01 0.02 0.005 0 0 0</pose>
//!     <geometry>
//!       <cylinder><radius>0.002</radius><length>0.015</length></cylinder>
//!     </geometry>
//!   </fallback_visual>
//! </antenna>
//! ```

use quick_xml::de::from_str;
use quick_xml::se::Serializer;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// URI for firmware manifest (e.g., "https://firmware.cognipilot.org/mr_mcxn_t1/optical-flow")
    /// The daemon appends "/latest.json" to fetch the manifest.
    /// Required for firmware update checking - no default fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware_manifest_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<String>,
}

/// Discovery information embedded in HCDF
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discovered {
    pub ip: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<String>,
}

/// Network interface configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterface {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@type", default, skip_serializing_if = "Option::is_none")]
    pub interface_type: Option<String>,
    #[serde(rename = "@ports", default, skip_serializing_if = "Option::is_none")]
    pub ports: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub switch: Option<SwitchInfo>,
}

/// Network switch information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchInfo {
    #[serde(rename = "@chip", default, skip_serializing_if = "Option::is_none")]
    pub chip: Option<String>,
}

/// Network configuration for a device
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Network {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interface: Vec<NetworkInterface>,
}

/// MCU (Microcontroller) element in HCDF
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mcu {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@hwid", default, skip_serializing_if = "Option::is_none")]
    pub hwid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pose_cg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mass: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub software: Option<Software>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovered: Option<Discovered>,
    /// Legacy single model reference (deprecated, use visuals instead)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelRef>,
    /// Multiple visual elements with individual poses
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub visual: Vec<Visual>,
    /// Reference frames for this component
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frame: Vec<Frame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<Network>,
}

/// Child element types that can be interleaved in a Comp/Mcu
/// Using $value enum pattern to handle non-consecutive XML elements
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CompChild {
    Description(String),
    #[serde(rename = "pose_cg")]
    PoseCg(String),
    Mass(f64),
    Board(String),
    Software(Software),
    Discovered(Discovered),
    Model(ModelRef),
    Visual(Visual),
    Frame(Frame),
    Network(Network),
    Port(Port),
    Antenna(Antenna),
    Sensor(Sensor),
}

/// Internal struct for deserializing Comp with interleaved children
#[derive(Debug, Clone, Deserialize)]
struct CompRaw {
    #[serde(rename = "@name")]
    name: String,
    #[serde(rename = "@role", default)]
    role: Option<String>,
    #[serde(rename = "@hwid", default)]
    hwid: Option<String>,
    #[serde(rename = "$value", default)]
    children: Vec<CompChild>,
}

impl From<CompRaw> for Comp {
    fn from(raw: CompRaw) -> Self {
        let mut comp = Comp {
            name: raw.name,
            role: raw.role,
            hwid: raw.hwid,
            description: None,
            pose_cg: None,
            mass: None,
            board: None,
            software: None,
            discovered: None,
            model: None,
            visual: Vec::new(),
            frame: Vec::new(),
            network: None,
            port: Vec::new(),
            antenna: Vec::new(),
            sensor: Vec::new(),
        };

        for child in raw.children {
            match child {
                CompChild::Description(v) => comp.description = Some(v),
                CompChild::PoseCg(v) => comp.pose_cg = Some(v),
                CompChild::Mass(v) => comp.mass = Some(v),
                CompChild::Board(v) => comp.board = Some(v),
                CompChild::Software(v) => comp.software = Some(v),
                CompChild::Discovered(v) => comp.discovered = Some(v),
                CompChild::Model(v) => comp.model = Some(v),
                CompChild::Visual(v) => comp.visual.push(v),
                CompChild::Frame(v) => comp.frame.push(v),
                CompChild::Network(v) => comp.network = Some(v),
                CompChild::Port(v) => comp.port.push(v),
                CompChild::Antenna(v) => comp.antenna.push(v),
                CompChild::Sensor(v) => comp.sensor.push(v),
            }
        }

        comp
    }
}

/// Companion computer element in HCDF
#[derive(Debug, Clone, Serialize)]
pub struct Comp {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@role", default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(rename = "@hwid", default, skip_serializing_if = "Option::is_none")]
    pub hwid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pose_cg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mass: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub software: Option<Software>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovered: Option<Discovered>,
    /// Legacy single model reference (deprecated, use visuals instead)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelRef>,
    /// Multiple visual elements with individual poses
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub visual: Vec<Visual>,
    /// Reference frames for this component
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frame: Vec<Frame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<Network>,
    /// Ports (wired connection interfaces)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub port: Vec<Port>,
    /// Antennas (wireless connection interfaces)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub antenna: Vec<Antenna>,
    /// Sensors
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sensor: Vec<Sensor>,
}

impl<'de> Deserialize<'de> for Comp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = CompRaw::deserialize(deserializer)?;
        Ok(raw.into())
    }
}

/// Reference to a 3D model file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRef {
    #[serde(rename = "@href")]
    pub href: String,
    /// SHA256 hash of the model file for cache validation
    #[serde(rename = "@sha", default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
}

/// Visual element - a 3D model with a pose offset
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Visual {
    #[serde(rename = "@name")]
    pub name: String,
    /// Toggle group name for visibility control (e.g., "case")
    #[serde(rename = "@toggle", default, skip_serializing_if = "Option::is_none")]
    pub toggle: Option<String>,
    /// Pose offset: "x y z roll pitch yaw" (meters, radians)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pose: Option<String>,
    /// Reference to 3D model
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Pose offset: "x y z roll pitch yaw" (meters, radians)
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

// ============ GEOMETRY PRIMITIVES ============

/// Box geometry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxGeometry {
    /// Size as "x y z" in meters
    pub size: String,
}

impl BoxGeometry {
    /// Parse size string into [x, y, z]
    pub fn parse_size(&self) -> Option<[f64; 3]> {
        let parts: Vec<f64> = self.size.split_whitespace()
            .filter_map(|p| p.parse().ok())
            .collect();
        if parts.len() == 3 {
            Some([parts[0], parts[1], parts[2]])
        } else {
            None
        }
    }
}

/// Cylinder geometry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CylinderGeometry {
    pub radius: f64,
    pub length: f64,
}

/// Sphere geometry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SphereGeometry {
    pub radius: f64,
}

/// Cone geometry (circular FOV) - deprecated, use conical_frustum instead
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConeGeometry {
    /// Base radius at max range
    pub radius: f64,
    /// Sensing distance
    pub length: f64,
}

/// Conical frustum geometry (circular cross-section FOV)
/// Used for emitters, optical flow sensors, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConicalFrustumGeometry {
    /// Near plane distance (meters)
    pub near: f64,
    /// Far plane distance (meters)
    pub far: f64,
    /// Field of view angle (radians) - half angle from center
    pub fov: f64,
}

/// Pyramidal frustum geometry (rectangular cross-section FOV)
/// Used for cameras, ToF sensors with rectangular arrays, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PyramidalFrustumGeometry {
    /// Near plane distance (meters)
    pub near: f64,
    /// Far plane distance (meters)
    pub far: f64,
    /// Horizontal FOV in radians
    pub hfov: f64,
    /// Vertical FOV in radians
    pub vfov: f64,
}

/// Frustum geometry (rectangular FOV) - deprecated, use pyramidal_frustum instead
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrustumGeometry {
    /// Near plane distance
    pub near: f64,
    /// Far plane distance
    pub far: f64,
    /// Horizontal FOV in radians
    pub hfov: f64,
    /// Vertical FOV in radians
    pub vfov: f64,
}

/// Geometry element (can contain one of the primitives)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Geometry {
    #[serde(default)]
    pub box_: Option<BoxGeometry>,
    #[serde(rename = "box", default)]
    pub box_geom: Option<BoxGeometry>,
    #[serde(default)]
    pub cylinder: Option<CylinderGeometry>,
    #[serde(default)]
    pub sphere: Option<SphereGeometry>,
    /// Deprecated: use conical_frustum instead
    #[serde(default)]
    pub cone: Option<ConeGeometry>,
    /// Deprecated: use pyramidal_frustum instead
    #[serde(default)]
    pub frustum: Option<FrustumGeometry>,
    /// Conical frustum (circular cross-section)
    #[serde(default)]
    pub conical_frustum: Option<ConicalFrustumGeometry>,
    /// Pyramidal frustum (rectangular cross-section)
    #[serde(default)]
    pub pyramidal_frustum: Option<PyramidalFrustumGeometry>,
}

impl Geometry {
    /// Get the box geometry (handles both field names)
    pub fn get_box(&self) -> Option<&BoxGeometry> {
        self.box_geom.as_ref().or(self.box_.as_ref())
    }
}

// ============ PORTS ============

/// Value with optional unit attribute
/// Used for capability values like speed, bitrate, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueWithUnit {
    #[serde(rename = "@unit", default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(rename = "$value")]
    pub value: String,
}

impl ValueWithUnit {
    /// Parse the value as f64
    pub fn parse_value(&self) -> Option<f64> {
        self.value.parse().ok()
    }

    /// Parse the value as u64
    pub fn parse_value_u64(&self) -> Option<u64> {
        self.value.parse().ok()
    }
}

/// Voltage capability with range (min/max) and nominal value
/// Example: `<voltage unit="V" min="7" max="28">12</voltage>`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoltageCapability {
    #[serde(rename = "@unit", default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(rename = "@min", default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(rename = "@max", default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Nominal voltage value
    #[serde(rename = "$value", default)]
    pub value: Option<String>,
}

impl VoltageCapability {
    /// Format as display string (e.g., "12V (7-28V)")
    pub fn to_display_string(&self) -> String {
        let unit = self.unit.as_deref().unwrap_or("V");
        let nominal = self.value.as_deref().unwrap_or("");
        match (self.min, self.max, nominal.is_empty()) {
            (Some(min), Some(max), false) => format!("{}{} ({}-{}{})", nominal, unit, min, max, unit),
            (Some(min), Some(max), true) => format!("{}-{}{}", min, max, unit),
            (None, Some(max), false) => format!("{}{} (max {}{})", nominal, unit, max, unit),
            (Some(min), None, false) => format!("{}{} (min {}{})", nominal, unit, min, unit),
            _ if !nominal.is_empty() => format!("{}{}", nominal, unit),
            _ => String::new(),
        }
    }
}

/// Current capability with max value
/// Example: `<current unit="A" max="3"/>`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentCapability {
    #[serde(rename = "@unit", default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(rename = "@max", default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Optional nominal value
    #[serde(rename = "$value", default)]
    pub value: Option<String>,
}

impl CurrentCapability {
    /// Format as display string (e.g., "3A max")
    pub fn to_display_string(&self) -> String {
        let unit = self.unit.as_deref().unwrap_or("A");
        match (&self.value, self.max) {
            (Some(v), Some(max)) if !v.is_empty() => format!("{}{} (max {}{})", v, unit, max, unit),
            (_, Some(max)) => format!("{}{} max", max, unit),
            (Some(v), None) if !v.is_empty() => format!("{}{}", v, unit),
            _ => String::new(),
        }
    }
}

/// Power capability with max value (in watts)
/// Example: `<power unit="W" max="36"/>`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerCapability {
    #[serde(rename = "@unit", default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(rename = "@max", default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Optional nominal value
    #[serde(rename = "$value", default)]
    pub value: Option<String>,
}

impl PowerCapability {
    /// Format as display string (e.g., "36W max")
    pub fn to_display_string(&self) -> String {
        let unit = self.unit.as_deref().unwrap_or("W");
        match (&self.value, self.max) {
            (Some(v), Some(max)) if !v.is_empty() => format!("{}{} (max {}{})", v, unit, max, unit),
            (_, Some(max)) => format!("{}{} max", max, unit),
            (Some(v), None) if !v.is_empty() => format!("{}{}", v, unit),
            _ => String::new(),
        }
    }
}

/// Port capabilities - type-specific properties
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PortCapabilities {
    // === Data Capabilities ===
    /// Network speed (e.g., for ethernet) - typically in Mbps
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<ValueWithUnit>,
    /// Bitrate (e.g., for CAN, serial) - typically in bps
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<ValueWithUnit>,
    /// Baud rate (e.g., for UART/serial) - typically in baud
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baud: Option<ValueWithUnit>,
    /// Physical layer standard (e.g., "1000BASE-T", "1000BASE-T1", "100BASE-TX")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standard: Option<String>,
    /// Protocol variants (e.g., "TSN", "CAN-FD", "PoDL", "PoE+") - supports multiple protocols
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protocol: Vec<String>,

    // === Power Capabilities (available on any port type) ===
    /// Voltage with range (min/max) and nominal value
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voltage: Option<VoltageCapability>,
    /// Maximum current
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<CurrentCapability>,
    /// Maximum power in watts
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power: Option<PowerCapability>,
    /// Energy capacity for batteries
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity: Option<ValueWithUnit>,
    /// Physical connector type (e.g., "XT60", "RJ45", "USB-C", "JST-GH")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector: Option<String>,
}

/// Fallback visual for ports/antennas when mesh reference unavailable
/// Follows URDF/SDF pattern with pose and geometry as siblings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackVisual {
    /// Pose offset: "x y z roll pitch yaw" (meters, radians)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pose: Option<String>,
    /// Geometry primitive for visualization
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<Geometry>,
}

impl FallbackVisual {
    /// Parse the pose string into a Pose struct
    pub fn parse_pose(&self) -> Option<Pose> {
        self.pose.as_ref().and_then(|s| parse_pose_string(s))
    }
}

/// Port element - physical connection interface
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Port {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@type")]
    pub port_type: String,
    /// Reference to visual containing the mesh (e.g., "board")
    #[serde(rename = "@visual", default, skip_serializing_if = "Option::is_none")]
    pub visual: Option<String>,
    /// GLTF mesh node name within the visual (e.g., "port_eth0")
    #[serde(rename = "@mesh", default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<String>,
    /// Port capabilities (speed, bitrate, protocol, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<PortCapabilities>,
    /// Fallback visual when mesh reference unavailable
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_visual: Option<FallbackVisual>,
    /// Legacy: Pose at port level (deprecated, use fallback_visual instead)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pose: Option<String>,
    /// Legacy: Geometry at port level (deprecated, use fallback_visual instead)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub geometry: Vec<Geometry>,
}

impl Port {
    /// Parse the pose string into a Pose struct
    /// Checks fallback_visual first, then legacy pose field
    pub fn parse_pose(&self) -> Option<Pose> {
        // Prefer fallback_visual pose
        if let Some(ref fv) = self.fallback_visual {
            if let Some(pose) = fv.parse_pose() {
                return Some(pose);
            }
        }
        // Fall back to legacy pose field
        self.pose.as_ref().and_then(|s| parse_pose_string(s))
    }

    /// Get the geometry for visualization
    /// Checks fallback_visual first, then legacy geometry field
    pub fn get_geometry(&self) -> Option<&Geometry> {
        // Prefer fallback_visual geometry
        if let Some(ref fv) = self.fallback_visual {
            if fv.geometry.is_some() {
                return fv.geometry.as_ref();
            }
        }
        // Fall back to legacy geometry field
        self.geometry.first()
    }

    /// Check if this port uses a mesh reference (vs fallback visual)
    pub fn has_mesh_reference(&self) -> bool {
        self.visual.is_some() && self.mesh.is_some()
    }
}

// ============ ANTENNAS ============

/// Antenna capabilities - type-specific properties for wireless interfaces
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AntennaCapabilities {
    /// Frequency bands (e.g., ["L1", "L2", "L5"] for GNSS, ["2.4 GHz", "5 GHz"] for WiFi)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub band: Vec<String>,
    /// Legacy: frequency with unit (deprecated, use band instead)
    /// Example: `<frequency unit="GHz">5.5</frequency>` -> "5.5 GHz"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency: Option<ValueWithUnit>,
    /// Antenna gain in dBi
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain: Option<ValueWithUnit>,
    /// PHY/MAC standards (e.g., "802.11ax", "802.15.4", "Bluetooth 5.4")
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub standard: Vec<String>,
    /// Higher-layer protocols (e.g., "Thread", "6LoWPAN", "Matter", "WPA3")
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protocol: Vec<String>,
    /// Polarization (e.g., "RHCP", "linear", "circular")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polarization: Option<String>,
}

impl AntennaCapabilities {
    /// Get all frequency bands, combining new `band` elements with legacy `frequency`
    pub fn get_bands(&self) -> Vec<String> {
        let mut bands = self.band.clone();
        // Add legacy frequency if present
        if let Some(ref freq) = self.frequency {
            let freq_str = if let Some(ref unit) = freq.unit {
                format!("{} {}", freq.value, unit)
            } else {
                freq.value.clone()
            };
            if !bands.contains(&freq_str) {
                bands.push(freq_str);
            }
        }
        bands
    }
}

/// Antenna element - wireless connection interface
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Antenna {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@type")]
    pub antenna_type: String,
    /// Reference to visual containing the mesh (e.g., "board")
    #[serde(rename = "@visual", default, skip_serializing_if = "Option::is_none")]
    pub visual: Option<String>,
    /// GLTF mesh node name within the visual (e.g., "gnss_antenna")
    #[serde(rename = "@mesh", default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<String>,
    /// Antenna capabilities (frequency, gain, protocol, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<AntennaCapabilities>,
    /// Fallback visual when mesh reference unavailable
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_visual: Option<FallbackVisual>,
    /// Legacy: Pose at antenna level (deprecated, use fallback_visual instead)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pose: Option<String>,
    /// Legacy: Geometry at antenna level (deprecated, use fallback_visual instead)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<Geometry>,
}

impl Antenna {
    /// Parse the pose string into a Pose struct
    /// Checks fallback_visual first, then legacy pose field
    pub fn parse_pose(&self) -> Option<Pose> {
        // Prefer fallback_visual pose
        if let Some(ref fv) = self.fallback_visual {
            if let Some(pose) = fv.parse_pose() {
                return Some(pose);
            }
        }
        // Fall back to legacy pose field
        self.pose.as_ref().and_then(|s| parse_pose_string(s))
    }

    /// Get the geometry for visualization
    /// Checks fallback_visual first, then legacy geometry field
    pub fn get_geometry(&self) -> Option<&Geometry> {
        // Prefer fallback_visual geometry
        if let Some(ref fv) = self.fallback_visual {
            if fv.geometry.is_some() {
                return fv.geometry.as_ref();
            }
        }
        // Fall back to legacy geometry field
        self.geometry.as_ref()
    }

    /// Check if this antenna uses a mesh reference (vs fallback visual)
    pub fn has_mesh_reference(&self) -> bool {
        self.visual.is_some() && self.mesh.is_some()
    }
}

// ============ AXIS ALIGNMENT ============

/// Axis alignment for sensor driver transforms
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AxisAlign {
    /// Output X comes from this hardware axis (X, -X, Y, -Y, Z, -Z)
    #[serde(rename = "@x", default = "default_axis_x")]
    pub x: String,
    /// Output Y comes from this hardware axis
    #[serde(rename = "@y", default = "default_axis_y")]
    pub y: String,
    /// Output Z comes from this hardware axis
    #[serde(rename = "@z", default = "default_axis_z")]
    pub z: String,
}

fn default_axis_x() -> String { "X".to_string() }
fn default_axis_y() -> String { "Y".to_string() }
fn default_axis_z() -> String { "Z".to_string() }

/// Axis mapping value
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AxisMap {
    X, NegX, Y, NegY, Z, NegZ
}

impl AxisMap {
    /// Parse axis string (e.g., "X", "-Y", "Z")
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "X" => Some(AxisMap::X),
            "-X" => Some(AxisMap::NegX),
            "Y" => Some(AxisMap::Y),
            "-Y" => Some(AxisMap::NegY),
            "Z" => Some(AxisMap::Z),
            "-Z" => Some(AxisMap::NegZ),
            _ => None,
        }
    }

    /// Convert to unit vector [x, y, z]
    pub fn to_vec3(&self) -> [f32; 3] {
        match self {
            AxisMap::X => [1.0, 0.0, 0.0],
            AxisMap::NegX => [-1.0, 0.0, 0.0],
            AxisMap::Y => [0.0, 1.0, 0.0],
            AxisMap::NegY => [0.0, -1.0, 0.0],
            AxisMap::Z => [0.0, 0.0, 1.0],
            AxisMap::NegZ => [0.0, 0.0, -1.0],
        }
    }
}

impl AxisAlign {
    /// Parse to AxisMap tuple
    pub fn parse_axes(&self) -> Option<(AxisMap, AxisMap, AxisMap)> {
        let x = AxisMap::parse(&self.x)?;
        let y = AxisMap::parse(&self.y)?;
        let z = AxisMap::parse(&self.z)?;
        Some((x, y, z))
    }

    /// Convert to 3x3 rotation matrix (column-major)
    pub fn to_rotation_matrix(&self) -> Option<[[f32; 3]; 3]> {
        let (x, y, z) = self.parse_axes()?;
        Some([x.to_vec3(), y.to_vec3(), z.to_vec3()])
    }
}

impl Default for AxisAlign {
    fn default() -> Self {
        Self {
            x: "X".to_string(),
            y: "Y".to_string(),
            z: "Z".to_string(),
        }
    }
}

// ============ SENSOR DRIVER ============

/// Sensor driver with axis alignment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorDriver {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "axis-align", default)]
    pub axis_align: Option<AxisAlign>,
}

// ============ SENSOR SUBTYPES ============

/// Inertial sensor (accelerometer, gyroscope, or combined)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InertialSensor {
    /// Type: accel, gyro, accel_gyro
    #[serde(rename = "@type")]
    pub sensor_type: String,
    /// Pose: "x y z roll pitch yaw"
    #[serde(default)]
    pub pose: Option<String>,
    /// Driver configuration
    #[serde(default)]
    pub driver: Option<SensorDriver>,
    /// Geometry (usually not needed for inertial)
    #[serde(default)]
    pub geometry: Option<Geometry>,
}

impl InertialSensor {
    pub fn parse_pose(&self) -> Option<Pose> {
        self.pose.as_ref().and_then(|s| parse_pose_string(s))
    }
}

/// Electromagnetic sensor (magnetometer, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmSensor {
    /// Type: mag, metal_detector, eddy_current, emf
    #[serde(rename = "@type")]
    pub sensor_type: String,
    #[serde(default)]
    pub pose: Option<String>,
    #[serde(default)]
    pub driver: Option<SensorDriver>,
    #[serde(default)]
    pub geometry: Option<Geometry>,
}

impl EmSensor {
    pub fn parse_pose(&self) -> Option<Pose> {
        self.pose.as_ref().and_then(|s| parse_pose_string(s))
    }
}

/// Field of View element - named FOV with pose, color, and geometry
/// Used for sensors with multiple optical paths (emitter/collector, stereo, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fov {
    /// FOV name (e.g., "emitter", "collector", "left", "right")
    #[serde(rename = "@name")]
    pub name: String,
    /// Custom color in hex format (e.g., "#ff4444")
    #[serde(rename = "@color", default)]
    pub color: Option<String>,
    /// Pose offset relative to sensor origin: "x y z roll pitch yaw"
    #[serde(default)]
    pub pose: Option<String>,
    /// FOV geometry
    #[serde(default)]
    pub geometry: Option<Geometry>,
}

impl Fov {
    /// Parse the pose string into a Pose struct
    pub fn parse_pose(&self) -> Option<Pose> {
        self.pose.as_ref().and_then(|s| parse_pose_string(s))
    }

    /// Parse the color string into RGB values (0.0-1.0)
    pub fn parse_color(&self) -> Option<(f32, f32, f32)> {
        self.color.as_ref().and_then(|s| parse_hex_color(s))
    }
}

/// Parse a hex color string (e.g., "#ff4444" or "ff4444") into RGB (0.0-1.0)
pub fn parse_hex_color(s: &str) -> Option<(f32, f32, f32)> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0))
}

/// Optical sensor (camera, lidar, tof, optical_flow)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpticalSensor {
    /// Type: camera, lidar, tof, optical_flow
    #[serde(rename = "@type")]
    pub sensor_type: String,
    #[serde(default)]
    pub pose: Option<String>,
    #[serde(default)]
    pub driver: Option<SensorDriver>,
    /// Legacy: single FOV geometry (deprecated, use fov elements instead)
    #[serde(default)]
    pub geometry: Option<Geometry>,
    /// Multiple named FOVs with individual poses and colors
    #[serde(default)]
    pub fov: Vec<Fov>,
}

impl OpticalSensor {
    pub fn parse_pose(&self) -> Option<Pose> {
        self.pose.as_ref().and_then(|s| parse_pose_string(s))
    }
}

/// RF sensor (gnss, uwb, radar)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfSensor {
    /// Type: gnss, uwb, radar, radio_altimeter
    #[serde(rename = "@type")]
    pub sensor_type: String,
    #[serde(default)]
    pub pose: Option<String>,
    #[serde(default)]
    pub driver: Option<SensorDriver>,
    #[serde(default)]
    pub geometry: Option<Geometry>,
}

impl RfSensor {
    pub fn parse_pose(&self) -> Option<Pose> {
        self.pose.as_ref().and_then(|s| parse_pose_string(s))
    }
}

/// Chemical sensor (gas, ph, humidity)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChemicalSensor {
    /// Type: gas, ph, humidity
    #[serde(rename = "@type")]
    pub sensor_type: String,
    #[serde(default)]
    pub pose: Option<String>,
    #[serde(default)]
    pub driver: Option<SensorDriver>,
    #[serde(default)]
    pub geometry: Option<Geometry>,
}

impl ChemicalSensor {
    pub fn parse_pose(&self) -> Option<Pose> {
        self.pose.as_ref().and_then(|s| parse_pose_string(s))
    }
}

/// Force sensor (strain, pressure, torque, load_cell)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForceSensor {
    /// Type: strain, pressure, torque, load_cell
    #[serde(rename = "@type")]
    pub sensor_type: String,
    #[serde(default)]
    pub pose: Option<String>,
    #[serde(default)]
    pub driver: Option<SensorDriver>,
    #[serde(default)]
    pub geometry: Option<Geometry>,
}

impl ForceSensor {
    pub fn parse_pose(&self) -> Option<Pose> {
        self.pose.as_ref().and_then(|s| parse_pose_string(s))
    }
}

/// Sensor element with typed sub-sensors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sensor {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(default)]
    pub pose_cg: Option<String>,
    /// Inertial sensors (accel, gyro, accel_gyro)
    #[serde(default)]
    pub inertial: Vec<InertialSensor>,
    /// Electromagnetic sensors (mag, etc.)
    #[serde(default)]
    pub em: Vec<EmSensor>,
    /// Optical sensors (camera, lidar, tof, optical_flow)
    #[serde(default)]
    pub optical: Vec<OpticalSensor>,
    /// RF sensors (gnss, uwb, radar)
    #[serde(default)]
    pub rf: Vec<RfSensor>,
    /// Chemical sensors (gas, ph, humidity)
    #[serde(default)]
    pub chemical: Vec<ChemicalSensor>,
    /// Force sensors (strain, pressure, torque, load_cell)
    #[serde(default)]
    pub force: Vec<ForceSensor>,
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

    /// Serialize to XML string with proper indentation for readability
    pub fn to_xml(&self) -> Result<String, HcdfError> {
        let mut buffer = String::new();
        let mut ser = Serializer::new(&mut buffer);
        ser.indent(' ', 2);
        self.serialize(ser)
            .map_err(|e| HcdfError::SerializeError(e.to_string()))?;
        Ok(format!("<?xml version='1.0'?>\n{}", buffer))
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
                sw.hash = device.firmware.image_hash.clone();
            }
            mcu.discovered = Some(Discovered {
                ip: device.discovery.ip.to_string(),
                port: device.discovery.switch_port,
                last_seen: Some(device.discovery.last_seen.to_rfc3339()),
            });
            // Update pose_cg from device pose (preserves position edits)
            if let Some(pose) = device.pose {
                mcu.pose_cg = Some(format!("{} {} {} {} {} {}", pose[0], pose[1], pose[2], pose[3], pose[4], pose[5]));
            }
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
                    firmware_manifest_uri: device.firmware_manifest_uri.clone(),
                    hash: device.firmware.image_hash.clone(),
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

    #[test]
    fn test_parse_hcdf_v2_with_ports_and_sensors() {
        let xml = r#"<?xml version='1.0'?>
<hcdf version="2.0">
  <comp name="optical-flow-assembly" role="sensor">
    <description>Test assembly</description>

    <port name="ETH0" type="ethernet">
      <pose>0.022 -0.015 -0.009 0 0 0</pose>
      <geometry>
        <box>
          <size>0.008 0.006 0.003</size>
        </box>
      </geometry>
    </port>

    <port name="CAN0" type="CAN">
      <pose>-0.022 -0.015 -0.009 0 0 0</pose>
    </port>

    <sensor name="imu_hub">
      <inertial type="accel_gyro">
        <pose>0.016 -0.001 -0.008 0 0 0</pose>
        <driver name="icm45686">
          <axis-align x="Y" y="-X" z="Z"/>
        </driver>
      </inertial>
    </sensor>

    <sensor name="mag0">
      <em type="mag">
        <pose>0.021 0.001 -0.010 0 0 0</pose>
        <driver name="bmm350">
          <axis-align x="X" y="Y" z="Z"/>
        </driver>
      </em>
    </sensor>

    <sensor name="tof">
      <optical type="tof">
        <pose>-0.008 0 0.003 0 0 0</pose>
        <driver name="afbr_s50"/>
        <geometry>
          <frustum>
            <near>0.001</near>
            <far>0.30</far>
            <hfov>0.1047</hfov>
            <vfov>0.1047</vfov>
          </frustum>
        </geometry>
      </optical>
    </sensor>

    <sensor name="flow">
      <optical type="optical_flow">
        <pose>0 0 0.002 0 0 0</pose>
        <geometry>
          <cone>
            <radius>0.30</radius>
            <length>1.0</length>
          </cone>
        </geometry>
      </optical>
    </sensor>

  </comp>
</hcdf>"#;

        let hcdf = Hcdf::from_xml(xml).unwrap();
        assert_eq!(hcdf.version, "2.0");
        assert_eq!(hcdf.comp.len(), 1);

        let comp = &hcdf.comp[0];

        // Check ports
        assert_eq!(comp.port.len(), 2);
        assert_eq!(comp.port[0].name, "ETH0");
        assert_eq!(comp.port[0].port_type, "ethernet");
        assert_eq!(comp.port[1].name, "CAN0");
        assert_eq!(comp.port[1].port_type, "CAN");

        // Check port geometry
        assert_eq!(comp.port[0].geometry.len(), 1);
        let box_geom = comp.port[0].geometry[0].get_box().unwrap();
        let size = box_geom.parse_size().unwrap();
        assert!((size[0] - 0.008).abs() < 0.0001);

        // Check port pose
        let port_pose = comp.port[0].parse_pose().unwrap();
        assert!((port_pose.x - 0.022).abs() < 0.0001);

        // Check sensors
        assert_eq!(comp.sensor.len(), 4);

        // Check IMU sensor
        let imu = &comp.sensor[0];
        assert_eq!(imu.name, "imu_hub");
        assert_eq!(imu.inertial.len(), 1);
        assert_eq!(imu.inertial[0].sensor_type, "accel_gyro");

        // Check axis alignment
        let driver = imu.inertial[0].driver.as_ref().unwrap();
        assert_eq!(driver.name, "icm45686");
        let axis_align = driver.axis_align.as_ref().unwrap();
        assert_eq!(axis_align.x, "Y");
        assert_eq!(axis_align.y, "-X");
        assert_eq!(axis_align.z, "Z");

        // Check rotation matrix from axis align
        let mat = axis_align.to_rotation_matrix().unwrap();
        assert_eq!(mat[0], [0.0, 1.0, 0.0]); // X from Y
        assert_eq!(mat[1], [-1.0, 0.0, 0.0]); // Y from -X
        assert_eq!(mat[2], [0.0, 0.0, 1.0]); // Z from Z

        // Check mag sensor
        let mag = &comp.sensor[1];
        assert_eq!(mag.name, "mag0");
        assert_eq!(mag.em.len(), 1);
        assert_eq!(mag.em[0].sensor_type, "mag");

        // Check tof sensor with frustum geometry
        let tof = &comp.sensor[2];
        assert_eq!(tof.name, "tof");
        assert_eq!(tof.optical.len(), 1);
        assert_eq!(tof.optical[0].sensor_type, "tof");
        let frustum = tof.optical[0].geometry.as_ref().unwrap().frustum.as_ref().unwrap();
        assert!((frustum.far - 0.30).abs() < 0.001);
        assert!((frustum.hfov - 0.1047).abs() < 0.001);

        // Check optical flow sensor with cone geometry
        let flow = &comp.sensor[3];
        assert_eq!(flow.name, "flow");
        let cone = flow.optical[0].geometry.as_ref().unwrap().cone.as_ref().unwrap();
        assert!((cone.radius - 0.30).abs() < 0.001);
        assert!((cone.length - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_axis_map_parsing() {
        assert_eq!(AxisMap::parse("X"), Some(AxisMap::X));
        assert_eq!(AxisMap::parse("-X"), Some(AxisMap::NegX));
        assert_eq!(AxisMap::parse("Y"), Some(AxisMap::Y));
        assert_eq!(AxisMap::parse("-Y"), Some(AxisMap::NegY));
        assert_eq!(AxisMap::parse("Z"), Some(AxisMap::Z));
        assert_eq!(AxisMap::parse("-Z"), Some(AxisMap::NegZ));
        assert_eq!(AxisMap::parse("invalid"), None);
    }

    #[test]
    fn test_axis_align_to_rotation_matrix() {
        // Identity
        let identity = AxisAlign::default();
        let mat = identity.to_rotation_matrix().unwrap();
        assert_eq!(mat[0], [1.0, 0.0, 0.0]);
        assert_eq!(mat[1], [0.0, 1.0, 0.0]);
        assert_eq!(mat[2], [0.0, 0.0, 1.0]);

        // 90 degree rotation around Z: x->Y, y->-X, z->Z
        let rotated = AxisAlign {
            x: "Y".to_string(),
            y: "-X".to_string(),
            z: "Z".to_string(),
        };
        let mat = rotated.to_rotation_matrix().unwrap();
        assert_eq!(mat[0], [0.0, 1.0, 0.0]);
        assert_eq!(mat[1], [-1.0, 0.0, 0.0]);
        assert_eq!(mat[2], [0.0, 0.0, 1.0]);
    }

    #[test]
    fn test_parse_port_with_capabilities_and_fallback_visual() {
        let xml = r#"<?xml version='1.0'?>
<hcdf version="2.1">
  <comp name="rtk-gnss" role="sensor">
    <description>RTK GNSS assembly</description>

    <!-- Port with mesh reference and capabilities -->
    <port name="ETH0" type="ethernet" visual="board" mesh="ETH0">
      <capabilities>
        <speed unit="Mbps">1000</speed>
      </capabilities>
    </port>

    <!-- Port with fallback visual (no mesh available) -->
    <port name="CAN0" type="CAN">
      <capabilities>
        <bitrate unit="bps">500000</bitrate>
        <protocol>CAN-FD</protocol>
      </capabilities>
      <fallback_visual>
        <pose>-0.0225 -0.0155 -0.0085 0 0 0</pose>
        <geometry>
          <box><size>0.005 0.004 0.003</size></box>
        </geometry>
      </fallback_visual>
    </port>

    <!-- Port with serial capabilities -->
    <port name="UART0" type="serial">
      <capabilities>
        <baud unit="baud">115200</baud>
        <protocol>RS-232</protocol>
      </capabilities>
      <fallback_visual>
        <pose>0.01 0.005 -0.008 0 0 0</pose>
        <geometry>
          <box><size>0.003 0.002 0.002</size></box>
        </geometry>
      </fallback_visual>
    </port>

  </comp>
</hcdf>"#;

        let hcdf = Hcdf::from_xml(xml).unwrap();
        assert_eq!(hcdf.version, "2.1");
        assert_eq!(hcdf.comp.len(), 1);

        let comp = &hcdf.comp[0];
        assert_eq!(comp.port.len(), 3);

        // Check ETH0 - mesh reference with capabilities
        let eth0 = &comp.port[0];
        assert_eq!(eth0.name, "ETH0");
        assert_eq!(eth0.port_type, "ethernet");
        assert_eq!(eth0.visual, Some("board".to_string()));
        assert_eq!(eth0.mesh, Some("ETH0".to_string()));
        assert!(eth0.has_mesh_reference());
        assert!(eth0.fallback_visual.is_none());

        // Check ETH0 capabilities
        let eth_caps = eth0.capabilities.as_ref().unwrap();
        let speed = eth_caps.speed.as_ref().unwrap();
        assert_eq!(speed.value, "1000");
        assert_eq!(speed.unit, Some("Mbps".to_string()));
        assert_eq!(speed.parse_value_u64(), Some(1000));

        // Check CAN0 - fallback visual with capabilities
        let can0 = &comp.port[1];
        assert_eq!(can0.name, "CAN0");
        assert_eq!(can0.port_type, "CAN");
        assert!(!can0.has_mesh_reference());

        // Check CAN0 capabilities
        let can_caps = can0.capabilities.as_ref().unwrap();
        let bitrate = can_caps.bitrate.as_ref().unwrap();
        assert_eq!(bitrate.value, "500000");
        assert_eq!(bitrate.unit, Some("bps".to_string()));
        assert_eq!(can_caps.protocol, vec!["CAN-FD".to_string()]);

        // Check CAN0 fallback visual
        let can_fv = can0.fallback_visual.as_ref().unwrap();
        let can_pose = can_fv.parse_pose().unwrap();
        assert!((can_pose.x - (-0.0225)).abs() < 0.0001);
        assert!((can_pose.y - (-0.0155)).abs() < 0.0001);

        // Check CAN0 geometry via get_geometry helper
        let can_geom = can0.get_geometry().unwrap();
        let box_geom = can_geom.get_box().unwrap();
        let size = box_geom.parse_size().unwrap();
        assert!((size[0] - 0.005).abs() < 0.0001);

        // Check parse_pose uses fallback_visual
        let can_pose_via_port = can0.parse_pose().unwrap();
        assert!((can_pose_via_port.x - (-0.0225)).abs() < 0.0001);

        // Check UART0 - serial with baud rate
        let uart0 = &comp.port[2];
        assert_eq!(uart0.name, "UART0");
        assert_eq!(uart0.port_type, "serial");
        let uart_caps = uart0.capabilities.as_ref().unwrap();
        let baud = uart_caps.baud.as_ref().unwrap();
        assert_eq!(baud.value, "115200");
        assert_eq!(baud.unit, Some("baud".to_string()));
        assert_eq!(uart_caps.protocol, vec!["RS-232".to_string()]);
    }

    #[test]
    fn test_port_legacy_compatibility() {
        // Test that legacy port format still works (backwards compatibility)
        let xml = r#"<?xml version='1.0'?>
<hcdf version="2.0">
  <comp name="test" role="sensor">
    <port name="ETH0" type="ethernet">
      <pose>0.022 -0.015 -0.009 0 0 0</pose>
      <geometry>
        <box><size>0.008 0.006 0.003</size></box>
      </geometry>
    </port>
  </comp>
</hcdf>"#;

        let hcdf = Hcdf::from_xml(xml).unwrap();
        let port = &hcdf.comp[0].port[0];

        // Legacy pose should work via parse_pose
        let pose = port.parse_pose().unwrap();
        assert!((pose.x - 0.022).abs() < 0.0001);

        // Legacy geometry should work via get_geometry
        let geom = port.get_geometry().unwrap();
        let box_geom = geom.get_box().unwrap();
        let size = box_geom.parse_size().unwrap();
        assert!((size[0] - 0.008).abs() < 0.0001);

        // No fallback_visual in legacy format
        assert!(port.fallback_visual.is_none());
    }

    #[test]
    fn test_parse_port_power_capabilities() {
        let xml = r#"<?xml version='1.0'?>
<hcdf version="2.1">
  <comp name="test" role="compute">
    <!-- Power input port with voltage range and max draw -->
    <port name="pwr_in" type="POWER" visual="main_board" mesh="pwr">
      <capabilities>
        <voltage unit="V" min="7" max="28">12</voltage>
        <current unit="A" max="3"/>
        <power unit="W" max="36"/>
        <connector>XT30</connector>
      </capabilities>
    </port>

    <!-- Ethernet with PoDL power -->
    <port name="eth_podl" type="ethernet" visual="board" mesh="eth">
      <capabilities>
        <speed unit="Mbps">1000</speed>
        <standard>1000BASE-T1</standard>
        <protocol>PoDL</protocol>
        <voltage unit="V" min="12" max="48">24</voltage>
        <power unit="W" max="50"/>
      </capabilities>
    </port>

    <!-- Battery output port with capacity -->
    <port name="bat_out" type="POWER" visual="battery" mesh="output">
      <capabilities>
        <voltage unit="V" min="10.5" max="12.6">12</voltage>
        <current unit="A" max="10"/>
        <power unit="W" max="120"/>
        <capacity unit="Wh">55.5</capacity>
        <connector>XT60</connector>
      </capabilities>
    </port>
  </comp>
</hcdf>"#;

        let hcdf = Hcdf::from_xml(xml).unwrap();
        let comp = &hcdf.comp[0];
        assert_eq!(comp.port.len(), 3);

        // Check power input port
        let pwr_in = &comp.port[0];
        assert_eq!(pwr_in.name, "pwr_in");
        assert_eq!(pwr_in.port_type, "POWER");
        let caps = pwr_in.capabilities.as_ref().unwrap();

        // Check voltage with range
        let voltage = caps.voltage.as_ref().unwrap();
        assert_eq!(voltage.unit, Some("V".to_string()));
        assert_eq!(voltage.min, Some(7.0));
        assert_eq!(voltage.max, Some(28.0));
        assert_eq!(voltage.value, Some("12".to_string()));
        assert_eq!(voltage.to_display_string(), "12V (7-28V)");

        // Check current
        let current = caps.current.as_ref().unwrap();
        assert_eq!(current.unit, Some("A".to_string()));
        assert_eq!(current.max, Some(3.0));
        assert_eq!(current.to_display_string(), "3A max");

        // Check power
        let power = caps.power.as_ref().unwrap();
        assert_eq!(power.unit, Some("W".to_string()));
        assert_eq!(power.max, Some(36.0));
        assert_eq!(power.to_display_string(), "36W max");

        // Check connector
        assert_eq!(caps.connector, Some("XT30".to_string()));

        // Check Ethernet with PoDL - has both data and power capabilities
        let eth_podl = &comp.port[1];
        assert_eq!(eth_podl.name, "eth_podl");
        let eth_caps = eth_podl.capabilities.as_ref().unwrap();
        // Data capabilities
        assert_eq!(eth_caps.speed.as_ref().unwrap().value, "1000");
        assert_eq!(eth_caps.standard, Some("1000BASE-T1".to_string()));
        assert_eq!(eth_caps.protocol, vec!["PoDL".to_string()]);
        // Power capabilities
        let eth_voltage = eth_caps.voltage.as_ref().unwrap();
        assert_eq!(eth_voltage.to_display_string(), "24V (12-48V)");

        // Check battery port with capacity
        let bat_out = &comp.port[2];
        let bat_caps = bat_out.capabilities.as_ref().unwrap();
        let capacity = bat_caps.capacity.as_ref().unwrap();
        assert_eq!(capacity.value, "55.5");
        assert_eq!(capacity.unit, Some("Wh".to_string()));
        assert_eq!(bat_caps.connector, Some("XT60".to_string()));
    }

    #[test]
    fn test_parse_antenna_with_capabilities_and_fallback_visual() {
        let xml = r#"<?xml version='1.0'?>
<hcdf version="2.1">
  <comp name="rtk-gnss" role="sensor">
    <description>RTK GNSS assembly</description>

    <!-- Antenna with mesh reference and capabilities -->
    <antenna name="GNSS0" type="gnss" visual="board" mesh="GNSS_ANT">
      <capabilities>
        <band>L1</band>
        <band>L2</band>
        <band>L5</band>
        <gain unit="dBi">3.5</gain>
        <polarization>RHCP</polarization>
      </capabilities>
    </antenna>

    <!-- Tri-radio antenna with fallback visual -->
    <antenna name="WIFI0" type="wifi">
      <capabilities>
        <band>2.4 GHz</band>
        <band>5 GHz</band>
        <gain unit="dBi">2.0</gain>
        <standard>802.11ax</standard>
        <protocol>WPA3</protocol>
      </capabilities>
      <fallback_visual>
        <pose>0.01 0.02 0.005 0 0 0</pose>
        <geometry>
          <cylinder><radius>0.002</radius><length>0.015</length></cylinder>
        </geometry>
      </fallback_visual>
    </antenna>

    <!-- 802.15.4 antenna with multiple standards and protocols -->
    <antenna name="WPAN0" type="802.15.4">
      <capabilities>
        <band>2.4 GHz</band>
        <standard>Bluetooth 5.4</standard>
        <standard>802.15.4</standard>
        <protocol>Thread</protocol>
        <protocol>6LoWPAN</protocol>
        <protocol>Matter</protocol>
      </capabilities>
      <fallback_visual>
        <pose>-0.005 0.01 0.003 0 0 0</pose>
        <geometry>
          <box><size>0.003 0.002 0.001</size></box>
        </geometry>
      </fallback_visual>
    </antenna>

  </comp>
</hcdf>"#;

        let hcdf = Hcdf::from_xml(xml).unwrap();
        assert_eq!(hcdf.version, "2.1");
        assert_eq!(hcdf.comp.len(), 1);

        let comp = &hcdf.comp[0];
        assert_eq!(comp.antenna.len(), 3);

        // Check GNSS0 - mesh reference with capabilities
        let gnss0 = &comp.antenna[0];
        assert_eq!(gnss0.name, "GNSS0");
        assert_eq!(gnss0.antenna_type, "gnss");
        assert_eq!(gnss0.visual, Some("board".to_string()));
        assert_eq!(gnss0.mesh, Some("GNSS_ANT".to_string()));
        assert!(gnss0.has_mesh_reference());
        assert!(gnss0.fallback_visual.is_none());

        // Check GNSS0 capabilities - multiple bands
        let gnss_caps = gnss0.capabilities.as_ref().unwrap();
        assert_eq!(gnss_caps.band, vec!["L1".to_string(), "L2".to_string(), "L5".to_string()]);
        let gain = gnss_caps.gain.as_ref().unwrap();
        assert_eq!(gain.value, "3.5");
        assert_eq!(gain.unit, Some("dBi".to_string()));
        assert_eq!(gnss_caps.polarization, Some("RHCP".to_string()));

        // Check WIFI0 - tri-radio with standards and protocols
        let wifi0 = &comp.antenna[1];
        assert_eq!(wifi0.name, "WIFI0");
        assert_eq!(wifi0.antenna_type, "wifi");
        assert!(!wifi0.has_mesh_reference());

        // Check WIFI0 capabilities
        let wifi_caps = wifi0.capabilities.as_ref().unwrap();
        assert_eq!(wifi_caps.band, vec!["2.4 GHz".to_string(), "5 GHz".to_string()]);
        assert_eq!(wifi_caps.standard, vec!["802.11ax".to_string()]);
        assert_eq!(wifi_caps.protocol, vec!["WPA3".to_string()]);

        // Check WIFI0 fallback visual
        let wifi_fv = wifi0.fallback_visual.as_ref().unwrap();
        let wifi_pose = wifi_fv.parse_pose().unwrap();
        assert!((wifi_pose.x - 0.01).abs() < 0.0001);
        assert!((wifi_pose.y - 0.02).abs() < 0.0001);

        // Check WIFI0 geometry via get_geometry helper
        let wifi_geom = wifi0.get_geometry().unwrap();
        let cyl = wifi_geom.cylinder.as_ref().unwrap();
        assert!((cyl.radius - 0.002).abs() < 0.0001);
        assert!((cyl.length - 0.015).abs() < 0.0001);

        // Check parse_pose uses fallback_visual
        let wifi_pose_via_antenna = wifi0.parse_pose().unwrap();
        assert!((wifi_pose_via_antenna.x - 0.01).abs() < 0.0001);

        // Check WPAN0 - 802.15.4 with multiple standards and protocols
        let wpan0 = &comp.antenna[2];
        assert_eq!(wpan0.name, "WPAN0");
        assert_eq!(wpan0.antenna_type, "802.15.4");
        let wpan_caps = wpan0.capabilities.as_ref().unwrap();
        assert_eq!(wpan_caps.band, vec!["2.4 GHz".to_string()]);
        assert_eq!(wpan_caps.standard, vec!["Bluetooth 5.4".to_string(), "802.15.4".to_string()]);
        assert_eq!(wpan_caps.protocol, vec!["Thread".to_string(), "6LoWPAN".to_string(), "Matter".to_string()]);
        let wpan_geom = wpan0.get_geometry().unwrap();
        let box_geom = wpan_geom.get_box().unwrap();
        let size = box_geom.parse_size().unwrap();
        assert!((size[0] - 0.003).abs() < 0.0001);
    }

    #[test]
    fn test_antenna_legacy_compatibility() {
        // Test that legacy antenna format still works (backwards compatibility)
        let xml = r#"<?xml version='1.0'?>
<hcdf version="2.0">
  <comp name="test" role="sensor">
    <antenna name="GNSS0" type="gnss">
      <pose>0.01 0.02 0.005 0 0 0</pose>
      <geometry>
        <cylinder><radius>0.005</radius><length>0.01</length></cylinder>
      </geometry>
    </antenna>
  </comp>
</hcdf>"#;

        let hcdf = Hcdf::from_xml(xml).unwrap();
        let antenna = &hcdf.comp[0].antenna[0];

        // Legacy pose should work via parse_pose
        let pose = antenna.parse_pose().unwrap();
        assert!((pose.x - 0.01).abs() < 0.0001);

        // Legacy geometry should work via get_geometry
        let geom = antenna.get_geometry().unwrap();
        let cyl = geom.cylinder.as_ref().unwrap();
        assert!((cyl.radius - 0.005).abs() < 0.0001);

        // No fallback_visual in legacy format
        assert!(antenna.fallback_visual.is_none());
    }
}

    #[test]
    fn test_parse_interleaved_ports_and_antennas() {
        // Test with ports interleaved with antennas - this is common in real HCDF files
        // quick_xml requires special handling for non-consecutive elements of the same type
        let xml = r#"<?xml version='1.0'?>
<hcdf version="2.0">
  <comp name="test" role="compute">
    <port name="eth0" type="ethernet" visual="board" mesh="rj45">
      <capabilities><speed unit="Mbps">1000</speed></capabilities>
    </port>
    <port name="eth1" type="ethernet" visual="board" mesh="port1">
      <capabilities><speed unit="Mbps">100</speed></capabilities>
    </port>
    <antenna name="wifi" type="wifi" visual="board" mesh="ant0">
      <capabilities><band>2.4 GHz</band></capabilities>
    </antenna>
    <port name="can0" type="CAN" visual="board" mesh="can0">
      <capabilities><bitrate unit="bps">500000</bitrate></capabilities>
    </port>
    <sensor name="imu">
      <inertial type="accel_gyro">
        <pose>0 0 0 0 0 0</pose>
      </inertial>
    </sensor>
    <visual name="board">
      <pose>0 0 0 0 0 0</pose>
      <model href="test.glb" sha=""/>
    </visual>
  </comp>
</hcdf>"#;

        let hcdf = Hcdf::from_xml(xml);
        assert!(hcdf.is_ok(), "Failed to parse: {:?}", hcdf.err());

        let hcdf = hcdf.unwrap();
        let comp = &hcdf.comp[0];

        assert_eq!(comp.port.len(), 3);
        assert_eq!(comp.antenna.len(), 1);
        assert_eq!(comp.sensor.len(), 1);
        assert_eq!(comp.visual.len(), 1);
    }
