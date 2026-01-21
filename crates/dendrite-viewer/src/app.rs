//! Bevy application setup

use bevy::prelude::*;
use bevy::winit::WinitSettings;
use bevy_egui::EguiPlugin;
use bevy_picking::{DefaultPickingPlugins, prelude::MeshPickingPlugin};
use std::time::Duration;

use crate::file_picker::FilePickerPlugin;
use crate::models::ModelsPlugin;
use crate::scene::ScenePlugin;
use crate::ui::UiPlugin;

/// Device data from the backend
#[derive(Debug, Clone, Resource, Default)]
pub struct DeviceRegistry {
    pub devices: Vec<DeviceData>,
    pub connected: bool,
}

/// Visual element data - a 3D model with a pose offset
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
pub struct FrameData {
    pub name: String,
    pub description: Option<String>,
    /// Pose offset: (x, y, z, roll, pitch, yaw) in meters/radians
    pub pose: Option<[f64; 6]>,
}

/// Axis alignment for sensor driver transforms
#[derive(Debug, Clone)]
pub struct AxisAlignData {
    pub x: String,
    pub y: String,
    pub z: String,
}

/// Geometry data for visualization
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
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

/// Port capabilities data - type-specific properties
#[derive(Debug, Clone, Default)]
pub struct PortCapabilitiesData {
    // === Data Capabilities ===
    /// Network speed (e.g., "1000 Mbps" for ethernet)
    pub speed: Option<String>,
    /// Bitrate (e.g., "500000 bps" for CAN)
    pub bitrate: Option<String>,
    /// Baud rate (e.g., "115200 baud" for UART)
    pub baud: Option<String>,
    /// Physical layer standard (e.g., "1000BASE-T", "1000BASE-T1")
    pub standard: Option<String>,
    /// Protocol variants (e.g., ["TSN", "CAN-FD", "PoDL", "PoE+"])
    pub protocols: Vec<String>,

    // === Power Capabilities ===
    /// Voltage with range (e.g., "12V (7-28V)")
    pub voltage: Option<String>,
    /// Maximum current (e.g., "3A max")
    pub current: Option<String>,
    /// Maximum power in watts (e.g., "36W max")
    pub power_watts: Option<String>,
    /// Energy capacity for batteries (e.g., "55.5 Wh")
    pub capacity: Option<String>,
    /// Physical connector type (e.g., "XT60", "USB-C")
    pub connector: Option<String>,
}

/// Port data - physical connection interface
#[derive(Debug, Clone)]
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
    /// Port capabilities (speed, bitrate, protocol, etc.)
    pub capabilities: Option<PortCapabilitiesData>,
}

/// Antenna capabilities data - type-specific properties for wireless interfaces
#[derive(Debug, Clone, Default)]
pub struct AntennaCapabilitiesData {
    /// Frequency bands (e.g., ["2.4 GHz", "5 GHz"] or ["L1", "L2", "L5"])
    pub bands: Vec<String>,
    /// Antenna gain (e.g., "3.5 dBi")
    pub gain: Option<String>,
    /// PHY/MAC standards (e.g., ["802.11ax", "802.15.4", "Bluetooth 5.4"])
    pub standards: Vec<String>,
    /// Higher-layer protocols (e.g., ["Thread", "6LoWPAN", "Matter"])
    pub protocols: Vec<String>,
    /// Polarization (e.g., "RHCP", "linear")
    pub polarization: Option<String>,
}

/// Antenna data - wireless connection interface
#[derive(Debug, Clone)]
pub struct AntennaData {
    pub name: String,
    pub antenna_type: String,
    /// Pose offset: (x, y, z, roll, pitch, yaw) in meters/radians
    pub pose: Option<[f64; 6]>,
    /// Geometry for visualization (fallback if no mesh_name)
    pub geometry: Option<GeometryData>,
    /// Reference to visual containing the mesh (e.g., "board")
    pub visual_name: Option<String>,
    /// GLTF mesh node name within the visual (e.g., "gnss_antenna")
    pub mesh_name: Option<String>,
    /// Antenna capabilities (frequency, gain, protocol, etc.)
    pub capabilities: Option<AntennaCapabilitiesData>,
}

