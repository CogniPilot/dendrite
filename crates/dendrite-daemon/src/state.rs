//! Application state management

use anyhow::Result;
use dendrite_core::{Device, DeviceId, FragmentDatabase, Hcdf, Topology};
use dendrite_discovery::{DiscoveryEvent, DiscoveryScanner};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info, warn};

use crate::config::Config;

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

        // Apply fragment matching if device doesn't have a model path
        let mut device = device.clone();
        if device.model_path.is_none() {
            if let (Some(board), Some(app)) = (&device.info.board, &device.firmware.name) {
                let mut fragments = self.fragments.write().await;
                if let Some(model) = fragments.get_model(board, app) {
                    info!(
                        device = %device.id,
                        board = %board,
                        app = %app,
                        model = %model,
                        "Matched device to model via fragment database"
                    );
                    device.model_path = Some(model);
                    // Update the device in the scanner silently (don't trigger new events)
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

