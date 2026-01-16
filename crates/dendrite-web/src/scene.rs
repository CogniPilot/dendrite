//! 3D scene management

use bevy::prelude::*;
use bevy::ecs::system::SystemParam;
use bevy::input::mouse::{MouseMotion, MouseWheel};
use bevy::render::alpha::AlphaMode;
use bevy::camera::primitives::MeshAabb;  // Trait for compute_aabb
use bevy_egui::{egui, EguiContexts};
use bevy_picking::prelude::{Click, Out, Over, Pointer, PointerButton};

use crate::app::{ActiveRotationAxis, ActiveRotationField, CameraSettings, DeviceOrientations, DevicePositions, DeviceRegistry, FirmwareCheckState, FirmwareStatusData, FrameVisibility, SelectedDevice, ShowRotationAxis, WorldSettings};
use crate::models::{ExcludeFromBounds, PortEntity, PortMeshTarget, SensorAxisEntity, SensorFovEntity};
use crate::network::HeartbeatState;

pub struct ScenePlugin;

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_scene)
            .add_systems(Update, (
                update_camera,
                handle_deselection,
                update_device_positions,
                update_device_orientations,
                update_selection_highlight,
                update_effective_rotation_axis,
                update_world_visibility,
                update_grid_spacing,
                update_frame_gizmos,
                render_frame_tooltip,
                render_sensor_axis_tooltip,
                render_sensor_fov_tooltip,
                render_port_tooltip,
            ))
            // Use observers for picking events (Bevy 0.17 pattern)
            .add_observer(on_device_clicked)
            .add_observer(on_frame_gizmo_over)
            .add_observer(on_frame_gizmo_out)
            .add_observer(on_sensor_axis_over)
            .add_observer(on_sensor_axis_out)
            .add_observer(on_sensor_fov_over)
            .add_observer(on_sensor_fov_out)
            .add_observer(on_port_over)
            .add_observer(on_port_out);
    }
}

/// Observer: Handle device selection when clicked using bevy_picking
/// For GLTF models, the click target is a mesh child - we traverse up to find DeviceEntity
fn on_device_clicked(
    trigger: On<Pointer<Click>>,
    device_query: Query<(&DeviceEntity, &GlobalTransform)>,
    parent_query: Query<&ChildOf>,
    mut selected: ResMut<SelectedDevice>,
    mut camera_settings: ResMut<CameraSettings>,
) {
    // Access the event to get button and target
    let event = trigger.event();

    // Only handle left/primary clicks (works for both mouse and touch)
    if event.button != PointerButton::Primary {
        return;
    }

    // Start from the clicked entity and walk up the hierarchy
    // The entity field on the event contains the clicked entity
    let mut current = event.entity;

    // Try to find DeviceEntity on this entity or any ancestor
    loop {
        // Check if current entity is a device
        if let Ok((device, transform)) = device_query.get(current) {
            selected.0 = Some(device.device_id.clone());
            // Center camera on selected device
            camera_settings.target_focus = transform.translation();
            return;
        }

        // Try to get parent
        if let Ok(child_of) = parent_query.get(current) {
            current = child_of.parent();
        } else {
            // No parent, stop searching
            break;
        }
    }
}

/// Marker component for the main camera
#[derive(Component)]
pub struct MainCamera;

/// Marker component for device entities
#[derive(Component)]
pub struct DeviceEntity {
    pub device_id: String,
}

/// Marker for the parent device (reserved for future use)
#[derive(Component)]
#[allow(dead_code)]
pub struct ParentDevice;

/// Marker for grid lines
#[derive(Component)]
pub struct GridLine;

/// Marker for world axis lines (the center X, Y, Z axes)
#[derive(Component)]
pub struct WorldAxis;

/// Marker for device connection lines (reserved for future use)
#[derive(Component)]
#[allow(dead_code)]
pub struct ConnectionLine;

/// Marker for selection highlight box
#[derive(Component)]
pub struct SelectionHighlight {
    pub target_device: String,
    pub offset: Vec3, // Offset from device center
    pub is_online: bool, // Track device status for color
}

/// Marker for rotation axis indicator (body-frame axes)
#[derive(Component)]
pub struct RotationAxisIndicator {
    pub target_device: String,
    pub offset: Vec3,
    pub axis: ActiveRotationAxis,
}

/// Marker for effective rotation axis indicator (shows actual rotation axis for Euler XYZ)
#[derive(Component)]
#[allow(dead_code)]
pub struct EffectiveRotationAxis {
    pub target_device: String,
}

/// Marker for reference frame gizmos (coordinate frame visualization)
#[derive(Component)]
pub struct FrameGizmo {
    /// Parent device ID
    pub device_id: String,
    /// Frame name from HCDF
    pub frame_name: String,
    /// Description for tooltip
    pub description: String,
}

fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    world_settings: Res<WorldSettings>,
) {
    // ENU coordinate system: X=East, Y=North, Z=Up
    // Camera - positioned for ENU view (Z is up)
    commands.spawn((
        Camera3d {
            ..default()
        },
        Projection::Perspective(PerspectiveProjection {
            near: 0.001, // Very close clipping plane (1mm)
            far: 1000.0,
            ..default()
        }),
        Transform::from_xyz(0.5, -0.5, 0.4).looking_at(Vec3::ZERO, Vec3::Z),
        MainCamera,
    ));

    // Ambient light - brighter
    // More realistic ambient lighting - softer
    commands.insert_resource(AmbientLight {
        color: Color::srgb(0.9, 0.95, 1.0), // Slightly blue-tinted for realism
        brightness: 200.0, // Much lower for more contrast
        ..default()
    });

    // Directional light (from above in ENU) - like sunlight
    commands.spawn((
        DirectionalLight {
            illuminance: 5000.0, // Reduced for more realistic lighting
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(2.0, 2.0, 4.0).looking_at(Vec3::ZERO, Vec3::Z),
    ));

    // Point light for fill - softer
    commands.spawn((
        PointLight {
            intensity: 100000.0, // Much lower for subtle fill
            shadows_enabled: false,
            color: Color::srgb(1.0, 0.95, 0.9), // Warm fill light
            ..default()
        },
        Transform::from_xyz(-1.0, -1.0, 2.0),
    ));

    // Create grid lines on X-Y plane (ground plane in ENU)
    let grid_size = 10;
    let grid_spacing = world_settings.grid_spacing;
    let grid_extent = (grid_size as f32) * grid_spacing;
    let thickness = world_settings.grid_line_thickness;
    let alpha = world_settings.grid_alpha;

    // Determine initial visibility based on settings
    let initial_visibility = if world_settings.show_grid {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };

    let line_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.4, 0.4, 0.4, alpha),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });

    // Lines along X (East)
    let line_mesh_x = meshes.add(Cuboid::new(grid_extent * 2.0, thickness, thickness));
    // Lines along Y (North)
    let line_mesh_y = meshes.add(Cuboid::new(thickness, grid_extent * 2.0, thickness));

    // Grid lines parallel to X axis (varying Y)
    for i in -grid_size..=grid_size {
        let y = i as f32 * grid_spacing;
        commands.spawn((
            Mesh3d(line_mesh_x.clone()),
            MeshMaterial3d(line_material.clone()),
            Transform::from_translation(Vec3::new(0.0, y, 0.0)),
            GridLine,
            initial_visibility,
        ));
    }

    // Grid lines parallel to Y axis (varying X)
    for i in -grid_size..=grid_size {
        let x = i as f32 * grid_spacing;
        commands.spawn((
            Mesh3d(line_mesh_y.clone()),
            MeshMaterial3d(line_material.clone()),
            Transform::from_translation(Vec3::new(x, 0.0, 0.0)),
            GridLine,
            initial_visibility,
        ));
    }

    // World axis parameters
    let world_axis_length = 0.3;
    let world_axis_thickness = 0.003;
    let world_cone_height = world_axis_thickness * 4.0;
    let world_cone_radius = world_axis_thickness * 2.5;

    // X axis (red, East) - cylinder + cone
    let x_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.9, 0.2, 0.2),
        unlit: true,
        ..default()
    });
    // Cylinder along X: rotate -90 around Z to turn Y-aligned cylinder into X-aligned
    commands.spawn((
        Mesh3d(meshes.add(Cylinder::new(world_axis_thickness, world_axis_length))),
        MeshMaterial3d(x_material.clone()),
        Transform::from_translation(Vec3::new(world_axis_length / 2.0, 0.0, 0.001))
            .with_rotation(Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2)),
        WorldAxis,
    ));
    // X cone at end
    commands.spawn((
        Mesh3d(meshes.add(Cone::new(world_cone_radius, world_cone_height))),
        MeshMaterial3d(x_material),
        Transform::from_translation(Vec3::new(world_axis_length + world_cone_height / 2.0, 0.0, 0.001))
            .with_rotation(Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2)),
        WorldAxis,
    ));

    // Y axis (green, North) - cylinder + cone
    let y_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.2, 0.9, 0.2),
        unlit: true,
        ..default()
    });
    // Cylinder along Y: no rotation needed, cylinder is Y-aligned by default
    commands.spawn((
        Mesh3d(meshes.add(Cylinder::new(world_axis_thickness, world_axis_length))),
        MeshMaterial3d(y_material.clone()),
        Transform::from_translation(Vec3::new(0.0, world_axis_length / 2.0, 0.001)),
        WorldAxis,
    ));
    // Y cone at end
    commands.spawn((
        Mesh3d(meshes.add(Cone::new(world_cone_radius, world_cone_height))),
        MeshMaterial3d(y_material),
        Transform::from_translation(Vec3::new(0.0, world_axis_length + world_cone_height / 2.0, 0.001)),
        WorldAxis,
    ));

    // Z axis (blue, Up) - cylinder + cone
    let z_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.2, 0.2, 0.9),
        unlit: true,
        ..default()
    });
    // Cylinder along Z: rotate +90 around X to turn Y-aligned cylinder into Z-aligned
    commands.spawn((
        Mesh3d(meshes.add(Cylinder::new(world_axis_thickness, world_axis_length))),
        MeshMaterial3d(z_material.clone()),
        Transform::from_translation(Vec3::new(0.0, 0.0, world_axis_length / 2.0))
            .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
        WorldAxis,
    ));
    // Z cone at end
    commands.spawn((
        Mesh3d(meshes.add(Cone::new(world_cone_radius, world_cone_height))),
        MeshMaterial3d(z_material),
        Transform::from_translation(Vec3::new(0.0, 0.0, world_axis_length + world_cone_height / 2.0))
            .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
        WorldAxis,
    ));

    // Parent device will be spawned dynamically from network data

    // Example child devices arranged around parent (X-Y plane)
    let child_positions = [
        (0.15, 0.0),   // Port 1 - East
        (0.1, 0.12),   // Port 2 - NE
        (-0.1, 0.12),  // Port 3 - NW
        (-0.15, 0.0),  // Port 4 - West
        (-0.1, -0.12), // Port 5 - SW
        (0.1, -0.12),  // Port 6 - SE
    ];

    let child_colors = [
        Color::srgb(0.8, 0.3, 0.3), // Red
        Color::srgb(0.3, 0.8, 0.3), // Green
        Color::srgb(0.8, 0.8, 0.3), // Yellow
        Color::srgb(0.8, 0.3, 0.8), // Magenta
        Color::srgb(0.3, 0.8, 0.8), // Cyan
        Color::srgb(0.8, 0.5, 0.2), // Orange
    ];

    for (i, ((x, y), color)) in child_positions.iter().zip(child_colors.iter()).enumerate() {
        // Child device cube (X-width, Y-depth, Z-height)
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.04, 0.03, 0.015))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: *color,
                metallic: 0.2,
                perceptual_roughness: 0.7,
                ..default()
            })),
            Transform::from_translation(Vec3::new(*x, *y, 0.0075)),
            DeviceEntity {
                device_id: format!("device-{}", i + 1),
            },
        ));

        // Connection lines removed - no longer spawning connection lines between parent and child devices
    }
}

