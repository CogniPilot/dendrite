//! UI overlays using bevy_egui

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};

use crate::app::{ActiveRotationAxis, ActiveRotationField, CameraSettings, ConnectionDialog, DeviceOrientations, DevicePositions, DeviceRegistry, DeviceStatus, FirmwareCheckState, FirmwareStatusData, FrameVisibility, GraphVisualization, OtaState, SelectedDevice, ShowRotationAxis, TopologyData, TopologyNode, UiLayout, WorldSettings};
use crate::network::{cancel_ota_update, check_all_firmware, DaemonConfig, HeartbeatState, NetworkInterfaces, OtaUpdateState, PendingFirmwareData, PendingHcdfExport, ReconnectEvent, start_ota_update, toggle_heartbeat, trigger_scan_on_interface, upload_local_firmware, export_hcdf, import_hcdf, save_hcdf_to_server, update_device_position};
use crate::file_picker::{FileFilter, FilePickerContext, FilePickerState, PendingFileResults, trigger_file_open, trigger_file_save};

/// Grouped system parameters for the main UI system to work around Bevy's 16-param limit
#[derive(SystemParam)]
pub struct UiParams<'w, 's> {
    pub contexts: EguiContexts<'w, 's>,
    pub registry: Res<'w, DeviceRegistry>,
    pub selected: ResMut<'w, SelectedDevice>,
    pub camera_settings: ResMut<'w, CameraSettings>,
    pub positions: ResMut<'w, DevicePositions>,
    pub orientations: ResMut<'w, DeviceOrientations>,
    pub active_rotation_field: ResMut<'w, ActiveRotationField>,
    pub show_rotation_axis: ResMut<'w, ShowRotationAxis>,
    pub world_settings: ResMut<'w, WorldSettings>,
    pub frame_visibility: ResMut<'w, FrameVisibility>,
    pub device_query: Query<'w, 's, (&'static crate::scene::DeviceEntity, &'static mut Transform)>,
    pub network_interfaces: ResMut<'w, NetworkInterfaces>,
    pub heartbeat_state: ResMut<'w, HeartbeatState>,
    pub firmware_state: ResMut<'w, FirmwareCheckState>,
    pub pending_firmware: Res<'w, PendingFirmwareData>,
    pub ui_layout: ResMut<'w, UiLayout>,
    pub daemon_config: Res<'w, DaemonConfig>,
    pub connection_dialog: ResMut<'w, ConnectionDialog>,
    pub reconnect_events: MessageWriter<'w, ReconnectEvent>,
    pub ota_state: ResMut<'w, OtaState>,
    pub file_picker_state: ResMut<'w, FilePickerState>,
    pub pending_file_results: Res<'w, PendingFileResults>,
    pub pending_hcdf_export: Res<'w, PendingHcdfExport>,
    pub graph_vis: ResMut<'w, GraphVisualization>,
}

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        // UI layout updates run in Update
        app.add_systems(Update, (update_ui_layout, process_file_picker_results))
            // Main UI system runs in EguiPrimaryContextPass for proper input handling (bevy_egui 0.38+)
            .add_systems(EguiPrimaryContextPass, ui_system);
    }
}

