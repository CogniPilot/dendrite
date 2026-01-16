//! Application state management

use anyhow::Result;
use dendrite_core::{Comp, Device, DeviceAxisAlign, DeviceFrame, DeviceFov, DeviceGeometry, DeviceId, DevicePort, DeviceSensor, DeviceVisual, FragmentDatabase, Hcdf, Topology, parse_pose_string, sha256_hex};
use dendrite_core::hcdf::{Geometry, Sensor, Fov};
use dendrite_discovery::{DiscoveryEvent, DiscoveryScanner};
use dendrite_mcumgr::query::query_hcdf_info;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::firmware_fetch::FirmwareFetcher;
use crate::hcdf_fetch::HcdfFetcher;
use crate::ota::OtaService;

/// Result of fetching and parsing an HCDF fragment
#[derive(Debug, Default)]
struct HcdfFragmentData {
    visuals: Vec<DeviceVisual>,
    frames: Vec<DeviceFrame>,
    ports: Vec<DevicePort>,
    sensors: Vec<DeviceSensor>,
}

/// Shared application state
pub struct AppState {
    /// Discovery scanner
    pub scanner: Arc<DiscoveryScanner>,
    /// HCDF document
    pub hcdf: Arc<RwLock<Hcdf>>,
    /// Device topology
    pub topology: Arc<RwLock<Topology>>,
    /// Fragment database for board/app to model mapping
    pub fragments: Arc<RwLock<FragmentDatabase>>,
    /// Remote HCDF fetcher with caching
    pub hcdf_fetcher: Arc<HcdfFetcher>,
    /// Firmware manifest fetcher
    pub firmware_fetcher: Arc<FirmwareFetcher>,
    /// OTA update service
    pub ota_service: Arc<OtaService>,
    /// Configuration
    pub config: Config,
    /// Event broadcast for WebSocket clients
    pub events: broadcast::Sender<DiscoveryEvent>,
}

impl AppState {
    /// Create new application state
    pub async fn new(config: Config) -> Result<Arc<Self>> {
        // Load or create HCDF document
        let hcdf = load_or_create_hcdf(&config.hcdf.path)?;

        // Build initial topology from HCDF
        let topology = Topology::from_hcdf(&hcdf);

        // Load fragment database
        let fragments = load_fragments(&config.fragments.path);

        // Create HCDF fetcher with cache in fragments directory
        let cache_dir = Path::new(&config.fragments.path)
            .parent()
            .unwrap_or(Path::new("."))
            .join("cache");
        let hcdf_fetcher = Arc::new(HcdfFetcher::new(cache_dir)?);

        // Create firmware fetcher
        let firmware_fetcher = Arc::new(FirmwareFetcher::new()?);

        // Create OTA service
        let ota_service = Arc::new(OtaService::new(firmware_fetcher.clone()));

        // Create discovery scanner
        let scanner_config = config.to_scanner_config();
        let scanner = Arc::new(DiscoveryScanner::new(scanner_config));

        // Create event channel
        let (events, _) = broadcast::channel(100);

        let state = Arc::new(Self {
            scanner,
            hcdf: Arc::new(RwLock::new(hcdf)),
            topology: Arc::new(RwLock::new(topology)),
            fragments: Arc::new(RwLock::new(fragments)),
            hcdf_fetcher,
            firmware_fetcher,
            ota_service,
            config,
            events,
        });

        // Start forwarding scanner events
        let state_clone = state.clone();
        let mut rx = state.scanner.subscribe();
        tokio::spawn(async move {
            while let Ok(event) = rx.recv().await {
                // Update HCDF and topology on device changes, get updated event
                let updated_event = match &event {
                    DiscoveryEvent::DeviceDiscovered(device) => {
                        let updated = state_clone.update_device(device).await;
                        DiscoveryEvent::DeviceDiscovered(updated)
                    }
                    DiscoveryEvent::DeviceUpdated(device) => {
                        let updated = state_clone.update_device(device).await;
                        DiscoveryEvent::DeviceUpdated(updated)
                    }
                    DiscoveryEvent::DeviceOffline(_id) => {
                        // Could mark device as offline in HCDF
                        event.clone()
                    }
                    _ => event.clone(),
                };

                // Forward updated event to WebSocket clients
                let _ = state_clone.events.send(updated_event);
            }
        });

        Ok(state)
    }

