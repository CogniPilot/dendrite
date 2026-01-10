//! Bevy application setup

use bevy::prelude::*;
use bevy_egui::EguiPlugin;
use bevy_picking::{DefaultPickingPlugins, prelude::MeshPickingPlugin};

use crate::models::ModelsPlugin;
use crate::network::NetworkPlugin;
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
    /// Legacy single model path (for backward compatibility)
    pub model_path: Option<String>,
    /// Composite visuals with individual poses
    pub visuals: Vec<VisualData>,
    /// Reference frames for this device
    pub frames: Vec<FrameData>,
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

/// Frame and visual visibility settings (per-device)
#[derive(Debug, Clone, Resource, Default)]
pub struct FrameVisibility {
    /// Per-device frame visibility (device_id -> show_frames)
    pub device_frames: std::collections::HashMap<String, bool>,
    /// Currently hovered frame (device_id:frame_name)
    pub hovered_frame: Option<String>,
    /// Per-device, per-toggle-group hidden state: (device_id, toggle_group) -> is_hidden
    /// Default is visible (not hidden), so only hidden groups are tracked
    pub hidden_toggles: std::collections::HashMap<(String, String), bool>,
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
}

/// World visualization settings
#[derive(Debug, Clone, Resource)]
pub struct WorldSettings {
    pub show_grid: bool,
    pub show_axis: bool,
    pub grid_spacing: f32,
    pub grid_line_thickness: f32,
    pub grid_alpha: f32,
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
        self.is_mobile = width < 800.0 || (width < height && width < 600.0);

        // Scale up UI elements on mobile for better touch targets
        self.ui_scale = if self.is_mobile { 1.3 } else { 1.0 };
    }

    /// Get the width for side panels
    pub fn panel_width(&self) -> f32 {
        if self.is_mobile {
            // On mobile, panels take more of the screen when shown
            (self.screen_width * 0.85).min(350.0)
        } else {
            250.0
        }
    }
}

/// Connection dialog state for remote daemon configuration
#[derive(Debug, Clone, Resource)]
pub struct ConnectionDialog {
    /// Whether the connection dialog is shown
    pub show: bool,
    /// Input field for daemon address (e.g., "192.168.1.100:8080")
    pub daemon_address: String,
    /// Error message if connection failed
    pub error: Option<String>,
}

impl Default for ConnectionDialog {
    fn default() -> Self {
        Self {
            show: false,
            daemon_address: String::new(),
            error: None,
        }
    }
}

/// Run the Bevy application
pub fn run() {
    App::new()
        .insert_resource(ClearColor(Color::srgb(0.1, 0.1, 0.15))) // Dark blue-gray background
        // Bevy 0.17+ has built-in https:// asset loading via the "https" feature
        .add_plugins(DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Dendrite - Hardware Visualization".to_string(),
                    canvas: Some("#dendrite-canvas".to_string()),
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
        .init_resource::<FrameVisibility>()
        .init_resource::<WorldSettings>()
        .init_resource::<UiLayout>()
        .init_resource::<ConnectionDialog>()
        .add_plugins(NetworkPlugin)
        .add_plugins(ScenePlugin)
        .add_plugins(ModelsPlugin)
        .add_plugins(UiPlugin)
        .run();
}