/// Sensor data - sensor with pose, axis alignment, and optional FOV geometry
#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
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
    /// Antennas on this device
    pub antennas: Vec<AntennaData>,
    /// Sensors on this device
    pub sensors: Vec<SensorData>,
    pub last_seen: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeviceStatus {
    Online,
    Offline,
    #[default]
    Unknown,
}

/// Currently selected device
#[derive(Debug, Clone, Resource, Default)]
pub struct SelectedDevice(pub Option<String>);

/// Camera controller settings
#[derive(Debug, Clone, Resource)]
pub struct CameraSettings {
    pub distance: f32,
    pub target_distance: f32, // For smooth zoom
    pub azimuth: f32,
    pub elevation: f32,
    pub target: Vec3,
    pub target_focus: Vec3, // For smooth re-centering
    pub sensitivity: f32,
    pub zoom_speed: f32,
    pub smooth_factor: f32,
}

impl Default for CameraSettings {
    fn default() -> Self {
        Self {
            distance: 0.6,
            target_distance: 0.6,
            azimuth: 0.8,  // Start rotated ~45 degrees
            elevation: 0.5, // Slightly elevated view
            target: Vec3::ZERO,
            target_focus: Vec3::ZERO,
            sensitivity: 0.005,
            zoom_speed: 0.1,
            smooth_factor: 0.15,
        }
    }
}

/// Tracked device positions for UI display
#[derive(Debug, Clone, Resource, Default)]
pub struct DevicePositions {
    pub positions: std::collections::HashMap<String, Vec3>,
}

/// Tracked device orientations (Roll, Pitch, Yaw in radians, FLU body frame)
/// This stores the canonical Euler angles to avoid gimbal lock issues
#[derive(Debug, Clone, Resource, Default)]
pub struct DeviceOrientations {
    pub orientations: std::collections::HashMap<String, Vec3>, // (roll, pitch, yaw) in radians
}

/// Which rotation axis is currently being edited
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActiveRotationAxis {
    #[default]
    None,
    Roll,  // X axis
    Pitch, // Y axis
    Yaw,   // Z axis
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
        Self(false) // Hidden by default
    }
}