/// Camera orbit/pan/zoom controls
/// Uses egui's wants_pointer_input() to avoid camera movement when interacting with UI
fn update_camera(
    mut camera_query: Query<&mut Transform, With<MainCamera>>,
    mut settings: ResMut<CameraSettings>,
    mut mouse_motion: MessageReader<MouseMotion>,
    mut mouse_wheel: MessageReader<MouseWheel>,
    mouse_button: Res<ButtonInput<MouseButton>>,
    touch_input: Res<Touches>,
    time: Res<Time>,
    mut contexts: EguiContexts,
) {
    // Check if egui wants the pointer - bevy_egui/picking handles this via the unified picking system
    let egui_wants_pointer = contexts.ctx_mut().map(|ctx| ctx.wants_pointer_input()).unwrap_or(false);

    // Collect mouse motion delta
    let total_motion: Vec2 = mouse_motion.read().map(|m| m.delta).sum();

    // Orbit with left mouse drag (only when UI doesn't want pointer)
    if mouse_button.pressed(MouseButton::Left) && !egui_wants_pointer {
        settings.azimuth -= total_motion.x * settings.sensitivity;
        settings.elevation = (settings.elevation - total_motion.y * settings.sensitivity)
            .clamp(-1.5, 1.5);
    }

    // Pan with right mouse drag (ENU: vertical plane - right and up)
    if mouse_button.pressed(MouseButton::Right) && !egui_wants_pointer {
        let right = Vec3::new(settings.azimuth.sin(), -settings.azimuth.cos(), 0.0);
        let up = Vec3::Z;
        let pan_speed = settings.distance * 0.002;
        settings.target_focus += right * total_motion.x * pan_speed;
        settings.target_focus += up * total_motion.y * pan_speed;
    }

    // Translate with middle mouse drag (ground plane X-Y)
    if mouse_button.pressed(MouseButton::Middle) && !egui_wants_pointer {
        let right = Vec3::new(-settings.azimuth.sin(), settings.azimuth.cos(), 0.0);
        let forward = Vec3::new(settings.azimuth.cos(), settings.azimuth.sin(), 0.0);
        let pan_speed = settings.distance * 0.002;
        settings.target_focus -= right * total_motion.x * pan_speed;
        settings.target_focus += forward * total_motion.y * pan_speed;
    }

    // Zoom with scroll wheel
    if !egui_wants_pointer {
        for scroll in mouse_wheel.read() {
            let zoom_factor = 1.0 - scroll.y * settings.zoom_speed * 0.3;
            settings.target_distance = (settings.target_distance * zoom_factor).clamp(0.05, 5.0);
        }
    } else {
        // Drain scroll events when UI has focus
        mouse_wheel.read().for_each(drop);
    }

    // Touch support: single finger orbit
    if touch_input.iter().count() == 1 && !egui_wants_pointer {
        if let Some(touch) = touch_input.iter().next() {
            let delta = touch.delta();
            if delta != Vec2::ZERO {
                settings.azimuth -= delta.x * settings.sensitivity;
                settings.elevation = (settings.elevation - delta.y * settings.sensitivity)
                    .clamp(-1.5, 1.5);
            }
        }
    }

    // Pinch to zoom
    if touch_input.iter().count() == 2 {
        let touches: Vec<_> = touch_input.iter().collect();
        if let (Some(t1), Some(t2)) = (touches.get(0), touches.get(1)) {
            let curr_dist = t1.position().distance(t2.position());
            let prev_dist = (t1.position() - t1.delta()).distance(t2.position() - t2.delta());
            let zoom_factor = prev_dist / curr_dist.max(1.0);
            settings.target_distance = (settings.target_distance * zoom_factor).clamp(0.05, 5.0);
        }
    }

    // Smooth interpolation for zoom and target
    let dt = time.delta_secs();
    let lerp_factor = 1.0 - (-settings.smooth_factor * 60.0 * dt).exp();
    settings.distance += (settings.target_distance - settings.distance) * lerp_factor;
    let target_delta = (settings.target_focus - settings.target) * lerp_factor;
    settings.target += target_delta;

    // Update camera position (ENU: Z is up, spherical coordinates)
    if let Ok(mut transform) = camera_query.single_mut() {
        let x = settings.distance * settings.azimuth.cos() * settings.elevation.cos();
        let y = settings.distance * settings.azimuth.sin() * settings.elevation.cos();
        let z = settings.distance * settings.elevation.sin();

        transform.translation = settings.target + Vec3::new(x, y, z);
        transform.look_at(settings.target, Vec3::Z);
    }
}

/// Handle Escape key to deselect current selection
fn handle_deselection(
    mut selected: ResMut<SelectedDevice>,
    keyboard: Res<ButtonInput<KeyCode>>,
) {
    if keyboard.just_pressed(KeyCode::Escape) {
        selected.0 = None;
    }
}

/// Update device positions resource for UI display
fn update_device_positions(
    device_query: Query<(&DeviceEntity, &Transform)>,
    mut positions: ResMut<DevicePositions>,
) {
    positions.positions.clear();
    for (device, transform) in device_query.iter() {
        positions.positions.insert(device.device_id.clone(), transform.translation);
    }
}