    /// Update device in HCDF and topology, returns the (potentially modified) device
    /// This applies fragment matching, fetches remote HCDF data, and updates topology
    pub async fn update_device(&self, device: &Device) -> Device {
        let parent_name = self.config.parent.as_ref().map(|p| p.name.as_str());

        // Apply fragment matching if device doesn't have visuals
        let mut device = device.clone();

        // Preserve existing pose from HCDF if device doesn't have one
        // This ensures positions are restored on page refresh
        if device.pose.is_none() {
            let hcdf = self.hcdf.read().await;
            if let Some(mcu) = hcdf.mcu.iter().find(|m| m.hwid.as_deref() == Some(device.id.as_str())) {
                if let Some(pose_str) = &mcu.pose_cg {
                    if let Some(pose) = parse_pose_string(pose_str) {
                        device.pose = Some(pose.to_array());
                        debug!(device = %device.id, pose = ?pose_str, "Restored pose from HCDF");
                    }
                }
            }
        }
        if device.visuals.is_empty() {
            if let (Some(board), Some(app)) = (&device.info.board, &device.firmware.name) {
                // Try to fetch remote HCDF first (MCUmgr query + remote fetch)
                let remote_fragment = self.try_fetch_remote_hcdf(&device, board, app).await;

                if let Some(fragment_data) = remote_fragment {
                    info!(
                        device = %device.id,
                        board = %board,
                        app = %app,
                        visuals = fragment_data.visuals.len(),
                        frames = fragment_data.frames.len(),
                        ports = fragment_data.ports.len(),
                        sensors = fragment_data.sensors.len(),
                        "Applied remote HCDF fragment"
                    );
                    device.visuals = fragment_data.visuals;
                    device.frames = fragment_data.frames;
                    device.ports = fragment_data.ports;
                    device.sensors = fragment_data.sensors;
                } else {
                    // Fall back to local fragment database
                    let mut fragments = self.fragments.write().await;
                    if let Some(fragment) = fragments.find_fragment(board, app) {
                        info!(
                            device = %device.id,
                            board = %board,
                            app = %app,
                            visuals = fragment.visuals.len(),
                            frames = fragment.frames.len(),
                            "Matched device to local fragment"
                        );

                        // Convert fragment visuals to device visuals
                        device.visuals = fragment.visuals.iter().map(|v| {
                            DeviceVisual {
                                name: v.name.clone(),
                                toggle: v.toggle.clone(),
                                pose: v.pose.as_ref().and_then(|p| parse_pose_string(p)).map(|p| p.to_array()),
                                model_path: v.model.as_ref().map(|m| m.href.clone()),
                                model_sha: v.model.as_ref().and_then(|m| m.sha.clone()),
                            }
                        }).collect();

                        // Convert fragment frames to device frames
                        device.frames = fragment.frames.iter().map(|f| {
                            DeviceFrame {
                                name: f.name.clone(),
                                description: f.description.clone(),
                                pose: f.pose.as_ref().and_then(|p| parse_pose_string(p)).map(|p| p.to_array()),
                            }
                        }).collect();

                        // Convert fragment ports to device ports
                        device.ports = fragment.ports.iter().map(|p| {
                            DevicePort {
                                name: p.name.clone(),
                                port_type: p.port_type.clone(),
                                pose: p.pose.as_ref().and_then(|pose| parse_pose_string(pose)).map(|pose| pose.to_array()),
                                geometry: p.geometry.iter().filter_map(convert_geometry).collect(),
                                visual_name: p.visual.clone(),
                                mesh_name: p.mesh.clone(),
                            }
                        }).collect();

                        // Convert fragment sensors to device sensors
                        device.sensors = fragment.sensors.iter()
                            .flat_map(convert_sensor)
                            .collect();
                    }
                }

                // Also set legacy model_path for backward compatibility
                if device.model_path.is_none() {
                    device.model_path = device.visuals.first()
                        .and_then(|v| v.model_path.clone());
                }

                // Update the device in the scanner silently (don't trigger new events)
                if !device.visuals.is_empty() || !device.ports.is_empty() || !device.sensors.is_empty() {
                    self.scanner.update_device_silent(device.clone()).await;
                }
            }
        }

        // Update HCDF
        {
            let mut hcdf = self.hcdf.write().await;
            hcdf.upsert_device(&device, parent_name);
        }

        // Rebuild topology
        {
            let devices = self.scanner.devices().await;
            let parent_id = self.config.parent.as_ref().map(|p| DeviceId::from_hwid(&p.name));
            let new_topology = Topology::from_devices(&devices, parent_id.as_ref());
            *self.topology.write().await = new_topology;
        }

        debug!(device = %device.id, "Updated device in state");
        device
    }