/// Frame and visual visibility settings (per-device)
#[derive(Debug, Clone, Resource, Default)]
pub struct FrameVisibility {
    /// Per-device frame visibility (device_id -> show_frames)
    /// This also controls sensor axis frame visibility
    pub device_frames: std::collections::HashMap<String, bool>,
    /// Currently hovered frame (device_id:frame_name)
    pub hovered_frame: Option<String>,
    /// Whether the frame hover came from a click/tap (sticky until next click)
    pub hovered_frame_from_click: bool,
    /// Per-device, per-toggle-group hidden state: (device_id, toggle_group) -> is_hidden
    /// Default is visible (not hidden), so only hidden groups are tracked
    pub hidden_toggles: std::collections::HashMap<(String, String), bool>,
    /// Per-device sensor (FOV) visibility (device_id -> show_sensors)
    pub device_sensors: std::collections::HashMap<String, bool>,
    /// Currently hovered sensor axis frame (device_id:sensor_name)
    pub hovered_sensor_axis: Option<String>,
    /// Whether the sensor axis hover came from a click/tap (sticky until next click)
    pub hovered_sensor_axis_from_click: bool,
    /// Currently hovered sensor FOV (device_id:sensor_name)
    pub hovered_sensor_fov: Option<String>,
    /// Whether the sensor FOV hover came from a click/tap (sticky until next click)
    pub hovered_sensor_fov_from_click: bool,
    /// Currently hovered sensor from UI panel (device_id:sensor_name)
    /// When set, other sensors reduce to 30% alpha
    pub hovered_sensor_from_ui: Option<String>,
    /// Per-device port visibility (device_id -> show_ports)
    pub device_ports: std::collections::HashMap<String, bool>,
    /// Currently hovered port (device_id:port_name)
    pub hovered_port: Option<String>,
    /// Whether the current port hover came from UI (true) or 3D (false)
    pub hovered_port_from_ui: bool,
    /// Per-device antenna visibility (device_id -> show_antennas)
    pub device_antennas: std::collections::HashMap<String, bool>,
    /// Currently hovered antenna (device_id:antenna_name)
    pub hovered_antenna: Option<String>,
    /// Whether the current antenna hover came from UI (true) or 3D (false)
    pub hovered_antenna_from_ui: bool,
    /// Per-sensor axis alignment mode: (device_id, sensor_name) -> show_aligned
    /// Default is true (show aligned), false shows raw physical axes
    pub sensor_axis_aligned: std::collections::HashMap<(String, String), bool>,
    /// Per-sensor FOV visibility: (device_id, sensor_name) -> show_fov
    /// Default is true (show FOV), only tracks disabled sensors
    pub sensor_fov_visible: std::collections::HashMap<(String, String), bool>,
    /// Per-frame visibility: (device_id, frame_name) -> show_frame
    /// Default is true (show frame), only tracks disabled frames
    pub frame_visible: std::collections::HashMap<(String, String), bool>,
    /// Per-sensor axis visibility: (device_id, sensor_name) -> show_axis
    /// Default is true (show axis), only tracks disabled sensor axes
    pub sensor_axis_visible: std::collections::HashMap<(String, String), bool>,
}

impl FrameVisibility {
    /// Check if frames should be shown for a specific device
    pub fn show_frames_for(&self, device_id: &str) -> bool {
        self.device_frames.get(device_id).copied().unwrap_or(false)
    }

    /// Set frame visibility for a specific device
    pub fn set_show_frames(&mut self, device_id: &str, show: bool) {
        self.device_frames.insert(device_id.to_string(), show);
    }

    /// Check if a toggle group is hidden for a specific device
    pub fn is_toggle_hidden(&self, device_id: &str, toggle_group: &str) -> bool {
        self.hidden_toggles
            .get(&(device_id.to_string(), toggle_group.to_string()))
            .copied()
            .unwrap_or(false) // Default: visible (not hidden)
    }

    /// Set whether a toggle group is hidden for a specific device
    pub fn set_toggle_hidden(&mut self, device_id: &str, toggle_group: &str, hidden: bool) {
        let key = (device_id.to_string(), toggle_group.to_string());
        if hidden {
            self.hidden_toggles.insert(key, true);
        } else {
            self.hidden_toggles.remove(&key);
        }
    }

    /// Get all unique toggle groups from a device's visuals
    pub fn get_toggle_groups(visuals: &[VisualData]) -> Vec<String> {
        let mut groups: Vec<String> = visuals
            .iter()
            .filter_map(|v| v.toggle.clone())
            .collect();
        groups.sort();
        groups.dedup();
        groups
    }

    /// Check if sensors (FOV) should be shown for a specific device
    pub fn show_sensors_for(&self, device_id: &str) -> bool {
        self.device_sensors.get(device_id).copied().unwrap_or(false)
    }

    /// Set sensor (FOV) visibility for a specific device
    pub fn set_show_sensors(&mut self, device_id: &str, show: bool) {
        self.device_sensors.insert(device_id.to_string(), show);
    }

    /// Check if ports should be shown for a specific device
    pub fn show_ports_for(&self, device_id: &str) -> bool {
        self.device_ports.get(device_id).copied().unwrap_or(false)
    }

    /// Set port visibility for a specific device
    pub fn set_show_ports(&mut self, device_id: &str, show: bool) {
        self.device_ports.insert(device_id.to_string(), show);
    }

