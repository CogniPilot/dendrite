//! glTF model loading and management

use bevy::asset::LoadState;
use bevy::ecs::schedule::ApplyDeferred;
use bevy::prelude::*;
use std::collections::HashMap;

use crate::app::{AxisAlignData, DeviceRegistry, DeviceStatus, FrameVisibility, GeometryData, PortData, SensorData, VisualData};
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
    /// Model path (for debugging)
    pub model_path: Option<String>,
}

/// Marker component for entities that should be excluded from bounding box calculations
/// (sensors, ports, FOV geometry, etc.)
#[derive(Component)]
pub struct ExcludeFromBounds;

/// Whether the sensor axis frame shows aligned or raw axes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensorAxisMode {
    /// Show axes according to axis_align transformation (default view)
    Aligned,
    /// Show raw physical axes (standard XYZ)
    Raw,
}

/// Component marking a sensor axis frame entity (XYZ gizmo showing sensor coordinate frame)
/// These are shown with "Show Reference Frames" toggle since sensors are a form of reference frame
#[derive(Component)]
pub struct SensorAxisEntity {
    /// Parent device ID
    pub device_id: String,
    /// Sensor name
    pub sensor_name: String,
    /// Sensor category (inertial, optical, etc.)
    pub category: String,
    /// Sensor type (optical_flow, tof, etc.)
    pub sensor_type: String,
    /// Driver name
    pub driver: Option<String>,
    /// Axis alignment info for tooltip
    pub axis_align: Option<AxisAlignData>,
    /// Whether this is the aligned or raw axis frame
    pub mode: SensorAxisMode,
}

/// Component marking a sensor FOV geometry entity (cone or frustum)
/// These are shown with "Show Sensors" toggle
#[derive(Component)]
pub struct SensorFovEntity {
    /// Parent device ID
    pub device_id: String,
    /// Sensor name
    pub sensor_name: String,
    /// FOV name (e.g., "emitter", "collector") - None for legacy geometry
    pub fov_name: Option<String>,
    /// Sensor category (inertial, optical, etc.)
    pub category: String,
    /// Sensor type (optical_flow, tof, etc.)
    pub sensor_type: String,
    /// Driver name
    pub driver: Option<String>,
    /// Axis alignment info for tooltip
    pub axis_align: Option<AxisAlignData>,
}

/// Component marking a port visualization entity
#[derive(Component)]
pub struct PortEntity {
    /// Parent device ID
    pub device_id: String,
    /// Port name
    pub port_name: String,
    /// Port type
    pub port_type: String,
}

/// Component marking a port geometry entity (highlight box)
#[derive(Component)]
pub struct PortGeometryEntity {
    /// Parent device ID
    pub device_id: String,
    /// Port name
    pub port_name: String,
}

/// Component to tag GLTF mesh nodes with their name for port highlighting
#[derive(Component, Debug, Clone)]
pub struct GltfNodeName {
    pub name: String,
}

/// Component to mark a mesh as being a port target for highlighting
#[derive(Component, Debug)]
pub struct PortMeshTarget {
    /// Parent device ID
    pub device_id: String,
    /// Port name this mesh represents
    pub port_name: String,
    /// Port type for coloring
    pub port_type: String,
}

pub struct ModelsPlugin;

impl Plugin for ModelsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ModelCache>()
            .init_resource::<SensorPortCache>()
            .init_resource::<PendingPortMeshes>()
            .add_systems(Update, load_models)
            .add_systems(Update, sync_device_entities.after(load_models))
            .add_systems(Update, sync_sensor_entities.after(sync_device_entities))
            .add_systems(Update, sync_port_entities.after(sync_device_entities))
            .add_systems(Update, (
                tag_gltf_node_names.after(sync_device_entities),
                ApplyDeferred,
                link_port_meshes,
            ).chain())
            .add_systems(Update, update_visual_visibility.after(sync_device_entities))
            .add_systems(Update, update_sensor_axis_visibility.after(sync_sensor_entities))
            .add_systems(Update, update_sensor_axis_hover_alpha.after(update_sensor_axis_visibility))
            .add_systems(Update, update_sensor_fov_visibility.after(sync_sensor_entities))
            .add_systems(Update, update_sensor_fov_hover_alpha.after(update_sensor_fov_visibility))
            .add_systems(Update, update_port_visibility.after(sync_port_entities))
            .add_systems(Update, update_port_mesh_highlighting.after(link_port_meshes));
    }
}

/// Cache of loaded model handles
#[derive(Resource, Default)]
pub struct ModelCache {
    pub models: HashMap<String, Handle<Scene>>,
    pub loading: HashMap<String, Handle<Gltf>>,
    pub ready: HashMap<String, bool>,
}

/// Cache to track which sensors/ports have been spawned for each device
#[derive(Resource, Default)]
pub struct SensorPortCache {
    /// Set of (device_id, sensor_name) pairs that have been spawned
    pub spawned_sensors: std::collections::HashSet<(String, String)>,
    /// Set of (device_id, port_name) pairs that have been spawned
    pub spawned_ports: std::collections::HashSet<(String, String)>,
}

/// Pending port mesh assignment
#[derive(Debug, Clone)]
pub struct PendingPortMesh {
    pub device_id: String,
    pub port_name: String,
    /// Visual name containing the mesh (e.g., "board")
    pub visual_name: Option<String>,
    /// Mesh node name within the visual (e.g., "port_eth0")
    pub mesh_name: String,
    pub port_type: String,
}