/// Update device orientations resource for UI display
/// Extract Roll, Pitch, Yaw from quaternion rotation
fn update_device_orientations(
    device_query: Query<(&DeviceEntity, &Transform)>,
    mut orientations: ResMut<DeviceOrientations>,
) {
    // Only add new devices that don't already have stored orientations
    // This preserves user-set Euler angles and avoids gimbal lock conversion issues
    for (device, transform) in device_query.iter() {
        if !orientations.orientations.contains_key(&device.device_id) {
            // Initialize with quaternion-to-Euler conversion for new devices
            let (roll, pitch, yaw) = transform.rotation.to_euler(EulerRot::XYZ);
            orientations.orientations.insert(device.device_id.clone(), Vec3::new(roll, pitch, yaw));
        }
    }
}

/// Grouped system parameters for the selection highlight system to work around Bevy's 16-param limit
#[derive(SystemParam)]
pub struct SelectionHighlightParams<'w, 's> {
    pub commands: Commands<'w, 's>,
    pub selected: Res<'w, SelectedDevice>,
    pub active_rotation_field: Res<'w, ActiveRotationField>,
    pub show_rotation_axis: Res<'w, ShowRotationAxis>,
    pub registry: Res<'w, DeviceRegistry>,
    pub heartbeat_state: Res<'w, HeartbeatState>,
    pub firmware_state: Res<'w, FirmwareCheckState>,
    pub device_query: Query<'w, 's, (Entity, &'static DeviceEntity, &'static Transform), (Without<SelectionHighlight>, Without<RotationAxisIndicator>)>,
    pub highlight_query: Query<'w, 's, (Entity, &'static mut SelectionHighlight, &'static MeshMaterial3d<StandardMaterial>)>,
    pub axis_query: Query<'w, 's, (Entity, &'static RotationAxisIndicator, &'static MeshMaterial3d<StandardMaterial>)>,
    pub highlight_transform_query: Query<'w, 's, &'static mut Transform, With<SelectionHighlight>>,
    pub axis_transform_query: Query<'w, 's, &'static mut Transform, (With<RotationAxisIndicator>, Without<SelectionHighlight>)>,
    pub children_query: Query<'w, 's, &'static Children>,
    pub mesh_query: Query<'w, 's, (&'static Mesh3d, &'static GlobalTransform)>,
    pub exclude_query: Query<'w, 's, Entity, With<ExcludeFromBounds>>,
    pub meshes: ResMut<'w, Assets<Mesh>>,
    pub materials: ResMut<'w, Assets<StandardMaterial>>,
}

