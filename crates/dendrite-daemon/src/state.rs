//! Application state management

use anyhow::Result;
use dendrite_core::{Comp, Device, DeviceFrame, DeviceId, DeviceVisual, FragmentDatabase, Hcdf, Topology, parse_pose_string, sha256_hex};
use dendrite_discovery::{DiscoveryEvent, DiscoveryScanner};
use dendrite_mcumgr::query::query_hcdf_info;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::hcdf_fetch::HcdfFetcher;

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
    async fn update_device(&self, device: &Device) -> Device {
        let parent_name = self.config.parent.as_ref().map(|p| p.name.as_str());

        // Apply fragment matching if device doesn't have visuals
        let mut device = device.clone();
        if device.visuals.is_empty() {
            if let (Some(board), Some(app)) = (&device.info.board, &device.firmware.name) {
                // Try to fetch remote HCDF first (MCUmgr query + remote fetch)
                let remote_fragment = self.try_fetch_remote_hcdf(&device, board, app).await;

                if let Some((visuals, frames)) = remote_fragment {
                    info!(
                        device = %device.id,
                        board = %board,
                        app = %app,
                        visuals = visuals.len(),
                        frames = frames.len(),
                        "Applied remote HCDF fragment"
                    );
                    device.visuals = visuals;
                    device.frames = frames;
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
                    }
                }

                // Also set legacy model_path for backward compatibility
                if device.model_path.is_none() {
                    device.model_path = device.visuals.first()
                        .and_then(|v| v.model_path.clone());
                }

                // Update the device in the scanner silently (don't trigger new events)
                if !device.visuals.is_empty() {
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
    /// 4. Parse and return visuals/frames
    async fn try_fetch_remote_hcdf(
        &self,
        device: &Device,
        board: &str,
        app: &str,
    ) -> Option<(Vec<DeviceVisual>, Vec<DeviceFrame>)> {
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
                })
            })?;

        // Convert to DeviceVisual/DeviceFrame, fetching and caching models
        let mut visuals: Vec<DeviceVisual> = Vec::new();
        for v in comp.visual.iter() {
            let model_path = if let Some(model_ref) = &v.model {
                // Resolve the model URL
                let model_url = resolve_model_url(&model_ref.href, &root_url);

                // Fetch and cache the model
                match self.hcdf_fetcher.fetch_model(
                    &model_url,
                    model_ref.sha.as_deref(),
                    &hcdf_sha,
                ).await {
                    Ok(Some(cached_path)) => {
                        // Return path relative to /models/ endpoint (strip "models/" prefix)
                        let path = cached_path.strip_prefix("models/").unwrap_or(&cached_path);
                        Some(path.to_string())
                    }
                    Ok(None) => {
                        // Fallback to remote URL if caching fails
                        warn!(
                            device = %device.id,
                            model = %model_ref.href,
                            "Failed to cache model, using remote URL"
                        );
                        Some(model_url)
                    }
                    Err(e) => {
                        warn!(
                            device = %device.id,
                            model = %model_ref.href,
                            error = %e,
                            "Error fetching model, using remote URL"
                        );
                        Some(resolve_model_url(&model_ref.href, &root_url))
                    }
                }
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

        if visuals.is_empty() && frames.is_empty() {
            return None;
        }

        Some((visuals, frames))
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