/// Tracks pending port mesh assignments
/// Used to link GLTF meshes to ports after they're spawned
#[derive(Resource, Default)]
pub struct PendingPortMeshes {
    pub pending: Vec<PendingPortMesh>,
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
                        tracing::info!("Spawning visual {} for device {} from {}", visual.name, device.id, asset_path);
                        let child = commands.spawn((
                            SceneRoot(scene_handle.clone()),
                            visual_transform,
                            VisualEntity {
                                device_id: device.id.clone(),
                                visual_name: visual.name.clone(),
                                toggle: visual.toggle.clone(),
                                model_path: Some(asset_path.clone()),
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

/// Sync sensor entities with the registry - creates axis frames and FOV geometry
/// - Axis frames are created for ALL sensors (controlled by "Show Reference Frames")
/// - FOV geometry is created only for sensors with geometry (controlled by "Show Sensors")
fn sync_sensor_entities(
    mut commands: Commands,
    registry: Res<DeviceRegistry>,
    device_query: Query<(Entity, &DeviceEntity)>,
    mut sensor_port_cache: ResMut<SensorPortCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Build a map of device_id -> entity for parenting
    let device_entities: HashMap<String, Entity> = device_query
        .iter()
        .map(|(e, d)| (d.device_id.clone(), e))
        .collect();

    for device in &registry.devices {
        // Skip if device entity doesn't exist yet
        let Some(&parent_entity) = device_entities.get(&device.id) else {
            continue;
        };

        for sensor in &device.sensors {
            let cache_key = (device.id.clone(), sensor.name.clone());

            // Skip if already spawned
            if sensor_port_cache.spawned_sensors.contains(&cache_key) {
                continue;
            }

            // Mark as spawned
            sensor_port_cache.spawned_sensors.insert(cache_key);

            let sensor_transform = pose_to_transform(sensor.pose);

            // Spawn ALIGNED sensor axis frame (shows axis_align transformation)
            let aligned_entity = spawn_sensor_axis_frame(
                &mut commands,
                &mut meshes,
                &mut materials,
                &device.id,
                sensor,
                sensor_transform,
                SensorAxisMode::Aligned,
            );
            commands.entity(parent_entity).add_child(aligned_entity);

            // Spawn RAW sensor axis frame (shows physical XYZ axes)
            let raw_entity = spawn_sensor_axis_frame(
                &mut commands,
                &mut meshes,
                &mut materials,
                &device.id,
                sensor,
                sensor_transform,
                SensorAxisMode::Raw,
            );
            commands.entity(parent_entity).add_child(raw_entity);

            // Spawn FOV geometry for sensors
            // First, spawn any named FOVs with custom colors and poses
            for fov in &sensor.fovs {
                if let Some(ref geometry) = fov.geometry {
                    // Calculate FOV transform: sensor pose + FOV pose offset
                    let fov_transform = if let Some(fov_pose) = fov.pose {
                        let fov_local = pose_to_transform(Some(fov_pose));
                        Transform::from_matrix(sensor_transform.to_matrix() * fov_local.to_matrix())
                    } else {
                        sensor_transform
                    };

                    let fov_entity = spawn_sensor_fov_with_color(
                        &mut commands,
                        &mut meshes,
                        &mut materials,
                        &device.id,
                        sensor,
                        Some(&fov.name),
                        geometry,
                        fov_transform,
                        fov.color,
                    );
                    commands.entity(parent_entity).add_child(fov_entity);
                }
            }

            // Legacy: spawn single geometry if no fovs defined (backward compatibility)
            if sensor.fovs.is_empty() {
                if let Some(ref geometry) = sensor.geometry {
                    let fov_entity = spawn_sensor_fov_with_color(
                        &mut commands,
                        &mut meshes,
                        &mut materials,
                        &device.id,
                        sensor,
                        None,
                        geometry,
                        sensor_transform,
                        None,
                    );
                    commands.entity(parent_entity).add_child(fov_entity);
                }
            }
        }
    }
}

/// Spawn a sensor axis frame with XYZ axes showing the sensor's coordinate frame
/// Mode determines whether to show aligned (axis_align) or raw (physical) axes
fn spawn_sensor_axis_frame(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    device_id: &str,
    sensor: &SensorData,
    transform: Transform,
    mode: SensorAxisMode,
) -> Entity {
    let axis_length = 0.008; // 8mm axis length
    let axis_radius = 0.0004; // 0.4mm radius
    let cone_radius = 0.0012; // 1.2mm cone radius
    let cone_height = 0.002; // 2mm cone height

    // Colors for axes (with alpha for hover effects)
    let x_color = Color::srgba(1.0, 0.2, 0.2, 1.0); // Red for X
    let y_color = Color::srgba(0.2, 1.0, 0.2, 1.0); // Green for Y
    let z_color = Color::srgba(0.2, 0.2, 1.0, 1.0); // Blue for Z

    // Create materials with alpha blending for hover effects
    let x_material = materials.add(StandardMaterial {
        base_color: x_color,
        emissive: x_color.into(),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let y_material = materials.add(StandardMaterial {
        base_color: y_color,
        emissive: y_color.into(),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let z_material = materials.add(StandardMaterial {
        base_color: z_color,
        emissive: z_color.into(),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });

    // Create cylinder mesh for axes
    let axis_mesh = meshes.add(Cylinder::new(axis_radius, axis_length));
    let cone_mesh = meshes.add(Cone::new(cone_radius, cone_height));

    // Spawn parent entity for the sensor frame
    let parent = commands.spawn((
        transform,
        Visibility::Hidden, // Start hidden, controlled by visibility system
        ExcludeFromBounds, // Exclude from bounding box calculation
        SensorAxisEntity {
            device_id: device_id.to_string(),
            sensor_name: sensor.name.clone(),
            category: sensor.category.clone(),
            sensor_type: sensor.sensor_type.clone(),
            driver: sensor.driver.clone(),
            axis_align: sensor.axis_align.clone(),
            mode,
        },
    )).id();

    // Get axis directions based on mode
    let (x_dir, y_dir, z_dir) = match mode {
        SensorAxisMode::Aligned => {
            // Use axis_align transformation if present, otherwise standard XYZ
            if let Some(ref align) = sensor.axis_align {
                (
                    axis_string_to_vec3(&align.x),
                    axis_string_to_vec3(&align.y),
                    axis_string_to_vec3(&align.z),
                )
            } else {
                (Vec3::X, Vec3::Y, Vec3::Z)
            }
        }
        SensorAxisMode::Raw => {
            // Always use standard XYZ for raw mode
            (Vec3::X, Vec3::Y, Vec3::Z)
        }
    };

    // Spawn X axis (cylinder + cone)
    let x_axis = spawn_axis_with_direction(commands, &axis_mesh, &cone_mesh, x_material.clone(), x_dir, axis_length, cone_height);
    commands.entity(parent).add_child(x_axis);

    // Spawn Y axis
    let y_axis = spawn_axis_with_direction(commands, &axis_mesh, &cone_mesh, y_material.clone(), y_dir, axis_length, cone_height);
    commands.entity(parent).add_child(y_axis);

    // Spawn Z axis
    let z_axis = spawn_axis_with_direction(commands, &axis_mesh, &cone_mesh, z_material.clone(), z_dir, axis_length, cone_height);
    commands.entity(parent).add_child(z_axis);

    parent
}

/// Spawn a single axis (cylinder + cone) pointing in a given direction
fn spawn_axis_with_direction(
    commands: &mut Commands,
    axis_mesh: &Handle<Mesh>,
    cone_mesh: &Handle<Mesh>,
    material: Handle<StandardMaterial>,
    direction: Vec3,
    axis_length: f32,
    cone_height: f32,
) -> Entity {
    // Calculate rotation to point in direction
    // Default cylinder is along Y axis
    let rotation = if direction.abs_diff_eq(Vec3::Y, 0.001) {
        Quat::IDENTITY
    } else if direction.abs_diff_eq(Vec3::NEG_Y, 0.001) {
        Quat::from_rotation_x(std::f32::consts::PI)
    } else {
        Quat::from_rotation_arc(Vec3::Y, direction.normalize())
    };

    let axis_parent = commands.spawn((
        Transform::IDENTITY,
        Visibility::Inherited,
    )).id();

    // Cylinder centered at half the length
    let cylinder = commands.spawn((
        Mesh3d(axis_mesh.clone()),
        MeshMaterial3d(material.clone()),
        Transform::from_translation(direction.normalize() * axis_length / 2.0)
            .with_rotation(rotation),
    )).id();
    commands.entity(axis_parent).add_child(cylinder);

    // Cone at the end
    let cone = commands.spawn((
        Mesh3d(cone_mesh.clone()),
        MeshMaterial3d(material),
        Transform::from_translation(direction.normalize() * (axis_length + cone_height / 2.0))
            .with_rotation(rotation),
    )).id();
    commands.entity(axis_parent).add_child(cone);

    axis_parent
}

/// Convert axis string ("X", "-X", "Y", "-Y", "Z", "-Z") to Vec3
fn axis_string_to_vec3(s: &str) -> Vec3 {
    match s.trim() {
        "X" => Vec3::X,
        "-X" => Vec3::NEG_X,
        "Y" => Vec3::Y,
        "-Y" => Vec3::NEG_Y,
        "Z" => Vec3::Z,
        "-Z" => Vec3::NEG_Z,
        _ => Vec3::X, // Default to X
    }
}

/// Spawn sensor FOV geometry (cone or frustum) with optional custom color
fn spawn_sensor_fov_with_color(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    device_id: &str,
    sensor: &SensorData,
    fov_name: Option<&str>,
    geometry: &GeometryData,
    transform: Transform,
    color: Option<[f32; 3]>,
) -> Entity {
    // Use custom color if provided, otherwise default cyan
    let (r, g, b) = color.map(|c| (c[0], c[1], c[2])).unwrap_or((0.3, 0.8, 1.0));

    // Semi-transparent material for FOV (unlit for consistent color on both sides)
    let fov_material = materials.add(StandardMaterial {
        base_color: Color::srgba(r, g, b, 0.15), // Custom color, very transparent
        alpha_mode: AlphaMode::Blend,
        cull_mode: None, // Show both sides
        unlit: true, // No lighting = consistent color regardless of face direction
        ..default()
    });

    // Create the sensor FOV entity component with full metadata
    let fov_component = SensorFovEntity {
        device_id: device_id.to_string(),
        sensor_name: sensor.name.clone(),
        fov_name: fov_name.map(|s| s.to_string()),
        category: sensor.category.clone(),
        sensor_type: sensor.sensor_type.clone(),
        driver: sensor.driver.clone(),
        axis_align: sensor.axis_align.clone(),
    };

    match geometry {
        GeometryData::Cone { radius, length } => {
            // Cone pointing in +Z direction (forward for optical flow)
            let cone_mesh = meshes.add(Cone::new(*radius as f32, *length as f32));
            commands.spawn((
                Mesh3d(cone_mesh),
                MeshMaterial3d(fov_material),
                // Rotate cone to point +Z (default cone points +Y, so rotate -90° around X)
                transform.with_rotation(
                    transform.rotation * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
                ).with_translation(
                    transform.translation + transform.rotation * Vec3::new(0.0, 0.0, (*length as f32) / 2.0)
                ),
                Visibility::Hidden,
                ExcludeFromBounds,
                fov_component,
            )).id()
        }
        GeometryData::Frustum { near, far, hfov, vfov } |
        GeometryData::PyramidalFrustum { near, far, hfov, vfov } => {
            // Create proper frustum mesh pointing in +Z direction (rectangular cross-section)
            let frustum_mesh = create_frustum_mesh(*near as f32, *far as f32, *hfov as f32, *vfov as f32);

            commands.spawn((
                Mesh3d(meshes.add(frustum_mesh)),
                MeshMaterial3d(fov_material),
                transform,
                Visibility::Hidden,
                ExcludeFromBounds,
                fov_component,
            )).id()
        }
        GeometryData::ConicalFrustum { near, far, fov } => {
            // Create conical frustum mesh pointing in +Z direction (circular cross-section)
            let conical_mesh = create_conical_frustum_mesh(*near as f32, *far as f32, *fov as f32);

            commands.spawn((
                Mesh3d(meshes.add(conical_mesh)),
                MeshMaterial3d(fov_material),
                transform,
                Visibility::Hidden,
                ExcludeFromBounds,
                fov_component,
            )).id()
        }
        _ => {
            // Other geometry types not supported for FOV
            commands.spawn((
                Transform::IDENTITY,
                Visibility::Hidden,
                ExcludeFromBounds,
            )).id()
        }
    }
}

/// Create a frustum mesh for camera/ToF FOV visualization
/// Creates a proper 3D frustum with 8 vertices (4 near plane, 4 far plane) pointing in +Z
fn create_frustum_mesh(near: f32, far: f32, hfov: f32, vfov: f32) -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::Indices;
    use bevy::render::render_resource::PrimitiveTopology;

    // Calculate half-sizes at near and far planes
    let near_half_w = near * (hfov / 2.0).tan();
    let near_half_h = near * (vfov / 2.0).tan();
    let far_half_w = far * (hfov / 2.0).tan();
    let far_half_h = far * (vfov / 2.0).tan();

    // 8 vertices: 4 on near plane, 4 on far plane
    // Near plane (z = near): bottom-left, bottom-right, top-right, top-left
    // Far plane (z = far): bottom-left, bottom-right, top-right, top-left
    let vertices: Vec<[f32; 3]> = vec![
        // Near plane (indices 0-3)
        [-near_half_w, -near_half_h, near], // 0: near bottom-left
        [ near_half_w, -near_half_h, near], // 1: near bottom-right
        [ near_half_w,  near_half_h, near], // 2: near top-right
        [-near_half_w,  near_half_h, near], // 3: near top-left
        // Far plane (indices 4-7)
        [-far_half_w, -far_half_h, far], // 4: far bottom-left
        [ far_half_w, -far_half_h, far], // 5: far bottom-right
        [ far_half_w,  far_half_h, far], // 6: far top-right
        [-far_half_w,  far_half_h, far], // 7: far top-left
    ];

    // Indices for 6 faces (2 triangles each = 12 triangles = 36 indices)
    // Using counter-clockwise winding for front faces
    let indices: Vec<u32> = vec![
        // Near face (pointing towards origin, so clockwise from outside = CCW from inside)
        0, 2, 1,
        0, 3, 2,
        // Far face (pointing away from origin)
        4, 5, 6,
        4, 6, 7,
        // Bottom face
        0, 1, 5,
        0, 5, 4,
        // Top face
        3, 6, 2,
        3, 7, 6,
        // Left face
        0, 4, 7,
        0, 7, 3,
        // Right face
        1, 2, 6,
        1, 6, 5,
    ];

    // Simple normals (pointing outward for each face - we'll use flat shading)
    // For transparency with cull_mode: None, normals are less critical
    let normals: Vec<[f32; 3]> = vec![
        [0.0, 0.0, -1.0], // near vertices point back
        [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0],
        [0.0, 0.0, -1.0],
        [0.0, 0.0, 1.0],  // far vertices point forward
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
    ];

    // UV coordinates (simple planar mapping)
    let uvs: Vec<[f32; 2]> = vec![
        [0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0],
        [0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0],
    ];

    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices))
}

/// Create a conical frustum mesh for circular FOV visualization
/// Creates a truncated cone with circular near and far planes pointing in +Z
fn create_conical_frustum_mesh(near: f32, far: f32, fov: f32) -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::Indices;
    use bevy::render::render_resource::PrimitiveTopology;

    // Number of segments around the circle (higher = smoother)
    const SEGMENTS: usize = 24;

    // Calculate radii at near and far planes based on FOV half-angle
    let near_radius = near * fov.tan();
    let far_radius = far * fov.tan();

    // Generate vertices: near ring, far ring, plus centers
    let mut vertices: Vec<[f32; 3]> = Vec::with_capacity(SEGMENTS * 2 + 2);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(SEGMENTS * 2 + 2);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(SEGMENTS * 2 + 2);

    // Near ring (indices 0..SEGMENTS)
    for i in 0..SEGMENTS {
        let angle = (i as f32 / SEGMENTS as f32) * std::f32::consts::TAU;
        let (sin_a, cos_a) = angle.sin_cos();
        vertices.push([cos_a * near_radius, sin_a * near_radius, near]);
        // Normal points outward and back (simplified)
        normals.push([cos_a, sin_a, -fov.cos()]);
        uvs.push([(i as f32) / SEGMENTS as f32, 0.0]);
    }

    // Far ring (indices SEGMENTS..SEGMENTS*2)
    for i in 0..SEGMENTS {
        let angle = (i as f32 / SEGMENTS as f32) * std::f32::consts::TAU;
        let (sin_a, cos_a) = angle.sin_cos();
        vertices.push([cos_a * far_radius, sin_a * far_radius, far]);
        normals.push([cos_a, sin_a, fov.cos()]);
        uvs.push([(i as f32) / SEGMENTS as f32, 1.0]);
    }

    // Near center (index SEGMENTS*2)
    vertices.push([0.0, 0.0, near]);
    normals.push([0.0, 0.0, -1.0]);
    uvs.push([0.5, 0.0]);

    // Far center (index SEGMENTS*2+1)
    vertices.push([0.0, 0.0, far]);
    normals.push([0.0, 0.0, 1.0]);
    uvs.push([0.5, 1.0]);

    let near_center = SEGMENTS * 2;
    let far_center = SEGMENTS * 2 + 1;

    // Generate indices
    let mut indices: Vec<u32> = Vec::with_capacity(SEGMENTS * 12);

    for i in 0..SEGMENTS {
        let next = (i + 1) % SEGMENTS;

        // Side face quad (2 triangles)
        // Near ring vertex, far ring vertex, far ring next, near ring next
        let n0 = i as u32;
        let n1 = next as u32;
        let f0 = (SEGMENTS + i) as u32;
        let f1 = (SEGMENTS + next) as u32;

        indices.extend_from_slice(&[n0, f0, f1]);
        indices.extend_from_slice(&[n0, f1, n1]);

        // Near cap (triangles from center)
        indices.extend_from_slice(&[near_center as u32, n1, n0]);

        // Far cap (triangles from center)
        indices.extend_from_slice(&[far_center as u32, f0, f1]);
    }

    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices))
}

/// Sync port entities with the registry - creates highlight boxes or links to GLTF meshes
fn sync_port_entities(
    mut commands: Commands,
    registry: Res<DeviceRegistry>,
    device_query: Query<(Entity, &DeviceEntity)>,
    mut sensor_port_cache: ResMut<SensorPortCache>,
    mut pending_port_meshes: ResMut<PendingPortMeshes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let device_entities: HashMap<String, Entity> = device_query
        .iter()
        .map(|(e, d)| (d.device_id.clone(), e))
        .collect();

    for device in &registry.devices {
        let Some(&parent_entity) = device_entities.get(&device.id) else {
            continue;
        };

        for port in &device.ports {
            let cache_key = (device.id.clone(), port.name.clone());

            if sensor_port_cache.spawned_ports.contains(&cache_key) {
                continue;
            }

            sensor_port_cache.spawned_ports.insert(cache_key);

            // If port has mesh_name, register it for GLTF mesh linking (no fallback geometry)
            if let Some(ref mesh_name) = port.mesh_name {
                pending_port_meshes.pending.push(PendingPortMesh {
                    device_id: device.id.clone(),
                    port_name: port.name.clone(),
                    visual_name: port.visual_name.clone(),
                    mesh_name: mesh_name.clone(),
                    port_type: port.port_type.clone(),
                });
                tracing::info!("Registered pending port mesh: {} -> {} (visual: {:?}) for device {}", port.name, mesh_name, port.visual_name, device.id);
            } else {
                // No mesh_name - use geometry-based highlighting
                let port_transform = pose_to_transform(port.pose);

                // Spawn port highlight geometry
                for geometry in &port.geometry {
                    let port_entity = spawn_port_geometry(
                        &mut commands,
                        &mut meshes,
                        &mut materials,
                        &device.id,
                        port,
                        geometry,
                        port_transform,
                    );
                    commands.entity(parent_entity).add_child(port_entity);
                }

                // If no geometry, spawn a default small box
                if port.geometry.is_empty() {
                    let default_geometry = GeometryData::Box { size: [0.005, 0.003, 0.002] };
                    let port_entity = spawn_port_geometry(
                        &mut commands,
                        &mut meshes,
                        &mut materials,
                        &device.id,
                        port,
                        &default_geometry,
                        port_transform,
                    );
                    commands.entity(parent_entity).add_child(port_entity);
                }
            }
        }
    }
}

/// Spawn a port geometry as a transparent highlight
fn spawn_port_geometry(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    device_id: &str,
    port: &PortData,
    geometry: &GeometryData,
    transform: Transform,
) -> Entity {
    // Color based on port type
    let port_color = match port.port_type.to_lowercase().as_str() {
        "ethernet" => Color::srgba(0.2, 0.8, 0.2, 0.4), // Green
        "can" => Color::srgba(1.0, 0.8, 0.2, 0.4),      // Yellow/Orange
        "spi" => Color::srgba(0.8, 0.2, 0.8, 0.4),      // Magenta
        "i2c" => Color::srgba(0.2, 0.8, 0.8, 0.4),      // Cyan
        "uart" => Color::srgba(0.8, 0.4, 0.2, 0.4),     // Orange
        "usb" => Color::srgba(0.2, 0.4, 0.8, 0.4),      // Blue
        _ => Color::srgba(0.5, 0.5, 0.5, 0.4),          // Gray
    };

    let port_material = materials.add(StandardMaterial {
        base_color: port_color,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        ..default()
    });

    let mesh = match geometry {
        GeometryData::Box { size } => {
            meshes.add(Cuboid::new(size[0] as f32, size[1] as f32, size[2] as f32))
        }
        GeometryData::Cylinder { radius, length } => {
            meshes.add(Cylinder::new(*radius as f32, *length as f32))
        }
        GeometryData::Sphere { radius } => {
            meshes.add(Sphere::new(*radius as f32))
        }
        _ => meshes.add(Cuboid::new(0.005, 0.003, 0.002)), // Default
    };

    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(port_material),
        transform,
        Visibility::Hidden,
        ExcludeFromBounds,
        PortEntity {
            device_id: device_id.to_string(),
            port_name: port.name.clone(),
            port_type: port.port_type.clone(),
        },
        PortGeometryEntity {
            device_id: device_id.to_string(),
            port_name: port.name.clone(),
        },
    )).id()
}

