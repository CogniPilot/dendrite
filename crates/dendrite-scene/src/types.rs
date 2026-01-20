//! Shared types for device data, visualization settings, and UI state

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Visual element data - a 3D model with a pose offset
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualData {
    pub name: String,
    /// Toggle group name for visibility control (e.g., "case")
    pub toggle: Option<String>,
    /// Pose offset: (x, y, z, roll, pitch, yaw) in meters/radians
    pub pose: Option<[f64; 6]>,
    /// Model file path
    pub model_path: Option<String>,
    /// Model SHA for cache validation
    pub model_sha: Option<String>,
}

/// Reference frame data - a named coordinate frame
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameData {
    pub name: String,
    pub description: Option<String>,
    /// Pose offset: (x, y, z, roll, pitch, yaw) in meters/radians
    pub pose: Option<[f64; 6]>,
}

/// Axis alignment for sensor driver transforms
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AxisAlignData {
    pub x: String,
    pub y: String,
    pub z: String,
}

/// Geometry data for visualization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GeometryData {
    Box { size: [f64; 3] },
    Cylinder { radius: f64, length: f64 },
    Sphere { radius: f64 },
    /// Deprecated: use ConicalFrustum
    Cone { radius: f64, length: f64 },
    /// Deprecated: use PyramidalFrustum
    Frustum { near: f64, far: f64, hfov: f64, vfov: f64 },
    /// Conical frustum (circular cross-section FOV)
    ConicalFrustum { near: f64, far: f64, fov: f64 },
    /// Pyramidal frustum (rectangular cross-section FOV)
    PyramidalFrustum { near: f64, far: f64, hfov: f64, vfov: f64 },
}

/// Field of View data - named FOV with pose, color, and geometry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FovData {
    /// FOV name (e.g., "emitter", "collector")
    pub name: String,
    /// Custom color as RGB (0.0-1.0)
    pub color: Option<[f32; 3]>,
    /// Pose offset relative to sensor
    pub pose: Option<[f64; 6]>,
    /// FOV geometry
    pub geometry: Option<GeometryData>,
}

/// Port data - physical connection interface
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortData {
    pub name: String,
    pub port_type: String,
    /// Pose offset: (x, y, z, roll, pitch, yaw) in meters/radians
    pub pose: Option<[f64; 6]>,
    /// Geometry for visualization (fallback if no mesh_name)
    pub geometry: Vec<GeometryData>,
    /// Reference to visual containing the mesh (e.g., "board")
    pub visual_name: Option<String>,
    /// GLTF mesh node name within the visual (e.g., "port_eth0")
    pub mesh_name: Option<String>,
}

/// Sensor data - sensor with pose, axis alignment, and optional FOV geometry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorData {
    pub name: String,
    /// Sensor category (inertial, em, optical, rf, force, chemical)
    pub category: String,
    /// Sensor type within category (accel_gyro, mag, optical_flow, tof, etc.)
    pub sensor_type: String,
    /// Driver name (icm45686, bmm350, etc.)
    pub driver: Option<String>,
    /// Pose offset: (x, y, z, roll, pitch, yaw) in meters/radians
    pub pose: Option<[f64; 6]>,
    /// Axis alignment for driver transforms
    pub axis_align: Option<AxisAlignData>,
    /// Legacy: single geometry for FOV visualization (deprecated, use fovs)
    pub geometry: Option<GeometryData>,
    /// Multiple named FOVs with individual poses and colors
    pub fovs: Vec<FovData>,
}

/// Device status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DeviceStatus {
    Online,
    Offline,
    #[default]
    Unknown,
}

/// Core device data for visualization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceData {
    pub id: String,
    pub name: String,
    pub board: Option<String>,
    pub ip: String,
    pub port: Option<u8>,
    pub status: DeviceStatus,
    pub version: Option<String>,
    pub position: Option<[f64; 3]>,
    /// Orientation as [roll, pitch, yaw] in radians
    pub orientation: Option<[f64; 3]>,
    /// Legacy single model path (for backward compatibility)
    pub model_path: Option<String>,
    /// Composite visuals with individual poses
    pub visuals: Vec<VisualData>,
    /// Reference frames for this device
    pub frames: Vec<FrameData>,
    /// Ports on this device
    pub ports: Vec<PortData>,
    /// Sensors on this device
    pub sensors: Vec<SensorData>,
    pub last_seen: Option<String>,
}

/// Currently selected device
#[derive(Debug, Clone, Resource, Default)]
pub struct SelectedDevice(pub Option<String>);