    /// Check if antennas should be shown for a specific device
    pub fn show_antennas_for(&self, device_id: &str) -> bool {
        self.device_antennas.get(device_id).copied().unwrap_or(false)
    }

    /// Set antenna visibility for a specific device
    pub fn set_show_antennas(&mut self, device_id: &str, show: bool) {
        self.device_antennas.insert(device_id.to_string(), show);
    }

    /// Check if a sensor should show axis-aligned view (default: true)
    pub fn is_sensor_axis_aligned(&self, device_id: &str, sensor_name: &str) -> bool {
        self.sensor_axis_aligned
            .get(&(device_id.to_string(), sensor_name.to_string()))
            .copied()
            .unwrap_or(true) // Default: show aligned
    }

    /// Set whether a sensor should show axis-aligned view
    pub fn set_sensor_axis_aligned(&mut self, device_id: &str, sensor_name: &str, aligned: bool) {
        let key = (device_id.to_string(), sensor_name.to_string());
        if aligned {
            // Default is aligned, so remove from map
            self.sensor_axis_aligned.remove(&key);
        } else {
            self.sensor_axis_aligned.insert(key, false);
        }
    }

    /// Check if a specific sensor's FOV should be shown (default: true)
    pub fn is_sensor_fov_visible(&self, device_id: &str, sensor_name: &str) -> bool {
        self.sensor_fov_visible
            .get(&(device_id.to_string(), sensor_name.to_string()))
            .copied()
            .unwrap_or(true) // Default: show FOV
    }

    /// Set whether a specific sensor's FOV should be shown
    pub fn set_sensor_fov_visible(&mut self, device_id: &str, sensor_name: &str, visible: bool) {
        let key = (device_id.to_string(), sensor_name.to_string());
        if visible {
            // Default is visible, so remove from map
            self.sensor_fov_visible.remove(&key);
        } else {
            self.sensor_fov_visible.insert(key, false);
        }
    }

    /// Check if a specific named frame should be shown (default: true)
    pub fn is_frame_visible(&self, device_id: &str, frame_name: &str) -> bool {
        self.frame_visible
            .get(&(device_id.to_string(), frame_name.to_string()))
            .copied()
            .unwrap_or(true) // Default: show frame
    }

    /// Set whether a specific named frame should be shown
    pub fn set_frame_visible(&mut self, device_id: &str, frame_name: &str, visible: bool) {
        let key = (device_id.to_string(), frame_name.to_string());
        if visible {
            // Default is visible, so remove from map
            self.frame_visible.remove(&key);
        } else {
            self.frame_visible.insert(key, false);
        }
    }

    /// Check if a specific sensor's axis frame should be shown (default: true)
    pub fn is_sensor_axis_visible(&self, device_id: &str, sensor_name: &str) -> bool {
        self.sensor_axis_visible
            .get(&(device_id.to_string(), sensor_name.to_string()))
            .copied()
            .unwrap_or(true) // Default: show axis
    }

    /// Set whether a specific sensor's axis frame should be shown
    pub fn set_sensor_axis_visible(&mut self, device_id: &str, sensor_name: &str, visible: bool) {
        let key = (device_id.to_string(), sensor_name.to_string());
        if visible {
            // Default is visible, so remove from map
            self.sensor_axis_visible.remove(&key);
        } else {
            self.sensor_axis_visible.insert(key, false);
        }
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
    /// Render scale factor (1.0 = native, 0.5 = half resolution for performance)
    pub render_scale: f32,
    // Track previous values to detect specific changes
    prev_spacing: f32,
    prev_thickness: f32,
    prev_alpha: f32,
}

impl Default for WorldSettings {
    fn default() -> Self {
        Self {
            show_grid: true,
            show_axis: true,
            grid_spacing: 0.1, // 10cm default spacing
            grid_line_thickness: 0.0002, // 0.2mm default thickness
            grid_alpha: 0.5, // 50% transparent by default
            render_scale: 1.0, // Native resolution by default
            prev_spacing: 0.1,
            prev_thickness: 0.0002,
            prev_alpha: 0.5,
        }
    }
}

impl WorldSettings {
    /// Check if grid geometry needs to be regenerated (spacing, thickness, or alpha changed)
    pub fn needs_grid_regeneration(&self) -> bool {
        self.grid_spacing != self.prev_spacing ||
        self.grid_line_thickness != self.prev_thickness ||
        self.grid_alpha != self.prev_alpha
    }