/// Convert pose array to Transform
fn pose_to_transform(pose: Option<[f64; 6]>) -> Transform {
    if let Some(p) = pose {
        let translation = Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32);
        let rotation = Quat::from_euler(
            EulerRot::ZYX,
            p[5] as f32, // yaw
            p[4] as f32, // pitch
            p[3] as f32, // roll
        );
        Transform::from_translation(translation).with_rotation(rotation)
    } else {
        Transform::IDENTITY
    }
}

/// Update sensor axis frame visibility based on "Show Reference Frames" toggle
/// Sensor axes are treated as reference frames since they define sensor coordinate systems
fn update_sensor_axis_visibility(
    frame_visibility: Res<FrameVisibility>,
    mut sensor_axes: Query<(&SensorAxisEntity, &mut Visibility)>,
) {
    for (sensor_axis, mut visibility) in sensor_axes.iter_mut() {
        // First check if frames are shown at all for this device
        let frames_visible = frame_visibility.show_frames_for(&sensor_axis.device_id);
        if !frames_visible {
            *visibility = Visibility::Hidden;
            continue;
        }

        // Check if this specific sensor's axis frame is individually visible
        if !frame_visibility.is_sensor_axis_visible(&sensor_axis.device_id, &sensor_axis.sensor_name) {
            *visibility = Visibility::Hidden;
            continue;
        }

        // Check if this sensor should show aligned or raw axes
        let show_aligned = frame_visibility.is_sensor_axis_aligned(
            &sensor_axis.device_id,
            &sensor_axis.sensor_name,
        );

        // Show the axis frame only if its mode matches the current setting
        let should_show = match sensor_axis.mode {
            SensorAxisMode::Aligned => show_aligned,
            SensorAxisMode::Raw => !show_aligned,
        };

        *visibility = if should_show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

/// Update sensor FOV visibility based on "Show Sensors" toggle and per-sensor visibility
fn update_sensor_fov_visibility(
    frame_visibility: Res<FrameVisibility>,
    mut sensor_fovs: Query<(&SensorFovEntity, &mut Visibility)>,
) {
    for (fov, mut visibility) in sensor_fovs.iter_mut() {
        // Check device-level sensor toggle first
        let device_sensors_visible = frame_visibility.show_sensors_for(&fov.device_id);
        if !device_sensors_visible {
            *visibility = Visibility::Hidden;
            continue;
        }

        // Check per-sensor FOV visibility
        let sensor_fov_visible = frame_visibility.is_sensor_fov_visible(&fov.device_id, &fov.sensor_name);
        *visibility = if sensor_fov_visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

/// Update port visibility and highlighting based on device settings and hover state
fn update_port_visibility(
    frame_visibility: Res<FrameVisibility>,
    mut ports: Query<(&PortEntity, &mut Visibility, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (port, mut visibility, material_handle) in ports.iter_mut() {
        let should_show = frame_visibility.show_ports_for(&port.device_id);
        *visibility = if should_show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };

        // Check if this port is hovered
        let port_key = format!("{}:{}", port.device_id, port.port_name);
        let is_hovered = frame_visibility.hovered_port.as_ref() == Some(&port_key);

        // Update material alpha based on hover state
        if let Some(material) = materials.get_mut(&material_handle.0) {
            let base_alpha = 0.4;
            let hovered_alpha = 0.9;
            let target_alpha = if is_hovered { hovered_alpha } else { base_alpha };

            // Get current color and update alpha
            let mut color = material.base_color.to_srgba();
            color.alpha = target_alpha;
            material.base_color = Color::srgba(color.red, color.green, color.blue, color.alpha);
        }
    }
}

/// Update sensor axis material alpha based on UI hover state
/// Default: 50% alpha, hovered: 100%, others when hovered: 10%
fn update_sensor_axis_hover_alpha(
    frame_visibility: Res<FrameVisibility>,
    sensor_axes: Query<(Entity, &SensorAxisEntity, Option<&Children>)>,
    children_query: Query<&Children>,
    mut material_query: Query<&MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let hovered_sensor = &frame_visibility.hovered_sensor_from_ui;

    for (entity, sensor_axis, children_opt) in sensor_axes.iter() {
        let sensor_key = format!("{}:{}", sensor_axis.device_id, sensor_axis.sensor_name);

        // Determine target alpha:
        // - No hover active: 50% (default)
        // - This sensor hovered: 100%
        // - Another sensor hovered: 10%
        let target_alpha = match hovered_sensor {
            None => 0.5,                                      // Default: 50%
            Some(hovered) if hovered == &sensor_key => 1.0,   // Hovered: 100%
            Some(_) => 0.1,                                   // Others: 10%
        };

        // Update all descendant materials if children exist
        if let Some(children) = children_opt {
            update_descendant_materials_alpha(
                entity,
                children,
                &children_query,
                &mut material_query,
                &mut materials,
                target_alpha,
            );
        }
    }
}

/// Update sensor FOV material alpha based on UI hover state
/// Default: 10% alpha, hovered: 20%, others when hovered: 2%
fn update_sensor_fov_hover_alpha(
    frame_visibility: Res<FrameVisibility>,
    mut sensor_fovs: Query<(&SensorFovEntity, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let hovered_sensor = &frame_visibility.hovered_sensor_from_ui;

    for (fov, material_handle) in sensor_fovs.iter_mut() {
        let sensor_key = format!("{}:{}", fov.device_id, fov.sensor_name);

        // Determine target alpha:
        // - No hover active: 10% (default)
        // - This sensor hovered: 20%
        // - Another sensor hovered: 2%
        let target_alpha = match hovered_sensor {
            None => 0.1,                                      // Default: 10%
            Some(hovered) if hovered == &sensor_key => 0.2,   // Hovered: 20%
            Some(_) => 0.02,                                  // Others: 2%
        };

        if let Some(material) = materials.get_mut(&material_handle.0) {
            let mut color = material.base_color.to_srgba();
            color.alpha = target_alpha;
            material.base_color = Color::srgba(color.red, color.green, color.blue, color.alpha);
        }
    }
}

/// Recursively update material alpha for all descendants of an entity
fn update_descendant_materials_alpha(
    _parent: Entity,
    children: &Children,
    children_query: &Query<&Children>,
    material_query: &mut Query<&MeshMaterial3d<StandardMaterial>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    target_alpha: f32,
) {
    for child in children.iter() {
        // Try to update this child's material
        if let Ok(material_handle) = material_query.get(child) {
            if let Some(material) = materials.get_mut(&material_handle.0) {
                // Update alpha while preserving RGB
                let mut color = material.base_color.to_srgba();
                color.alpha = target_alpha;
                material.base_color = Color::srgba(color.red, color.green, color.blue, color.alpha);

                // Also update emissive alpha for unlit materials
                let emissive = material.emissive.to_f32_array_no_alpha();
                material.emissive = bevy::color::LinearRgba::new(
                    emissive[0] * target_alpha,
                    emissive[1] * target_alpha,
                    emissive[2] * target_alpha,
                    1.0,
                );
            }
        }

        // Recurse into grandchildren
        if let Ok(grandchildren) = children_query.get(child) {
            update_descendant_materials_alpha(
                child,
                grandchildren,
                children_query,
                material_query,
                materials,
                target_alpha,
            );
        }
    }
}

/// Tag GLTF entities with their node names from the Name component
/// Bevy's GLTF loader adds Name components based on the node names in the file
/// We tag ALL named entities (not just meshes) so we can find parent nodes like "ETH0"
fn tag_gltf_node_names(
    mut commands: Commands,
    named_entities: Query<(Entity, &Name), Without<GltfNodeName>>,
) {
    for (entity, name) in named_entities.iter() {
        // Tag this entity with its GLTF node name
        commands.entity(entity).insert(GltfNodeName {
            name: name.to_string(),
        });
    }
}

/// Link pending port meshes to their GLTF mesh entities
/// Scopes the search by visual_name when specified
/// Finds named parent nodes (like "ETH0") and tags their child mesh entities
fn link_port_meshes(
    mut commands: Commands,
    mut pending: ResMut<PendingPortMeshes>,
    device_query: Query<(Entity, &DeviceEntity)>,
    visual_query: Query<(Entity, &VisualEntity)>,
    children_query: Query<&Children>,
    node_name_query: Query<(Entity, &GltfNodeName)>,
    mesh_query: Query<Entity, With<Mesh3d>>,
) {
    if pending.pending.is_empty() {
        return;
    }

    // Build device entity map
    let device_entities: HashMap<String, Entity> = device_query
        .iter()
        .map(|(e, d)| (d.device_id.clone(), e))
        .collect();

    // Build a map of (device_id, visual_name) -> (visual entity, model_path)
    let visual_entities: HashMap<(String, String), (Entity, Option<String>)> = visual_query
        .iter()
        .map(|(e, v)| ((v.device_id.clone(), v.visual_name.clone()), (e, v.model_path.clone())))
        .collect();

    // Build a map of all node names to entities for quick lookup
    let node_map: HashMap<String, Entity> = node_name_query
        .iter()
        .map(|(e, n)| (n.name.clone(), e))
        .collect();

    // If no GLTF nodes have been tagged yet, wait for next frame
    // This handles the timing where visuals are spawned but nodes aren't tagged yet
    if node_map.is_empty() {
        return;
    }

    // Process pending port meshes
    let mut remaining = Vec::new();
    for port_mesh in pending.pending.drain(..) {
        // Find the device entity
        let Some(&device_entity) = device_entities.get(&port_mesh.device_id) else {
            remaining.push(port_mesh);
            continue;
        };

        // Determine the search root: either the specific visual or the whole device
        let (search_root, model_path) = if let Some(ref visual_name) = port_mesh.visual_name {
            // Scope search to the specific visual
            let key = (port_mesh.device_id.clone(), visual_name.clone());
            if let Some((visual_entity, model_path)) = visual_entities.get(&key) {
                (*visual_entity, model_path.clone())
            } else {
                // Visual not spawned yet, keep trying
                remaining.push(port_mesh);
                continue;
            }
        } else {
            // No visual specified, search the whole device hierarchy
            (device_entity, None)
        };

        // Look for node with matching name in the search root's hierarchy
        if let Some(&node_entity) = node_map.get(&port_mesh.mesh_name) {
            // Check if this node is a descendant of the search root
            if is_descendant_of(node_entity, search_root, &children_query) {
                // Find mesh entities - either this node has a mesh, or check children
                let mesh_entity = if mesh_query.get(node_entity).is_ok() {
                    // The node itself has a mesh
                    Some(node_entity)
                } else {
                    // Look for first child with a mesh
                    find_child_mesh(node_entity, &children_query, &mesh_query)
                };

                if let Some(mesh_entity) = mesh_entity {
                    // Tag this mesh as a port target
                    commands.entity(mesh_entity).insert(PortMeshTarget {
                        device_id: port_mesh.device_id.clone(),
                        port_name: port_mesh.port_name.clone(),
                        port_type: port_mesh.port_type.clone(),
                    });
                    tracing::debug!(
                        "Linked port {} to mesh entity {:?} (node {:?}, visual: {:?}) for device {}",
                        port_mesh.port_name, mesh_entity, node_entity, port_mesh.visual_name, port_mesh.device_id
                    );
                } else {
                    tracing::warn!(
                        "Port {} node '{}' found but has no mesh children (visual: {:?}, model: {:?}, device: {})",
                        port_mesh.port_name, port_mesh.mesh_name, port_mesh.visual_name, model_path, port_mesh.device_id
                    );
                }
            } else {
                tracing::warn!(
                    "Port {} node '{}' found but NOT a descendant of visual (visual: {:?}, model: {:?}, device: {})",
                    port_mesh.port_name, port_mesh.mesh_name, port_mesh.visual_name, model_path, port_mesh.device_id
                );
            }
        } else {
            // Check if this visual has ANY descendants with GltfNodeName
            // If not, the model's nodes haven't been tagged yet - keep waiting
            let visual_has_tagged_nodes = has_any_tagged_descendant(search_root, &children_query, &node_name_query);
            if !visual_has_tagged_nodes {
                // Model nodes not tagged yet, keep in pending
                remaining.push(port_mesh);
            } else {
                // Model is tagged but this specific node doesn't exist
                tracing::warn!(
                    "Port {} mesh '{}' not found in visual's scene graph (visual: {:?}, model: {:?}, device: {})",
                    port_mesh.port_name, port_mesh.mesh_name, port_mesh.visual_name, model_path, port_mesh.device_id
                );
            }
        }
    }

    pending.pending = remaining;
}

/// Check if an entity is a descendant of another entity
fn is_descendant_of(
    entity: Entity,
    potential_ancestor: Entity,
    children_query: &Query<&Children>,
) -> bool {
    // Check direct children first
    if let Ok(children) = children_query.get(potential_ancestor) {
        for child in children.iter() {
            if child == entity {
                return true;
            }
            // Recursively check grandchildren
            if is_descendant_of(entity, child, children_query) {
                return true;
            }
        }
    }
    false
}

/// Check if any descendant of entity has a GltfNodeName component
fn has_any_tagged_descendant(
    entity: Entity,
    children_query: &Query<&Children>,
    node_name_query: &Query<(Entity, &GltfNodeName)>,
) -> bool {
    if let Ok(children) = children_query.get(entity) {
        for child in children.iter() {
            // Check if this child has GltfNodeName
            if node_name_query.get(child).is_ok() {
                return true;
            }
            // Recursively check grandchildren
            if has_any_tagged_descendant(child, children_query, node_name_query) {
                return true;
            }
        }
    }
    false
}

/// Find the first child entity that has a Mesh3d component
fn find_child_mesh(
    parent: Entity,
    children_query: &Query<&Children>,
    mesh_query: &Query<Entity, With<Mesh3d>>,
) -> Option<Entity> {
    if let Ok(children) = children_query.get(parent) {
        for child in children.iter() {
            // Check if this child has a mesh
            if mesh_query.get(child).is_ok() {
                return Some(child);
            }
            // Recursively check grandchildren
            if let Some(mesh) = find_child_mesh(child, children_query, mesh_query) {
                return Some(mesh);
            }
        }
    }
    None
}

/// Cached original material properties for restoring when unhovered
#[derive(Clone)]
struct OriginalMaterialProps {
    base_color: Color,
    emissive: bevy::color::LinearRgba,
}

/// Update port mesh highlighting with emissive glow based on hover state
/// Each port mesh gets its own cloned material to avoid affecting other meshes
fn update_port_mesh_highlighting(
    mut commands: Commands,
    frame_visibility: Res<FrameVisibility>,
    port_meshes: Query<(Entity, &PortMeshTarget, Option<&MeshMaterial3d<StandardMaterial>>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut port_materials: Local<HashMap<Entity, (Handle<StandardMaterial>, OriginalMaterialProps)>>,
    mut warned_ports: Local<std::collections::HashSet<String>>,
) {
    for (entity, port_target, material_handle_opt) in port_meshes.iter() {
        let Some(material_handle) = material_handle_opt else {
            // Entity has PortMeshTarget but no StandardMaterial - GLTF may use different material
            if warned_ports.insert(port_target.port_name.clone()) {
                tracing::warn!(
                    "Port mesh {} has no MeshMaterial3d<StandardMaterial>",
                    port_target.port_name
                );
            }
            continue;
        };

        let port_key = format!("{}:{}", port_target.device_id, port_target.port_name);
        let is_hovered = frame_visibility.hovered_port.as_ref() == Some(&port_key);
        let ports_visible = frame_visibility.show_ports_for(&port_target.device_id);

        // Ensure this port has its own cloned material (not shared with other meshes)
        let (own_material_handle, original_props) = port_materials.entry(entity).or_insert_with(|| {
            // Clone the original material so we can modify it independently
            if let Some(original_material) = materials.get(&material_handle.0) {
                let props = OriginalMaterialProps {
                    base_color: original_material.base_color,
                    emissive: original_material.emissive,
                };
                let cloned = original_material.clone();
                let handle = materials.add(cloned);
                // Update the entity to use the cloned material
                commands.entity(entity).insert(MeshMaterial3d(handle.clone()));
                (handle, props)
            } else {
                let props = OriginalMaterialProps {
                    base_color: Color::srgba(0.5, 0.5, 0.5, 1.0),
                    emissive: bevy::color::LinearRgba::new(0.0, 0.0, 0.0, 1.0),
                };
                (material_handle.0.clone(), props)
            }
        });

        if let Some(material) = materials.get_mut(own_material_handle) {
            if is_hovered && ports_visible {
                let (r, g, b) = port_type_to_color(&port_target.port_type);
                material.base_color = Color::srgba(r, g, b, 1.0);
                material.emissive = bevy::color::LinearRgba::new(r * 0.3, g * 0.3, b * 0.3, 1.0);
            } else {
                material.base_color = original_props.base_color.clone();
                material.emissive = original_props.emissive;
            }
        }
    }
}

/// Get highlight color for port type as (r, g, b)
fn port_type_to_color(port_type: &str) -> (f32, f32, f32) {
    match port_type.to_lowercase().as_str() {
        "ethernet" => (0.2, 0.8, 0.2),  // Green
        "can" => (1.0, 0.8, 0.2),       // Yellow/Orange
        "spi" => (0.8, 0.2, 0.8),       // Magenta
        "i2c" => (0.2, 0.8, 0.8),       // Cyan
        "uart" => (0.8, 0.4, 0.2),      // Orange
        "usb" => (0.2, 0.4, 0.8),       // Blue
        _ => (0.5, 0.5, 0.5),           // Gray
    }
}