/// Tracked device positions for UI display
#[derive(Debug, Clone, Resource, Default)]
pub struct DevicePositions {
    pub positions: HashMap<String, Vec3>,
}

/// Tracked device orientations (Roll, Pitch, Yaw in radians, FLU body frame)
#[derive(Debug, Clone, Resource, Default)]
pub struct DeviceOrientations {
    pub orientations: HashMap<String, Vec3>,
}

/// Which rotation axis is currently being edited
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActiveRotationAxis {
    #[default]
    None,
    Roll,
    Pitch,
    Yaw,
}

/// Tracks which rotation axis field is currently active in the UI
#[derive(Debug, Clone, Resource, Default)]
pub struct ActiveRotationField {
    pub axis: ActiveRotationAxis,
}

/// Whether to show the rotation axis visualization when editing orientation
#[derive(Debug, Clone, Resource)]
pub struct ShowRotationAxis(pub bool);

impl Default for ShowRotationAxis {
    fn default() -> Self {
        Self(false)
    }
}

/// World visualization settings
#[derive(Debug, Clone, Resource)]
pub struct WorldSettings {
    pub show_grid: bool,
    pub show_axis: bool,
    pub grid_spacing: f32,
    pub grid_line_thickness: f32,
    pub grid_alpha: f32,
}

impl Default for WorldSettings {
    fn default() -> Self {
        Self {
            show_grid: true,
            show_axis: true,
            grid_spacing: 0.1,
            grid_line_thickness: 0.0002,
            grid_alpha: 0.5,
        }
    }
}

impl WorldSettings {
    /// Check if grid geometry needs to be regenerated
    pub fn needs_grid_regeneration(&self, prev: &WorldSettings) -> bool {
        self.grid_spacing != prev.grid_spacing
            || self.grid_line_thickness != prev.grid_line_thickness
            || self.grid_alpha != prev.grid_alpha
    }
}

/// Frame and visual visibility settings (per-device)
#[derive(Debug, Clone, Resource, Default)]
pub struct FrameVisibility {
    /// Per-device frame visibility (device_id -> show_frames)
    pub device_frames: HashMap<String, bool>,
    /// Currently hovered frame (device_id:frame_name)
    pub hovered_frame: Option<String>,
    /// Whether the frame hover came from a click/tap (sticky until next click)
    pub hovered_frame_from_click: bool,
    /// Per-device, per-toggle-group hidden state
    pub hidden_toggles: HashMap<(String, String), bool>,
    /// Per-device sensor (FOV) visibility
    pub device_sensors: HashMap<String, bool>,
    /// Currently hovered sensor axis frame
    pub hovered_sensor_axis: Option<String>,
    pub hovered_sensor_axis_from_click: bool,
    /// Currently hovered sensor FOV
    pub hovered_sensor_fov: Option<String>,
    pub hovered_sensor_fov_from_click: bool,
    /// Currently hovered sensor from UI panel
    pub hovered_sensor_from_ui: Option<String>,
    /// Per-device port visibility
    pub device_ports: HashMap<String, bool>,
    /// Currently hovered port
    pub hovered_port: Option<String>,
    pub hovered_port_from_ui: bool,
    /// Per-sensor axis alignment mode
    pub sensor_axis_aligned: HashMap<(String, String), bool>,
    /// Per-sensor FOV visibility
    pub sensor_fov_visible: HashMap<(String, String), bool>,
    /// Per-frame visibility
    pub frame_visible: HashMap<(String, String), bool>,
    /// Per-sensor axis visibility
    pub sensor_axis_visible: HashMap<(String, String), bool>,
}

impl FrameVisibility {
    pub fn show_frames_for(&self, device_id: &str) -> bool {
        self.device_frames.get(device_id).copied().unwrap_or(false)
    }

    pub fn set_show_frames(&mut self, device_id: &str, show: bool) {
        self.device_frames.insert(device_id.to_string(), show);
    }

    pub fn is_toggle_hidden(&self, device_id: &str, toggle_group: &str) -> bool {
        self.hidden_toggles
            .get(&(device_id.to_string(), toggle_group.to_string()))
            .copied()
            .unwrap_or(false)
    }

    pub fn set_toggle_hidden(&mut self, device_id: &str, toggle_group: &str, hidden: bool) {
        let key = (device_id.to_string(), toggle_group.to_string());
        if hidden {
            self.hidden_toggles.insert(key, true);
        } else {
            self.hidden_toggles.remove(&key);
        }
    }

