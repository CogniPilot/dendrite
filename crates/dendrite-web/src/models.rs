//! glTF model loading and management

use bevy::asset::LoadState;
use bevy::prelude::*;
use std::collections::HashMap;

use crate::app::{DeviceRegistry, DeviceStatus, FrameVisibility, VisualData};
use crate::scene::DeviceEntity;

/// Component marking a visual child entity
#[derive(Component)]
pub struct VisualEntity {
    /// Parent device ID
    pub device_id: String,
    /// Visual name
    pub visual_name: String,
    /// Toggle group name (if any)
    pub toggle: Option<String>,
}

pub struct ModelsPlugin;

impl Plugin for ModelsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ModelCache>()
            .add_systems(Update, load_models)
            .add_systems(Update, sync_device_entities.after(load_models))
            .add_systems(Update, update_visual_visibility.after(sync_device_entities));
    }
}

/// Cache of loaded model handles
#[derive(Resource, Default)]
pub struct ModelCache {
    pub models: HashMap<String, Handle<Scene>>,
    pub loading: HashMap<String, Handle<Gltf>>,
    pub ready: HashMap<String, bool>,
}

/// Check loading state and extract scenes from loaded GLTFs
fn load_models(
    mut model_cache: ResMut<ModelCache>,
    asset_server: Res<AssetServer>,
    gltf_assets: Res<Assets<Gltf>>,
) {
    // Check each loading GLTF
    let loading_keys: Vec<String> = model_cache.loading.keys().cloned().collect();
    for key in loading_keys {
        let handle = model_cache.loading.get(&key).unwrap();

        match asset_server.get_load_state(handle.id()) {
            Some(LoadState::Loaded) => {
                // GLTF is loaded, extract the default scene
                if let Some(gltf) = gltf_assets.get(handle) {
                    if let Some(scene_handle) = gltf.default_scene.clone() {
                        tracing::info!("Model loaded: {}", key);
                        model_cache.models.insert(key.clone(), scene_handle);
                        model_cache.ready.insert(key.clone(), true);
                    } else if !gltf.scenes.is_empty() {
                        // Use first scene if no default
                        let scene_handle = gltf.scenes[0].clone();
                        tracing::info!("Model loaded (first scene): {}", key);
                        model_cache.models.insert(key.clone(), scene_handle);
                        model_cache.ready.insert(key.clone(), true);
                    }
                }
                model_cache.loading.remove(&key);
            }
            Some(LoadState::Failed(_)) => {
                tracing::error!("Failed to load model: {}", key);
                model_cache.loading.remove(&key);
                model_cache.ready.insert(key, false);
            }
            _ => {
                // Still loading
            }
        }
    }
}

