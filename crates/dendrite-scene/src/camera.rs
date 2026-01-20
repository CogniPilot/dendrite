//! Camera controls and orbit navigation

use bevy::prelude::*;

/// Camera controller settings
#[derive(Debug, Clone, Resource)]
pub struct CameraSettings {
    pub distance: f32,
    pub target_distance: f32,
    pub azimuth: f32,
    pub elevation: f32,
    pub target: Vec3,
    pub target_focus: Vec3,
    pub sensitivity: f32,
    pub zoom_speed: f32,
    pub smooth_factor: f32,
}

impl Default for CameraSettings {
    fn default() -> Self {
        Self {
            distance: 0.6,
            target_distance: 0.6,
            azimuth: 0.8,
            elevation: 0.5,
            target: Vec3::ZERO,
            target_focus: Vec3::ZERO,
            sensitivity: 0.005,
            zoom_speed: 0.1,
            smooth_factor: 0.15,
        }
    }
}

/// Marker component for the main camera
#[derive(Component)]
pub struct MainCamera;

/// Plugin for camera controls
pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraSettings>();
        // Camera update systems will be added here
    }
}