/// Update selection highlight - show bounding box and rotation axes
fn update_selection_highlight(mut params: SelectionHighlightParams) {
    // Get currently selected device ID
    let selected_id = params.selected.0.as_ref();

    // Remove highlights for devices that are no longer selected
    for (entity, highlight, _) in params.highlight_query.iter_mut() {
        if selected_id != Some(&highlight.target_device) {
            params.commands.entity(entity).despawn();
        }
    }

    // Remove axis indicators for devices that are no longer selected OR when checkbox is unchecked
    for (entity, axis, _) in params.axis_query.iter() {
        if selected_id != Some(&axis.target_device) || !params.show_rotation_axis.0 {
            params.commands.entity(entity).despawn();
        }
    }

    // Check if we need to create a new highlight
    let Some(selected_id) = selected_id else {
        return;
    };

    // Check if highlight already exists
    let highlight_exists = params.highlight_query.iter_mut().any(|(_, h, _)| &h.target_device == selected_id);

    // Check if axis indicators exist (separate from highlight)
    let axis_exists = params.axis_query.iter().any(|(_, a, _)| &a.target_device == selected_id);

    // Get device status from registry
    let device_is_online = params.registry.devices.iter()
        .find(|d| &d.id == selected_id)
        .map(|d| d.status == crate::app::DeviceStatus::Online)
        .unwrap_or(false);

    // Find the selected device position
    let mut selected_device_pos = None;
    for (entity, device, transform) in params.device_query.iter() {
        if &device.device_id == selected_id {
            selected_device_pos = Some((entity, transform.translation, transform.clone()));
            break;
        }
    }

    let Some((entity, device_pos, device_transform)) = selected_device_pos else {
        return;
    };

    // Update existing highlight positions, rotations, and colors
    if highlight_exists {
        // Find device transform
        for (_, device, transform) in params.device_query.iter() {
            if &device.device_id == selected_id {
                let device_pos = transform.translation;
                let device_rotation = transform.rotation;

                // Update highlight box edges to rotate with device and update colors
                for (highlight_entity, mut highlight, material_handle) in params.highlight_query.iter_mut() {
                    if &highlight.target_device == selected_id {
                        if let Ok(mut highlight_transform) = params.highlight_transform_query.get_mut(highlight_entity) {
                            // Rotate the offset by the device rotation, then add to position
                            let rotated_offset = device_rotation * highlight.offset;
                            highlight_transform.translation = device_pos + rotated_offset;
                            highlight_transform.rotation = device_rotation;
                        }

                        // Update color if status changed or heartbeat state changed
                        let should_update = highlight.is_online != device_is_online;
                        if should_update {
                            highlight.is_online = device_is_online; // Update the stored status
                        }
                        // Always update material to reflect heartbeat/firmware state
                        // Priority: Offline (red) > Firmware outdated (yellow) > Online (green/white)
                        if let Some(material) = params.materials.get_mut(&material_handle.0) {
                            // Check if firmware is outdated (only when firmware checking is enabled)
                            let is_firmware_outdated = params.firmware_state.enabled
                                && matches!(
                                    params.firmware_state.device_status.get(selected_id),
                                    Some(FirmwareStatusData::UpdateAvailable { .. })
                                );

                            if !device_is_online {
                                // Device is offline - always show red regardless of heartbeat state
                                material.base_color = Color::srgba(0.6, 0.1, 0.1, 0.5);
                                material.emissive = bevy::color::LinearRgba::new(0.3, 0.05, 0.05, 1.0);
                            } else if is_firmware_outdated {
                                // Device has outdated firmware - show yellow
                                material.base_color = Color::srgba(0.8, 0.7, 0.2, 0.5);
                                material.emissive = bevy::color::LinearRgba::new(0.4, 0.35, 0.1, 1.0);
                            } else if !params.heartbeat_state.enabled {
                                // Device is online but heartbeat is off - show white (status unknown)
                                material.base_color = Color::srgba(0.8, 0.8, 0.8, 0.5);
                                material.emissive = bevy::color::LinearRgba::new(0.2, 0.2, 0.2, 1.0);
                            } else {
                                // Device is online and heartbeat is on - show green
                                material.base_color = Color::srgba(0.3, 0.8, 0.3, 0.5);
                                material.emissive = bevy::color::LinearRgba::new(0.15, 0.4, 0.15, 1.0);
                            }
                        }
                    }
                }
                break;
            }
        }
    } else {
        // Create highlight box
        // Compute bounding box from all child meshes in device-local space
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);
        let mut found_mesh = false;

        // Collect entities to skip (visualization entities that shouldn't affect bounding box)
        let skip_entities: std::collections::HashSet<Entity> = params.exclude_query.iter().collect();

        // Recursively find all mesh children and compute bounds in device-local space
        fn collect_bounds(
            entity: Entity,
            children_query: &Query<&Children>,
            mesh_query: &Query<(&Mesh3d, &GlobalTransform)>,
            mesh_assets: &Assets<Mesh>,
            device_world_pos: Vec3,
            device_rotation_inv: Quat,
            min: &mut Vec3,
            max: &mut Vec3,
            found: &mut bool,
            skip_entities: &std::collections::HashSet<Entity>,
        ) {
            // Skip visualization entities (sensors, ports, FOV geometry)
            if skip_entities.contains(&entity) {
                return;
            }

            // Check if this entity has a mesh
            if let Ok((mesh_handle, global_transform)) = mesh_query.get(entity) {
                if let Some(mesh) = mesh_assets.get(&mesh_handle.0) {
                    if let Some(aabb) = mesh.compute_aabb() {
                        // Transform AABB corners from mesh-local to device-local space
                        let center = Vec3::from(aabb.center);
                        let half = Vec3::from(aabb.half_extents);

                        // Get the 8 corners of the AABB in mesh-local space
                        let corners = [
                            center + Vec3::new(-half.x, -half.y, -half.z),
                            center + Vec3::new( half.x, -half.y, -half.z),
                            center + Vec3::new(-half.x,  half.y, -half.z),
                            center + Vec3::new( half.x,  half.y, -half.z),
                            center + Vec3::new(-half.x, -half.y,  half.z),
                            center + Vec3::new( half.x, -half.y,  half.z),
                            center + Vec3::new(-half.x,  half.y,  half.z),
                            center + Vec3::new( half.x,  half.y,  half.z),
                        ];

                        // Transform corners: mesh-local -> world -> device-local
                        for corner in corners {
                            // Mesh-local to world
                            let world_corner = global_transform.transform_point(corner);
                            // World to device-local (undo device translation and rotation)
                            let local_corner = device_rotation_inv * (world_corner - device_world_pos);
                            *min = min.min(local_corner);
                            *max = max.max(local_corner);
                        }
                        *found = true;
                    }
                }
            }

            // Check children
            if let Ok(children) = children_query.get(entity) {
                for child in children.iter() {
                    collect_bounds(child, children_query, mesh_query, mesh_assets, device_world_pos, device_rotation_inv, min, max, found, skip_entities);
                }
            }
        }

        // Get inverse of device rotation for converting world -> device-local
        let device_rotation_inv = device_transform.rotation.inverse();
        collect_bounds(entity, &params.children_query, &params.mesh_query, params.meshes.as_ref(), device_pos, device_rotation_inv, &mut min, &mut max, &mut found_mesh, &skip_entities);

        // Use default size if no mesh bounds found
        let (box_min, box_max) = if found_mesh {
            (min, max)
        } else {
            // Default fallback size
            (Vec3::splat(-0.04), Vec3::splat(0.04))
        };

        // Add padding to the actual min/max bounds
        let padding = 0.005;
        let padded_min = box_min - Vec3::splat(padding);
        let padded_max = box_max + Vec3::splat(padding);

        // Calculate dimensions for edge lengths
        let box_size = padded_max - padded_min;
        let edge_thickness = 0.003 / 6.0; // 6x smaller lines

        // Store half for axis length calculation (used later)
        let half = box_size / 2.0;

        // Color based on device status, firmware status, and heartbeat state
        // Priority: Offline (red) > Firmware outdated (yellow) > Online (green/white)
        // Check if firmware is outdated (only when firmware checking is enabled)
        let is_firmware_outdated = params.firmware_state.enabled
            && matches!(
                params.firmware_state.device_status.get(selected_id),
                Some(FirmwareStatusData::UpdateAvailable { .. })
            );

        let (base_color, emissive) = if !device_is_online {
            // Device is offline - always show red regardless of heartbeat state
            (Color::srgba(0.6, 0.1, 0.1, 0.5), bevy::color::LinearRgba::new(0.3, 0.05, 0.05, 1.0))
        } else if is_firmware_outdated {
            // Device has outdated firmware - show yellow
            (Color::srgba(0.8, 0.7, 0.2, 0.5), bevy::color::LinearRgba::new(0.4, 0.35, 0.1, 1.0))
        } else if !params.heartbeat_state.enabled {
            // Device is online but heartbeat is off - show white (status unknown)
            (Color::srgba(0.8, 0.8, 0.8, 0.5), bevy::color::LinearRgba::new(0.2, 0.2, 0.2, 1.0))
        } else {
            // Device is online and heartbeat is on - show green
            (Color::srgba(0.3, 0.8, 0.3, 0.5), bevy::color::LinearRgba::new(0.15, 0.4, 0.15, 1.0))
        };

        let highlight_material = params.materials.add(StandardMaterial {
            base_color,
            emissive,
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            ..default()
        });

        // Create 12 edges of the bounding box using actual min/max coordinates
        // This properly handles asymmetric bounding boxes where the mesh center != rotation center
        let x_min = padded_min.x;
        let x_max = padded_max.x;
        let y_min = padded_min.y;
        let y_max = padded_max.y;
        let z_min = padded_min.z;
        let z_max = padded_max.z;
        let x_mid = (x_min + x_max) / 2.0;
        let y_mid = (y_min + y_max) / 2.0;
        let z_mid = (z_min + z_max) / 2.0;

        let edges = [
            // Bottom face (z = z_min) - 4 edges
            // Edge along X at (y_min, z_min)
            (Vec3::new(x_mid, y_min, z_min), Vec3::new(box_size.x, edge_thickness, edge_thickness)),
            // Edge along X at (y_max, z_min)
            (Vec3::new(x_mid, y_max, z_min), Vec3::new(box_size.x, edge_thickness, edge_thickness)),
            // Edge along Y at (x_min, z_min)
            (Vec3::new(x_min, y_mid, z_min), Vec3::new(edge_thickness, box_size.y, edge_thickness)),
            // Edge along Y at (x_max, z_min)
            (Vec3::new(x_max, y_mid, z_min), Vec3::new(edge_thickness, box_size.y, edge_thickness)),

            // Top face (z = z_max) - 4 edges
            // Edge along X at (y_min, z_max)
            (Vec3::new(x_mid, y_min, z_max), Vec3::new(box_size.x, edge_thickness, edge_thickness)),
            // Edge along X at (y_max, z_max)
            (Vec3::new(x_mid, y_max, z_max), Vec3::new(box_size.x, edge_thickness, edge_thickness)),
            // Edge along Y at (x_min, z_max)
            (Vec3::new(x_min, y_mid, z_max), Vec3::new(edge_thickness, box_size.y, edge_thickness)),
            // Edge along Y at (x_max, z_max)
            (Vec3::new(x_max, y_mid, z_max), Vec3::new(edge_thickness, box_size.y, edge_thickness)),

            // Vertical edges (along Z) - 4 edges
            (Vec3::new(x_min, y_min, z_mid), Vec3::new(edge_thickness, edge_thickness, box_size.z)),
            (Vec3::new(x_max, y_min, z_mid), Vec3::new(edge_thickness, edge_thickness, box_size.z)),
            (Vec3::new(x_min, y_max, z_mid), Vec3::new(edge_thickness, edge_thickness, box_size.z)),
            (Vec3::new(x_max, y_max, z_mid), Vec3::new(edge_thickness, edge_thickness, box_size.z)),
        ];

        for (offset, size) in edges {
            params.commands.spawn((
                Mesh3d(params.meshes.add(Cuboid::new(size.x, size.y, size.z))),
                MeshMaterial3d(highlight_material.clone()),
                Transform::from_translation(device_pos + offset),
                SelectionHighlight {
                    target_device: selected_id.clone(),
                    offset,
                    is_online: device_is_online,
                },
            ));
        }

    }

    // Create rotation axis indicators only if checkbox is checked AND they don't exist yet
    // This block runs independently of highlight creation so toggling the checkbox works
    if params.show_rotation_axis.0 && !axis_exists {
        // (FLU: Forward=X/Red, Left=Y/Green, Up=Z/Blue)
        // Use a default axis length (can be adjusted based on typical device sizes)
        let axis_length = 0.06; // 6cm default axis length
        let axis_thickness = 0.002;
        let cone_height = axis_thickness * 3.0;
        let cone_radius = axis_thickness * 2.0;

        // Body-frame axis directions for positioning
        let body_x = device_transform.rotation * Vec3::X;
        let body_y = device_transform.rotation * Vec3::Y;
        let body_z = device_transform.rotation * Vec3::Z;

        // All axis shafts use device_transform.rotation directly
        let shaft_rotation = device_transform.rotation;

        // X axis (Roll/Forward - Red)
        let x_axis_material = params.materials.add(StandardMaterial {
            base_color: Color::srgb(0.5, 0.1, 0.1),
            emissive: bevy::color::LinearRgba::new(0.5, 0.1, 0.1, 1.0),
            unlit: false,
            alpha_mode: AlphaMode::Blend,
            ..default()
        });
        let x_shaft_center = device_pos + body_x * (axis_length / 2.0);
        params.commands.spawn((
            Mesh3d(params.meshes.add(Cylinder::new(axis_thickness, axis_length))),
            MeshMaterial3d(x_axis_material.clone()),
            Transform::from_translation(x_shaft_center)
                .with_rotation(shaft_rotation),
            RotationAxisIndicator {
                target_device: selected_id.clone(),
                offset: Vec3::X * (axis_length / 2.0),
                axis: ActiveRotationAxis::Roll,
            },
        ));
        let x_cone_center = device_pos + body_x * (axis_length + cone_height / 2.0);
        let x_cone_rotation = shaft_rotation * Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2);
        params.commands.spawn((
            Mesh3d(params.meshes.add(Cone::new(cone_radius, cone_height))),
            MeshMaterial3d(x_axis_material),
            Transform::from_translation(x_cone_center)
                .with_rotation(x_cone_rotation),
            RotationAxisIndicator {
                target_device: selected_id.clone(),
                offset: Vec3::X * (axis_length + cone_height / 2.0),
                axis: ActiveRotationAxis::Roll,
            },
        ));

        // Y axis (Pitch/Left - Green)
        let y_axis_material = params.materials.add(StandardMaterial {
            base_color: Color::srgb(0.1, 0.5, 0.1),
            emissive: bevy::color::LinearRgba::new(0.1, 0.5, 0.1, 1.0),
            unlit: false,
            alpha_mode: AlphaMode::Blend,
            ..default()
        });
        let y_shaft_center = device_pos + body_y * (axis_length / 2.0);
        params.commands.spawn((
            Mesh3d(params.meshes.add(Cylinder::new(axis_thickness, axis_length))),
            MeshMaterial3d(y_axis_material.clone()),
            Transform::from_translation(y_shaft_center)
                .with_rotation(shaft_rotation),
            RotationAxisIndicator {
                target_device: selected_id.clone(),
                offset: Vec3::Y * (axis_length / 2.0),
                axis: ActiveRotationAxis::Pitch,
            },
        ));
        let y_cone_center = device_pos + body_y * (axis_length + cone_height / 2.0);
        let y_cone_rotation = shaft_rotation;
        params.commands.spawn((
            Mesh3d(params.meshes.add(Cone::new(cone_radius, cone_height))),
            MeshMaterial3d(y_axis_material),
            Transform::from_translation(y_cone_center)
                .with_rotation(y_cone_rotation),
            RotationAxisIndicator {
                target_device: selected_id.clone(),
                offset: Vec3::Y * (axis_length + cone_height / 2.0),
                axis: ActiveRotationAxis::Pitch,
            },
        ));

        // Z axis (Yaw/Up - Blue)
        let z_axis_material = params.materials.add(StandardMaterial {
            base_color: Color::srgb(0.1, 0.1, 0.5),
            emissive: bevy::color::LinearRgba::new(0.1, 0.1, 0.5, 1.0),
            unlit: false,
            alpha_mode: AlphaMode::Blend,
            ..default()
        });
        let z_shaft_center = device_pos + body_z * (axis_length / 2.0);
        params.commands.spawn((
            Mesh3d(params.meshes.add(Cylinder::new(axis_thickness, axis_length))),
            MeshMaterial3d(z_axis_material.clone()),
            Transform::from_translation(z_shaft_center)
                .with_rotation(shaft_rotation),
            RotationAxisIndicator {
                target_device: selected_id.clone(),
                offset: Vec3::Z * (axis_length / 2.0),
                axis: ActiveRotationAxis::Yaw,
            },
        ));
        let z_cone_center = device_pos + body_z * (axis_length + cone_height / 2.0);
        let z_cone_rotation = shaft_rotation * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        params.commands.spawn((
            Mesh3d(params.meshes.add(Cone::new(cone_radius, cone_height))),
            MeshMaterial3d(z_axis_material),
            Transform::from_translation(z_cone_center)
                .with_rotation(z_cone_rotation),
            RotationAxisIndicator {
                target_device: selected_id.clone(),
                offset: Vec3::Z * (axis_length + cone_height / 2.0),
                axis: ActiveRotationAxis::Yaw,
            },
        ));
    }

    // Update existing axis indicator positions and rotations to follow the device
    if axis_exists {
        // Find device transform
        for (_, device, transform) in params.device_query.iter() {
            if &device.device_id == selected_id {
                let device_pos = transform.translation;
                let device_rotation = transform.rotation;

                // Update all axis indicators
                // All use device_rotation as base, cones have additional local rotation
                for (axis_entity, axis, material_handle) in params.axis_query.iter() {
                    if &axis.target_device == selected_id {
                        // Update position and rotation (no scaling)
                        if let Ok(mut axis_transform) = params.axis_transform_query.get_mut(axis_entity) {
                            // Position: rotate the stored offset by device rotation
                            let rotated_offset = device_rotation * axis.offset;
                            axis_transform.translation = device_pos + rotated_offset;

                            // Rotation: all shafts use device_rotation directly
                            // Cones need additional local rotation to point tip along the axis
                            let offset_normalized = axis.offset.normalize_or_zero();

                            axis_transform.rotation = match axis.axis {
                                ActiveRotationAxis::Roll => {
                                    // X axis: cone has larger offset, needs -90 Z rotation
                                    if offset_normalized.x.abs() > 0.9 {
                                        device_rotation * Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2)
                                    } else {
                                        device_rotation
                                    }
                                }
                                ActiveRotationAxis::Pitch => {
                                    // Y axis: cone tip already points along Y, just device rotation
                                    device_rotation
                                }
                                ActiveRotationAxis::Yaw => {
                                    // Z axis: cone has larger offset, needs +90 X rotation
                                    if offset_normalized.z.abs() > 0.9 {
                                        device_rotation * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)
                                    } else {
                                        device_rotation
                                    }
                                }
                                ActiveRotationAxis::None => device_rotation,
                            };
                        }

                        // Update material based on active rotation field
                        if let Some(material) = params.materials.get_mut(&material_handle.0) {
                            let is_active = match params.active_rotation_field.axis {
                                ActiveRotationAxis::Roll => axis.axis == ActiveRotationAxis::Roll,
                                ActiveRotationAxis::Pitch => axis.axis == ActiveRotationAxis::Pitch,
                                ActiveRotationAxis::Yaw => axis.axis == ActiveRotationAxis::Yaw,
                                ActiveRotationAxis::None => false,
                            };

                            // Define base colors for each axis with transparency
                            // Use moderate emissive values - not too bright
                            let (base_color, emissive, unlit) = match axis.axis {
                                ActiveRotationAxis::Roll => {
                                    if is_active {
                                        // Brighter red when active - moderate emissive
                                        (Color::srgba(0.6, 0.15, 0.15, 1.0), bevy::color::LinearRgba::new(1.2, 0.3, 0.3, 1.0), false)
                                    } else if params.active_rotation_field.axis != ActiveRotationAxis::None {
                                        // Dimmed red when another axis is active (50% transparency)
                                        (Color::srgba(0.4, 0.1, 0.1, 0.5), bevy::color::LinearRgba::new(0.0, 0.0, 0.0, 1.0), true)
                                    } else {
                                        // Normal red
                                        (Color::srgba(0.5, 0.1, 0.1, 1.0), bevy::color::LinearRgba::new(0.5, 0.1, 0.1, 1.0), false)
                                    }
                                }
                                ActiveRotationAxis::Pitch => {
                                    if is_active {
                                        // Brighter green when active - moderate emissive
                                        (Color::srgba(0.15, 0.6, 0.15, 1.0), bevy::color::LinearRgba::new(0.3, 1.2, 0.3, 1.0), false)
                                    } else if params.active_rotation_field.axis != ActiveRotationAxis::None {
                                        // Dimmed green when another axis is active (50% transparency)
                                        (Color::srgba(0.1, 0.4, 0.1, 0.5), bevy::color::LinearRgba::new(0.0, 0.0, 0.0, 1.0), true)
                                    } else {
                                        // Normal green
                                        (Color::srgba(0.1, 0.5, 0.1, 1.0), bevy::color::LinearRgba::new(0.1, 0.5, 0.1, 1.0), false)
                                    }
                                }
                                ActiveRotationAxis::Yaw => {
                                    if is_active {
                                        // Brighter blue when active - moderate emissive
                                        (Color::srgba(0.15, 0.15, 0.6, 1.0), bevy::color::LinearRgba::new(0.3, 0.3, 1.2, 1.0), false)
                                    } else if params.active_rotation_field.axis != ActiveRotationAxis::None {
                                        // Dimmed blue when another axis is active (50% transparency)
                                        (Color::srgba(0.1, 0.1, 0.4, 0.5), bevy::color::LinearRgba::new(0.0, 0.0, 0.0, 1.0), true)
                                    } else {
                                        // Normal blue
                                        (Color::srgba(0.1, 0.1, 0.5, 1.0), bevy::color::LinearRgba::new(0.1, 0.1, 0.5, 1.0), false)
                                    }
                                }
                                ActiveRotationAxis::None => {
                                    // Shouldn't happen, but default to gray
                                    (Color::srgba(0.5, 0.5, 0.5, 1.0), bevy::color::LinearRgba::new(0.2, 0.2, 0.2, 1.0), true)
                                }
                            };

                            material.base_color = base_color;
                            material.emissive = emissive;
                            material.unlit = unlit;
                            material.alpha_mode = AlphaMode::Blend; // Ensure alpha blending is enabled
                        }
                    }
                }
                break;
            }
        }
    }
}