    /// Mark current values as previous (call after regeneration)
    pub fn mark_grid_regenerated(&mut self) {
        self.prev_spacing = self.grid_spacing;
        self.prev_thickness = self.grid_line_thickness;
        self.prev_alpha = self.grid_alpha;
    }
}

/// UI layout settings for responsive design
#[derive(Debug, Clone, Resource)]
pub struct UiLayout {
    /// Whether the left panel (device list) is visible
    pub show_left_panel: bool,
    /// Whether the right panel (device details) is visible
    pub show_right_panel: bool,
    /// Current screen width
    pub screen_width: f32,
    /// Current screen height
    pub screen_height: f32,
    /// Whether we're on a small screen (mobile/tablet)
    pub is_mobile: bool,
    /// Scale factor for UI elements on mobile
    pub ui_scale: f32,
}

impl Default for UiLayout {
    fn default() -> Self {
        Self {
            show_left_panel: true,
            show_right_panel: true,
            screen_width: 1920.0,
            screen_height: 1080.0,
            is_mobile: false,
            ui_scale: 1.0,
        }
    }
}

impl UiLayout {
    /// Update layout based on screen dimensions
    pub fn update_for_screen(&mut self, width: f32, height: f32) {
        self.screen_width = width;
        self.screen_height = height;

        // Consider mobile if width < 800 or if it's a portrait orientation with width < 600
        let was_mobile = self.is_mobile;
        self.is_mobile = width < 800.0 || (width < height && width < 600.0);

        // On first detection of mobile mode, close the left panel
        if self.is_mobile && !was_mobile {
            self.show_left_panel = false;
        }

        // Keep scale at 1.0 for mobile - smaller, more compact UI
        self.ui_scale = 1.0;
    }

    /// Get the width for the left panel (device list)
    pub fn panel_width(&self) -> f32 {
        if self.is_mobile {
            // On mobile, panel is ~45% of screen width for compact display
            (self.screen_width * 0.45).min(200.0)
        } else {
            250.0
        }
    }

