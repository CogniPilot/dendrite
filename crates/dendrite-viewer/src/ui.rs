//! UI overlays using bevy_egui

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};

use crate::app::{ActiveRotationAxis, ActiveRotationField, AntennaCapabilitiesData, AntennaData, AxisAlignData, CameraSettings, DeviceData, DeviceOrientations, DevicePositions, DeviceRegistry, DeviceStatus, FovData, FrameData, FrameVisibility, GeometryData, GraphVisualization, PortCapabilitiesData, PortData, SelectedDevice, SensorData, ShowRotationAxis, TopologyData, TopologyNode, UiLayout, VisualData, WorldSettings};
use crate::file_picker::{FileFilter, FilePickerContext, FilePickerState, PendingFileResults, trigger_file_open};
use dendrite_core::hcdf::Hcdf;

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
    pub ui_layout: ResMut<'w, UiLayout>,
    pub pending_file_results: Res<'w, PendingFileResults>,
    pub graph_vis: ResMut<'w, GraphVisualization>,
    pub pending_removals: ResMut<'w, PendingDeviceRemovals>,
    pub url_input: ResMut<'w, HcdfUrlInput>,
    pub hosted_mode: Res<'w, HostedMode>,
}

pub struct UiPlugin;

/// Tracks if running in hosted mode (e.g., dendrite.cognipilot.org)
/// In hosted mode, local file import is disabled - all data comes via URL
#[derive(Resource, Default)]
pub struct HostedMode(pub bool);

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        // Initialize resources
        app.init_resource::<PendingHcdfContent>()
            .init_resource::<PendingDeviceRemovals>()
            .init_resource::<HcdfUrlInput>()
            .init_resource::<HcdfBaseUrl>()
            .init_resource::<HostedMode>()
            // Check URL parameters on startup
            .add_systems(Startup, (check_url_parameters, detect_hosted_mode))
            // UI layout updates run in Update
            .add_systems(Update, (update_ui_layout, process_file_picker_results, process_device_removals, process_pending_hcdf, process_url_fetch_results))
            // Main UI system runs in EguiPrimaryContextPass for proper input handling (bevy_egui 0.38+)
            .add_systems(EguiPrimaryContextPass, ui_system);
    }
}

/// Process pending device removals
fn process_device_removals(
    mut registry: ResMut<DeviceRegistry>,
    mut pending_removals: ResMut<PendingDeviceRemovals>,
    mut positions: ResMut<DevicePositions>,
    mut orientations: ResMut<DeviceOrientations>,
    mut frame_visibility: ResMut<FrameVisibility>,
) {
    if pending_removals.0.is_empty() {
        return;
    }

    for device_id in pending_removals.0.drain(..) {
        // Remove from registry
        registry.devices.retain(|d| d.id != device_id);

        // Clean up associated state
        positions.positions.remove(&device_id);
        orientations.orientations.remove(&device_id);
        frame_visibility.device_frames.remove(&device_id);
        frame_visibility.device_sensors.remove(&device_id);
        frame_visibility.device_ports.remove(&device_id);

        tracing::info!("Removed device: {}", device_id);
    }
}

/// Process pending HCDF content and populate the device registry
fn process_pending_hcdf(
    mut pending_hcdf: ResMut<PendingHcdfContent>,
    mut registry: ResMut<DeviceRegistry>,
    mut positions: ResMut<DevicePositions>,
    mut orientations: ResMut<DeviceOrientations>,
    mut frame_visibility: ResMut<FrameVisibility>,
) {
    // Take pending content if available
    let Some(xml_content) = pending_hcdf.0.take() else {
        return;
    };

    tracing::info!("Processing HCDF content ({} bytes)", xml_content.len());

    // Parse HCDF XML
    let hcdf = match Hcdf::from_xml(&xml_content) {
        Ok(hcdf) => hcdf,
        Err(e) => {
            tracing::error!("Failed to parse HCDF: {:?}", e);
            return;
        }
    };

    // Clear existing devices and state
    registry.devices.clear();
    positions.positions.clear();
    orientations.orientations.clear();
    frame_visibility.device_frames.clear();
    frame_visibility.device_sensors.clear();
    frame_visibility.device_ports.clear();

    // Process MCUs
    for mcu in &hcdf.mcu {
        let device = convert_mcu_to_device(mcu);
        tracing::info!("Added MCU device: {} ({})", device.name, device.id);
        registry.devices.push(device);
    }

    // Process Comps
    for comp in &hcdf.comp {
        let device = convert_comp_to_device(comp);
        tracing::info!("Added Comp device: {} ({})", device.name, device.id);
        registry.devices.push(device);
    }

    // Mark registry as connected (we have data)
    registry.connected = true;

    tracing::info!("HCDF processing complete: {} devices loaded", registry.devices.len());
}