/// Update effective rotation axis indicator
/// Shows the actual axis that rotation will occur around for XYZ Euler angles
fn update_effective_rotation_axis(
    mut commands: Commands,
    selected: Res<SelectedDevice>,
    active_rotation_field: Res<ActiveRotationField>,
    show_rotation_axis: Res<ShowRotationAxis>,
    orientations: Res<DeviceOrientations>,
    device_query: Query<(&DeviceEntity, &Transform)>,
    effective_axis_query: Query<Entity, With<EffectiveRotationAxis>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Despawn existing effective axis indicators
    for entity in effective_axis_query.iter() {
        commands.entity(entity).despawn();
    }

    // Don't show if rotation axis checkbox is unchecked
    if !show_rotation_axis.0 {
        return;
    }

    // Only show when a device is selected and a rotation field is active
    let Some(selected_id) = &selected.0 else {
        return;
    };

    if active_rotation_field.axis == ActiveRotationAxis::None {
        return;
    }

    // Find the selected device
    let Some((_, device_transform)) = device_query.iter().find(|(d, _)| &d.device_id == selected_id) else {
        return;
    };

    // Get the current Euler angles
    let orient = orientations.orientations.get(selected_id).cloned().unwrap_or(Vec3::ZERO);
    let roll = orient.x;
    let pitch = orient.y;
    // yaw = orient.z (not needed for computing effective axes)

    // Compute the effective rotation axis based on XYZ Euler order
    // Roll (X): always around world X axis
    // Pitch (Y): around the X-rotated Y axis
    // Yaw (Z): around the X-then-Y rotated Z axis
    let effective_axis = match active_rotation_field.axis {
        ActiveRotationAxis::Roll => {
            // Roll is always around world X
            Vec3::X
        }
        ActiveRotationAxis::Pitch => {
            // Pitch is around the X-rotated Y axis
            let roll_quat = Quat::from_rotation_x(roll);
            roll_quat * Vec3::Y
        }
        ActiveRotationAxis::Yaw => {
            // Yaw is around the X-then-Y rotated Z axis
            let roll_quat = Quat::from_rotation_x(roll);
            let pitch_quat = Quat::from_rotation_y(pitch);
            roll_quat * pitch_quat * Vec3::Z
        }
        ActiveRotationAxis::None => return,
    };

    let device_pos = device_transform.translation;

    // Effective axis: white, 3x thinner than body axes, 50% alpha
    // Max length = 2x bounding box (estimate ~0.08m bounding box, so ~0.16m total = 0.08m half)
    let axis_half_length = 0.08; // Half length, total = 2x bounding box
    let axis_thickness = 0.002 / 3.0; // 3x thinner than body axes
    let cone_height = axis_thickness * 4.0;
    let cone_radius = axis_thickness * 3.0;

    let effective_axis_material = materials.add(StandardMaterial {
        base_color: Color::srgba(1.0, 1.0, 1.0, 0.5), // White with 50% alpha
        emissive: bevy::color::LinearRgba::new(0.5, 0.5, 0.5, 1.0), // Subtle glow
        unlit: false,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });

    // Calculate rotation to align cylinder with effective axis
    // Cylinder defaults to Y-axis, need to rotate to align with effective_axis
    let rotation = if effective_axis.dot(Vec3::Y).abs() > 0.999 {
        // Already aligned with Y or -Y
        if effective_axis.y > 0.0 {
            Quat::IDENTITY
        } else {
            Quat::from_rotation_x(std::f32::consts::PI)
        }
    } else {
        Quat::from_rotation_arc(Vec3::Y, effective_axis)
    };

    // Rotation for negative cone (pointing in opposite direction)
    let neg_rotation = if effective_axis.dot(Vec3::Y).abs() > 0.999 {
        if effective_axis.y > 0.0 {
            Quat::from_rotation_x(std::f32::consts::PI)
        } else {
            Quat::IDENTITY
        }
    } else {
        Quat::from_rotation_arc(Vec3::Y, -effective_axis)
    };

    // Spawn cylinder for the effective axis (centered on device)
    commands.spawn((
        Mesh3d(meshes.add(Cylinder::new(axis_thickness, axis_half_length * 2.0))),
        MeshMaterial3d(effective_axis_material.clone()),
        Transform::from_translation(device_pos)
            .with_rotation(rotation),
        EffectiveRotationAxis {
            target_device: selected_id.clone(),
        },
    ));

    // Add cone at positive end
    let pos_cone_offset = effective_axis * (axis_half_length + cone_height / 2.0);
    commands.spawn((
        Mesh3d(meshes.add(Cone::new(cone_radius, cone_height))),
        MeshMaterial3d(effective_axis_material.clone()),
        Transform::from_translation(device_pos + pos_cone_offset)
            .with_rotation(rotation),
        EffectiveRotationAxis {
            target_device: selected_id.clone(),
        },
    ));

    // Add cone at negative end
    let neg_cone_offset = -effective_axis * (axis_half_length + cone_height / 2.0);
    commands.spawn((
        Mesh3d(meshes.add(Cone::new(cone_radius, cone_height))),
        MeshMaterial3d(effective_axis_material),
        Transform::from_translation(device_pos + neg_cone_offset)
            .with_rotation(neg_rotation),
        EffectiveRotationAxis {
            target_device: selected_id.clone(),
        },
    ));
}