    /// Get the width for the right panel (device details) - narrower on mobile
    pub fn right_panel_width(&self) -> f32 {
        if self.is_mobile {
            // On mobile, narrower panel - wide enough for labels + input boxes with suffix
            // Label (~40px) + input box with " m" suffix (~100px) + padding (~40px) = ~180px
            180.0
        } else {
            300.0
        }
    }
}

/// Graph visualization overlay state
#[derive(Debug, Clone, Resource)]
pub struct GraphVisualization {
    /// Whether the graph overlay is shown
    pub show: bool,
    /// Pan offset for scrolling the graph
    pub pan_offset: [f32; 2],
    /// Zoom level (1.0 = 100%)
    pub zoom: f32,
    /// Currently hovered node ID
    pub hovered_node: Option<String>,
    /// Cached topology data
    pub topology: Option<TopologyData>,
}

/// Topology data for graph visualization
#[derive(Debug, Clone)]
pub struct TopologyData {
    pub nodes: Vec<TopologyNode>,
    pub root: Option<String>,
}

/// A node in the topology graph
#[derive(Debug, Clone)]
pub struct TopologyNode {
    pub id: String,
    pub name: String,
    pub board: Option<String>,
    pub is_parent: bool,
    pub port: Option<u8>,
    pub children: Vec<String>,
}

impl Default for GraphVisualization {
    fn default() -> Self {
        Self {
            show: false,
            pan_offset: [0.0, 0.0],
            zoom: 1.0,
            hovered_node: None,
            topology: None,
        }
    }
}

/// Run the Bevy application
pub fn run() {
    App::new()
        .insert_resource(ClearColor(Color::srgb(0.1, 0.1, 0.15))) // Dark blue-gray background
        // Start with default continuous rendering - mobile will switch to power-saving mode
        .insert_resource(WinitSettings::default())
        // Bevy 0.17+ has built-in https:// asset loading via the "https" feature
        .add_plugins(DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Dendrite HCDF Viewer".to_string(),
                    canvas: Some("#viewer-canvas".to_string()),
                    fit_canvas_to_parent: true,
                    prevent_default_event_handling: false,
                    ..default()
                }),
                ..default()
            })
            .set(AssetPlugin {
                // Load assets from root (daemon serves /models directly)
                file_path: "".to_string(),
                // Don't look for .meta files - server doesn't have them
                meta_check: bevy::asset::AssetMetaCheck::Never,
                ..default()
            })
        )
        // Add bevy_picking from the crate (required for bevy_egui picking feature)
        // DefaultPickingPlugins provides core picking (PointerInputPlugin, PickingPlugin, InteractionPlugin)
        // MeshPickingPlugin must be added separately for 3D mesh raycasting
        // These must be added BEFORE EguiPlugin so it can detect PickingPlugin
        .add_plugins(DefaultPickingPlugins)
        .add_plugins(MeshPickingPlugin)
        .add_plugins(EguiPlugin::default())
        .init_resource::<DeviceRegistry>()
        .init_resource::<SelectedDevice>()
        .init_resource::<CameraSettings>()
        .init_resource::<DevicePositions>()
        .init_resource::<DeviceOrientations>()
        .init_resource::<ActiveRotationField>()
        .init_resource::<ShowRotationAxis>()
        .init_resource::<FrameVisibility>()
        .init_resource::<WorldSettings>()
        .init_resource::<UiLayout>()
        .init_resource::<GraphVisualization>()
        .add_plugins(FilePickerPlugin)
        .add_plugins(ScenePlugin)
        .add_plugins(ModelsPlugin)
        .add_plugins(UiPlugin)
        .add_systems(Update, (
            adjust_power_settings_for_mobile,
            apply_render_scale,
        ))
        .run();
}

/// Adjust power settings based on mobile detection
/// On mobile, use power saving mode. On desktop, use continuous rendering for smooth 3D.
fn adjust_power_settings_for_mobile(
    layout: Res<UiLayout>,
    mut winit_settings: ResMut<WinitSettings>,
) {
    // Only update if mobile status changed
    if !layout.is_changed() {
        return;
    }

    if layout.is_mobile {
        // Mobile: Power saving - reactive rendering with low idle rate
        use bevy::winit::UpdateMode;
        winit_settings.focused_mode = UpdateMode::reactive_low_power(Duration::from_millis(100)); // 10 FPS max when idle
        winit_settings.unfocused_mode = UpdateMode::reactive_low_power(Duration::from_millis(500)); // 2 FPS when unfocused
    } else {
        // Desktop: Continuous rendering for smooth 3D interaction
        *winit_settings = WinitSettings::default();
    }
}

/// Apply render scale - currently disabled as scale_factor_override doesn't work correctly in WASM
/// TODO: Implement proper render-to-texture scaling for mobile performance
fn apply_render_scale(
    _world_settings: Res<WorldSettings>,
    _windows: Query<&mut Window>,
) {
    // scale_factor_override causes rendering to only fill part of the canvas in WASM
    // Proper implementation would require render-to-texture with upscaling
    // For now, this is a no-op - hiding the slider from UI until properly implemented
}