    pub fn get_toggle_groups(visuals: &[VisualData]) -> Vec<String> {
        let mut groups: Vec<String> = visuals
            .iter()
            .filter_map(|v| v.toggle.clone())
            .collect();
        groups.sort();
        groups.dedup();
        groups
    }

    pub fn show_sensors_for(&self, device_id: &str) -> bool {
        self.device_sensors.get(device_id).copied().unwrap_or(false)
    }

    pub fn set_show_sensors(&mut self, device_id: &str, show: bool) {
        self.device_sensors.insert(device_id.to_string(), show);
    }

    pub fn show_ports_for(&self, device_id: &str) -> bool {
        self.device_ports.get(device_id).copied().unwrap_or(false)
    }

    pub fn set_show_ports(&mut self, device_id: &str, show: bool) {
        self.device_ports.insert(device_id.to_string(), show);
    }

    pub fn is_sensor_axis_aligned(&self, device_id: &str, sensor_name: &str) -> bool {
        self.sensor_axis_aligned
            .get(&(device_id.to_string(), sensor_name.to_string()))
            .copied()
            .unwrap_or(true)
    }

    pub fn set_sensor_axis_aligned(&mut self, device_id: &str, sensor_name: &str, aligned: bool) {
        let key = (device_id.to_string(), sensor_name.to_string());
        if aligned {
            self.sensor_axis_aligned.remove(&key);
        } else {
            self.sensor_axis_aligned.insert(key, false);
        }
    }

    pub fn is_sensor_fov_visible(&self, device_id: &str, sensor_name: &str) -> bool {
        self.sensor_fov_visible
            .get(&(device_id.to_string(), sensor_name.to_string()))
            .copied()
            .unwrap_or(true)
    }

    pub fn set_sensor_fov_visible(&mut self, device_id: &str, sensor_name: &str, visible: bool) {
        let key = (device_id.to_string(), sensor_name.to_string());
        if visible {
            self.sensor_fov_visible.remove(&key);
        } else {
            self.sensor_fov_visible.insert(key, false);
        }
    }

    pub fn is_frame_visible(&self, device_id: &str, frame_name: &str) -> bool {
        self.frame_visible
            .get(&(device_id.to_string(), frame_name.to_string()))
            .copied()
            .unwrap_or(true)
    }

    pub fn set_frame_visible(&mut self, device_id: &str, frame_name: &str, visible: bool) {
        let key = (device_id.to_string(), frame_name.to_string());
        if visible {
            self.frame_visible.remove(&key);
        } else {
            self.frame_visible.insert(key, false);
        }
    }

    pub fn is_sensor_axis_visible(&self, device_id: &str, sensor_name: &str) -> bool {
        self.sensor_axis_visible
            .get(&(device_id.to_string(), sensor_name.to_string()))
            .copied()
            .unwrap_or(true)
    }

    pub fn set_sensor_axis_visible(&mut self, device_id: &str, sensor_name: &str, visible: bool) {
        let key = (device_id.to_string(), sensor_name.to_string());
        if visible {
            self.sensor_axis_visible.remove(&key);
        } else {
            self.sensor_axis_visible.insert(key, false);
        }
    }
}

/// UI layout detection and responsive settings
#[derive(Debug, Clone, Resource)]
pub struct UiLayout {
    pub is_mobile: bool,
    pub screen_width: f32,
    pub screen_height: f32,
    pub show_left_panel: bool,
    pub show_right_panel: bool,
}

impl Default for UiLayout {
    fn default() -> Self {
        Self {
            is_mobile: false,
            screen_width: 1920.0,
            screen_height: 1080.0,
            show_left_panel: true,
            show_right_panel: true,
        }
    }
}

impl UiLayout {
    pub fn update_from_window(&mut self, width: f32, height: f32) {
        self.screen_width = width;
        self.screen_height = height;
        // Consider mobile if width < 800 or in portrait orientation
        self.is_mobile = width < 800.0 || (height > width * 1.2);
    }

    pub fn left_panel_width(&self) -> f32 {
        if self.is_mobile {
            self.screen_width * 0.85
        } else {
            280.0
        }
    }

    pub fn right_panel_width(&self) -> f32 {
        if self.is_mobile {
            self.screen_width * 0.85
        } else {
            320.0
        }
    }

    pub fn ui_scale(&self) -> f32 {
        if self.is_mobile { 1.2 } else { 1.0 }
    }
}
