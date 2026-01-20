//! Scene setup - lights, grid, axis, and environment

use bevy::prelude::*;

/// Marker component for the main directional light
#[derive(Component)]
pub struct MainDirectionalLight;

/// Marker component for grid lines
#[derive(Component)]
pub struct GridLine;

/// Marker component for world axis visualization
#[derive(Component)]
pub struct WorldAxis;

/// Plugin for scene setup
pub struct SceneSetupPlugin;

impl Plugin for SceneSetupPlugin {
    fn build(&self, app: &mut App) {
        // Scene setup systems will be added here
    }
}