    /// Get all devices
    pub async fn devices(&self) -> Vec<Device> {
        self.scanner.devices().await
    }

    /// Get device by ID
    pub async fn get_device(&self, id: &str) -> Option<Device> {
        self.scanner.get_device(&DeviceId(id.to_string())).await
    }

    /// Get current topology
    pub async fn get_topology(&self) -> Topology {
        self.topology.read().await.clone()
    }

    /// Get current HCDF document
    pub async fn get_hcdf(&self) -> Hcdf {
        self.hcdf.read().await.clone()
    }

    /// Save HCDF to file
    pub async fn save_hcdf(&self) -> Result<()> {
        let hcdf = self.hcdf.read().await;
        let path = Path::new(&self.config.hcdf.path);
        hcdf.to_file(path)?;
        info!(path = %path.display(), "Saved HCDF");
        Ok(())
    }

    /// Subscribe to events
    pub fn subscribe(&self) -> broadcast::Receiver<DiscoveryEvent> {
        self.events.subscribe()
    }

    /// Try to fetch remote HCDF for a device
    ///
    /// 1. Query device via MCUmgr for HCDF URL + SHA
    /// 2. If device doesn't support HCDF group, construct fallback URL from board/app
    /// 3. Fetch HCDF (using cache if SHA matches)
    /// 4. Parse and return visuals, frames, ports, and sensors
    async fn try_fetch_remote_hcdf(
        &self,
        device: &Device,
        board: &str,
        app: &str,
    ) -> Option<HcdfFragmentData> {
        // Try to query HCDF info from device via MCUmgr
        let (device_url, device_sha) = match query_hcdf_info(device.discovery.ip, device.discovery.port).await {
            Ok(Some(info)) => {
                info!(
                    device = %device.id,
                    url = ?info.url,
                    sha = ?info.sha,
                    "Device reported HCDF info"
                );
                (info.url, info.sha)
            }
            Ok(None) => {
                debug!(device = %device.id, "Device doesn't support HCDF group, using fallback URL");
                (None, None)
            }
            Err(e) => {
                debug!(device = %device.id, error = %e, "Failed to query HCDF info, using fallback URL");
                (None, None)
            }
        };

        // Determine the base URL for resolving relative model paths
        let hcdf_url = device_url.clone()
            .unwrap_or_else(|| crate::hcdf_fetch::HcdfFetcher::construct_url(board, app));
        let root_url = get_root_url(&hcdf_url);

        // Fetch HCDF (from device URL or fallback)
        let hcdf_content = match self.hcdf_fetcher.fetch_hcdf(
            board,
            app,
            device_url.as_deref(),
            device_sha.as_deref(),
        ).await {
            Ok(Some(content)) => content,
            Ok(None) => {
                debug!(device = %device.id, "No remote HCDF available");
                return None;
            }
            Err(e) => {
                warn!(device = %device.id, error = %e, "Failed to fetch remote HCDF");
                return None;
            }
        };

        // Compute HCDF SHA for cache linking
        let hcdf_sha = sha256_hex(hcdf_content.as_bytes());

        // Parse the HCDF content
        let hcdf = match dendrite_core::Hcdf::from_xml(&hcdf_content) {
            Ok(h) => h,
            Err(e) => {
                warn!(device = %device.id, error = %e, "Failed to parse remote HCDF");
                return None;
            }
        };

        // Extract visuals and frames from first comp/mcu element
        let comp = hcdf.comp.into_iter().next()
            .or_else(|| {
                hcdf.mcu.into_iter().next().map(|m| Comp {
                    name: m.name,
                    role: None,
                    hwid: m.hwid,
                    description: m.description,
                    pose_cg: m.pose_cg,
                    mass: m.mass,
                    board: m.board,
                    software: m.software,
                    discovered: m.discovered,
                    model: m.model,
                    visual: m.visual,
                    frame: m.frame,
                    network: m.network,
                    port: Vec::new(),
                    antenna: Vec::new(),
                    sensor: Vec::new(),
                })
            })?;

        // Convert to DeviceVisual/DeviceFrame, fetching and caching models
        let mut visuals: Vec<DeviceVisual> = Vec::new();
        for v in comp.visual.iter() {
            let model_path = if let Some(model_ref) = &v.model {
                // Resolve the model URL
                let model_url = resolve_model_url(&model_ref.href, &root_url);

                // Fetch and cache the model (for local serving)
                // But always return the remote URL for frontend to load directly
                // This ensures the frontend can load from hcdf.cognipilot.org over HTTPS
                let _ = self.hcdf_fetcher.fetch_model(
                    &model_url,
                    model_ref.sha.as_deref(),
                    &hcdf_sha,
                ).await;

                Some(model_url)
            } else {
                None
            };

            visuals.push(DeviceVisual {
                name: v.name.clone(),
                toggle: v.toggle.clone(),
                pose: v.pose.as_ref().and_then(|p| parse_pose_string(p)).map(|p| p.to_array()),
                model_path,
                model_sha: v.model.as_ref().and_then(|m| m.sha.clone()),
            });
        }

        let frames: Vec<DeviceFrame> = comp.frame.iter().map(|f| {
            DeviceFrame {
                name: f.name.clone(),
                description: f.description.clone(),
                pose: f.pose.as_ref().and_then(|p| parse_pose_string(p)).map(|p| p.to_array()),
            }
        }).collect();

        // Convert ports
        let ports: Vec<DevicePort> = comp.port.iter().map(|p| {
            DevicePort {
                name: p.name.clone(),
                port_type: p.port_type.clone(),
                pose: p.pose.as_ref().and_then(|pose| parse_pose_string(pose)).map(|pose| pose.to_array()),
                geometry: p.geometry.iter().filter_map(convert_geometry).collect(),
                visual_name: p.visual.clone(),
                mesh_name: p.mesh.clone(),
            }
        }).collect();

        // Convert sensors
        let sensors: Vec<DeviceSensor> = comp.sensor.iter()
            .flat_map(convert_sensor)
            .collect();

        if visuals.is_empty() && frames.is_empty() && ports.is_empty() && sensors.is_empty() {
            return None;
        }

        Some(HcdfFragmentData { visuals, frames, ports, sensors })
    }
}

