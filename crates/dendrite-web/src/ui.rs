//! UI overlays using bevy_egui

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};

use crate::app::{ActiveRotationAxis, ActiveRotationField, CameraSettings, ConnectionDialog, DeviceOrientations, DevicePositions, DeviceRegistry, DeviceStatus, FrameVisibility, SelectedDevice, ShowRotationAxis, UiLayout, WorldSettings};
use crate::network::{DaemonConfig, HeartbeatState, NetworkInterfaces, ReconnectEvent, toggle_heartbeat, trigger_scan_on_interface};

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        // UI layout updates run in Update
        app.add_systems(Update, update_ui_layout)
            // Main UI system runs in EguiPrimaryContextPass for proper input handling (bevy_egui 0.38+)
            .add_systems(EguiPrimaryContextPass, ui_system);
    }
}

/// Update UI layout based on window size
fn update_ui_layout(
    windows: Query<&Window>,
    mut ui_layout: ResMut<UiLayout>,
) {
    if let Ok(window) = windows.single() {
        let width = window.width();
        let height = window.height();

        // Only update if dimensions changed significantly
        if (ui_layout.screen_width - width).abs() > 1.0
            || (ui_layout.screen_height - height).abs() > 1.0
        {
            ui_layout.update_for_screen(width, height);
        }
    }
}