/// Check URL parameters on startup for ?hcdf=URL
#[allow(unused_variables, unused_mut)]
fn check_url_parameters(mut url_input: ResMut<HcdfUrlInput>) {
    #[cfg(target_arch = "wasm32")]
    {
        let window = match web_sys::window() {
            Some(w) => w,
            None => return,
        };

        let location = match window.location().href() {
            Ok(href) => href,
            Err(_) => return,
        };

        // Parse URL for ?hcdf= parameter
        if let Ok(url) = web_sys::Url::new(&location) {
            let params = url.search_params();
            if let Some(hcdf_url) = params.get("hcdf") {
                tracing::info!("Loading HCDF from URL parameter: {}", hcdf_url);
                url_input.url = hcdf_url.clone();
                // Trigger fetch
                fetch_hcdf_from_url(&hcdf_url, url_input.pending_result.clone());
                url_input.loading = true;
            }
        }
    }
}

/// Detect if running in hosted mode (dendrite.cognipilot.org)
/// In hosted mode, local file import is disabled
#[allow(unused_variables, unused_mut)]
fn detect_hosted_mode(mut hosted_mode: ResMut<HostedMode>) {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(window) = web_sys::window() {
            if let Ok(hostname) = window.location().hostname() {
                // Check if we're on the production host
                if hostname == "dendrite.cognipilot.org" {
                    tracing::info!("Running in hosted mode on {}", hostname);
                    hosted_mode.0 = true;
                }
            }
        }
    }
}