/// Update visibility of world grid and axis based on settings
fn update_world_visibility(
    world_settings: Res<WorldSettings>,
    mut grid_query: Query<&mut Visibility, (With<GridLine>, Without<WorldAxis>)>,
    mut axis_query: Query<&mut Visibility, With<WorldAxis>>,
) {
    // Only update if settings changed
    if !world_settings.is_changed() {
        return;
    }

    // Update grid visibility
    let grid_visibility = if world_settings.show_grid {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };

    for mut visibility in grid_query.iter_mut() {
        *visibility = grid_visibility;
    }

    // Update world axis visibility
    let axis_visibility = if world_settings.show_axis {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };

    for mut visibility in axis_query.iter_mut() {
        *visibility = axis_visibility;
    }
}

/// Regenerate grid when spacing or thickness changes
fn update_grid_spacing(
    mut commands: Commands,
    mut world_settings: ResMut<WorldSettings>,
    grid_query: Query<Entity, With<GridLine>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Only regenerate if spacing or thickness actually changed
    if !world_settings.is_changed() {
        return;
    }

    // Check if grid geometry parameters changed (not just visibility toggles)
    if !world_settings.needs_grid_regeneration() {
        return;
    }

    // Remove old grid lines
    for entity in grid_query.iter() {
        commands.entity(entity).despawn();
    }

    // Create new grid with updated spacing, thickness, and alpha
    let grid_size = 10;
    let grid_spacing = world_settings.grid_spacing;
    let grid_extent = (grid_size as f32) * grid_spacing;
    let thickness = world_settings.grid_line_thickness;
    let alpha = world_settings.grid_alpha;

    // Determine initial visibility based on current setting
    let initial_visibility = if world_settings.show_grid {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };

    let line_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.4, 0.4, 0.4, alpha),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });

    // Lines along X (East)
    let line_mesh_x = meshes.add(Cuboid::new(grid_extent * 2.0, thickness, thickness));
    // Lines along Y (North)
    let line_mesh_y = meshes.add(Cuboid::new(thickness, grid_extent * 2.0, thickness));

    // Grid lines parallel to X axis (varying Y)
    for i in -grid_size..=grid_size {
        let y = i as f32 * grid_spacing;
        commands.spawn((
            Mesh3d(line_mesh_x.clone()),
            MeshMaterial3d(line_material.clone()),
            Transform::from_translation(Vec3::new(0.0, y, 0.0)),
            GridLine,
            initial_visibility,
        ));
    }

    // Grid lines parallel to Y axis (varying X)
    for i in -grid_size..=grid_size {
        let x = i as f32 * grid_spacing;
        commands.spawn((
            Mesh3d(line_mesh_y.clone()),
            MeshMaterial3d(line_material.clone()),
            Transform::from_translation(Vec3::new(x, 0.0, 0.0)),
            GridLine,
            initial_visibility,
        ));
    }

    // Mark that we've regenerated the grid with current values
    world_settings.mark_grid_regenerated();
}