fn ui_system(
    mut contexts: EguiContexts,
    registry: Res<DeviceRegistry>,
    mut selected: ResMut<SelectedDevice>,
    mut camera_settings: ResMut<CameraSettings>,
    mut positions: ResMut<DevicePositions>,
    mut orientations: ResMut<DeviceOrientations>,
    mut rotation_state: (ResMut<ActiveRotationField>, ResMut<ShowRotationAxis>),
    mut world_settings: ResMut<WorldSettings>,
    mut frame_visibility: ResMut<FrameVisibility>,
    mut device_query: Query<(&crate::scene::DeviceEntity, &mut Transform)>,
    mut network_interfaces: ResMut<NetworkInterfaces>,
    mut heartbeat_state: ResMut<HeartbeatState>,
    mut ui_layout: ResMut<UiLayout>,
    daemon_config: Res<DaemonConfig>,
    mut connection_dialog: ResMut<ConnectionDialog>,
    mut reconnect_events: MessageWriter<ReconnectEvent>,
) {
    let (ref mut active_rotation_field, ref mut show_rotation_axis) = rotation_state;
    let is_mobile = ui_layout.is_mobile;
    let panel_width = ui_layout.panel_width();
    let ui_scale = ui_layout.ui_scale;

    // Get the egui context - early return if not available
    let Ok(ctx) = contexts.ctx_mut() else { return };

    // Set up style for mobile - larger text and touch targets
    if is_mobile {
        let mut style = (*ctx.style()).clone();
        style.spacing.button_padding = egui::vec2(12.0, 8.0);
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        ctx.set_style(style);
    }

    // Mobile: Show toggle buttons at top
    if is_mobile {
        egui::TopBottomPanel::top("mobile_toolbar")
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // Menu toggle button
                    let menu_text = if ui_layout.show_left_panel { "☰ Menu" } else { "☰" };
                    if ui.button(egui::RichText::new(menu_text).size(16.0 * ui_scale)).clicked() {
                        ui_layout.show_left_panel = !ui_layout.show_left_panel;
                        // Hide other panel when opening this one on mobile
                        if ui_layout.show_left_panel {
                            ui_layout.show_right_panel = false;
                        }
                    }

                    ui.separator();

                    // Connection status indicator
                    let status_color = if registry.connected {
                        egui::Color32::GREEN
                    } else {
                        egui::Color32::RED
                    };
                    ui.colored_label(status_color, "●");

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Details toggle (only if device selected)
                        if selected.0.is_some() {
                            let details_text = if ui_layout.show_right_panel { "Details ✕" } else { "Details" };
                            if ui.button(egui::RichText::new(details_text).size(16.0 * ui_scale)).clicked() {
                                ui_layout.show_right_panel = !ui_layout.show_right_panel;
                                // Hide other panel when opening this one on mobile
                                if ui_layout.show_right_panel {
                                    ui_layout.show_left_panel = false;
                                }
                            }
                        }
                    });
                });
            });
    }

    // Device list panel (left side)
    if !is_mobile || ui_layout.show_left_panel {
        egui::SidePanel::left("devices_panel")
            .default_width(panel_width)
            .resizable(!is_mobile)
            .show(ctx, |ui| {
                // On mobile, add a close button at the top
                if is_mobile {
                    ui.horizontal(|ui| {
                        ui.heading(egui::RichText::new("Devices").size(18.0 * ui_scale));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(egui::RichText::new("✕").size(18.0 * ui_scale)).clicked() {
                                ui_layout.show_left_panel = false;
                            }
                        });
                    });
                } else {
                    ui.heading("Devices");
                }

                ui.separator();

                // Connection status
                let status_color = if registry.connected {
                    egui::Color32::GREEN
                } else {
                    egui::Color32::RED
                };
                ui.horizontal(|ui| {
                    ui.colored_label(status_color, "●");
                    if registry.connected {
                        // Show truncated URL when connected
                        let url_display = if daemon_config.http_url.len() > 25 {
                            format!("{}...", &daemon_config.http_url[..22])
                        } else {
                            daemon_config.http_url.clone()
                        };
                        ui.label(url_display);
                    } else {
                        ui.label("Disconnected");
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Connect").clicked() {
                            connection_dialog.show = true;
                            // Pre-fill with current address if we have one
                            if daemon_config.http_url.starts_with("http://") {
                                connection_dialog.daemon_address = daemon_config.http_url
                                    .trim_start_matches("http://")
                                    .to_string();
                            } else if daemon_config.http_url.starts_with("https://") {
                                connection_dialog.daemon_address = daemon_config.http_url
                                    .trim_start_matches("https://")
                                    .to_string();
                            }
                        }
                    });
                });
                ui.separator();

                // Network Interface Selector
                egui::CollapsingHeader::new(egui::RichText::new("Discovery").size(14.0 * ui_scale))
                    .default_open(!is_mobile) // Collapsed by default on mobile
                    .show(ui, |ui| {
                        if network_interfaces.interfaces.is_empty() {
                            ui.label("Loading interfaces...");
                        } else {
                            // Interface dropdown
                            let selected_label = network_interfaces.selected_index
                                .and_then(|i| network_interfaces.interfaces.get(i))
                                .map(|iface| format!("{} ({})", iface.name, iface.ip))
                                .unwrap_or_else(|| "Select interface".to_string());

                            // Collect interface labels to avoid borrow issues
                            let interface_labels: Vec<_> = network_interfaces.interfaces.iter()
                                .map(|iface| format!("{} ({}/{})", iface.name, iface.subnet, iface.prefix_len))
                                .collect();
                            let current_selected = network_interfaces.selected_index;

                            let mut new_selected = current_selected;
                            egui::ComboBox::from_label("Interface")
                                .selected_text(&selected_label)
                                .show_ui(ui, |ui| {
                                    for (i, label) in interface_labels.iter().enumerate() {
                                        if ui.selectable_label(
                                            current_selected == Some(i),
                                            label
                                        ).clicked() {
                                            new_selected = Some(i);
                                        }
                                    }
                                });
                            network_interfaces.selected_index = new_selected;

                            // Show selected subnet info
                            if let Some(i) = network_interfaces.selected_index {
                                if let Some(iface) = network_interfaces.interfaces.get(i) {
                                    ui.label(format!("Subnet: {}/{}", iface.subnet, iface.prefix_len));

                                    // Scan button - larger on mobile
                                    let subnet = iface.subnet.clone();
                                    let prefix = iface.prefix_len;
                                    let button = if is_mobile {
                                        egui::Button::new(egui::RichText::new("Scan Network").size(16.0 * ui_scale))
                                            .min_size(egui::vec2(0.0, 40.0))
                                    } else {
                                        egui::Button::new("Scan Network")
                                    };
                                    if ui.add(button).clicked() {
                                        trigger_scan_on_interface(&subnet, prefix, &daemon_config.http_url);
                                        network_interfaces.scan_in_progress = true;
                                    }
                                }
                            }
                        }

                        // Connection checking checkbox
                        ui.add_space(8.0);
                        let mut check_connection = heartbeat_state.enabled;
                        if ui.checkbox(&mut check_connection, "Check connection").changed() {
                            heartbeat_state.enabled = check_connection;
                            toggle_heartbeat(check_connection, &daemon_config.http_url);
                        }
                        ui.label(
                            egui::RichText::new("Sends ARP pings to check device connectivity")
                                .size(11.0 * ui_scale)
                                .color(egui::Color32::GRAY)
                        );
                    });

                ui.separator();

                // Device list
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for device in &registry.devices {
                        let is_selected = selected.0.as_ref() == Some(&device.id);

                        // Device name color depends on device status and heartbeat state
                        // Offline devices always show red (they were seen offline)
                        // Online devices show white when heartbeat is off (status unknown)
                        let name_color = match device.status {
                            DeviceStatus::Offline => egui::Color32::from_rgb(200, 100, 100), // Always red
                            DeviceStatus::Online => {
                                if heartbeat_state.enabled {
                                    egui::Color32::from_rgb(100, 200, 100) // Green when checking
                                } else {
                                    egui::Color32::from_rgb(200, 200, 200) // White when not checking
                                }
                            }
                            DeviceStatus::Unknown => egui::Color32::GRAY,
                        };

                        let text = egui::RichText::new(&device.name)
                            .color(name_color)
                            .size(14.0 * ui_scale);

                        // On mobile, make the entire row a larger touch target
                        let response = if is_mobile {
                            ui.add_sized(
                                [ui.available_width(), 36.0 * ui_scale],
                                egui::Button::new(text).selected(is_selected)
                            )
                        } else {
                            ui.selectable_label(is_selected, text)
                        };

                        if response.clicked() {
                            selected.0 = Some(device.id.clone());
                            // On mobile, show the details panel when a device is selected
                            if is_mobile {
                                ui_layout.show_right_panel = true;
                                ui_layout.show_left_panel = false;
                            }
                        }

                        // Show inline details on desktop only (mobile uses right panel)
                        // Note: last_seen is shown in right panel, not here
                        if is_selected && !is_mobile {
                            ui.indent("device_details", |ui| {
                                ui.label(format!("ID: {}", &device.id));
                                ui.label(format!("IP: {}", &device.ip));
                                if let Some(board) = &device.board {
                                    ui.label(format!("Board: {}", board));
                                }
                                if let Some(port) = device.port {
                                    ui.label(format!("Port: {}", port));
                                }
                                if let Some(version) = &device.version {
                                    ui.label(format!("Firmware: {}", version));
                                }
                            });
                        }
                    }
                });

                ui.separator();

                ui.label(format!("{} devices", registry.devices.len()));

                ui.separator();

                // World Settings - collapsible section
                egui::CollapsingHeader::new(egui::RichText::new("World Settings").size(14.0 * ui_scale))
                    .default_open(false)
                    .show(ui, |ui| {
                        // Reset view button
                        let reset_button = if is_mobile {
                            egui::Button::new(egui::RichText::new("Reset View").size(14.0 * ui_scale))
                                .min_size(egui::vec2(0.0, 36.0))
                        } else {
                            egui::Button::new("Reset View")
                        };
                        if ui.add(reset_button).clicked() {
                            camera_settings.target_focus = Vec3::ZERO;
                            camera_settings.target_distance = 0.6;
                            camera_settings.azimuth = 0.8;
                            camera_settings.elevation = 0.5;
                        }

                        ui.separator();

                        // Grid toggle
                        ui.checkbox(&mut world_settings.show_grid, "Show Grid");

                        // Axis toggle
                        ui.checkbox(&mut world_settings.show_axis, "Show World Axis");

                        ui.separator();

                        // Grid spacing control
                        ui.label("Grid Spacing:");
                        ui.add(
                            egui::DragValue::new(&mut world_settings.grid_spacing)
                                .speed(0.01)
                                .range(0.01..=1.0)
                                .suffix(" m")
                        );

                        // Grid line thickness control
                        ui.label("Line Thickness:");
                        ui.add(
                            egui::DragValue::new(&mut world_settings.grid_line_thickness)
                                .speed(0.0001)
                                .range(0.0001..=0.01)
                                .suffix(" m")
                        );

                        // Grid alpha control
                        ui.label("Grid Opacity:");
                        ui.add(
                            egui::Slider::new(&mut world_settings.grid_alpha, 0.0..=1.0)
                        );
                    });
            });
    }

    // Info panel (bottom) - hide on mobile to save space
    if !is_mobile {
        egui::TopBottomPanel::bottom("info_panel")
            .max_height(100.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Dendrite - CogniPilot Hardware Visualization");
                    ui.separator();
                    ui.label("ENU: X=East, Y=North, Z=Up | FLU: Forward=X, Left=Y, Up=Z");
                    ui.separator();
                    ui.label("Drag to orbit | Scroll to zoom | Right-drag to pan");
                });
            });
    }

    // Selected device details (right side, only if selected)
    if let Some(id) = selected.0.clone() {
        if let Some(device) = registry.devices.iter().find(|d| d.id == id) {
            if !is_mobile || ui_layout.show_right_panel {
                egui::SidePanel::right("details_panel")
                    .default_width(if is_mobile { panel_width } else { 300.0 })
                    .resizable(!is_mobile)
                    .show(ctx, |ui| {
                        // On mobile, add close button
                        if is_mobile {
                            ui.horizontal(|ui| {
                                ui.heading(egui::RichText::new(&device.name).size(18.0 * ui_scale));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.button(egui::RichText::new("✕").size(18.0 * ui_scale)).clicked() {
                                        ui_layout.show_right_panel = false;
                                    }
                                });
                            });
                        } else {
                            ui.heading(&device.name);
                        }

                        ui.separator();

                        egui::ScrollArea::vertical().show(ui, |ui| {
                            egui::Grid::new("device_grid")
                                .num_columns(2)
                                .spacing([10.0, 4.0 * ui_scale])
                                .show(ui, |ui| {
                                    ui.label("Device ID:");
                                    ui.label(&device.id);
                                    ui.end_row();

                                    ui.label("Status:");
                                    // Show "Unknown" when heartbeat checking is off (only for online devices)
                                    // Offline devices always show "Offline" - they were seen offline
                                    let status_str = match device.status {
                                        DeviceStatus::Offline => "Offline",
                                        DeviceStatus::Online => {
                                            if heartbeat_state.enabled {
                                                "Online"
                                            } else {
                                                "Unknown"
                                            }
                                        }
                                        DeviceStatus::Unknown => "Unknown",
                                    };
                                    ui.label(status_str);
                                    ui.end_row();

                                    ui.label("IP Address:");
                                    ui.label(&device.ip);
                                    ui.end_row();

                                    if let Some(port) = device.port {
                                        ui.label("Switch Port:");
                                        ui.label(format!("{}", port));
                                        ui.end_row();
                                    }

                                    if let Some(ref board) = device.board {
                                        ui.label("Board:");
                                        ui.label(board);
                                        ui.end_row();
                                    }

                                    if let Some(ref version) = device.version {
                                        ui.label("Firmware:");
                                        ui.label(version);
                                        ui.end_row();
                                    }

                                    ui.label("Last Seen:");
                                    // Show "Now" if device is online, otherwise show the timestamp
                                    if device.status == DeviceStatus::Online {
                                        ui.label("Now");
                                    } else if let Some(ref last_seen) = device.last_seen {
                                        ui.label(format_last_seen(last_seen));
                                    } else {
                                        ui.label("Unknown");
                                    }
                                    ui.end_row();

                                    // Editable Position (ENU)
                                    ui.label("Position (ENU):");
                                    ui.label("");
                                    ui.end_row();

                                    let current_pos = positions.positions.get(&id).cloned().unwrap_or(Vec3::ZERO);

                                    // Editable X field
                                    ui.label("  X (East):");
                                    let mut x_val = current_pos.x;
                                    let x_response = ui.add(
                                        egui::DragValue::new(&mut x_val)
                                            .speed(0.01)
                                            .suffix(" m")
                                    );
                                    ui.end_row();

                                    // Editable Y field
                                    ui.label("  Y (North):");
                                    let mut y_val = current_pos.y;
                                    let y_response = ui.add(
                                        egui::DragValue::new(&mut y_val)
                                            .speed(0.01)
                                            .suffix(" m")
                                    );
                                    ui.end_row();

                                    // Editable Z field
                                    ui.label("  Z (Up):");
                                    let mut z_val = current_pos.z;
                                    let z_response = ui.add(
                                        egui::DragValue::new(&mut z_val)
                                            .speed(0.01)
                                            .suffix(" m")
                                    );
                                    ui.end_row();

                                    // Apply position changes if any field was modified
                                    if x_response.changed() || y_response.changed() || z_response.changed() {
                                        let new_pos = Vec3::new(x_val, y_val, z_val);

                                        // Update stored position
                                        positions.positions.insert(id.clone(), new_pos);

                                        // Update the device's transform
                                        for (device, mut transform) in device_query.iter_mut() {
                                            if device.device_id == id {
                                                transform.translation = new_pos;
                                                break;
                                            }
                                        }
                                    }

                                    // Show rotation axis checkbox (unchecked by default)
                                    ui.label("Show Rotation Axis:");
                                    if ui.checkbox(&mut show_rotation_axis.0, "").changed() {
                                        // Value already updated by checkbox
                                    }
                                    ui.end_row();

                                    // Show orientation from 3D scene
                                    // Get stored Euler angles (these are display values, not used to compute rotation)
                                    let orient = orientations.orientations.get(&id).cloned().unwrap_or(Vec3::ZERO);

                                    ui.label("Orientation (FLU):");
                                    ui.label("");
                                    ui.end_row();

                                    // Editable Roll field
                                    ui.label("  Roll:");
                                    let mut roll_deg = orient.x.to_degrees();
                                    let roll_response = ui.add(
                                        egui::DragValue::new(&mut roll_deg)
                                            .speed(1.0)
                                            .suffix("°")
                                    );
                                    let roll_active = roll_response.has_focus() || roll_response.dragged() || roll_response.hovered();
                                    ui.end_row();

                                    // Editable Pitch field
                                    ui.label("  Pitch:");
                                    let mut pitch_deg = orient.y.to_degrees();
                                    let pitch_response = ui.add(
                                        egui::DragValue::new(&mut pitch_deg)
                                            .speed(1.0)
                                            .suffix("°")
                                    );
                                    let pitch_active = pitch_response.has_focus() || pitch_response.dragged() || pitch_response.hovered();
                                    ui.end_row();

                                    // Editable Yaw field
                                    ui.label("  Yaw:");
                                    let mut yaw_deg = orient.z.to_degrees();
                                    let yaw_response = ui.add(
                                        egui::DragValue::new(&mut yaw_deg)
                                            .speed(1.0)
                                            .suffix("°")
                                    );
                                    let yaw_active = yaw_response.has_focus() || yaw_response.dragged() || yaw_response.hovered();
                                    ui.end_row();

                                    // Update active rotation field based on which is active
                                    let new_axis = if roll_active {
                                        ActiveRotationAxis::Roll
                                    } else if pitch_active {
                                        ActiveRotationAxis::Pitch
                                    } else if yaw_active {
                                        ActiveRotationAxis::Yaw
                                    } else {
                                        ActiveRotationAxis::None
                                    };

                                    // Only update if changed to trigger change detection
                                    if active_rotation_field.axis != new_axis {
                                        active_rotation_field.axis = new_axis;
                                    }

                                    // Apply Euler XYZ rotation
                                    if roll_response.changed() || pitch_response.changed() || yaw_response.changed() {
                                        let roll_rad = roll_deg.to_radians();
                                        let pitch_rad = pitch_deg.to_radians();
                                        let yaw_rad = yaw_deg.to_radians();

                                        // Store the Euler angles
                                        orientations.orientations.insert(
                                            id.clone(),
                                            Vec3::new(roll_rad, pitch_rad, yaw_rad)
                                        );

                                        // Update the device's rotation quaternion using XYZ Euler order
                                        for (device, mut transform) in device_query.iter_mut() {
                                            if device.device_id == id {
                                                transform.rotation = Quat::from_euler(
                                                    EulerRot::XYZ,
                                                    roll_rad,
                                                    pitch_rad,
                                                    yaw_rad
                                                );
                                                break;
                                            }
                                        }
                                    }
                                });

                            ui.separator();

                            // Per-device frame visibility toggle (if device has frames or sensors)
                            // Sensor axis frames are also controlled by this toggle
                            let frame_count = device.frames.len();
                            let sensor_count = device.sensors.len();
                            if frame_count > 0 || sensor_count > 0 {
                                let mut show_frames = frame_visibility.show_frames_for(&id);
                                if ui.checkbox(&mut show_frames, "Show Reference Frames").changed() {
                                    frame_visibility.set_show_frames(&id, show_frames);
                                }
                                // Build description showing both frame and sensor counts
                                let description = match (frame_count, sensor_count) {
                                    (0, s) => format!("{} sensor frame(s)", s),
                                    (f, 0) => format!("{} frame(s) defined", f),
                                    (f, s) => format!("{} frame(s) + {} sensor", f, s),
                                };
                                ui.label(
                                    egui::RichText::new(description)
                                        .size(11.0 * ui_scale)
                                        .color(egui::Color32::GRAY)
                                );

                                // Individual frame toggles (collapsible, only shown when frames are enabled)
                                if show_frames && (frame_count > 0 || sensor_count > 0) {
                                    let header_text = format!("Frame Details ({})", frame_count + sensor_count);
                                    egui::CollapsingHeader::new(egui::RichText::new(&header_text).size(12.0 * ui_scale))
                                        .default_open(false)
                                        .show(ui, |ui| {
                                            // Named frames section
                                            if frame_count > 0 {
                                                ui.label(
                                                    egui::RichText::new("Named Frames")
                                                        .size(10.0 * ui_scale)
                                                        .color(egui::Color32::GRAY)
                                                );
                                                for frame in &device.frames {
                                                    ui.horizontal(|ui| {
                                                        let mut frame_vis = frame_visibility.is_frame_visible(&id, &frame.name);
                                                        if ui.checkbox(&mut frame_vis, "").changed() {
                                                            frame_visibility.set_frame_visible(&id, &frame.name, frame_vis);
                                                        }
                                                        ui.label(
                                                            egui::RichText::new(&frame.name)
                                                                .size(11.0 * ui_scale)
                                                                .color(egui::Color32::LIGHT_GREEN)
                                                        );
                                                    });
                                                    // Show description if available
                                                    if let Some(ref desc) = frame.description {
                                                        ui.indent("frame_desc", |ui| {
                                                            ui.label(
                                                                egui::RichText::new(desc)
                                                                    .size(9.0 * ui_scale)
                                                                    .color(egui::Color32::GRAY)
                                                            );
                                                        });
                                                    }
                                                }
                                            }

                                            // Sensor axis frames section
                                            if sensor_count > 0 {
                                                if frame_count > 0 {
                                                    ui.add_space(4.0);
                                                }
                                                ui.label(
                                                    egui::RichText::new("Sensor Frames")
                                                        .size(10.0 * ui_scale)
                                                        .color(egui::Color32::GRAY)
                                                );
                                                let mut any_sensor_hovered_in_frames = false;
                                                for sensor in &device.sensors {
                                                    let sensor_key = format!("{}:{}", id, sensor.name);
                                                    let is_hovered = frame_visibility.hovered_sensor_from_ui.as_ref() == Some(&sensor_key);

                                                    // Highlight color when hovered
                                                    let name_color = if is_hovered {
                                                        egui::Color32::WHITE
                                                    } else {
                                                        egui::Color32::LIGHT_BLUE
                                                    };

                                                    ui.horizontal(|ui| {
                                                        let mut axis_vis = frame_visibility.is_sensor_axis_visible(&id, &sensor.name);
                                                        if ui.checkbox(&mut axis_vis, "").changed() {
                                                            frame_visibility.set_sensor_axis_visible(&id, &sensor.name, axis_vis);
                                                        }

                                                        // Use selectable_label for hover detection
                                                        let response = ui.selectable_label(
                                                            is_hovered,
                                                            egui::RichText::new(&sensor.name)
                                                                .size(11.0 * ui_scale)
                                                                .color(name_color)
                                                        );

                                                        // Track hover state
                                                        if response.hovered() {
                                                            frame_visibility.hovered_sensor_from_ui = Some(sensor_key);
                                                            any_sensor_hovered_in_frames = true;
                                                        }
                                                    });
                                                }

                                                // Clear hover if no sensor in this device's frame section is hovered
                                                if !any_sensor_hovered_in_frames {
                                                    if let Some(ref hovered) = frame_visibility.hovered_sensor_from_ui.clone() {
                                                        if hovered.starts_with(&format!("{}:", id)) {
                                                            frame_visibility.hovered_sensor_from_ui = None;
                                                        }
                                                    }
                                                }
                                            }
                                        });
                                }

                                ui.separator();
                            }

                            // Per-device sensor visibility toggle (only if device has sensors with FOV)
                            if !device.sensors.is_empty() {
                                // Count sensors with FOV (visualizable) - check both legacy geometry and new fovs
                                let fov_sensor_count = device.sensors.iter()
                                    .filter(|s| s.geometry.is_some() || !s.fovs.is_empty())
                                    .count();

                                // Show Sensors checkbox controls FOV visualization
                                if fov_sensor_count > 0 {
                                    let mut show_sensors = frame_visibility.show_sensors_for(&id);
                                    if ui.checkbox(&mut show_sensors, "Show Sensors").changed() {
                                        frame_visibility.set_show_sensors(&id, show_sensors);
                                    }
                                    ui.label(
                                        egui::RichText::new(format!("{} sensor(s) with FOV", fov_sensor_count))
                                            .size(11.0 * ui_scale)
                                            .color(egui::Color32::GRAY)
                                    );
                                }

                                // Collapsible sensor list with details
                                let header_text = format!("Sensor Details ({})", device.sensors.len());
                                egui::CollapsingHeader::new(egui::RichText::new(&header_text).size(12.0 * ui_scale))
                                    .default_open(false)
                                    .show(ui, |ui| {
                                        ui.label(
                                            egui::RichText::new("Sensor axes shown with Reference Frames")
                                                .size(10.0 * ui_scale)
                                                .color(egui::Color32::GRAY)
                                        );
                                        let mut any_sensor_hovered = false;
                                        for sensor in &device.sensors {
                                            let sensor_key = format!("{}:{}", id, sensor.name);
                                            let is_hovered = frame_visibility.hovered_sensor_from_ui.as_ref() == Some(&sensor_key);
                                            let has_fov = sensor.geometry.is_some() || !sensor.fovs.is_empty();
                                            // Highlight color when hovered
                                            let name_color = if is_hovered {
                                                egui::Color32::WHITE
                                            } else if has_fov {
                                                egui::Color32::LIGHT_BLUE
                                            } else {
                                                egui::Color32::LIGHT_GRAY
                                            };

                                            // Build sensor label text
                                            let label_text = if has_fov {
                                                format!("{} (FOV)", sensor.name)
                                            } else {
                                                sensor.name.clone()
                                            };

                                            // Use selectable_label for built-in hover detection
                                            let response = ui.selectable_label(
                                                is_hovered,
                                                egui::RichText::new(&label_text)
                                                    .size(12.0 * ui_scale)
                                                    .color(name_color)
                                            );

                                            // Track hover state
                                            if response.hovered() {
                                                frame_visibility.hovered_sensor_from_ui = Some(sensor_key);
                                                any_sensor_hovered = true;
                                            }
                                            ui.indent("sensor_detail", |ui| {
                                                ui.label(
                                                    egui::RichText::new(format!("{}/{}", sensor.category, sensor.sensor_type))
                                                        .size(10.0 * ui_scale)
                                                        .color(egui::Color32::GRAY)
                                                );
                                                if let Some(ref driver) = sensor.driver {
                                                    ui.label(
                                                        egui::RichText::new(format!("Driver: {}", driver))
                                                            .size(10.0 * ui_scale)
                                                            .color(egui::Color32::GRAY)
                                                    );
                                                }
                                                // Per-sensor FOV visibility toggle (only for sensors with FOV)
                                                if has_fov {
                                                    ui.horizontal(|ui| {
                                                        let mut show_fov = frame_visibility.is_sensor_fov_visible(&id, &sensor.name);
                                                        if ui.checkbox(&mut show_fov, "").changed() {
                                                            frame_visibility.set_sensor_fov_visible(&id, &sensor.name, show_fov);
                                                        }
                                                        ui.label(
                                                            egui::RichText::new("Show FOV")
                                                                .size(10.0 * ui_scale)
                                                                .color(egui::Color32::LIGHT_BLUE)
                                                        );
                                                    });
                                                    // Show individual FOV names with their colors
                                                    if !sensor.fovs.is_empty() {
                                                        ui.indent("fov_list", |ui| {
                                                            for fov in &sensor.fovs {
                                                                let fov_color = if let Some(c) = fov.color {
                                                                    egui::Color32::from_rgb(
                                                                        (c[0] * 255.0) as u8,
                                                                        (c[1] * 255.0) as u8,
                                                                        (c[2] * 255.0) as u8,
                                                                    )
                                                                } else {
                                                                    egui::Color32::LIGHT_BLUE
                                                                };
                                                                ui.horizontal(|ui| {
                                                                    // Color swatch
                                                                    let (rect, _) = ui.allocate_exact_size(
                                                                        egui::vec2(10.0 * ui_scale, 10.0 * ui_scale),
                                                                        egui::Sense::hover(),
                                                                    );
                                                                    ui.painter().rect_filled(rect, 2.0, fov_color);
                                                                    // FOV name
                                                                    ui.label(
                                                                        egui::RichText::new(&fov.name)
                                                                            .size(9.0 * ui_scale)
                                                                            .color(fov_color)
                                                                    );
                                                                });
                                                            }
                                                        });
                                                    }
                                                }
                                                // Axis alignment toggle (only for sensors with axis_align)
                                                if let Some(ref axis_align) = sensor.axis_align {
                                                    ui.horizontal(|ui| {
                                                        let mut show_aligned = frame_visibility.is_sensor_axis_aligned(&id, &sensor.name);
                                                        if ui.checkbox(&mut show_aligned, "").changed() {
                                                            frame_visibility.set_sensor_axis_aligned(&id, &sensor.name, show_aligned);
                                                        }
                                                        let label_text = if show_aligned {
                                                            format!("Aligned: X={} Y={} Z={}", axis_align.x, axis_align.y, axis_align.z)
                                                        } else {
                                                            "Raw axes".to_string()
                                                        };
                                                        ui.label(
                                                            egui::RichText::new(label_text)
                                                                .size(10.0 * ui_scale)
                                                                .color(egui::Color32::YELLOW)
                                                        );
                                                    });
                                                }
                                            });
                                        }

                                        // Clear hover if no sensor in this device is hovered
                                        if !any_sensor_hovered {
                                            if let Some(ref hovered) = frame_visibility.hovered_sensor_from_ui.clone() {
                                                if hovered.starts_with(&format!("{}:", id)) {
                                                    frame_visibility.hovered_sensor_from_ui = None;
                                                }
                                            }
                                        }
                                    });
                                ui.separator();
                            }

                            // Per-device port visibility toggle (only if device has ports)
                            if !device.ports.is_empty() {
                                let mut show_ports = frame_visibility.show_ports_for(&id);
                                if ui.checkbox(&mut show_ports, "Show Ports").changed() {
                                    frame_visibility.set_show_ports(&id, show_ports);
                                }
                                ui.label(
                                    egui::RichText::new(format!("{} port(s)", device.ports.len()))
                                        .size(11.0 * ui_scale)
                                        .color(egui::Color32::GRAY)
                                );

                                // Show port details when enabled
                                if show_ports {
                                    let mut any_port_hovered = false;
                                    ui.indent("ports", |ui| {
                                        for port in &device.ports {
                                            let port_key = format!("{}:{}", id, port.name);
                                            let is_hovered = frame_visibility.hovered_port.as_ref() == Some(&port_key);
                                            let port_color = match port.port_type.to_lowercase().as_str() {
                                                "ethernet" => egui::Color32::from_rgb(50, 200, 50),
                                                "can" => egui::Color32::from_rgb(255, 200, 50),
                                                "spi" => egui::Color32::from_rgb(200, 50, 200),
                                                "i2c" => egui::Color32::from_rgb(50, 200, 200),
                                                "uart" => egui::Color32::from_rgb(200, 100, 50),
                                                "usb" => egui::Color32::from_rgb(50, 100, 200),
                                                _ => egui::Color32::GRAY,
                                            };
                                            // Highlight text if hovered (either from UI or 3D view)
                                            let display_color = if is_hovered {
                                                egui::Color32::WHITE
                                            } else {
                                                port_color
                                            };

                                            // Build port label text
                                            let label_text = format!("{} ({})", port.name, port.port_type);

                                            // Use selectable_label for built-in hover detection
                                            let response = ui.selectable_label(
                                                is_hovered,
                                                egui::RichText::new(&label_text)
                                                    .size(12.0 * ui_scale)
                                                    .color(display_color)
                                            );

                                            // Set hovered_port when hovering over port name in UI
                                            if response.hovered() {
                                                frame_visibility.hovered_port = Some(port_key);
                                                frame_visibility.hovered_port_from_ui = true;
                                                any_port_hovered = true;
                                            }
                                        }
                                    });

                                    // Clear hovered_port only if:
                                    // 1. No port in this UI list is hovered, AND
                                    // 2. The hover was set by UI (not 3D), AND
                                    // 3. The currently hovered port belongs to this device
                                    if !any_port_hovered && frame_visibility.hovered_port_from_ui {
                                        if let Some(ref hovered) = frame_visibility.hovered_port.clone() {
                                            if hovered.starts_with(&format!("{}:", id)) {
                                                frame_visibility.hovered_port = None;
                                                frame_visibility.hovered_port_from_ui = false;
                                            }
                                        }
                                    }
                                }
                                ui.separator();
                            }

                            // Per-device visual toggle checkboxes (e.g., "Hide case")
                            let toggle_groups = FrameVisibility::get_toggle_groups(&device.visuals);
                            if !toggle_groups.is_empty() {
                                for toggle_group in &toggle_groups {
                                    // Format label as "Hide {group}" with capitalized group name
                                    let label = format!("Hide {}", capitalize_first(toggle_group));
                                    let mut is_hidden = frame_visibility.is_toggle_hidden(&id, toggle_group);
                                    if ui.checkbox(&mut is_hidden, &label).changed() {
                                        frame_visibility.set_toggle_hidden(&id, toggle_group, is_hidden);
                                    }
                                }
                                ui.separator();
                            }

                            // Controls help - shorter on mobile
                            if !is_mobile {
                                ui.label("Controls:");
                                ui.label("• Drag X/Y/Z values to move position");
                                ui.label("• Drag Roll/Pitch/Yaw values to rotate");
                                ui.label("• Click values to type exact numbers");
                                ui.separator();
                            }

                            // Show remove button only for offline devices
                            if device.status == DeviceStatus::Offline {
                                let remove_button = if is_mobile {
                                    egui::Button::new(
                                        egui::RichText::new("Remove Device")
                                            .size(16.0 * ui_scale)
                                            .color(egui::Color32::from_rgb(200, 100, 100))
                                    ).min_size(egui::vec2(0.0, 40.0))
                                } else {
                                    egui::Button::new("Remove Device")
                                };
                                if ui.add(remove_button).clicked() {
                                    crate::network::remove_device(&device.id, &daemon_config.http_url);
                                    selected.0 = None;
                                    ui_layout.show_right_panel = false;
                                }
                                ui.separator();
                            }

                            let close_button = if is_mobile {
                                egui::Button::new(egui::RichText::new("Close").size(16.0 * ui_scale))
                                    .min_size(egui::vec2(ui.available_width(), 40.0))
                            } else {
                                egui::Button::new("Close")
                            };
                            if ui.add(close_button).clicked() {
                                selected.0 = None;
                                ui_layout.show_right_panel = false;
                            }
                        });
                    });
            }
        }
    }

    // Connection dialog modal
    if connection_dialog.show {
        egui::Window::new("Connect to Daemon")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_min_width(300.0);

                ui.label("Enter the daemon address (host:port):");
                ui.add_space(8.0);

                // Address input
                let response = ui.add(
                    egui::TextEdit::singleline(&mut connection_dialog.daemon_address)
                        .hint_text("e.g., 192.168.1.100:8080")
                        .desired_width(280.0)
                );

                // Show error if any
                if let Some(error) = &connection_dialog.error {
                    ui.colored_label(egui::Color32::RED, error);
                }

                ui.add_space(8.0);

                // Show current connection info
                ui.label(format!("Current: {}", daemon_config.http_url));

                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    if ui.button("Connect").clicked() || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                        let addr = connection_dialog.daemon_address.trim();
                        if !addr.is_empty() {
                            reconnect_events.write(ReconnectEvent {
                                daemon_address: addr.to_string(),
                            });
                            connection_dialog.show = false;
                            connection_dialog.error = None;
                        } else {
                            connection_dialog.error = Some("Please enter a daemon address".to_string());
                        }
                    }

                    if ui.button("Cancel").clicked() {
                        connection_dialog.show = false;
                        connection_dialog.error = None;
                    }
                });

                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                // Help text
                ui.label("Tip: You can also use URL parameters:");
                ui.label("?daemon=192.168.1.100:8080");
            });
    }
}

/// Format a timestamp string (ISO 8601) to a human-readable format
fn format_last_seen(timestamp: &str) -> String {
    // Try to parse the ISO 8601 timestamp and format it nicely
    // Input format: "2026-01-10T03:50:54.127583515Z"
    // Output format: "2026-01-10 03:50:54"
    if let Some(t_pos) = timestamp.find('T') {
        let date = &timestamp[..t_pos];
        let time_part = &timestamp[t_pos + 1..];
        // Take just HH:MM:SS (first 8 chars of time part)
        let time = if time_part.len() >= 8 {
            &time_part[..8]
        } else {
            time_part.trim_end_matches('Z')
        };
        format!("{} {}", date, time)
    } else {
        timestamp.to_string()
    }
}

/// Capitalize the first character of a string
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}