/// Process pending URL fetch results
fn process_url_fetch_results(
    mut url_input: ResMut<HcdfUrlInput>,
    mut pending_hcdf: ResMut<PendingHcdfContent>,
    mut base_url: ResMut<HcdfBaseUrl>,
) {
    // Take the result from the mutex (if any) - this drops the lock immediately
    let fetch_result = {
        if let Ok(mut result) = url_input.pending_result.try_lock() {
            result.take()
        } else {
            None
        }
    };

    // Process the result after the lock is released
    if let Some(result) = fetch_result {
        url_input.loading = false;
        match result {
            Ok(content) => {
                tracing::info!("HCDF fetched from URL ({} bytes)", content.len());
                url_input.error = None;
                pending_hcdf.0 = Some(content);

                // Extract base URL for resolving relative model paths
                // e.g., "https://hcdf.cognipilot.org/mr_mcxn_t1/optical-flow/file.hcdf"
                //    -> "https://hcdf.cognipilot.org/"
                if let Some(pos) = url_input.url.rfind('/') {
                    // Get the domain/host part for model resolution
                    if let Some(scheme_end) = url_input.url.find("://") {
                        let after_scheme = &url_input.url[scheme_end + 3..];
                        if let Some(first_slash) = after_scheme.find('/') {
                            let domain_end = scheme_end + 3 + first_slash;
                            base_url.0 = Some(url_input.url[..domain_end + 1].to_string());
                            tracing::info!("Set HCDF base URL: {:?}", base_url.0);
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to fetch HCDF: {}", e);
                url_input.error = Some(e);
            }
        }
    }
}

/// Fetch HCDF content from a URL (async via wasm_bindgen_futures)
#[cfg(target_arch = "wasm32")]
pub fn fetch_hcdf_from_url(
    url: &str,
    pending_result: std::sync::Arc<std::sync::Mutex<Option<Result<String, String>>>>,
) {
    use wasm_bindgen::JsCast;

    let url = url.to_string();
    wasm_bindgen_futures::spawn_local(async move {
        let result = async {
            let window = web_sys::window().ok_or("No window")?;

            let resp = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(&url))
                .await
                .map_err(|e| format!("Fetch failed: {:?}", e))?;

            let resp: web_sys::Response = resp.dyn_into().map_err(|_| "Response cast failed")?;

            if !resp.ok() {
                return Err(format!("HTTP {}: {}", resp.status(), resp.status_text()));
            }

            let text = wasm_bindgen_futures::JsFuture::from(
                resp.text().map_err(|_| "Failed to get text")?
            )
                .await
                .map_err(|e| format!("Text extraction failed: {:?}", e))?;

            text.as_string().ok_or_else(|| "Not a string".to_string())
        }.await;

        if let Ok(mut pending) = pending_result.lock() {
            *pending = Some(result);
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub fn fetch_hcdf_from_url(
    _url: &str,
    pending_result: std::sync::Arc<std::sync::Mutex<Option<Result<String, String>>>>,
) {
    // Native: not supported yet
    if let Ok(mut pending) = pending_result.lock() {
        *pending = Some(Err("URL fetch not supported on native".to_string()));
    }
}

/// Convert an HCDF MCU to DeviceData
fn convert_mcu_to_device(mcu: &dendrite_core::hcdf::Mcu) -> DeviceData {
    let pose = mcu.pose_cg.as_ref().and_then(|s| dendrite_core::hcdf::parse_pose_string(s));
    let position = pose.as_ref().map(|p| [p.x, p.y, p.z]);
    let orientation = pose.as_ref().map(|p| [p.roll, p.pitch, p.yaw]);

    // Convert visuals
    let visuals: Vec<VisualData> = mcu.visual.iter().map(|v| {
        let pose = v.parse_pose();
        VisualData {
            name: v.name.clone(),
            toggle: v.toggle.clone(),
            pose: pose.map(|p| p.to_array()),
            model_path: v.model.as_ref().map(|m| m.href.clone()),
            model_sha: v.model.as_ref().and_then(|m| m.sha.clone()),
        }
    }).collect();

    // Convert frames
    let frames: Vec<FrameData> = mcu.frame.iter().map(|f| {
        let pose = f.parse_pose();
        FrameData {
            name: f.name.clone(),
            description: f.description.clone(),
            pose: pose.map(|p| p.to_array()),
        }
    }).collect();

    // Legacy model path
    let model_path = mcu.model.as_ref().map(|m| m.href.clone());

    DeviceData {
        id: mcu.hwid.clone().unwrap_or_else(|| mcu.name.clone()),
        name: mcu.name.clone(),
        board: mcu.board.clone(),
        ip: mcu.discovered.as_ref().map(|d| d.ip.clone()).unwrap_or_else(|| "0.0.0.0".to_string()),
        port: mcu.discovered.as_ref().and_then(|d| d.port),
        status: DeviceStatus::Online,
        version: mcu.software.as_ref().and_then(|s| s.version.clone()),
        position,
        orientation,
        model_path,
        visuals,
        frames,
        ports: Vec::new(), // MCUs don't have ports in current HCDF schema
        antennas: Vec::new(), // MCUs don't have antennas in current HCDF schema
        sensors: Vec::new(), // MCUs don't have sensors directly
        last_seen: mcu.discovered.as_ref().and_then(|d| d.last_seen.clone()),
    }
}

/// Convert an HCDF Comp to DeviceData
fn convert_comp_to_device(comp: &dendrite_core::hcdf::Comp) -> DeviceData {
    let pose = comp.pose_cg.as_ref().and_then(|s| dendrite_core::hcdf::parse_pose_string(s));
    let position = pose.as_ref().map(|p| [p.x, p.y, p.z]);
    let orientation = pose.as_ref().map(|p| [p.roll, p.pitch, p.yaw]);

    // Convert visuals
    let visuals: Vec<VisualData> = comp.visual.iter().map(|v| {
        let pose = v.parse_pose();
        VisualData {
            name: v.name.clone(),
            toggle: v.toggle.clone(),
            pose: pose.map(|p| p.to_array()),
            model_path: v.model.as_ref().map(|m| m.href.clone()),
            model_sha: v.model.as_ref().and_then(|m| m.sha.clone()),
        }
    }).collect();

    // Convert frames
    let frames: Vec<FrameData> = comp.frame.iter().map(|f| {
        let pose = f.parse_pose();
        FrameData {
            name: f.name.clone(),
            description: f.description.clone(),
            pose: pose.map(|p| p.to_array()),
        }
    }).collect();

    // Convert ports
    let ports: Vec<PortData> = comp.port.iter().map(|p| {
        // parse_pose handles both fallback_visual.pose and legacy pose field
        let pose = p.parse_pose();

        // Get geometry from fallback_visual or legacy geometry field
        let geometry: Vec<GeometryData> = if let Some(ref fv) = p.fallback_visual {
            // New schema: use fallback_visual geometry
            fv.geometry.as_ref()
                .and_then(|g| convert_geometry(g))
                .map(|g| vec![g])
                .unwrap_or_default()
        } else {
            // Legacy schema: use geometry vector
            p.geometry.iter().filter_map(|g| convert_geometry(g)).collect()
        };

        // Extract capabilities
        let capabilities = p.capabilities.as_ref().map(|caps| {
            PortCapabilitiesData {
                // Data capabilities
                speed: caps.speed.as_ref().map(|v| {
                    format!("{}{}", v.value, v.unit.as_ref().map(|u| format!(" {}", u)).unwrap_or_default())
                }),
                bitrate: caps.bitrate.as_ref().map(|v| {
                    format!("{}{}", v.value, v.unit.as_ref().map(|u| format!(" {}", u)).unwrap_or_default())
                }),
                baud: caps.baud.as_ref().map(|v| {
                    format!("{}{}", v.value, v.unit.as_ref().map(|u| format!(" {}", u)).unwrap_or_default())
                }),
                standard: caps.standard.clone(),
                protocols: caps.protocol.clone(),
                // Power capabilities
                voltage: caps.voltage.as_ref().map(|v| v.to_display_string()).filter(|s| !s.is_empty()),
                current: caps.current.as_ref().map(|v| v.to_display_string()).filter(|s| !s.is_empty()),
                power_watts: caps.power.as_ref().map(|v| v.to_display_string()).filter(|s| !s.is_empty()),
                capacity: caps.capacity.as_ref().map(|v| {
                    format!("{}{}", v.value, v.unit.as_ref().map(|u| format!(" {}", u)).unwrap_or_default())
                }),
                connector: caps.connector.clone(),
            }
        });

        PortData {
            name: p.name.clone(),
            port_type: p.port_type.clone(),
            pose: pose.map(|p| p.to_array()),
            geometry,
            visual_name: p.visual.clone(),
            mesh_name: p.mesh.clone(),
            capabilities,
        }
    }).collect();

    // Convert antennas
    let antennas: Vec<AntennaData> = comp.antenna.iter().map(|a| {
        // parse_pose handles both fallback_visual.pose and legacy pose field
        let pose = a.parse_pose();

        // Get geometry from fallback_visual or legacy geometry field
        let geometry: Option<GeometryData> = if let Some(ref fv) = a.fallback_visual {
            // New schema: use fallback_visual geometry
            fv.geometry.as_ref().and_then(|g| convert_geometry(g))
        } else {
            // Legacy schema: use geometry field
            a.geometry.as_ref().and_then(|g| convert_geometry(g))
        };

        // Extract capabilities
        let capabilities = a.capabilities.as_ref().map(|caps| {
            AntennaCapabilitiesData {
                bands: caps.get_bands(),
                gain: caps.gain.as_ref().map(|v| {
                    format!("{}{}", v.value, v.unit.as_ref().map(|u| format!(" {}", u)).unwrap_or_default())
                }),
                standards: caps.standard.clone(),
                protocols: caps.protocol.clone(),
                polarization: caps.polarization.clone(),
            }
        });

        AntennaData {
            name: a.name.clone(),
            antenna_type: a.antenna_type.clone(),
            pose: pose.map(|p| p.to_array()),
            geometry,
            visual_name: a.visual.clone(),
            mesh_name: a.mesh.clone(),
            capabilities,
        }
    }).collect();

    // Convert sensors from the Sensor container
    let mut sensors: Vec<SensorData> = Vec::new();
    for sensor_container in &comp.sensor {
        // Inertial sensors
        for inertial in &sensor_container.inertial {
            let pose = inertial.parse_pose();
            let axis_align = inertial.driver.as_ref().and_then(|d| {
                d.axis_align.as_ref().and_then(|a| a.parse_axes()).map(|(x, y, z)| {
                    AxisAlignData {
                        x: axis_map_to_string(&x),
                        y: axis_map_to_string(&y),
                        z: axis_map_to_string(&z),
                    }
                })
            });
            sensors.push(SensorData {
                name: format!("{}_{}", sensor_container.name, inertial.sensor_type),
                category: "inertial".to_string(),
                sensor_type: inertial.sensor_type.clone(),
                driver: inertial.driver.as_ref().map(|d| d.name.clone()),
                pose: pose.map(|p| p.to_array()),
                axis_align,
                geometry: inertial.geometry.as_ref().and_then(|g| convert_geometry(g)),
                fovs: Vec::new(),
            });
        }

        // EM sensors
        for em in &sensor_container.em {
            let pose = em.parse_pose();
            let axis_align = em.driver.as_ref().and_then(|d| {
                d.axis_align.as_ref().and_then(|a| a.parse_axes()).map(|(x, y, z)| {
                    AxisAlignData {
                        x: axis_map_to_string(&x),
                        y: axis_map_to_string(&y),
                        z: axis_map_to_string(&z),
                    }
                })
            });
            sensors.push(SensorData {
                name: format!("{}_{}", sensor_container.name, em.sensor_type),
                category: "em".to_string(),
                sensor_type: em.sensor_type.clone(),
                driver: em.driver.as_ref().map(|d| d.name.clone()),
                pose: pose.map(|p| p.to_array()),
                axis_align,
                geometry: em.geometry.as_ref().and_then(|g| convert_geometry(g)),
                fovs: Vec::new(),
            });
        }

        // Optical sensors
        for optical in &sensor_container.optical {
            let pose = optical.parse_pose();
            let axis_align = optical.driver.as_ref().and_then(|d| {
                d.axis_align.as_ref().and_then(|a| a.parse_axes()).map(|(x, y, z)| {
                    AxisAlignData {
                        x: axis_map_to_string(&x),
                        y: axis_map_to_string(&y),
                        z: axis_map_to_string(&z),
                    }
                })
            });
            // Convert FOVs
            let fovs: Vec<FovData> = optical.fov.iter().map(|f| {
                let fov_pose = f.parse_pose();
                let color = f.parse_color();
                FovData {
                    name: f.name.clone(),
                    color: color.map(|(r, g, b)| [r, g, b]),
                    pose: fov_pose.map(|p| p.to_array()),
                    geometry: f.geometry.as_ref().and_then(|g| convert_geometry(g)),
                }
            }).collect();

            sensors.push(SensorData {
                name: format!("{}_{}", sensor_container.name, optical.sensor_type),
                category: "optical".to_string(),
                sensor_type: optical.sensor_type.clone(),
                driver: optical.driver.as_ref().map(|d| d.name.clone()),
                pose: pose.map(|p| p.to_array()),
                axis_align,
                geometry: optical.geometry.as_ref().and_then(|g| convert_geometry(g)),
                fovs,
            });
        }

        // RF sensors
        for rf in &sensor_container.rf {
            let pose = rf.parse_pose();
            sensors.push(SensorData {
                name: format!("{}_{}", sensor_container.name, rf.sensor_type),
                category: "rf".to_string(),
                sensor_type: rf.sensor_type.clone(),
                driver: rf.driver.as_ref().map(|d| d.name.clone()),
                pose: pose.map(|p| p.to_array()),
                axis_align: None,
                geometry: rf.geometry.as_ref().and_then(|g| convert_geometry(g)),
                fovs: Vec::new(),
            });
        }

        // Force sensors
        for force in &sensor_container.force {
            let pose = force.parse_pose();
            sensors.push(SensorData {
                name: format!("{}_{}", sensor_container.name, force.sensor_type),
                category: "force".to_string(),
                sensor_type: force.sensor_type.clone(),
                driver: force.driver.as_ref().map(|d| d.name.clone()),
                pose: pose.map(|p| p.to_array()),
                axis_align: None,
                geometry: force.geometry.as_ref().and_then(|g| convert_geometry(g)),
                fovs: Vec::new(),
            });
        }

        // Chemical sensors
        for chemical in &sensor_container.chemical {
            let pose = chemical.parse_pose();
            sensors.push(SensorData {
                name: format!("{}_{}", sensor_container.name, chemical.sensor_type),
                category: "chemical".to_string(),
                sensor_type: chemical.sensor_type.clone(),
                driver: chemical.driver.as_ref().map(|d| d.name.clone()),
                pose: pose.map(|p| p.to_array()),
                axis_align: None,
                geometry: chemical.geometry.as_ref().and_then(|g| convert_geometry(g)),
                fovs: Vec::new(),
            });
        }
    }

    // Legacy model path
    let model_path = comp.model.as_ref().map(|m| m.href.clone());

    DeviceData {
        id: comp.hwid.clone().unwrap_or_else(|| comp.name.clone()),
        name: comp.name.clone(),
        board: comp.board.clone(),
        ip: comp.discovered.as_ref().map(|d| d.ip.clone()).unwrap_or_else(|| "0.0.0.0".to_string()),
        port: comp.discovered.as_ref().and_then(|d| d.port),
        status: DeviceStatus::Online,
        version: comp.software.as_ref().and_then(|s| s.version.clone()),
        position,
        orientation,
        model_path,
        visuals,
        frames,
        ports,
        antennas,
        sensors,
        last_seen: comp.discovered.as_ref().and_then(|d| d.last_seen.clone()),
    }
}

/// Convert AxisMap enum to string representation
fn axis_map_to_string(axis: &dendrite_core::hcdf::AxisMap) -> String {
    use dendrite_core::hcdf::AxisMap;
    match axis {
        AxisMap::X => "X".to_string(),
        AxisMap::NegX => "-X".to_string(),
        AxisMap::Y => "Y".to_string(),
        AxisMap::NegY => "-Y".to_string(),
        AxisMap::Z => "Z".to_string(),
        AxisMap::NegZ => "-Z".to_string(),
    }
}

/// Convert HCDF Geometry to GeometryData
fn convert_geometry(g: &dendrite_core::hcdf::Geometry) -> Option<GeometryData> {
    if let Some(b) = g.get_box() {
        let size = b.parse_size()?;
        return Some(GeometryData::Box { size });
    }
    if let Some(ref c) = g.cylinder {
        return Some(GeometryData::Cylinder {
            radius: c.radius,
            length: c.length,
        });
    }
    if let Some(ref s) = g.sphere {
        return Some(GeometryData::Sphere { radius: s.radius });
    }
    if let Some(ref c) = g.cone {
        return Some(GeometryData::Cone {
            radius: c.radius,
            length: c.length,
        });
    }
    if let Some(ref f) = g.frustum {
        return Some(GeometryData::PyramidalFrustum {
            near: f.near,
            far: f.far,
            hfov: f.hfov,
            vfov: f.vfov,
        });
    }
    if let Some(ref f) = g.conical_frustum {
        return Some(GeometryData::ConicalFrustum {
            near: f.near,
            far: f.far,
            fov: f.fov,
        });
    }
    if let Some(ref f) = g.pyramidal_frustum {
        return Some(GeometryData::PyramidalFrustum {
            near: f.near,
            far: f.far,
            hfov: f.hfov,
            vfov: f.vfov,
        });
    }
    None
}

/// Pending HCDF content to be loaded
#[derive(Resource, Default)]
pub struct PendingHcdfContent(pub Option<String>);

/// URL input state for loading HCDF from web
#[derive(Resource)]
pub struct HcdfUrlInput {
    /// Current URL text in the input field
    pub url: String,
    /// Whether a fetch is in progress
    pub loading: bool,
    /// Error message from last fetch attempt
    pub error: Option<String>,
    /// Pending fetch result (set by async callback)
    pub pending_result: std::sync::Arc<std::sync::Mutex<Option<Result<String, String>>>>,
}

impl Default for HcdfUrlInput {
    fn default() -> Self {
        Self {
            url: String::new(),
            loading: false,
            error: None,
            pending_result: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }
}

/// Base URL for resolving relative model paths (set when loading HCDF from URL)
#[derive(Resource, Default)]
pub struct HcdfBaseUrl(pub Option<String>);

/// Pending device removals (device IDs to remove from registry)
#[derive(Resource, Default)]
pub struct PendingDeviceRemovals(pub Vec<String>);

/// Process completed file picker results and dispatch to appropriate handlers
fn process_file_picker_results(
    mut file_picker_state: ResMut<FilePickerState>,
    mut pending_hcdf: ResMut<PendingHcdfContent>,
    mut base_url: ResMut<HcdfBaseUrl>,
) {
    // Process completed file picker results
    while let Some(result) = file_picker_state.take_result() {
        if !result.success {
            tracing::error!("File picker operation failed: {:?}", result.error);
            continue;
        }

        match result.context {
            FilePickerContext::FirmwareUpload { .. } => {
                // Firmware upload not supported in viewer mode
                tracing::warn!("Firmware upload not available in viewer mode");
            }
            FilePickerContext::HcdfImport => {
                if let Some(content) = result.content {
                    // Convert bytes to string and store for processing
                    if let Ok(xml) = String::from_utf8(content) {
                        tracing::warn!("HCDF file loaded: {} ({} bytes)", result.filename, xml.len());
                        pending_hcdf.0 = Some(xml);
                        // Clear base_url for local files - models will use default CDN
                        base_url.0 = None;
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

                // File loading UI - only show when NOT in hosted mode
                // In hosted mode, HCDF is loaded via URL parameters only
                if !params.hosted_mode.0 {
                    // File loading - Load HCDF button
                    let button = if is_mobile {
                        egui::Button::new(egui::RichText::new("Load File").size(16.0 * ui_scale))
                            .min_size(egui::vec2(0.0, 40.0))
                    } else {
                        egui::Button::new("Load File")
                    };
                    if ui.add(button).clicked() {
                        trigger_file_open(
                            &params.pending_file_results,
                            FilePickerContext::HcdfImport,
                            FileFilter::hcdf(),
                        );
                    }

                    // URL input for loading from web
                    ui.add_space(4.0);

                    // Calculate fixed width for text input (panel width minus button and padding)
                    let input_width = (panel_width - 60.0).max(80.0);

                    ui.horizontal(|ui| {
                        ui.set_max_width(panel_width);

                        let text_edit = egui::TextEdit::singleline(&mut params.url_input.url)
                            .hint_text("https://...")
                            .desired_width(input_width);
                        let response = ui.add(text_edit);

                        // Fetch button or loading indicator
                        if params.url_input.loading {
                            ui.spinner();
                        } else {
                            let fetch_enabled = !params.url_input.url.is_empty()
                                && (params.url_input.url.starts_with("http://")
                                    || params.url_input.url.starts_with("https://"));
                            if ui.add_enabled(fetch_enabled, egui::Button::new("Go")).clicked() {
                                fetch_hcdf_from_url(&params.url_input.url, params.url_input.pending_result.clone());
                                params.url_input.loading = true;
                                params.url_input.error = None;
                            }
                        }

                        // Also fetch on Enter key
                        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            if !params.url_input.url.is_empty() && !params.url_input.loading {
                                fetch_hcdf_from_url(&params.url_input.url, params.url_input.pending_result.clone());
                                params.url_input.loading = true;
                                params.url_input.error = None;
                            }
                        }
                    });

                    // Show error if any
                    if let Some(ref error) = params.url_input.error {
                        ui.label(
                            egui::RichText::new(error)
                                .size(10.0 * ui_scale)
                                .color(egui::Color32::from_rgb(255, 100, 100))
                        );
                    }

                    ui.label(
                        egui::RichText::new("Load .hcdf from file or URL")
                            .size(11.0 * ui_scale)
                        .color(egui::Color32::GRAY)
                    );

                    ui.separator();
                } // end if !hosted_mode

                // Device list
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for device in &params.registry.devices {
                        let is_selected = params.selected.0.as_ref() == Some(&device.id);

                        // Device name color - viewer mode uses simple white
                        let name_color = if device.status == DeviceStatus::Unknown {
                            egui::Color32::GRAY
                        } else {
                            egui::Color32::from_rgb(200, 200, 200) // White
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

                // HCDF Import - collapsible section (only in non-hosted mode)
                if !params.hosted_mode.0 {
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
                        });

                    ui.separator();
                }

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

                        // NOTE: Render scale feature removed - scale_factor_override doesn't work
                        // correctly in WASM (renders to partial canvas instead of downscaling)
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
                                    // Viewer mode: just show the status from HCDF
                                    let status_str = match device.status {
                                        DeviceStatus::Offline => "Offline",
                                        DeviceStatus::Online => "Loaded",
                                        DeviceStatus::Unknown => "Loaded",
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

                                });

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
                                                "power" => egui::Color32::from_rgb(255, 50, 50),  // Vibrant red
                                                "card" => egui::Color32::from_rgb(180, 180, 100), // Tan/khaki
                                                _ => egui::Color32::from_rgb(255, 0, 255),        // Bright magenta (unknown)
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

                            // Per-device antenna visibility toggle (only if device has antennas)
                            if !device.antennas.is_empty() {
                                let mut show_antennas = params.frame_visibility.show_antennas_for(&id);
                                if ui.checkbox(&mut show_antennas, "Show Antennas").changed() {
                                    params.frame_visibility.set_show_antennas(&id, show_antennas);
                                }
                                ui.label(
                                    egui::RichText::new(format!("{} antenna(s)", device.antennas.len()))
                                        .size(11.0 * ui_scale)
                                        .color(egui::Color32::GRAY)
                                );

                                // Show antenna details when enabled
                                if show_antennas {
                                    let mut any_antenna_hovered = false;
                                    ui.indent("antennas", |ui| {
                                        for antenna in &device.antennas {
                                            let antenna_key = format!("{}:{}", id, antenna.name);
                                            let is_hovered = params.frame_visibility.hovered_antenna.as_ref() == Some(&antenna_key);
                                            let antenna_color = match antenna.antenna_type.to_lowercase().as_str() {
                                                "wifi" | "wlan" => egui::Color32::from_rgb(50, 150, 255),
                                                "bluetooth" | "bt" => egui::Color32::from_rgb(100, 100, 255),
                                                "gnss" | "gps" => egui::Color32::from_rgb(50, 200, 100),
                                                "cellular" | "lte" | "5g" => egui::Color32::from_rgb(255, 150, 50),
                                                "nfc" => egui::Color32::from_rgb(200, 100, 200),
                                                "uwb" => egui::Color32::from_rgb(255, 200, 50),
                                                "lora" => egui::Color32::from_rgb(230, 128, 50),
                                                "802.15.4" | "wpan" | "zigbee" | "thread" => egui::Color32::from_rgb(153, 102, 51), // Brown/tan for WPAN
                                                _ => egui::Color32::from_rgb(255, 0, 0), // Red (unknown)
                                            };
                                            // Highlight text if hovered (either from UI or 3D view)
                                            let display_color = if is_hovered {
                                                egui::Color32::WHITE
                                            } else {
                                                antenna_color
                                            };

                                            // Build antenna label text
                                            let label_text = format!("{} ({})", antenna.name, antenna.antenna_type);

                                            // Use selectable_label for built-in hover detection
                                            let response = ui.selectable_label(
                                                is_hovered,
                                                egui::RichText::new(&label_text)
                                                    .size(12.0 * ui_scale)
                                                    .color(display_color)
                                            );

                                            // Set hovered_antenna when hovering over antenna name in UI
                                            if response.hovered() {
                                                params.frame_visibility.hovered_antenna = Some(antenna_key);
                                                params.frame_visibility.hovered_antenna_from_ui = true;
                                                any_antenna_hovered = true;
                                            }
                                        }
                                    });

                                    // Clear hovered_antenna only if:
                                    // 1. No antenna in this UI list is hovered, AND
                                    // 2. The hover was set by UI (not 3D), AND
                                    // 3. The currently hovered antenna belongs to this device
                                    if !any_antenna_hovered && params.frame_visibility.hovered_antenna_from_ui {
                                        if let Some(ref hovered) = params.frame_visibility.hovered_antenna.clone() {
                                            if hovered.starts_with(&format!("{}:", id)) {
                                                params.frame_visibility.hovered_antenna = None;
                                                params.frame_visibility.hovered_antenna_from_ui = false;
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

                            // Remove device button (available for all devices)
                            let remove_button = if is_mobile {
                                egui::Button::new(
                                    egui::RichText::new("Remove Device")
                                        .size(16.0 * ui_scale)
                                        .color(egui::Color32::from_rgb(200, 100, 100))
                                ).min_size(egui::vec2(0.0, 40.0))
                            } else {
                                egui::Button::new(
                                    egui::RichText::new("Remove Device")
                                        .color(egui::Color32::from_rgb(200, 100, 100))
                                )
                            };
                            if ui.add(remove_button).clicked() {
                                // Mark device for removal (processed by separate system)
                                params.pending_removals.0.push(id.clone());
                                params.selected.0 = None;
                                params.ui_layout.show_right_panel = false;
                            }
                            ui.separator();

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
