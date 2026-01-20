//! Dendrite Scene - Shared 3D rendering and UI components
//!
//! This crate provides the core 3D visualization functionality used by
//! both the daemon-connected viewer (dendrite-web) and the standalone
//! HCDF viewer (dendrite-viewer).

pub mod camera;
pub mod models;
pub mod scene;
pub mod types;
pub mod ui;

use bevy::prelude::*;

/// Plugin that sets up the shared 3D scene components
pub struct DendriteScenePlugin;

impl Plugin for DendriteScenePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(camera::CameraPlugin)
            .add_plugins(scene::SceneSetupPlugin)
            .add_plugins(models::ModelsPlugin);
    }
}

// Re-export commonly used types
pub use types::*;
pub use camera::CameraSettings;