/// Get the root URL from an HCDF URL (domain root for absolute paths like "models/")
fn get_root_url(hcdf_url: &str) -> String {
    // Extract scheme + host from URL
    // e.g., "https://hcdf.cognipilot.org/mr_mcxn_t1/optical-flow/optical-flow.hcdf"
    //    -> "https://hcdf.cognipilot.org/"
    if let Some(scheme_end) = hcdf_url.find("://") {
        let after_scheme = &hcdf_url[scheme_end + 3..];
        if let Some(path_start) = after_scheme.find('/') {
            return format!("{}/", &hcdf_url[..scheme_end + 3 + path_start]);
        }
    }
    hcdf_url.to_string()
}

/// Resolve a model path to absolute URL
fn resolve_model_url(model_path: &str, root_url: &str) -> String {
    // If already absolute URL, return as-is
    if model_path.starts_with("http://") || model_path.starts_with("https://") {
        return model_path.to_string();
    }

    // Handle relative paths - strip ./ prefix if present
    let path = model_path.trim_start_matches("./");

    // Models are at the root level of the domain (e.g., /models/...)
    format!("{}{}", root_url, path)
}

/// Convert HCDF Geometry to DeviceGeometry
fn convert_geometry(geom: &Geometry) -> Option<DeviceGeometry> {
    if let Some(ref box_geom) = geom.box_geom {
        // Parse size string "x y z" to [f64; 3]
        let parts: Vec<f64> = box_geom.size
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect();
        if parts.len() >= 3 {
            return Some(DeviceGeometry::Box { size: [parts[0], parts[1], parts[2]] });
        }
    }
    if let Some(ref cyl) = geom.cylinder {
        return Some(DeviceGeometry::Cylinder { radius: cyl.radius, length: cyl.length });
    }
    if let Some(ref sphere) = geom.sphere {
        return Some(DeviceGeometry::Sphere { radius: sphere.radius });
    }
    // New types first (preferred)
    if let Some(ref cf) = geom.conical_frustum {
        return Some(DeviceGeometry::ConicalFrustum {
            near: cf.near,
            far: cf.far,
            fov: cf.fov,
        });
    }
    if let Some(ref pf) = geom.pyramidal_frustum {
        return Some(DeviceGeometry::PyramidalFrustum {
            near: pf.near,
            far: pf.far,
            hfov: pf.hfov,
            vfov: pf.vfov,
        });
    }
    // Legacy types (deprecated)
    if let Some(ref cone) = geom.cone {
        return Some(DeviceGeometry::Cone { radius: cone.radius, length: cone.length });
    }
    if let Some(ref frustum) = geom.frustum {
        return Some(DeviceGeometry::Frustum {
            near: frustum.near,
            far: frustum.far,
            hfov: frustum.hfov,
            vfov: frustum.vfov,
        });
    }
    None
}