/// Update frame gizmos based on per-device visibility settings
fn update_frame_gizmos(
    mut commands: Commands,
    frame_visibility: Res<FrameVisibility>,
    registry: Res<DeviceRegistry>,
    device_query: Query<(Entity, &DeviceEntity)>,
    frame_gizmo_query: Query<(Entity, &FrameGizmo)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // First, despawn gizmos for devices/frames that have visibility turned off
    for (entity, gizmo) in frame_gizmo_query.iter() {
        // Despawn if device frames are hidden OR this specific frame is hidden
        if !frame_visibility.show_frames_for(&gizmo.device_id)
            || !frame_visibility.is_frame_visible(&gizmo.device_id, &gizmo.frame_name)
        {
            commands.entity(entity).despawn();
        }
    }

    // Count expected gizmos (only for devices with visibility on AND per-frame visibility)
    let expected_count: usize = registry.devices.iter()
        .filter(|d| frame_visibility.show_frames_for(&d.id))
        .map(|d| {
            d.frames.iter()
                .filter(|f| frame_visibility.is_frame_visible(&d.id, &f.name))
                .count() * 3 // 3 axis entities per frame
        })
        .sum();

    // Count current gizmos for visible devices and frames
    let current_count = frame_gizmo_query.iter()
        .filter(|(_, g)| {
            frame_visibility.show_frames_for(&g.device_id)
                && frame_visibility.is_frame_visible(&g.device_id, &g.frame_name)
        })
        .count();

    // Skip if gizmos already match expected count
    if current_count == expected_count && expected_count > 0 {
        return;
    }

    // If no devices have frames visible, we're done
    if expected_count == 0 {
        return;
    }

    // Despawn all existing gizmos to recreate (simpler than tracking changes)
    for (entity, _) in frame_gizmo_query.iter() {
        commands.entity(entity).despawn();
    }

    // Frame gizmo parameters
    let axis_length = 0.03; // 3cm axis length
    let axis_thickness = 0.001; // 1mm thickness
    let alpha = if frame_visibility.hovered_frame.is_some() { 0.5 } else { 0.7 };

    // Create materials for RGB axes with transparency
    let x_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.9, 0.2, 0.2, alpha),
        emissive: bevy::color::LinearRgba::new(0.3, 0.05, 0.05, 1.0),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let y_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.2, 0.9, 0.2, alpha),
        emissive: bevy::color::LinearRgba::new(0.05, 0.3, 0.05, 1.0),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let z_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.2, 0.2, 0.9, alpha),
        emissive: bevy::color::LinearRgba::new(0.05, 0.05, 0.3, 1.0),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });

    // Create cylinder meshes for axes
    let axis_mesh = meshes.add(Cylinder::new(axis_thickness, axis_length));

    // Iterate over devices with frame visibility enabled
    for device in &registry.devices {
        // Skip devices without frame visibility
        if !frame_visibility.show_frames_for(&device.id) {
            continue;
        }

        // Find the device entity to parent gizmos to
        let device_entity = device_query.iter()
            .find(|(_, d)| d.device_id == device.id)
            .map(|(e, _)| e);

        let device_entity = match device_entity {
            Some(e) => e,
            None => continue, // Device not yet spawned
        };

        for frame in &device.frames {
            // Skip frames that are individually hidden
            if !frame_visibility.is_frame_visible(&device.id, &frame.name) {
                continue;
            }

            // Parse frame pose (local to device)
            let frame_pose = frame.pose.unwrap_or([0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
            let frame_translation = Vec3::new(
                frame_pose[0] as f32,
                frame_pose[1] as f32,
                frame_pose[2] as f32,
            );
            let frame_rotation = Quat::from_euler(
                EulerRot::ZYX,
                frame_pose[5] as f32, // yaw
                frame_pose[4] as f32, // pitch
                frame_pose[3] as f32, // roll
            );

            let description = frame.description.clone().unwrap_or_default();

            // X axis (red) - cylinder rotated to point along X, positioned in local space
            let x_axis_local_pos = frame_translation + frame_rotation * Vec3::X * (axis_length / 2.0);
            let x_rotation = frame_rotation * Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2);
            let x_entity = commands.spawn((
                Mesh3d(axis_mesh.clone()),
                MeshMaterial3d(x_material.clone()),
                Transform::from_translation(x_axis_local_pos).with_rotation(x_rotation),
                FrameGizmo {
                    device_id: device.id.clone(),
                    frame_name: frame.name.clone(),
                    description: description.clone(),
                },
            )).id();
            commands.entity(device_entity).add_child(x_entity);

            // Y axis (green) - cylinder along Y (default orientation)
            let y_axis_local_pos = frame_translation + frame_rotation * Vec3::Y * (axis_length / 2.0);
            let y_entity = commands.spawn((
                Mesh3d(axis_mesh.clone()),
                MeshMaterial3d(y_material.clone()),
                Transform::from_translation(y_axis_local_pos).with_rotation(frame_rotation),
                FrameGizmo {
                    device_id: device.id.clone(),
                    frame_name: frame.name.clone(),
                    description: description.clone(),
                },
            )).id();
            commands.entity(device_entity).add_child(y_entity);

            // Z axis (blue) - cylinder rotated to point along Z
            let z_axis_local_pos = frame_translation + frame_rotation * Vec3::Z * (axis_length / 2.0);
            let z_rotation = frame_rotation * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
            let z_entity = commands.spawn((
                Mesh3d(axis_mesh.clone()),
                MeshMaterial3d(z_material.clone()),
                Transform::from_translation(z_axis_local_pos).with_rotation(z_rotation),
                FrameGizmo {
                    device_id: device.id.clone(),
                    frame_name: frame.name.clone(),
                    description: description.clone(),
                },
            )).id();
            commands.entity(device_entity).add_child(z_entity);
        }
    }
}

/// Observer: Handle mouse entering a frame gizmo
fn on_frame_gizmo_over(
    trigger: On<Pointer<Over>>,
    frame_query: Query<&FrameGizmo>,
    mut frame_visibility: ResMut<FrameVisibility>,
) {
    let entity = trigger.event().entity;

    if let Ok(frame_gizmo) = frame_query.get(entity) {
        // Set the hovered frame (device_id:frame_name format)
        let frame_key = format!("{}:{}", frame_gizmo.device_id, frame_gizmo.frame_name);
        frame_visibility.hovered_frame = Some(frame_key);
    }
}

/// Observer: Handle mouse leaving a frame gizmo
fn on_frame_gizmo_out(
    trigger: On<Pointer<Out>>,
    frame_query: Query<&FrameGizmo>,
    mut frame_visibility: ResMut<FrameVisibility>,
) {
    let entity = trigger.event().entity;

    // Only clear if this is actually a frame gizmo
    if frame_query.get(entity).is_ok() {
        frame_visibility.hovered_frame = None;
    }
}

/// Render tooltip for hovered frame using egui
fn render_frame_tooltip(
    mut contexts: EguiContexts,
    frame_visibility: Res<FrameVisibility>,
    frame_query: Query<&FrameGizmo>,
) {
    // Only show tooltip if a frame is hovered
    let Some(ref hovered_key) = frame_visibility.hovered_frame else {
        return;
    };

    // Find the frame gizmo with matching key to get description
    let mut frame_name = String::new();
    let mut description = String::new();

    for gizmo in frame_query.iter() {
        let key = format!("{}:{}", gizmo.device_id, gizmo.frame_name);
        if &key == hovered_key {
            frame_name = gizmo.frame_name.clone();
            description = gizmo.description.clone();
            break;
        }
    }

    if frame_name.is_empty() {
        return;
    }

    // Get the egui context
    let Ok(ctx) = contexts.ctx_mut() else { return };

    // Show tooltip at cursor position
    if let Some(pos) = ctx.pointer_hover_pos() {
        egui::Area::new(egui::Id::new("frame_tooltip"))
            .fixed_pos(egui::pos2(pos.x + 15.0, pos.y + 15.0))
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .show(ui, |ui| {
                        ui.set_min_width(120.0);
                        ui.label(egui::RichText::new(&frame_name).strong());
                        if !description.is_empty() {
                            ui.label(&description);
                        }
                    });
            });
    }
}

/// Observer: Handle mouse entering a sensor axis frame
fn on_sensor_axis_over(
    trigger: On<Pointer<Over>>,
    sensor_query: Query<&SensorAxisEntity>,
    mut frame_visibility: ResMut<FrameVisibility>,
) {
    let entity = trigger.event().entity;

    if let Ok(sensor_axis) = sensor_query.get(entity) {
        let sensor_key = format!("{}:{}", sensor_axis.device_id, sensor_axis.sensor_name);
        frame_visibility.hovered_sensor_axis = Some(sensor_key);
    }
}

/// Observer: Handle mouse leaving a sensor axis frame
fn on_sensor_axis_out(
    trigger: On<Pointer<Out>>,
    sensor_query: Query<&SensorAxisEntity>,
    mut frame_visibility: ResMut<FrameVisibility>,
) {
    let entity = trigger.event().entity;

    if sensor_query.get(entity).is_ok() {
        frame_visibility.hovered_sensor_axis = None;
    }
}