/// Sync device entities with the registry
fn sync_device_entities(
    mut commands: Commands,
    registry: Res<DeviceRegistry>,
    mut model_cache: ResMut<ModelCache>,
    asset_server: Res<AssetServer>,
    existing_devices: Query<(Entity, &DeviceEntity)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Collect existing device IDs
    let existing_ids: HashMap<String, Entity> = existing_devices
        .iter()
        .map(|(e, d)| (d.device_id.clone(), e))
        .collect();

    // Registry device IDs
    let registry_ids: std::collections::HashSet<String> = registry
        .devices
        .iter()
        .map(|d| d.id.clone())
        .collect();

    // Remove devices no longer in registry
    for (id, entity) in &existing_ids {
        if id != "parent" && !registry_ids.contains(id) {
            commands.entity(*entity).despawn();
        }
    }

    // Add or update devices
    // ENU coordinate system: X=East, Y=North, Z=Up
    for device in &registry.devices {
        let position = device
            .position
            .map(|p| Vec3::new(p[0] as f32, p[1] as f32, 0.01)) // ENU: X, Y on ground, Z slightly above
            .unwrap_or_else(|| {
                // Auto-arrange in circle on X-Y plane if no position
                let idx = registry.devices.iter().position(|d| d.id == device.id).unwrap_or(0);
                let angle = (idx as f32) * std::f32::consts::TAU / registry.devices.len().max(1) as f32;
                Vec3::new(0.15 * angle.cos(), 0.15 * angle.sin(), 0.01) // ENU: X-Y plane, Z=0.01
            });

        if existing_ids.contains_key(&device.id) {
            // Already spawned
            continue;
        }

        // Spawn new device entity
        let color = match device.status {
            DeviceStatus::Online => Color::srgb(0.2, 0.8, 0.3),
            DeviceStatus::Offline => Color::srgb(0.8, 0.2, 0.2),
            DeviceStatus::Unknown => Color::srgb(0.5, 0.5, 0.5),
        };

        // Check if device has composite visuals
        if !device.visuals.is_empty() {
            // Check if all visuals are ready (loaded or failed)
            let all_ready = device.visuals.iter().all(|v| {
                if let Some(ref model_path) = v.model_path {
                    let asset_path = normalize_model_path(model_path);
                    model_cache.ready.contains_key(&asset_path) || model_cache.models.contains_key(&asset_path)
                } else {
                    true // No model = ready
                }
            });

            // Start loading any visuals that aren't loading yet
            for visual in &device.visuals {
                if let Some(ref model_path) = visual.model_path {
                    let asset_path = normalize_model_path(model_path);
                    if !model_cache.loading.contains_key(&asset_path)
                        && !model_cache.models.contains_key(&asset_path)
                        && !model_cache.ready.contains_key(&asset_path)
                    {
                        tracing::info!("Starting to load visual model: {}", asset_path);
                        let handle: Handle<Gltf> = asset_server.load(&asset_path);
                        model_cache.loading.insert(asset_path.clone(), handle);
                    }
                }
            }

            // If not all ready, wait for next frame
            if !all_ready {
                continue;
            }

            // Spawn parent entity for the device
            let parent_entity = commands.spawn((
                Transform::from_translation(position),
                Visibility::default(),
                DeviceEntity {
                    device_id: device.id.clone(),
                },
            )).id();

            // Spawn child entities for each visual
            for visual in &device.visuals {
                let visual_transform = visual_to_transform(visual);

                if let Some(ref model_path) = visual.model_path {
                    let asset_path = normalize_model_path(model_path);
                    if let Some(scene_handle) = model_cache.models.get(&asset_path) {
                        tracing::info!("Spawning visual {} for device {}", visual.name, device.id);
                        let child = commands.spawn((
                            SceneRoot(scene_handle.clone()),
                            visual_transform,
                            VisualEntity {
                                device_id: device.id.clone(),
                                visual_name: visual.name.clone(),
                                toggle: visual.toggle.clone(),
                            },
                        )).id();
                        commands.entity(parent_entity).add_child(child);
                    }
                }
            }

            continue;
        }

        // Legacy: If device has a single model_path, try to load it from the server
        if let Some(ref model_path) = device.model_path {
            let asset_path = normalize_model_path(model_path);

            // Start loading if not already loading or loaded
            if !model_cache.loading.contains_key(&asset_path)
                && !model_cache.models.contains_key(&asset_path)
                && !model_cache.ready.contains_key(&asset_path)
            {
                tracing::info!("Starting to load model: {}", asset_path);
                let handle: Handle<Gltf> = asset_server.load(&asset_path);
                model_cache.loading.insert(asset_path.clone(), handle);
            }

            // If model is ready, spawn with scene
            if let Some(scene_handle) = model_cache.models.get(&asset_path) {
                tracing::info!("Spawning device {} with model", device.id);
                commands.spawn((
                    SceneRoot(scene_handle.clone()),
                    Transform::from_translation(position)
                        .with_scale(Vec3::splat(1.0)), // Scale 1.0 - models should be in meters
                    DeviceEntity {
                        device_id: device.id.clone(),
                    },
                ));
                continue;
            }

            // If still loading, don't spawn yet (will spawn on next frame when ready)
            if model_cache.loading.contains_key(&asset_path) {
                continue;
            }
        }

        // Fallback: spawn a colored cube (or model failed to load)
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.03, 0.015, 0.02))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: color,
                ..default()
            })),
            Transform::from_translation(position),
            DeviceEntity {
                device_id: device.id.clone(),
            },
        ));
    }
}

/// Normalize model path for asset loading
fn normalize_model_path(path: &str) -> String {
    // If it's an absolute URL, return as-is (Bevy can load from HTTP)
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_string();
    }

    // Strip leading slash
    let path = path.trim_start_matches('/');

    // Ensure it starts with "models/" for local paths
    if path.starts_with("models/") {
        path.to_string()
    } else {
        format!("models/{}", path)
    }
}

/// Convert visual pose to Transform
/// Pose is [x, y, z, roll, pitch, yaw] in meters/radians
fn visual_to_transform(visual: &VisualData) -> Transform {
    if let Some(pose) = visual.pose {
        let translation = Vec3::new(pose[0] as f32, pose[1] as f32, pose[2] as f32);
        // SDF convention: roll, pitch, yaw (extrinsic XYZ = intrinsic ZYX)
        let rotation = Quat::from_euler(
            EulerRot::ZYX,
            pose[5] as f32, // yaw (Z)
            pose[4] as f32, // pitch (Y)
            pose[3] as f32, // roll (X)
        );
        Transform::from_translation(translation).with_rotation(rotation)
    } else {
        Transform::IDENTITY
    }
}

/// Update visibility of visual entities based on toggle state
fn update_visual_visibility(
    frame_visibility: Res<FrameVisibility>,
    mut visuals: Query<(&VisualEntity, &mut Visibility)>,
) {
    for (visual_entity, mut visibility) in visuals.iter_mut() {
        // Only check visuals that have a toggle group
        if let Some(ref toggle_group) = visual_entity.toggle {
            let should_hide = frame_visibility.is_toggle_hidden(&visual_entity.device_id, toggle_group);
            *visibility = if should_hide {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
        }
    }
}