/// Convert HCDF Sensor to DeviceSensor entries
fn convert_sensor(sensor: &Sensor) -> Vec<DeviceSensor> {
    let mut results = Vec::new();

    // Process inertial sensors
    for inertial in &sensor.inertial {
        results.push(DeviceSensor {
            name: sensor.name.clone(),
            category: "inertial".to_string(),
            sensor_type: inertial.sensor_type.clone(),
            driver: inertial.driver.as_ref().map(|d| d.name.clone()),
            pose: inertial.pose.as_ref().and_then(|p| parse_pose_string(p)).map(|p| p.to_array()),
            axis_align: inertial.driver.as_ref()
                .and_then(|d| d.axis_align.as_ref())
                .map(|a| DeviceAxisAlign {
                    x: a.x.clone(),
                    y: a.y.clone(),
                    z: a.z.clone(),
                }),
            geometry: inertial.geometry.as_ref().and_then(convert_geometry),
            fovs: Vec::new(),
        });
    }

    // Process EM sensors (magnetometer)
    for em in &sensor.em {
        results.push(DeviceSensor {
            name: sensor.name.clone(),
            category: "em".to_string(),
            sensor_type: em.sensor_type.clone(),
            driver: em.driver.as_ref().map(|d| d.name.clone()),
            pose: em.pose.as_ref().and_then(|p| parse_pose_string(p)).map(|p| p.to_array()),
            axis_align: em.driver.as_ref()
                .and_then(|d| d.axis_align.as_ref())
                .map(|a| DeviceAxisAlign {
                    x: a.x.clone(),
                    y: a.y.clone(),
                    z: a.z.clone(),
                }),
            geometry: em.geometry.as_ref().and_then(convert_geometry),
            fovs: Vec::new(),
        });
    }

    // Process optical sensors (camera, lidar, tof, optical_flow)
    for optical in &sensor.optical {
        // Convert FOVs if present
        let fovs: Vec<DeviceFov> = optical.fov.iter().map(|f| convert_fov(f)).collect();

        results.push(DeviceSensor {
            name: sensor.name.clone(),
            category: "optical".to_string(),
            sensor_type: optical.sensor_type.clone(),
            driver: optical.driver.as_ref().map(|d| d.name.clone()),
            pose: optical.pose.as_ref().and_then(|p| parse_pose_string(p)).map(|p| p.to_array()),
            axis_align: optical.driver.as_ref()
                .and_then(|d| d.axis_align.as_ref())
                .map(|a| DeviceAxisAlign {
                    x: a.x.clone(),
                    y: a.y.clone(),
                    z: a.z.clone(),
                }),
            geometry: optical.geometry.as_ref().and_then(convert_geometry),
            fovs,
        });
    }

    // Process RF sensors (GNSS, UWB, radar)
    for rf in &sensor.rf {
        results.push(DeviceSensor {
            name: sensor.name.clone(),
            category: "rf".to_string(),
            sensor_type: rf.sensor_type.clone(),
            driver: rf.driver.as_ref().map(|d| d.name.clone()),
            pose: rf.pose.as_ref().and_then(|p| parse_pose_string(p)).map(|p| p.to_array()),
            axis_align: rf.driver.as_ref()
                .and_then(|d| d.axis_align.as_ref())
                .map(|a| DeviceAxisAlign {
                    x: a.x.clone(),
                    y: a.y.clone(),
                    z: a.z.clone(),
                }),
            geometry: rf.geometry.as_ref().and_then(convert_geometry),
            fovs: Vec::new(),
        });
    }

    // Process chemical sensors
    for chem in &sensor.chemical {
        results.push(DeviceSensor {
            name: sensor.name.clone(),
            category: "chemical".to_string(),
            sensor_type: chem.sensor_type.clone(),
            driver: chem.driver.as_ref().map(|d| d.name.clone()),
            pose: chem.pose.as_ref().and_then(|p| parse_pose_string(p)).map(|p| p.to_array()),
            axis_align: None, // Chemical sensors don't have axis alignment
            geometry: chem.geometry.as_ref().and_then(convert_geometry),
            fovs: Vec::new(),
        });
    }

    // Process force sensors (pressure, strain, torque, load cell)
    for force in &sensor.force {
        results.push(DeviceSensor {
            name: sensor.name.clone(),
            category: "force".to_string(),
            sensor_type: force.sensor_type.clone(),
            driver: force.driver.as_ref().map(|d| d.name.clone()),
            pose: force.pose.as_ref().and_then(|p| parse_pose_string(p)).map(|p| p.to_array()),
            axis_align: None, // Force sensors typically don't have axis alignment
            geometry: force.geometry.as_ref().and_then(convert_geometry),
            fovs: Vec::new(),
        });
    }

    results
}

