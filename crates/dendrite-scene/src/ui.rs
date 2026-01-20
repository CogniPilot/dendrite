//! Shared UI components for device display

use bevy::prelude::*;
use bevy_egui::egui;

use crate::types::*;

/// Render device details panel (shared between apps)
pub fn render_device_details(
    ui: &mut egui::Ui,
    device: &DeviceData,
    ui_layout: &UiLayout,
) {
    let ui_scale = ui_layout.ui_scale();
    let _is_mobile = ui_layout.is_mobile;

    // Device name and board
    ui.heading(egui::RichText::new(&device.name).size(18.0 * ui_scale));
    if let Some(board) = &device.board {
        ui.label(format!("Board: {}", board));
    }

    ui.separator();

    // Position
    if let Some(pos) = &device.position {
        ui.label(format!("Position: [{:.3}, {:.3}, {:.3}]", pos[0], pos[1], pos[2]));
    }

    // Orientation
    if let Some(ori) = &device.orientation {
        ui.label(format!(
            "Orientation: R:{:.1}° P:{:.1}° Y:{:.1}°",
            ori[0].to_degrees(),
            ori[1].to_degrees(),
            ori[2].to_degrees()
        ));
    }

    // Version
    if let Some(version) = &device.version {
        ui.label(format!("Version: {}", version));
    }
}

/// Render sensor list for a device
pub fn render_sensor_list(
    ui: &mut egui::Ui,
    device: &DeviceData,
    _ui_layout: &UiLayout,
) {
    if device.sensors.is_empty() {
        return;
    }

    ui.collapsing("Sensors", |ui| {
        for sensor in &device.sensors {
            ui.horizontal(|ui| {
                ui.label(&sensor.name);
                ui.label(
                    egui::RichText::new(&sensor.sensor_type)
                        .small()
                        .color(egui::Color32::GRAY),
                );
            });
        }
    });
}

/// Render port list for a device
pub fn render_port_list(
    ui: &mut egui::Ui,
    device: &DeviceData,
    _ui_layout: &UiLayout,
) {
    if device.ports.is_empty() {
        return;
    }

    ui.collapsing("Ports", |ui| {
        for port in &device.ports {
            ui.horizontal(|ui| {
                ui.label(&port.name);
                let color = match port.port_type.to_lowercase().as_str() {
                    "ethernet" => egui::Color32::from_rgb(50, 200, 50),
                    "can" => egui::Color32::from_rgb(255, 200, 50),
                    "spi" => egui::Color32::from_rgb(200, 50, 200),
                    "i2c" => egui::Color32::from_rgb(50, 200, 200),
                    "uart" => egui::Color32::from_rgb(200, 100, 50),
                    "usb" => egui::Color32::from_rgb(50, 100, 200),
                    _ => egui::Color32::GRAY,
                };
                ui.label(egui::RichText::new(&port.port_type).color(color));
            });
        }
    });
}

/// Render frame list for a device
pub fn render_frame_list(
    ui: &mut egui::Ui,
    device: &DeviceData,
    _ui_layout: &UiLayout,
) {
    if device.frames.is_empty() {
        return;
    }

    ui.collapsing("Frames", |ui| {
        for frame in &device.frames {
            ui.horizontal(|ui| {
                ui.label(&frame.name);
                if let Some(desc) = &frame.description {
                    ui.label(
                        egui::RichText::new(desc)
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                }
            });
        }
    });
}