/// Observer: Handle mouse entering a sensor FOV geometry
fn on_sensor_fov_over(
    trigger: On<Pointer<Over>>,
    sensor_query: Query<&SensorFovEntity>,
    mut frame_visibility: ResMut<FrameVisibility>,
) {
    let entity = trigger.event().entity;

    if let Ok(sensor_fov) = sensor_query.get(entity) {
        let sensor_key = format!("{}:{}", sensor_fov.device_id, sensor_fov.sensor_name);
        frame_visibility.hovered_sensor_fov = Some(sensor_key);
    }
}

/// Observer: Handle mouse leaving a sensor FOV geometry
fn on_sensor_fov_out(
    trigger: On<Pointer<Out>>,
    sensor_query: Query<&SensorFovEntity>,
    mut frame_visibility: ResMut<FrameVisibility>,
) {
    let entity = trigger.event().entity;

    if sensor_query.get(entity).is_ok() {
        frame_visibility.hovered_sensor_fov = None;
    }
}

/// Render tooltip for hovered sensor axis frame using egui
fn render_sensor_axis_tooltip(
    mut contexts: EguiContexts,
    frame_visibility: Res<FrameVisibility>,
    sensor_query: Query<&SensorAxisEntity>,
) {
    let Some(ref hovered_key) = frame_visibility.hovered_sensor_axis else {
        return;
    };

    // Find the sensor axis entity with matching key
    let mut sensor_name = String::new();
    let mut category = String::new();
    let mut sensor_type = String::new();
    let mut driver: Option<String> = None;
    let mut axis_align: Option<crate::app::AxisAlignData> = None;

    for sensor in sensor_query.iter() {
        let key = format!("{}:{}", sensor.device_id, sensor.sensor_name);
        if &key == hovered_key {
            sensor_name = sensor.sensor_name.clone();
            category = sensor.category.clone();
            sensor_type = sensor.sensor_type.clone();
            driver = sensor.driver.clone();
            axis_align = sensor.axis_align.clone();
            break;
        }
    }

    if sensor_name.is_empty() {
        return;
    }

    let Ok(ctx) = contexts.ctx_mut() else { return };

    if let Some(pos) = ctx.pointer_hover_pos() {
        egui::Area::new(egui::Id::new("sensor_axis_tooltip"))
            .fixed_pos(egui::pos2(pos.x + 15.0, pos.y + 15.0))
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .show(ui, |ui| {
                        ui.set_min_width(150.0);
                        ui.label(egui::RichText::new(format!("{} (sensor)", sensor_name)).strong());
                        ui.label(format!("{}/{}", category, sensor_type));
                        if let Some(ref drv) = driver {
                            ui.label(format!("Driver: {}", drv));
                        }
                        if let Some(ref align) = axis_align {
                            ui.label(
                                egui::RichText::new(format!("Axis: X={} Y={} Z={}", align.x, align.y, align.z))
                                    .color(egui::Color32::YELLOW)
                            );
                        }
                    });
            });
    }
}

/// Render tooltip for hovered sensor FOV using egui
fn render_sensor_fov_tooltip(
    mut contexts: EguiContexts,
    frame_visibility: Res<FrameVisibility>,
    sensor_query: Query<&SensorFovEntity>,
) {
    let Some(ref hovered_key) = frame_visibility.hovered_sensor_fov else {
        return;
    };

    // Find the sensor FOV entity with matching key
    let mut sensor_name = String::new();
    let mut category = String::new();
    let mut sensor_type = String::new();
    let mut driver: Option<String> = None;
    let mut axis_align: Option<crate::app::AxisAlignData> = None;

    for sensor in sensor_query.iter() {
        let key = format!("{}:{}", sensor.device_id, sensor.sensor_name);
        if &key == hovered_key {
            sensor_name = sensor.sensor_name.clone();
            category = sensor.category.clone();
            sensor_type = sensor.sensor_type.clone();
            driver = sensor.driver.clone();
            axis_align = sensor.axis_align.clone();
            break;
        }
    }

    if sensor_name.is_empty() {
        return;
    }

    let Ok(ctx) = contexts.ctx_mut() else { return };

    if let Some(pos) = ctx.pointer_hover_pos() {
        egui::Area::new(egui::Id::new("sensor_fov_tooltip"))
            .fixed_pos(egui::pos2(pos.x + 15.0, pos.y + 15.0))
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .show(ui, |ui| {
                        ui.set_min_width(150.0);
                        ui.label(egui::RichText::new(format!("{} FOV", sensor_name)).strong());
                        ui.label(format!("{}/{}", category, sensor_type));
                        if let Some(ref drv) = driver {
                            ui.label(format!("Driver: {}", drv));
                        }
                        if let Some(ref align) = axis_align {
                            ui.label(
                                egui::RichText::new(format!("Axis: X={} Y={} Z={}", align.x, align.y, align.z))
                                    .color(egui::Color32::YELLOW)
                            );
                        }
                    });
            });
    }
}

/// Observer: Handle mouse entering a port geometry
fn on_port_over(
    trigger: On<Pointer<Over>>,
    port_query: Query<&PortEntity>,
    port_mesh_query: Query<&PortMeshTarget>,
    mut frame_visibility: ResMut<FrameVisibility>,
) {
    let entity = trigger.event().entity;

    // Check PortEntity (fallback geometry)
    if let Ok(port) = port_query.get(entity) {
        let port_key = format!("{}:{}", port.device_id, port.port_name);
        frame_visibility.hovered_port = Some(port_key);
        frame_visibility.hovered_port_from_ui = false;
        return;
    }

    // Check PortMeshTarget (GLTF mesh-based ports)
    if let Ok(port_mesh) = port_mesh_query.get(entity) {
        let port_key = format!("{}:{}", port_mesh.device_id, port_mesh.port_name);
        frame_visibility.hovered_port = Some(port_key);
        frame_visibility.hovered_port_from_ui = false;
    }
}

/// Observer: Handle mouse leaving a port geometry
fn on_port_out(
    trigger: On<Pointer<Out>>,
    port_query: Query<&PortEntity>,
    port_mesh_query: Query<&PortMeshTarget>,
    mut frame_visibility: ResMut<FrameVisibility>,
) {
    let entity = trigger.event().entity;

    // Only clear if this is a port entity AND hover wasn't set by UI
    let is_port = port_query.get(entity).is_ok() || port_mesh_query.get(entity).is_ok();
    if is_port && !frame_visibility.hovered_port_from_ui {
        frame_visibility.hovered_port = None;
    }
}

/// Render tooltip for hovered port using egui
fn render_port_tooltip(
    mut contexts: EguiContexts,
    frame_visibility: Res<FrameVisibility>,
    port_query: Query<&PortEntity>,
    port_mesh_query: Query<&PortMeshTarget>,
) {
    let Some(ref hovered_key) = frame_visibility.hovered_port else {
        return;
    };

    // Find the port entity with matching key (check both PortEntity and PortMeshTarget)
    let mut port_name = String::new();
    let mut port_type = String::new();

    // Check PortEntity (fallback geometry)
    for port in port_query.iter() {
        let key = format!("{}:{}", port.device_id, port.port_name);
        if &key == hovered_key {
            port_name = port.port_name.clone();
            port_type = port.port_type.clone();
            break;
        }
    }

    // Check PortMeshTarget (GLTF mesh-based ports) if not found
    if port_name.is_empty() {
        for port_mesh in port_mesh_query.iter() {
            let key = format!("{}:{}", port_mesh.device_id, port_mesh.port_name);
            if &key == hovered_key {
                port_name = port_mesh.port_name.clone();
                port_type = port_mesh.port_type.clone();
                break;
            }
        }
    }

    if port_name.is_empty() {
        return;
    }

    let Ok(ctx) = contexts.ctx_mut() else { return };

    if let Some(pos) = ctx.pointer_hover_pos() {
        egui::Area::new(egui::Id::new("port_tooltip"))
            .fixed_pos(egui::pos2(pos.x + 15.0, pos.y + 15.0))
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .show(ui, |ui| {
                        ui.set_min_width(100.0);
                        ui.label(egui::RichText::new(&port_name).strong());
                        // Color the port type like in the UI
                        let type_color = match port_type.to_lowercase().as_str() {
                            "ethernet" => egui::Color32::from_rgb(50, 200, 50),
                            "can" => egui::Color32::from_rgb(255, 200, 50),
                            "spi" => egui::Color32::from_rgb(200, 50, 200),
                            "i2c" => egui::Color32::from_rgb(50, 200, 200),
                            "uart" => egui::Color32::from_rgb(200, 100, 50),
                            "usb" => egui::Color32::from_rgb(50, 100, 200),
                            _ => egui::Color32::GRAY,
                        };
                        ui.label(egui::RichText::new(&port_type).color(type_color));
                    });
            });
    }
}