/// Convert HCDF Fov to DeviceFov
fn convert_fov(fov: &Fov) -> DeviceFov {
    DeviceFov {
        name: fov.name.clone(),
        color: fov.parse_color().map(|(r, g, b)| [r, g, b]),
        pose: fov.parse_pose().map(|p| p.to_array()),
        geometry: fov.geometry.as_ref().and_then(convert_geometry),
    }
}

/// Load HCDF from file or create new
fn load_or_create_hcdf(path: &str) -> Result<Hcdf> {
    let path = Path::new(path);
    if path.exists() {
        match Hcdf::from_file(path) {
            Ok(hcdf) => {
                info!(path = %path.display(), "Loaded HCDF");
                return Ok(hcdf);
            }
            Err(e) => {
                warn!(path = %path.display(), error = %e, "Failed to load HCDF, creating new");
            }
        }
    }
    Ok(Hcdf::new())
}

/// Load fragment database from file or create empty
fn load_fragments(path: &str) -> FragmentDatabase {
    let path = Path::new(path);
    if path.exists() {
        match FragmentDatabase::from_file(path) {
            Ok(db) => {
                info!(
                    path = %path.display(),
                    count = db.index().fragment.len(),
                    "Loaded fragment database"
                );
                return db;
            }
            Err(e) => {
                warn!(path = %path.display(), error = %e, "Failed to load fragments, using empty database");
            }
        }
    } else {
        info!(path = %path.display(), "Fragment database not found, using empty database");
    }
    FragmentDatabase::empty()
}

