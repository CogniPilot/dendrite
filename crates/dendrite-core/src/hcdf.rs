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
    /// Ports (wired connection interfaces)
    #[serde(default)]
    pub port: Vec<Port>,
    /// Antennas (wireless connection interfaces)
    #[serde(default)]
    pub antenna: Vec<Antenna>,
    /// Sensors
    #[serde(default)]
    pub sensor: Vec<Sensor>,
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

/// Port element - physical connection interface
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Port {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@type")]
    pub port_type: String,
    /// Reference to visual containing the mesh (e.g., "board")
    #[serde(rename = "@visual", default)]
    pub visual: Option<String>,
    /// GLTF mesh node name within the visual (e.g., "port_eth0")
    #[serde(rename = "@mesh", default)]
    pub mesh: Option<String>,
    /// Pose: "x y z roll pitch yaw"
    #[serde(default)]
    pub pose: Option<String>,
    /// Geometry for visualization/interaction
    #[serde(default)]
    pub geometry: Vec<Geometry>,
}

impl Port {
    /// Parse the pose string into a Pose struct
    pub fn parse_pose(&self) -> Option<Pose> {
        self.pose.as_ref().and_then(|s| parse_pose_string(s))
    }
}

/// Antenna element - wireless connection interface
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Antenna {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@type")]
    pub antenna_type: String,
    /// Pose: "x y z roll pitch yaw"
    #[serde(default)]
    pub pose: Option<String>,
    /// Geometry for visualization
    #[serde(default)]
    pub geometry: Option<Geometry>,
}

impl Antenna {
    /// Parse the pose string into a Pose struct
    pub fn parse_pose(&self) -> Option<Pose> {
        self.pose.as_ref().and_then(|s| parse_pose_string(s))
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
}
