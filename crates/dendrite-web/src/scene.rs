//! 3D scene management

use bevy::prelude::*;
use bevy::input::mouse::{MouseMotion, MouseWheel};
use bevy::render::mesh::MeshAabb;
use bevy::render::alpha::AlphaMode;

use crate::app::{ActiveRotationAxis, ActiveRotationField, CameraSettings, DeviceOrientations, DevicePositions, SelectedDevice, WorldSettings};

pub struct ScenePlugin;

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TouchState>()
            .add_systems(Startup, setup_scene)
            .add_systems(Update, (
                update_camera,
                handle_device_interaction,
                handle_deselection,
                update_device_positions,
                update_device_orientations,
                update_selection_highlight,
                update_effective_rotation_axis,
                update_world_visibility,
                update_grid_spacing,
            ));
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

/// Marker for the parent device
#[derive(Component)]
pub struct ParentDevice;

/// Marker for grid lines
#[derive(Component)]
pub struct GridLine;

/// Marker for world axis lines (the center X, Y, Z axes)
#[derive(Component)]
pub struct WorldAxis;

/// Marker for device connection lines
#[derive(Component)]
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
pub struct EffectiveRotationAxis {
    pub target_device: String,
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

fn update_camera(
    mut camera_query: Query<&mut Transform, With<MainCamera>>,
    mut settings: ResMut<CameraSettings>,
    mut mouse_motion: EventReader<MouseMotion>,
    mut mouse_wheel: EventReader<MouseWheel>,
    mouse_button: Res<ButtonInput<MouseButton>>,
    touch_input: Res<Touches>,
    time: Res<Time>,
    mut contexts: bevy_egui::EguiContexts,
) {
    // Check if egui wants the mouse - if so, don't process camera controls
    let egui_wants_pointer = contexts.ctx_mut().wants_pointer_input();

    // Collect mouse motion delta
    let mut total_motion = Vec2::ZERO;
    for motion in mouse_motion.read() {
        total_motion += motion.delta;
    }

    // Orbit with left mouse drag (only when UI doesn't want pointer)
    if mouse_button.pressed(MouseButton::Left) && !egui_wants_pointer {
        settings.azimuth -= total_motion.x * settings.sensitivity;
        settings.elevation = (settings.elevation - total_motion.y * settings.sensitivity)
            .clamp(-1.5, 1.5);
    }

    // Pan with right mouse drag (ENU: vertical plane - right and up)
    if mouse_button.pressed(MouseButton::Right) && !egui_wants_pointer {
        // Camera position is at angle azimuth from +X axis
        // Camera right = perpendicular to view, rotated 90° clockwise (when viewed from above)
        // View direction projected to ground: (cos(az), sin(az))
        // Right = 90° CW rotation = (sin(az), -cos(az))
        let right = Vec3::new(settings.azimuth.sin(), -settings.azimuth.cos(), 0.0);
        let up = Vec3::Z;
        let pan_speed = settings.distance * 0.002;
        // Mouse right -> pan right, Mouse up -> pan up in Z
        settings.target_focus += right * total_motion.x * pan_speed;
        settings.target_focus += up * total_motion.y * pan_speed;
    }

    // Translate with middle mouse drag (ENU: ground plane X-Y)
    // Note: Middle mouse may not work in browsers due to WASM limitations
    if mouse_button.pressed(MouseButton::Middle) && !egui_wants_pointer {
        // Camera's right direction projected onto ground plane
        let right = Vec3::new(-settings.azimuth.sin(), settings.azimuth.cos(), 0.0);
        // Camera's forward direction projected onto ground plane
        let forward = Vec3::new(settings.azimuth.cos(), settings.azimuth.sin(), 0.0);
        let pan_speed = settings.distance * 0.002;
        // Mouse right -> move view right, Mouse up -> move view forward
        settings.target_focus -= right * total_motion.x * pan_speed;
        settings.target_focus += forward * total_motion.y * pan_speed;
    }

    // Zoom with scroll - smooth zoom using target_distance (reduced sensitivity)
    // Don't zoom if UI wants the pointer (scrolling in UI panels)
    if !egui_wants_pointer {
        for scroll in mouse_wheel.read() {
            let zoom_factor = 1.0 - scroll.y * settings.zoom_speed * 0.3; // Reduced by 70%
            settings.target_distance = (settings.target_distance * zoom_factor).clamp(0.05, 5.0);
        }
    } else {
        // Drain the scroll events even if we're not using them
        for _ in mouse_wheel.read() {}
    }

    // Touch support for mobile
    if touch_input.iter().count() == 1 && !egui_wants_pointer {
        for touch in touch_input.iter() {
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
            let prev_dist = (t1.position() - t1.delta())
                .distance(t2.position() - t2.delta());
            let zoom_factor = prev_dist / curr_dist.max(1.0);
            settings.target_distance = (settings.target_distance * zoom_factor).clamp(0.05, 5.0);
        }
    }

    // Smooth interpolation for zoom and target
    let dt = time.delta_secs();
    let lerp_factor = 1.0 - (-settings.smooth_factor * 60.0 * dt).exp();
    settings.distance = settings.distance + (settings.target_distance - settings.distance) * lerp_factor;
    settings.target = settings.target + (settings.target_focus - settings.target) * lerp_factor;

    // Update camera position (ENU: Z is up)
    if let Ok(mut transform) = camera_query.get_single_mut() {
        // Spherical coordinates with Z-up
        let x = settings.distance * settings.azimuth.cos() * settings.elevation.cos();
        let y = settings.distance * settings.azimuth.sin() * settings.elevation.cos();
        let z = settings.distance * settings.elevation.sin();

        transform.translation = settings.target + Vec3::new(x, y, z);
        transform.look_at(settings.target, Vec3::Z);
    }
}

/// Track touch state for tap detection
#[derive(Resource, Default)]
pub struct TouchState {
    /// Position where touch started
    start_position: Option<Vec2>,
    /// Whether this touch has moved significantly (is a drag, not a tap)
    is_dragging: bool,
}

/// Handle device selection via mouse click or touch tap
fn handle_device_interaction(
    mut selected: ResMut<SelectedDevice>,
    mut camera_settings: ResMut<CameraSettings>,
    camera_query: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    device_query: Query<(&DeviceEntity, &GlobalTransform)>,
    mouse_button: Res<ButtonInput<MouseButton>>,
    touch_input: Res<Touches>,
    windows: Query<&Window>,
    mut contexts: bevy_egui::EguiContexts,
    mut touch_state: ResMut<TouchState>,
) {
    // Check if egui wants the pointer
    let egui_wants_pointer = contexts.ctx_mut().wants_pointer_input();
    if egui_wants_pointer {
        return;
    }

    let window = windows.single();
    let mut selection_pos: Option<Vec2> = None;

    // Track touch state for tap detection
    if let Some(touch) = touch_input.iter().next() {
        if touch_input.just_pressed(touch.id()) {
            // Touch started
            touch_state.start_position = Some(touch.position());
            touch_state.is_dragging = false;
        } else if touch_state.start_position.is_some() {
            // Check if moved significantly (more than 10 pixels = dragging, not tapping)
            let start = touch_state.start_position.unwrap();
            if touch.position().distance(start) > 10.0 {
                touch_state.is_dragging = true;
            }
        }
    }

    // Detect touch release (tap) - select device if it wasn't a drag
    for touch in touch_input.iter() {
        if touch_input.just_released(touch.id()) {
            if !touch_state.is_dragging {
                if let Some(start_pos) = touch_state.start_position {
                    selection_pos = Some(start_pos);
                }
            }
            touch_state.start_position = None;
            touch_state.is_dragging = false;
        }
    }

    // Handle mouse click (desktop)
    if mouse_button.just_pressed(MouseButton::Left) {
        if let Some(cursor_pos) = window.cursor_position() {
            selection_pos = Some(cursor_pos);
        }
    }

    // Process selection if we have a position to check
    if let Some(pos) = selection_pos {
        let (camera, camera_transform) = camera_query.single();
        if let Ok(ray) = camera.viewport_to_world(camera_transform, pos) {
            let mut closest: Option<(f32, String, Vec3)> = None;

            for (device, transform) in device_query.iter() {
                let to_device = transform.translation() - ray.origin;
                let t = to_device.dot(*ray.direction);
                if t < 0.0 {
                    continue;
                }

                let closest_point = ray.origin + *ray.direction * t;
                let distance_sq = (closest_point - transform.translation()).length_squared();

                // Hit radius of 0.08 meters (increased from 0.05 for easier selection)
                if distance_sq < 0.08 * 0.08 {
                    if closest.is_none() || t < closest.as_ref().unwrap().0 {
                        closest = Some((t, device.device_id.clone(), transform.translation()));
                    }
                }
            }

            if let Some((_, id, pos)) = closest {
                selected.0 = Some(id);
                // Center camera on selected object
                camera_settings.target_focus = pos;
            }
        }
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

/// Update selection highlight - show bounding box and rotation axes
fn update_selection_highlight(
    mut commands: Commands,
    selected: Res<SelectedDevice>,
    active_rotation_field: Res<ActiveRotationField>,
    registry: Res<crate::app::DeviceRegistry>,
    device_query: Query<(Entity, &DeviceEntity, &Transform), (Without<SelectionHighlight>, Without<RotationAxisIndicator>)>,
    mut highlight_query: Query<(Entity, &mut SelectionHighlight, &MeshMaterial3d<StandardMaterial>)>,
    axis_query: Query<(Entity, &RotationAxisIndicator, &MeshMaterial3d<StandardMaterial>)>,
    mut highlight_transform_query: Query<&mut Transform, With<SelectionHighlight>>,
    mut axis_transform_query: Query<&mut Transform, (With<RotationAxisIndicator>, Without<SelectionHighlight>)>,
    children_query: Query<&Children>,
    mesh_query: Query<(&Mesh3d, &GlobalTransform)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Log when active rotation field changes
    if active_rotation_field.is_changed() {
        tracing::warn!("🎯 Active rotation field changed to: {:?}", active_rotation_field.axis);
    }
    // Get currently selected device ID
    let selected_id = selected.0.as_ref();

    // Remove highlights for devices that are no longer selected
    for (entity, highlight, _) in highlight_query.iter_mut() {
        if selected_id != Some(&highlight.target_device) {
            commands.entity(entity).despawn_recursive();
        }
    }

    // Remove axis indicators for devices that are no longer selected
    for (entity, axis, _) in axis_query.iter() {
        if selected_id != Some(&axis.target_device) {
            commands.entity(entity).despawn_recursive();
        }
    }

    // Check if we need to create a new highlight
    let Some(selected_id) = selected_id else {
        return;
    };

    // Check if highlight already exists
    let highlight_exists = highlight_query.iter_mut().any(|(_, h, _)| &h.target_device == selected_id);

    // Get device status from registry
    let device_is_online = registry.devices.iter()
        .find(|d| &d.id == selected_id)
        .map(|d| d.status == crate::app::DeviceStatus::Online)
        .unwrap_or(false);

    // Find the selected device position
    let mut selected_device_pos = None;
    for (entity, device, transform) in device_query.iter() {
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
        for (_, device, transform) in device_query.iter() {
            if &device.device_id == selected_id {
                let device_pos = transform.translation;
                let device_rotation = transform.rotation;

                // Update highlight box edges to rotate with device and update colors
                for (highlight_entity, mut highlight, material_handle) in highlight_query.iter_mut() {
                    if &highlight.target_device == selected_id {
                        if let Ok(mut highlight_transform) = highlight_transform_query.get_mut(highlight_entity) {
                            // Rotate the offset by the device rotation, then add to position
                            let rotated_offset = device_rotation * highlight.offset;
                            highlight_transform.translation = device_pos + rotated_offset;
                            highlight_transform.rotation = device_rotation;
                        }

                        // Update color if status changed
                        if highlight.is_online != device_is_online {
                            highlight.is_online = device_is_online; // Update the stored status
                            if let Some(material) = materials.get_mut(&material_handle.0) {
                                if device_is_online {
                                    // Light green with 50% transparency for online
                                    material.base_color = Color::srgba(0.3, 0.8, 0.3, 0.5);
                                    material.emissive = bevy::color::LinearRgba::new(0.15, 0.4, 0.15, 1.0);
                                } else {
                                    // Dark red with 50% transparency for offline
                                    material.base_color = Color::srgba(0.6, 0.1, 0.1, 0.5);
                                    material.emissive = bevy::color::LinearRgba::new(0.3, 0.05, 0.05, 1.0);
                                }
                            }
                        }
                    }
                }
                break;
            }
        }
    } else {
        // Create highlight box
        // Compute bounding box from all child meshes
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);
        let mut found_mesh = false;

        // Recursively find all mesh children
        fn collect_bounds(
            entity: Entity,
            children_query: &Query<&Children>,
            mesh_query: &Query<(&Mesh3d, &GlobalTransform)>,
            mesh_assets: &Assets<Mesh>,
            parent_transform: &Transform,
            min: &mut Vec3,
            max: &mut Vec3,
            found: &mut bool,
        ) {
            // Check if this entity has a mesh
            if let Ok((mesh_handle, _global_transform)) = mesh_query.get(entity) {
                if let Some(mesh) = mesh_assets.get(&mesh_handle.0) {
                    if let Some(aabb) = mesh.compute_aabb() {
                        let center = Vec3::from(aabb.center);
                        let half = Vec3::from(aabb.half_extents);
                        // Apply parent scale to the bounds
                        let scaled_center = center * parent_transform.scale;
                        let scaled_half = half * parent_transform.scale;
                        *min = min.min(scaled_center - scaled_half);
                        *max = max.max(scaled_center + scaled_half);
                        *found = true;
                    }
                }
            }

            // Check children
            if let Ok(children) = children_query.get(entity) {
                for &child in children.iter() {
                    collect_bounds(child, children_query, mesh_query, mesh_assets, parent_transform, min, max, found);
                }
            }
        }

        collect_bounds(entity, &children_query, &mesh_query, meshes.as_ref(), &device_transform, &mut min, &mut max, &mut found_mesh);

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

        // Color based on device status - light green for online, dark red for offline
        let (base_color, emissive) = if device_is_online {
            // Light green with 50% transparency for online
            (Color::srgba(0.3, 0.8, 0.3, 0.5), bevy::color::LinearRgba::new(0.15, 0.4, 0.15, 1.0))
        } else {
            // Dark red with 50% transparency for offline
            (Color::srgba(0.6, 0.1, 0.1, 0.5), bevy::color::LinearRgba::new(0.3, 0.05, 0.05, 1.0))
        };

        let highlight_material = materials.add(StandardMaterial {
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
            commands.spawn((
                Mesh3d(meshes.add(Cuboid::new(size.x, size.y, size.z))),
                MeshMaterial3d(highlight_material.clone()),
                Transform::from_translation(device_pos + offset),
                SelectionHighlight {
                    target_device: selected_id.clone(),
                    offset,
                    is_online: device_is_online,
                },
            ));
        }

        // Create rotation axis indicators (FLU: Forward=X/Red, Left=Y/Green, Up=Z/Blue)
        // Using Cuboid for axis shafts (axis-aligned, no rotation issues)
        // Axes START at the device center and extend outward (not through center)
        let axis_length = half.max_element() * 1.5; // Full length from center outward
        let axis_thickness = 0.002;
        let cone_height = axis_thickness * 3.0;
        let cone_radius = axis_thickness * 2.0;

        // Body-frame axis directions for positioning
        let body_x = device_transform.rotation * Vec3::X;
        let body_y = device_transform.rotation * Vec3::Y;
        let body_z = device_transform.rotation * Vec3::Z;

        // All axis shafts use device_transform.rotation directly
        // This works because each cuboid is defined in local body frame coordinates
        // and device_transform.rotation transforms from body frame to world frame
        let shaft_rotation = device_transform.rotation;

        // X axis (Roll/Forward - Red) - starts at center, extends along +X in body frame (FLU)
        // Make X axis 3x longer for debugging visibility
        let x_axis_material = materials.add(StandardMaterial {
            base_color: Color::srgb(0.5, 0.1, 0.1),
            emissive: bevy::color::LinearRgba::new(0.5, 0.1, 0.1, 1.0),
            unlit: false,
            alpha_mode: AlphaMode::Blend,
            ..default()
        });
        // Cuboid elongated along LOCAL X, rotated by device rotation to align with body X
        // Position: The cuboid's center needs to be at device_pos + body_x * (length/2)
        // But we must account for how the rotation affects the cuboid's center
        // Since the cuboid is centered at origin in local space, after rotation its center stays at origin
        // So we just translate to where we want the center to be
        let x_shaft_center = device_pos + body_x * (axis_length / 2.0);
        commands.spawn((
            // Mesh3d(meshes.add(Cuboid::new(axis_thickness, aaxis_length, axis_thickness))),
            Mesh3d(meshes.add(Cylinder::new(axis_thickness, axis_length))),
            MeshMaterial3d(x_axis_material.clone()),
            Transform::from_translation(x_shaft_center)
                .with_rotation(shaft_rotation),
            RotationAxisIndicator {
                target_device: selected_id.clone(),
                offset: Vec3::X * (axis_length / 2.0),
                axis: ActiveRotationAxis::Roll,
            },
        ));
        // Cone at positive X end - cone tip points along +Y by default
        let x_cone_center = device_pos + body_x * (axis_length + cone_height / 2.0);
        // Rotate cone so its +Y (tip) points along body_x
        // Use device rotation then rotate -90 around Z to turn Y into X
        let x_cone_rotation = shaft_rotation * Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2);
        commands.spawn((
            Mesh3d(meshes.add(Cone::new(cone_radius, cone_height))),
            MeshMaterial3d(x_axis_material),
            Transform::from_translation(x_cone_center)
                .with_rotation(x_cone_rotation),
            RotationAxisIndicator {
                target_device: selected_id.clone(),
                offset: Vec3::X * (axis_length + cone_height / 2.0),
                axis: ActiveRotationAxis::Roll,
            },
        ));

        // Y axis (Pitch/Left - Green) - starts at center, extends along +Y in body frame (FLU)
        let y_axis_material = materials.add(StandardMaterial {
            base_color: Color::srgb(0.1, 0.5, 0.1),
            emissive: bevy::color::LinearRgba::new(0.1, 0.5, 0.1, 1.0),
            unlit: false,
            alpha_mode: AlphaMode::Blend,
            ..default()
        });
        // Cuboid elongated along LOCAL Y
        let y_shaft_center = device_pos + body_y * (axis_length / 2.0);
        commands.spawn((
            // Mesh3d(meshes.add(Cuboid::new(axis_thickness, axis_length, axis_thickness))),
            Mesh3d(meshes.add(Cylinder::new(axis_thickness, axis_length))),
            MeshMaterial3d(y_axis_material.clone()),
            Transform::from_translation(y_shaft_center)
                .with_rotation(shaft_rotation),
            RotationAxisIndicator {
                target_device: selected_id.clone(),
                offset: Vec3::Y * (axis_length / 2.0),
                axis: ActiveRotationAxis::Pitch,
            },
        ));
        // Cone at positive Y end - cone tip already points along +Y, just apply device rotation
        let y_cone_center = device_pos + body_y * (axis_length + cone_height / 2.0);
        let y_cone_rotation = shaft_rotation; // No extra rotation needed, cone Y aligns with body Y
        commands.spawn((
            Mesh3d(meshes.add(Cone::new(cone_radius, cone_height))),
            MeshMaterial3d(y_axis_material),
            Transform::from_translation(y_cone_center)
                .with_rotation(y_cone_rotation),
            RotationAxisIndicator {
                target_device: selected_id.clone(),
                offset: Vec3::Y * (axis_length + cone_height / 2.0),
                axis: ActiveRotationAxis::Pitch,
            },
        ));

        // Z axis (Yaw/Up - Blue) - starts at center, extends along +Z in body frame (FLU)
        let z_axis_material = materials.add(StandardMaterial {
            base_color: Color::srgb(0.1, 0.1, 0.5),
            emissive: bevy::color::LinearRgba::new(0.1, 0.1, 0.5, 1.0),
            unlit: false,
            alpha_mode: AlphaMode::Blend,
            ..default()
        });
        // Cuboid elongated along LOCAL Z
        let z_shaft_center = device_pos + body_z * (axis_length / 2.0);
        commands.spawn((
            // Mesh3d(meshes.add(Cuboid::new(axis_thickness, axis_length, axis_thickness))),
            Mesh3d(meshes.add(Cylinder::new(axis_thickness, axis_length))),
            MeshMaterial3d(z_axis_material.clone()),
            Transform::from_translation(z_shaft_center)
                .with_rotation(shaft_rotation),
            RotationAxisIndicator {
                target_device: selected_id.clone(),
                offset: Vec3::Z * (axis_length / 2.0),
                axis: ActiveRotationAxis::Yaw,
            },
        ));
        // Cone at positive Z end - rotate cone so +Y points along +Z
        // Rotate +90 around X to turn Y into Z
        let z_cone_center = device_pos + body_z * (axis_length + cone_height / 2.0);
        let z_cone_rotation = shaft_rotation * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        commands.spawn((
            Mesh3d(meshes.add(Cone::new(cone_radius, cone_height))),
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
    let axis_exists = axis_query.iter().any(|(_, a, _)| &a.target_device == selected_id);
    if axis_exists {
        // Find device transform
        for (_, device, transform) in device_query.iter() {
            if &device.device_id == selected_id {
                let device_pos = transform.translation;
                let device_rotation = transform.rotation;

                // Body-frame axis directions
                let body_x = device_rotation * Vec3::X;
                let body_y = device_rotation * Vec3::Y;
                let body_z = device_rotation * Vec3::Z;

                // Update all axis indicators
                // All use device_rotation as base, cones have additional local rotation
                for (axis_entity, axis, material_handle) in axis_query.iter() {
                    if &axis.target_device == selected_id {
                        // Update position and rotation (no scaling)
                        if let Ok(mut axis_transform) = axis_transform_query.get_mut(axis_entity) {
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
                        if let Some(material) = materials.get_mut(&material_handle.0) {
                            let is_active = match active_rotation_field.axis {
                                ActiveRotationAxis::Roll => axis.axis == ActiveRotationAxis::Roll,
                                ActiveRotationAxis::Pitch => axis.axis == ActiveRotationAxis::Pitch,
                                ActiveRotationAxis::Yaw => axis.axis == ActiveRotationAxis::Yaw,
                                ActiveRotationAxis::None => false,
                            };

                            // Debug log when we actually update materials
                            if active_rotation_field.is_changed() {
                                tracing::warn!("  📝 Updating material for axis {:?}, is_active: {}", axis.axis, is_active);
                            }

                            // Define base colors for each axis with transparency
                            // Use moderate emissive values - not too bright
                            let (base_color, emissive, unlit) = match axis.axis {
                                ActiveRotationAxis::Roll => {
                                    if is_active {
                                        // Brighter red when active - moderate emissive
                                        (Color::srgba(0.6, 0.15, 0.15, 1.0), bevy::color::LinearRgba::new(1.2, 0.3, 0.3, 1.0), false)
                                    } else if active_rotation_field.axis != ActiveRotationAxis::None {
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
                                    } else if active_rotation_field.axis != ActiveRotationAxis::None {
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
                                    } else if active_rotation_field.axis != ActiveRotationAxis::None {
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

                            // Debug: log actual material values when changed
                            if active_rotation_field.is_changed() {
                                tracing::warn!("    💡 Material updated - base_color: {:?}, alpha: {}", base_color, base_color.alpha());
                            }
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
    orientations: Res<DeviceOrientations>,
    device_query: Query<(&DeviceEntity, &Transform)>,
    effective_axis_query: Query<Entity, With<EffectiveRotationAxis>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Despawn existing effective axis indicators
    for entity in effective_axis_query.iter() {
        commands.entity(entity).despawn_recursive();
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
        commands.entity(entity).despawn_recursive();
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