/// Process completed file picker results and dispatch to appropriate handlers
fn process_file_picker_results(
    mut file_picker_state: ResMut<FilePickerState>,
    pending_hcdf_export: Res<PendingHcdfExport>,
    daemon_config: Res<DaemonConfig>,
) {
    // Process completed file picker results
    while let Some(result) = file_picker_state.take_result() {
        if !result.success {
            tracing::error!("File picker operation failed: {:?}", result.error);
            continue;
        }

        match result.context {
            FilePickerContext::FirmwareUpload { device_id } => {
                if let Some(content) = result.content {
                    tracing::info!("Uploading local firmware to device {}: {} ({} bytes)",
                        device_id, result.filename, content.len());
                    upload_local_firmware(&device_id, content, &daemon_config.http_url);
                }
            }
            FilePickerContext::HcdfImport => {
                if let Some(content) = result.content {
                    // Convert bytes to string
                    if let Ok(xml) = String::from_utf8(content) {
                        tracing::warn!("Importing HCDF file: {} ({} bytes)", result.filename, xml.len());
                        import_hcdf(xml, false, &daemon_config.http_url);
                    } else {
                        tracing::error!("HCDF file is not valid UTF-8");
                    }
                }
            }
            FilePickerContext::HcdfExport => {
                // Export was completed (file saved via browser download)
                tracing::info!("HCDF export completed: {}", result.filename);
            }
            FilePickerContext::Custom(name) => {
                tracing::info!("Custom file picker result for '{}': {}", name, result.filename);
            }
        }
    }

    // Check for pending HCDF export data and trigger file save
    if let Ok(mut export_data) = pending_hcdf_export.0.lock() {
        if let Some(content) = export_data.take() {
            // We have HCDF content ready - this means we need to save it
            // But we need access to PendingFileResults to trigger the save
            // This will be handled in the UI system where we have access to all resources
            // For now, store it back - the UI will handle it
            *export_data = Some(content);
        }
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

fn ui_system(mut params: UiParams) {
    let is_mobile = params.ui_layout.is_mobile;
    let panel_width = params.ui_layout.panel_width();
    let ui_scale = params.ui_layout.ui_scale;

    // Get the egui context - early return if not available
    let Ok(ctx) = params.contexts.ctx_mut() else { return };

    // Set up style for mobile - compact but still touch-friendly
    if is_mobile {
        let mut style = (*ctx.style()).clone();
        style.spacing.button_padding = egui::vec2(6.0, 4.0);
        style.spacing.item_spacing = egui::vec2(4.0, 3.0);
        style.spacing.indent = 12.0; // Reduce indent for nested items
        ctx.set_style(style);
    }

    // Mobile: Show toggle buttons at BOTTOM to avoid curved screen edges
    if is_mobile {
        egui::TopBottomPanel::bottom("mobile_toolbar")
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // Menu toggle button (left side)
                    let menu_text = if params.ui_layout.show_left_panel { "☰ Menu" } else { "☰" };
                    if ui.button(egui::RichText::new(menu_text).size(16.0 * ui_scale)).clicked() {
                        params.ui_layout.show_left_panel = !params.ui_layout.show_left_panel;
                        // Hide other panel when opening this one on mobile
                        if params.ui_layout.show_left_panel {
                            params.ui_layout.show_right_panel = false;
                        }
                    }

                    ui.separator();

                    // Connection status indicator
                    let status_color = if params.registry.connected {
                        egui::Color32::GREEN
                    } else {
                        egui::Color32::RED
                    };
                    ui.colored_label(status_color, "●");

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Details toggle (only if device selected)
                        if params.selected.0.is_some() {
                            let details_text = if params.ui_layout.show_right_panel { "Details ✕" } else { "Details" };
                            if ui.button(egui::RichText::new(details_text).size(16.0 * ui_scale)).clicked() {
                                params.ui_layout.show_right_panel = !params.ui_layout.show_right_panel;
                                // Hide other panel when opening this one on mobile
                                if params.ui_layout.show_right_panel {
                                    params.ui_layout.show_left_panel = false;
                                }
                            }
                        }
                    });
                });
            });
    }

    // Device list panel (left side)
    if !is_mobile || params.ui_layout.show_left_panel {
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
                                params.ui_layout.show_left_panel = false;
                            }
                        });
                    });
                } else {
                    ui.heading("Devices");
                }

                ui.separator();

                // Wrap everything in a scroll area so the panel is scrollable
                egui::ScrollArea::vertical().show(ui, |ui| {

                // Connection status
                let status_color = if params.registry.connected {
                    egui::Color32::GREEN
                } else {
                    egui::Color32::RED
                };
                ui.horizontal(|ui| {
                    ui.colored_label(status_color, "●");
                    if params.registry.connected {
                        // Show truncated URL when connected
                        let url_display = if params.daemon_config.http_url.len() > 25 {
                            format!("{}...", &params.daemon_config.http_url[..22])
                        } else {
                            params.daemon_config.http_url.clone()
                        };
                        ui.label(url_display);
                    } else {
                        ui.label("Disconnected");
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Connect").clicked() {
                            params.connection_dialog.show = true;
                            // Pre-fill with current address if we have one
                            if params.daemon_config.http_url.starts_with("http://") {
                                params.connection_dialog.daemon_address = params.daemon_config.http_url
                                    .trim_start_matches("http://")
                                    .to_string();
                            } else if params.daemon_config.http_url.starts_with("https://") {
                                params.connection_dialog.daemon_address = params.daemon_config.http_url
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
                        if params.network_interfaces.interfaces.is_empty() {
                            ui.label("Loading interfaces...");
                        } else {
                            // Interface dropdown
                            let selected_label = params.network_interfaces.selected_index
                                .and_then(|i| params.network_interfaces.interfaces.get(i))
                                .map(|iface| format!("{} ({})", iface.name, iface.ip))
                                .unwrap_or_else(|| "Select interface".to_string());

                            // Collect interface labels to avoid borrow issues
                            let interface_labels: Vec<_> = params.network_interfaces.interfaces.iter()
                                .map(|iface| format!("{} ({}/{})", iface.name, iface.subnet, iface.prefix_len))
                                .collect();
                            let current_selected = params.network_interfaces.selected_index;

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
                            params.network_interfaces.selected_index = new_selected;

                            // Show selected subnet info
                            if let Some(i) = params.network_interfaces.selected_index {
                                if let Some(iface) = params.network_interfaces.interfaces.get(i) {
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
                                        trigger_scan_on_interface(&subnet, prefix, &params.daemon_config.http_url);
                                        params.network_interfaces.scan_in_progress = true;
                                    }
                                }
                            }
                        }

                        // Connection checking checkbox
                        ui.add_space(8.0);
                        let mut check_connection = params.heartbeat_state.enabled;
                        if ui.checkbox(&mut check_connection, "Check connection").changed() {
                            params.heartbeat_state.enabled = check_connection;
                            toggle_heartbeat(check_connection, &params.daemon_config.http_url);
                        }
                        ui.label(
                            egui::RichText::new("Sends ARP pings to check device connectivity")
                                .size(11.0 * ui_scale)
                                .color(egui::Color32::GRAY)
                        );

                        // Firmware checking checkbox
                        ui.add_space(4.0);
                        let mut check_firmware = params.firmware_state.enabled;
                        if ui.checkbox(&mut check_firmware, "Check firmware").changed() {
                            params.firmware_state.enabled = check_firmware;
                            if check_firmware {
                                // Trigger firmware check for all devices
                                check_all_firmware(&params.daemon_config.http_url, &params.pending_firmware);
                            } else {
                                // Clear firmware status when disabled
                                params.firmware_state.device_status.clear();
                            }
                        }
                        ui.label(
                            egui::RichText::new("Checks for firmware updates (yellow = update available)")
                                .size(11.0 * ui_scale)
                                .color(egui::Color32::GRAY)
                        );
                    });

                ui.separator();

                // Device list
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for device in &params.registry.devices {
                        let is_selected = params.selected.0.as_ref() == Some(&device.id);

                        // Device name color depends on device status, firmware status, and heartbeat state
                        // Priority: Offline (red) > Firmware outdated (yellow) > Online (green/white)
                        let name_color = if device.status == DeviceStatus::Offline {
                            egui::Color32::from_rgb(200, 100, 100) // Always red for offline
                        } else if params.firmware_state.enabled {
                            // Check firmware status when enabled
                            match params.firmware_state.device_status.get(&device.id) {
                                Some(FirmwareStatusData::UpdateAvailable { .. }) => {
                                    egui::Color32::from_rgb(230, 200, 50) // Yellow for outdated
                                }
                                Some(FirmwareStatusData::UpToDate) => {
                                    egui::Color32::from_rgb(100, 200, 100) // Green for up to date
                                }
                                _ => {
                                    // Unknown or loading - use connection status color
                                    if params.heartbeat_state.enabled && device.status == DeviceStatus::Online {
                                        egui::Color32::from_rgb(100, 200, 100) // Green
                                    } else {
                                        egui::Color32::from_rgb(200, 200, 200) // White
                                    }
                                }
                            }
                        } else {
                            // Firmware checking disabled - use connection status
                            if params.heartbeat_state.enabled && device.status == DeviceStatus::Online {
                                egui::Color32::from_rgb(100, 200, 100) // Green
                            } else if device.status == DeviceStatus::Unknown {
                                egui::Color32::GRAY
                            } else {
                                egui::Color32::from_rgb(200, 200, 200) // White
                            }
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
                            params.selected.0 = Some(device.id.clone());
                            // On mobile, show the details panel when a device is selected
                            if is_mobile {
                                params.ui_layout.show_right_panel = true;
                                params.ui_layout.show_left_panel = false;
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

                ui.label(format!("{} devices", params.registry.devices.len()));

                ui.separator();

                // HCDF Import/Export - collapsible section
                egui::CollapsingHeader::new(egui::RichText::new("HCDF Configuration").size(14.0 * ui_scale))
                    .default_open(true)
                    .show(ui, |ui| {
                        // Import button
                        ui.horizontal(|ui| {
                            let import_button = if is_mobile {
                                egui::Button::new(egui::RichText::new("Import").size(14.0 * ui_scale))
                                    .min_size(egui::vec2(0.0, 32.0))
                            } else {
                                egui::Button::new("Import")
                            };
                            if ui.add(import_button).clicked() {
                                tracing::warn!("Import button clicked, triggering file picker");
                                trigger_file_open(
                                    &params.pending_file_results,
                                    FilePickerContext::HcdfImport,
                                    FileFilter::hcdf(),
                                );
                            }
                            ui.label(
                                egui::RichText::new("Load .hcdf file")
                                    .size(10.0 * ui_scale)
                                    .color(egui::Color32::GRAY)
                            );
                        });

                        ui.add_space(4.0);

                        // Export options
                        ui.label(egui::RichText::new("Export:").size(12.0 * ui_scale));

                        ui.horizontal(|ui| {
                            // Save to Server button
                            let save_server_button = if is_mobile {
                                egui::Button::new(egui::RichText::new("Save to Server").size(14.0 * ui_scale))
                                    .min_size(egui::vec2(0.0, 32.0))
                            } else {
                                egui::Button::new("Save to Server")
                            };
                            if ui.add(save_server_button).clicked() {
                                save_hcdf_to_server(&params.daemon_config.http_url, None);
                            }
                        });
                        ui.label(
                            egui::RichText::new("Save to dendrite host filesystem")
                                .size(10.0 * ui_scale)
                                .color(egui::Color32::GRAY)
                        );

                        ui.horizontal(|ui| {
                            // Download button (browser download)
                            let download_button = if is_mobile {
                                egui::Button::new(egui::RichText::new("Download").size(14.0 * ui_scale))
                                    .min_size(egui::vec2(0.0, 32.0))
                            } else {
                                egui::Button::new("Download")
                            };
                            if ui.add(download_button).clicked() {
                                // Fetch HCDF from backend, then trigger browser download
                                export_hcdf(&params.daemon_config.http_url, &params.pending_hcdf_export);
                            }
                        });
                        ui.label(
                            egui::RichText::new("Download to this device")
                                .size(10.0 * ui_scale)
                                .color(egui::Color32::GRAY)
                        );

                        // Check if we have pending HCDF export data to save (for browser download)
                        if let Ok(mut export_data) = params.pending_hcdf_export.0.lock() {
                            if let Some(content) = export_data.take() {
                                // Trigger file save with the content
                                trigger_file_save(
                                    &params.pending_file_results,
                                    FilePickerContext::HcdfExport,
                                    "dendrite_config.hcdf",
                                    &content,
                                    "application/xml",
                                );
                            }
                        }
                    });

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
                            params.camera_settings.target_focus = Vec3::ZERO;
                            params.camera_settings.target_distance = 0.6;
                            params.camera_settings.azimuth = 0.8;
                            params.camera_settings.elevation = 0.5;
                        }

                        ui.separator();

                        // Grid toggle
                        ui.checkbox(&mut params.world_settings.show_grid, "Show Grid");

                        // Axis toggle
                        ui.checkbox(&mut params.world_settings.show_axis, "Show World Axis");

                        ui.separator();

                        // Grid spacing control
                        ui.label("Grid Spacing:");
                        ui.add(
                            egui::DragValue::new(&mut params.world_settings.grid_spacing)
                                .speed(0.01)
                                .range(0.01..=1.0)
                                .suffix(" m")
                        );

                        // Grid line thickness control
                        ui.label("Line Thickness:");
                        ui.add(
                            egui::DragValue::new(&mut params.world_settings.grid_line_thickness)
                                .speed(0.0001)
                                .range(0.0001..=0.01)
                                .suffix(" m")
                        );

                        // Grid alpha control
                        ui.label("Grid Opacity:");
                        ui.add(
                            egui::Slider::new(&mut params.world_settings.grid_alpha, 0.0..=1.0)
                        );
                    });

                ui.separator();

                // Topology Graph button
                let graph_button = if is_mobile {
                    egui::Button::new(egui::RichText::new("View Topology Graph").size(14.0 * ui_scale))
                        .min_size(egui::vec2(0.0, 40.0))
                } else {
                    egui::Button::new("View Topology Graph")
                };
                if ui.add_sized([ui.available_width(), 0.0], graph_button).clicked() {
                    params.graph_vis.show = true;
                    // Build topology from current device registry
                    let nodes: Vec<TopologyNode> = params.registry.devices.iter().map(|d| {
                        TopologyNode {
                            id: d.id.clone(),
                            name: d.name.clone(),
                            board: d.board.clone(),
                            is_parent: false, // TODO: detect parent from HCDF
                            port: d.port,
                            children: Vec::new(),
                        }
                    }).collect();
                    params.graph_vis.topology = Some(TopologyData {
                        nodes,
                        root: None,
                    });
                    params.graph_vis.pan_offset = [0.0, 0.0];
                    params.graph_vis.zoom = 1.0;
                }
                }); // End ScrollArea
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
    if let Some(id) = params.selected.0.clone() {
        if let Some(device) = params.registry.devices.iter().find(|d| d.id == id) {
            if !is_mobile || params.ui_layout.show_right_panel {
                let right_panel_width = params.ui_layout.right_panel_width();
                let mut panel = egui::SidePanel::right("details_panel")
                    .default_width(right_panel_width)
                    .resizable(!is_mobile);
                // On mobile, constrain panel to exact width
                if is_mobile {
                    panel = panel.exact_width(right_panel_width);
                }
                panel.show(ctx, |ui| {
                        // On mobile, add close button
                        if is_mobile {
                            ui.horizontal(|ui| {
                                ui.heading(egui::RichText::new(&device.name).size(18.0 * ui_scale));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.button(egui::RichText::new("✕").size(18.0 * ui_scale)).clicked() {
                                        params.ui_layout.show_right_panel = false;
                                    }
                                });
                            });
                        } else {
                            ui.heading(&device.name);
                        }

                        ui.separator();

                        // On mobile, use tighter spacing for grids
                        let grid_spacing = if is_mobile { [4.0, 3.0] } else { [10.0, 4.0 * ui_scale] };

                        egui::ScrollArea::vertical().show(ui, |ui| {
                            // On mobile, show ID outside grid so it can wrap
                            if is_mobile {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label("ID:");
                                    ui.label(egui::RichText::new(&device.id).small());
                                });
                            }

                            egui::Grid::new("device_grid")
                                .num_columns(2)
                                .spacing(grid_spacing)
                                .show(ui, |ui| {
                                    // On desktop, show ID in grid row
                                    if !is_mobile {
                                        ui.label("ID:");
                                        ui.label(&device.id);
                                        ui.end_row();
                                    }

                                    ui.label("Status:");
                                    // Show "Unknown" when heartbeat checking is off (only for online devices)
                                    // Offline devices always show "Offline" - they were seen offline
                                    let status_str = match device.status {
                                        DeviceStatus::Offline => "Offline",
                                        DeviceStatus::Online => {
                                            if params.heartbeat_state.enabled {
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

                                    // Firmware status (when checking is enabled)
                                    if params.firmware_state.enabled {
                                        ui.label("Firmware Status:");
                                        match params.firmware_state.device_status.get(&id) {
                                            Some(FirmwareStatusData::UpToDate) => {
                                                ui.colored_label(egui::Color32::from_rgb(100, 200, 100), "Up to date");
                                            }
                                            Some(FirmwareStatusData::UpdateAvailable { latest_version, .. }) => {
                                                ui.colored_label(
                                                    egui::Color32::from_rgb(230, 200, 50),
                                                    format!("Update: {}", latest_version)
                                                );
                                            }
                                            Some(FirmwareStatusData::Unknown) | None => {
                                                if params.firmware_state.loading.contains(&id) {
                                                    ui.label("Checking...");
                                                } else {
                                                    ui.colored_label(egui::Color32::GRAY, "Unknown");
                                                }
                                            }
                                            Some(FirmwareStatusData::CheckDisabled) => {
                                                ui.colored_label(egui::Color32::GRAY, "Disabled");
                                            }
                                        }
                                        ui.end_row();

                                        // Show changelog if update available
                                        if let Some(FirmwareStatusData::UpdateAvailable { changelog: Some(log), .. }) = params.firmware_state.device_status.get(&id) {
                                            ui.label("Changes:");
                                            ui.label(log);
                                            ui.end_row();
                                        }
                                    }
                                });

                            // OTA Update section (outside the grid for better button layout)
                            // Check if there's an active OTA update for this device
                            if let Some(ota_update) = params.ota_state.device_updates.get(&id).cloned() {
                                ui.separator();
                                ui.label("Firmware Update:");

                                // Show progress bar for download/upload phases
                                if let Some(progress) = ota_update.progress_value() {
                                    ui.add(egui::ProgressBar::new(progress)
                                        .text(ota_update.progress_text())
                                        .animate(true));
                                } else {
                                    // Show spinner for other states
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label(ota_update.progress_text());
                                    });
                                }

                                // Show result coloring for terminal states
                                match &ota_update {
                                    OtaUpdateState::Complete => {
                                        ui.colored_label(egui::Color32::from_rgb(100, 200, 100), "Update complete!");
                                        // Clear button to dismiss
                                        if ui.small_button("Dismiss").clicked() {
                                            params.ota_state.device_updates.remove(&id);
                                        }
                                    }
                                    OtaUpdateState::Failed { error } => {
                                        ui.colored_label(egui::Color32::from_rgb(200, 100, 100), format!("Failed: {}", error));
                                        if ui.small_button("Dismiss").clicked() {
                                            params.ota_state.device_updates.remove(&id);
                                        }
                                    }
                                    OtaUpdateState::Cancelled => {
                                        ui.colored_label(egui::Color32::GRAY, "Cancelled");
                                        if ui.small_button("Dismiss").clicked() {
                                            params.ota_state.device_updates.remove(&id);
                                        }
                                    }
                                    _ => {
                                        // In-progress states - show cancel button
                                        let id_clone = id.clone();
                                        let base_url = params.daemon_config.http_url.clone();
                                        if ui.button("Cancel Update").clicked() {
                                            cancel_ota_update(&id_clone, &base_url);
                                        }
                                    }
                                }
                            } else if params.firmware_state.enabled {
                                // Show update button if firmware is outdated and no update in progress
                                if let Some(FirmwareStatusData::UpdateAvailable { .. }) = params.firmware_state.device_status.get(&id) {
                                    ui.separator();
                                    let id_clone = id.clone();
                                    let base_url = params.daemon_config.http_url.clone();
                                    let update_button = if is_mobile {
                                        egui::Button::new(
                                            egui::RichText::new("Update Firmware")
                                                .size(16.0 * ui_scale)
                                                .color(egui::Color32::from_rgb(100, 180, 255))
                                        ).min_size(egui::vec2(0.0, 40.0))
                                    } else {
                                        egui::Button::new(
                                            egui::RichText::new("Update Firmware")
                                                .color(egui::Color32::from_rgb(100, 180, 255))
                                        )
                                    };
                                    if ui.add(update_button).clicked() {
                                        start_ota_update(&id_clone, &base_url);
                                    }
                                }
                            }

                            // Always show local firmware upload button (for dev images)
                            ui.separator();
                            ui.label(egui::RichText::new("Development").size(12.0 * ui_scale).color(egui::Color32::GRAY));
                            {
                                let id_clone = id.clone();
                                let upload_button = if is_mobile {
                                    egui::Button::new(
                                        egui::RichText::new("Upload Local Firmware")
                                            .size(14.0 * ui_scale)
                                            .color(egui::Color32::from_rgb(200, 150, 50))
                                    ).min_size(egui::vec2(0.0, 36.0))
                                } else {
                                    egui::Button::new(
                                        egui::RichText::new("Upload Local Firmware")
                                            .color(egui::Color32::from_rgb(200, 150, 50))
                                    )
                                };
                                if ui.add(upload_button).clicked() {
                                    // Open file picker for firmware
                                    trigger_file_open(
                                        &params.pending_file_results,
                                        FilePickerContext::FirmwareUpload { device_id: id_clone },
                                        FileFilter::firmware(),
                                    );
                                }
                                ui.label(
                                    egui::RichText::new("Upload .bin/.hex from your computer")
                                        .size(10.0 * ui_scale)
                                        .color(egui::Color32::GRAY)
                                );
                            }

                            ui.separator();

                            // Continue with position editing (re-enter grid)
                            egui::Grid::new("device_grid_pos")
                                .num_columns(2)
                                .spacing(grid_spacing)
                                .show(ui, |ui| {
                                    // Editable Position (ENU)
                                    ui.label("Position (ENU):");
                                    ui.label("");
                                    ui.end_row();

                                    let current_pos = params.positions.positions.get(&id).cloned().unwrap_or(Vec3::ZERO);

                                    // Position labels - shorter on mobile
                                    let (x_label, y_label, z_label) = if is_mobile {
                                        ("X:", "Y:", "Z:")
                                    } else {
                                        ("  X (East):", "  Y (North):", "  Z (Up):")
                                    };

                                    // Editable X field
                                    ui.label(x_label);
                                    let mut x_val = current_pos.x;
                                    let x_response = ui.add(
                                        egui::DragValue::new(&mut x_val)
                                            .speed(0.01)
                                            .suffix(" m")
                                    );
                                    ui.end_row();

                                    // Editable Y field
                                    ui.label(y_label);
                                    let mut y_val = current_pos.y;
                                    let y_response = ui.add(
                                        egui::DragValue::new(&mut y_val)
                                            .speed(0.01)
                                            .suffix(" m")
                                    );
                                    ui.end_row();

                                    // Editable Z field
                                    ui.label(z_label);
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
                                        params.positions.positions.insert(id.clone(), new_pos);

                                        // Update the device's transform
                                        for (device, mut transform) in params.device_query.iter_mut() {
                                            if device.device_id == id {
                                                transform.translation = new_pos;
                                                break;
                                            }
                                        }

                                        // Sync position to backend (updates HCDF)
                                        let orient = params.orientations.orientations.get(&id).cloned().unwrap_or(Vec3::ZERO);
                                        update_device_position(
                                            &id,
                                            [new_pos.x, new_pos.y, new_pos.z],
                                            Some([orient.x, orient.y, orient.z]),
                                            &params.daemon_config.http_url,
                                        );
                                    }

                                    // Show rotation axis checkbox (unchecked by default)
                                    ui.label("Show Rotation Axis:");
                                    if ui.checkbox(&mut params.show_rotation_axis.0, "").changed() {
                                        // Value already updated by checkbox
                                    }
                                    ui.end_row();

                                    // Show orientation from 3D scene
                                    // Get stored Euler angles (these are display values, not used to compute rotation)
                                    let orient = params.orientations.orientations.get(&id).cloned().unwrap_or(Vec3::ZERO);

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
                                    if params.active_rotation_field.axis != new_axis {
                                        params.active_rotation_field.axis = new_axis;
                                    }

                                    // Apply Euler XYZ rotation
                                    if roll_response.changed() || pitch_response.changed() || yaw_response.changed() {
                                        let roll_rad = roll_deg.to_radians();
                                        let pitch_rad = pitch_deg.to_radians();
                                        let yaw_rad = yaw_deg.to_radians();

                                        // Store the Euler angles
                                        params.orientations.orientations.insert(
                                            id.clone(),
                                            Vec3::new(roll_rad, pitch_rad, yaw_rad)
                                        );

                                        // Update the device's rotation quaternion using XYZ Euler order
                                        for (device, mut transform) in params.device_query.iter_mut() {
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

                                        // Sync orientation to backend (updates HCDF)
                                        let pos = params.positions.positions.get(&id).cloned().unwrap_or(Vec3::ZERO);
                                        update_device_position(
                                            &id,
                                            [pos.x, pos.y, pos.z],
                                            Some([roll_rad, pitch_rad, yaw_rad]),
                                            &params.daemon_config.http_url,
                                        );
                                    }
                                });

                            ui.separator();

                            // Per-device frame visibility toggle (if device has frames or sensors)
                            // Sensor axis frames are also controlled by this toggle
                            let frame_count = device.frames.len();
                            let sensor_count = device.sensors.len();
                            if frame_count > 0 || sensor_count > 0 {
                                let mut show_frames = params.frame_visibility.show_frames_for(&id);
                                if ui.checkbox(&mut show_frames, "Show Reference Frames").changed() {
                                    params.frame_visibility.set_show_frames(&id, show_frames);
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
                                                        let mut frame_vis = params.frame_visibility.is_frame_visible(&id, &frame.name);
                                                        if ui.checkbox(&mut frame_vis, "").changed() {
                                                            params.frame_visibility.set_frame_visible(&id, &frame.name, frame_vis);
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
                                                    let is_hovered = params.frame_visibility.hovered_sensor_from_ui.as_ref() == Some(&sensor_key);

                                                    // Highlight color when hovered
                                                    let name_color = if is_hovered {
                                                        egui::Color32::WHITE
                                                    } else {
                                                        egui::Color32::LIGHT_BLUE
                                                    };

                                                    ui.horizontal(|ui| {
                                                        let mut axis_vis = params.frame_visibility.is_sensor_axis_visible(&id, &sensor.name);
                                                        if ui.checkbox(&mut axis_vis, "").changed() {
                                                            params.frame_visibility.set_sensor_axis_visible(&id, &sensor.name, axis_vis);
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
                                                            params.frame_visibility.hovered_sensor_from_ui = Some(sensor_key);
                                                            any_sensor_hovered_in_frames = true;
                                                        }
                                                    });
                                                }

                                                // Clear hover if no sensor in this device's frame section is hovered
                                                if !any_sensor_hovered_in_frames {
                                                    if let Some(ref hovered) = params.frame_visibility.hovered_sensor_from_ui.clone() {
                                                        if hovered.starts_with(&format!("{}:", id)) {
                                                            params.frame_visibility.hovered_sensor_from_ui = None;
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
                                    let mut show_sensors = params.frame_visibility.show_sensors_for(&id);
                                    if ui.checkbox(&mut show_sensors, "Show Sensors").changed() {
                                        params.frame_visibility.set_show_sensors(&id, show_sensors);
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
                                            let is_hovered = params.frame_visibility.hovered_sensor_from_ui.as_ref() == Some(&sensor_key);
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
                                                params.frame_visibility.hovered_sensor_from_ui = Some(sensor_key);
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
                                                        let mut show_fov = params.frame_visibility.is_sensor_fov_visible(&id, &sensor.name);
                                                        if ui.checkbox(&mut show_fov, "").changed() {
                                                            params.frame_visibility.set_sensor_fov_visible(&id, &sensor.name, show_fov);
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
                                                        let mut show_aligned = params.frame_visibility.is_sensor_axis_aligned(&id, &sensor.name);
                                                        if ui.checkbox(&mut show_aligned, "").changed() {
                                                            params.frame_visibility.set_sensor_axis_aligned(&id, &sensor.name, show_aligned);
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
                                            if let Some(ref hovered) = params.frame_visibility.hovered_sensor_from_ui.clone() {
                                                if hovered.starts_with(&format!("{}:", id)) {
                                                    params.frame_visibility.hovered_sensor_from_ui = None;
                                                }
                                            }
                                        }
                                    });
                                ui.separator();
                            }

                            // Per-device port visibility toggle (only if device has ports)
                            if !device.ports.is_empty() {
                                let mut show_ports = params.frame_visibility.show_ports_for(&id);
                                if ui.checkbox(&mut show_ports, "Show Ports").changed() {
                                    params.frame_visibility.set_show_ports(&id, show_ports);
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
                                            let is_hovered = params.frame_visibility.hovered_port.as_ref() == Some(&port_key);
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
                                                params.frame_visibility.hovered_port = Some(port_key);
                                                params.frame_visibility.hovered_port_from_ui = true;
                                                any_port_hovered = true;
                                            }
                                        }
                                    });

                                    // Clear hovered_port only if:
                                    // 1. No port in this UI list is hovered, AND
                                    // 2. The hover was set by UI (not 3D), AND
                                    // 3. The currently hovered port belongs to this device
                                    if !any_port_hovered && params.frame_visibility.hovered_port_from_ui {
                                        if let Some(ref hovered) = params.frame_visibility.hovered_port.clone() {
                                            if hovered.starts_with(&format!("{}:", id)) {
                                                params.frame_visibility.hovered_port = None;
                                                params.frame_visibility.hovered_port_from_ui = false;
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
                                    let mut is_hidden = params.frame_visibility.is_toggle_hidden(&id, toggle_group);
                                    if ui.checkbox(&mut is_hidden, &label).changed() {
                                        params.frame_visibility.set_toggle_hidden(&id, toggle_group, is_hidden);
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
                                    crate::network::remove_device(&device.id, &params.daemon_config.http_url);
                                    params.selected.0 = None;
                                    params.ui_layout.show_right_panel = false;
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
                                params.selected.0 = None;
                                params.ui_layout.show_right_panel = false;
                            }
                        });
                    });
            }
        }
    }

    // Connection dialog modal
    if params.connection_dialog.show {
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
                    egui::TextEdit::singleline(&mut params.connection_dialog.daemon_address)
                        .hint_text("e.g., 192.168.1.100:8080")
                        .desired_width(280.0)
                );

                // Show error if any
                if let Some(error) = &params.connection_dialog.error {
                    ui.colored_label(egui::Color32::RED, error);
                }

                ui.add_space(8.0);

                // Show current connection info
                ui.label(format!("Current: {}", params.daemon_config.http_url));

                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    if ui.button("Connect").clicked() || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                        let addr = params.connection_dialog.daemon_address.trim();
                        if !addr.is_empty() {
                            params.reconnect_events.write(ReconnectEvent {
                                daemon_address: addr.to_string(),
                            });
                            params.connection_dialog.show = false;
                            params.connection_dialog.error = None;
                        } else {
                            params.connection_dialog.error = Some("Please enter a daemon address".to_string());
                        }
                    }

                    if ui.button("Cancel").clicked() {
                        params.connection_dialog.show = false;
                        params.connection_dialog.error = None;
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

    // Graph visualization overlay
    if params.graph_vis.show {
        let screen_rect = ctx.screen_rect();
        let window_size = egui::vec2(
            (screen_rect.width() * 0.85).min(900.0),
            (screen_rect.height() * 0.85).min(700.0),
        );

        egui::Window::new("Device Topology Graph")
            .collapsible(false)
            .resizable(true)
            .default_size(window_size)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                // Header with close button and controls
                ui.horizontal(|ui| {
                    ui.heading("Network Topology");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            params.graph_vis.show = false;
                        }
                        ui.separator();
                        // Zoom controls
                        if ui.button("-").clicked() {
                            params.graph_vis.zoom = (params.graph_vis.zoom - 0.1).max(0.3);
                        }
                        ui.label(format!("{:.0}%", params.graph_vis.zoom * 100.0));
                        if ui.button("+").clicked() {
                            params.graph_vis.zoom = (params.graph_vis.zoom + 0.1).min(3.0);
                        }
                        ui.separator();
                        if ui.button("Reset View").clicked() {
                            params.graph_vis.pan_offset = [0.0, 0.0];
                            params.graph_vis.zoom = 1.0;
                        }
                    });
                });

                ui.separator();

                // Graph canvas area
                let available = ui.available_size();
                let (response, painter) = ui.allocate_painter(available, egui::Sense::click_and_drag());

                // Handle panning
                if response.dragged() {
                    let delta = response.drag_delta();
                    params.graph_vis.pan_offset[0] += delta.x;
                    params.graph_vis.pan_offset[1] += delta.y;
                }

                // Handle scrolling for zoom
                let scroll_delta = ui.input(|i| i.raw_scroll_delta.y);
                if scroll_delta != 0.0 {
                    let zoom_factor = if scroll_delta > 0.0 { 1.1 } else { 0.9 };
                    params.graph_vis.zoom = (params.graph_vis.zoom * zoom_factor).clamp(0.3, 3.0);
                }

                // Background
                let rect = response.rect;
                painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(20, 25, 35));

                // Draw the topology graph
                // Clone topology to avoid borrow conflicts when updating hover/selection state
                let topology_clone = params.graph_vis.topology.clone();
                if let Some(topology) = topology_clone {
                    let zoom = params.graph_vis.zoom;
                    let pan = params.graph_vis.pan_offset;
                    let center = rect.center();

                    // Calculate node positions in a radial layout
                    let node_count = topology.nodes.len();
                    let radius = 150.0 * zoom;

                    // Track hover/click state changes to apply after rendering
                    let mut new_hovered: Option<String> = None;
                    let mut clicked_node: Option<String> = None;

                    // Draw connections and nodes
                    for (i, node) in topology.nodes.iter().enumerate() {
                        let angle = (i as f32 / node_count.max(1) as f32) * std::f32::consts::TAU;
                        let node_x = center.x + pan[0] + radius * angle.cos();
                        let node_y = center.y + pan[1] + radius * angle.sin();
                        let node_pos = egui::pos2(node_x, node_y);

                        // Draw connections to children
                        for child_id in &node.children {
                            if let Some((j, _)) = topology.nodes.iter().enumerate().find(|(_, n)| &n.id == child_id) {
                                let child_angle = (j as f32 / node_count.max(1) as f32) * std::f32::consts::TAU;
                                let child_x = center.x + pan[0] + radius * child_angle.cos();
                                let child_y = center.y + pan[1] + radius * child_angle.sin();
                                let child_pos = egui::pos2(child_x, child_y);

                                painter.line_segment(
                                    [node_pos, child_pos],
                                    egui::Stroke::new(2.0 * zoom, egui::Color32::from_rgb(100, 150, 200)),
                                );
                            }
                        }

                        // Draw node circle
                        let node_radius = 30.0 * zoom;
                        let is_hovered = params.graph_vis.hovered_node.as_ref() == Some(&node.id);
                        let is_selected = params.selected.0.as_ref() == Some(&node.id);

                        let fill_color = if is_selected {
                            egui::Color32::from_rgb(80, 180, 255)
                        } else if node.is_parent {
                            egui::Color32::from_rgb(255, 180, 80)
                        } else {
                            egui::Color32::from_rgb(60, 140, 200)
                        };

                        let stroke_color = if is_hovered {
                            egui::Color32::WHITE
                        } else {
                            egui::Color32::from_rgb(150, 180, 210)
                        };

                        painter.circle(
                            node_pos,
                            node_radius,
                            fill_color,
                            egui::Stroke::new(if is_hovered { 3.0 } else { 1.5 }, stroke_color),
                        );

                        // Draw node label
                        let font_size = 12.0 * zoom;
                        let text_color = egui::Color32::WHITE;

                        // Node name
                        painter.text(
                            node_pos,
                            egui::Align2::CENTER_CENTER,
                            &node.name,
                            egui::FontId::proportional(font_size),
                            text_color,
                        );

                        // Board type below
                        if let Some(ref board) = node.board {
                            painter.text(
                                egui::pos2(node_pos.x, node_pos.y + node_radius + 8.0 * zoom),
                                egui::Align2::CENTER_TOP,
                                board,
                                egui::FontId::proportional(font_size * 0.8),
                                egui::Color32::from_rgb(150, 160, 180),
                            );
                        }

                        // Port number if available
                        if let Some(port) = node.port {
                            painter.text(
                                egui::pos2(node_pos.x, node_pos.y - node_radius - 5.0 * zoom),
                                egui::Align2::CENTER_BOTTOM,
                                format!("Port {}", port),
                                egui::FontId::proportional(font_size * 0.7),
                                egui::Color32::from_rgb(180, 180, 100),
                            );
                        }

                        // Check for hover/click
                        let node_rect = egui::Rect::from_center_size(node_pos, egui::vec2(node_radius * 2.0, node_radius * 2.0));
                        if let Some(pointer_pos) = response.hover_pos() {
                            if node_rect.contains(pointer_pos) {
                                new_hovered = Some(node.id.clone());

                                // Click to select
                                if response.clicked() {
                                    clicked_node = Some(node.id.clone());
                                }
                            }
                        }
                    }

                    // Apply state changes after iteration
                    params.graph_vis.hovered_node = new_hovered;
                    if let Some(node_id) = clicked_node {
                        params.selected.0 = Some(node_id);
                        params.graph_vis.show = false; // Close graph and show device details
                    }
                } else {
                    // No topology data
                    painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "No devices discovered",
                        egui::FontId::proportional(16.0),
                        egui::Color32::GRAY,
                    );
                }

                // Instructions at bottom
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Drag to pan | Scroll to zoom | Click node to select").small().color(egui::Color32::GRAY));
                });
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
