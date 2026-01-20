//! 3D model loading and device visualization

use bevy::prelude::*;

/// Marker component for device entities
#[derive(Component)]
pub struct DeviceEntity {
    pub device_id: String,
}

/// Marker component for visual mesh entities
#[derive(Component)]
pub struct VisualEntity {
    pub device_id: String,
    pub visual_name: String,
    pub toggle_group: Option<String>,
}

/// Marker component for frame gizmo entities
#[derive(Component)]
pub struct FrameGizmoEntity {
    pub device_id: String,
    pub frame_name: String,
}

/// Marker component for sensor FOV entities
#[derive(Component)]
pub struct SensorFovEntity {
    pub device_id: String,
    pub sensor_name: String,
    pub fov_name: String,
}

/// Marker component for sensor axis gizmo entities
#[derive(Component)]
pub struct SensorAxisEntity {
    pub device_id: String,
    pub sensor_name: String,
}

/// Plugin for model loading
pub struct ModelsPlugin;

impl Plugin for ModelsPlugin {
    fn build(&self, _app: &mut App) {
        // Model loading systems will be added here
    }
}
