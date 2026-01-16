//! OTA (Over-The-Air) firmware update service
//!
//! This module handles firmware updates via MCUmgr image upload.
//! The update process:
//! 1. Download firmware binary from upstream
//! 2. Upload to device via MCUmgr
//! 3. Mark image as pending test
//! 4. Reset device
//! 5. Verify update succeeded

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, error, info, warn};

use crate::firmware_fetch::FirmwareFetcher;

/// MCUmgr port for device communication
const MCUMGR_PORT: u16 = 1337;

/// Update state for a device
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdateState {
    /// Downloading firmware binary from upstream
    Downloading { progress: f32 },
    /// Uploading firmware to device via MCUmgr
    Uploading { progress: f32 },
    /// Confirming (marking image as pending test)
    Confirming,
    /// Rebooting device
    Rebooting,
    /// Verifying update was successful
    Verifying,
    /// Update completed successfully
    Complete,
    /// Update failed
    Failed { error: String },
    /// Update was cancelled
    Cancelled,
}

impl UpdateState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, UpdateState::Complete | UpdateState::Failed { .. } | UpdateState::Cancelled)
    }
}

/// OTA update event sent via WebSocket
#[derive(Debug, Clone, Serialize)]
pub struct OtaEvent {
    pub device_id: String,
    pub state: UpdateState,
}

/// Information about a device being updated
#[derive(Debug, Clone)]
struct UpdateInfo {
    pub device_id: String,
    pub ip: String,
    pub board: String,
    pub app: String,
    pub state: UpdateState,
}

/// OTA update service
pub struct OtaService {
    /// Firmware fetcher for downloading binaries
    firmware_fetcher: Arc<FirmwareFetcher>,
    /// Active updates (device_id -> UpdateInfo)
    active_updates: Arc<RwLock<HashMap<String, UpdateInfo>>>,
    /// Event sender for update progress
    event_tx: broadcast::Sender<OtaEvent>,
}

impl OtaService {
    /// Create a new OTA service
    pub fn new(firmware_fetcher: Arc<FirmwareFetcher>) -> Self {
        let (event_tx, _) = broadcast::channel(100);
        Self {
            firmware_fetcher,
            active_updates: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
        }
    }

    /// Subscribe to OTA events
    pub fn subscribe(&self) -> broadcast::Receiver<OtaEvent> {
        self.event_tx.subscribe()
    }

    /// Get the current state of an update
    pub async fn get_state(&self, device_id: &str) -> Option<UpdateState> {
        let updates = self.active_updates.read().await;
        updates.get(device_id).map(|u| u.state.clone())
    }

    /// Get all active updates
    pub async fn get_all_updates(&self) -> Vec<(String, UpdateState)> {
        let updates = self.active_updates.read().await;
        updates
            .iter()
            .map(|(id, info)| (id.clone(), info.state.clone()))
            .collect()
    }

    /// Cancel an in-progress update
    pub async fn cancel_update(&self, device_id: &str) -> Result<()> {
        let mut updates = self.active_updates.write().await;
        if let Some(info) = updates.get_mut(device_id) {
            if !info.state.is_terminal() {
                info.state = UpdateState::Cancelled;
                self.send_event(device_id, UpdateState::Cancelled);
                info!("Cancelled update for device {}", device_id);
            }
        }
        Ok(())
    }

    /// Start a firmware update for a device
    ///
    /// This spawns an async task to handle the update process.
    /// Requires firmware_manifest_uri to be set (no default fallback).
    pub async fn start_update(
        &self,
        device_id: String,
        ip: String,
        board: String,
        app: String,
        firmware_manifest_uri: Option<String>,
    ) -> Result<()> {
        // Check if already updating
        {
            let updates = self.active_updates.read().await;
            if let Some(info) = updates.get(&device_id) {
                if !info.state.is_terminal() {
                    return Err(anyhow!("Update already in progress for device {}", device_id));
                }
            }
        }

        // Initialize update state
        {
            let mut updates = self.active_updates.write().await;
            updates.insert(
                device_id.clone(),
                UpdateInfo {
                    device_id: device_id.clone(),
                    ip: ip.clone(),
                    board: board.clone(),
                    app: app.clone(),
                    state: UpdateState::Downloading { progress: 0.0 },
                },
            );
        }

        self.send_event(&device_id, UpdateState::Downloading { progress: 0.0 });

        // Clone what we need for the spawned task
        let firmware_fetcher = self.firmware_fetcher.clone();
        let active_updates = self.active_updates.clone();
        let event_tx = self.event_tx.clone();

        // Spawn the update task
        tokio::spawn(async move {
            let result = Self::run_update(
                &firmware_fetcher,
                &active_updates,
                &event_tx,
                device_id.clone(),
                ip,
                board,
                app,
                firmware_manifest_uri,
            )
            .await;

            if let Err(e) = result {
                error!("Update failed for device {}: {}", device_id, e);
                let mut updates = active_updates.write().await;
                if let Some(info) = updates.get_mut(&device_id) {
                    info.state = UpdateState::Failed {
                        error: e.to_string(),
                    };
                }
                let _ = event_tx.send(OtaEvent {
                    device_id: device_id.clone(),
                    state: UpdateState::Failed {
                        error: e.to_string(),
                    },
                });
            }
        });

        Ok(())
    }

    /// Run the actual update process
    async fn run_update(
        firmware_fetcher: &FirmwareFetcher,
        active_updates: &RwLock<HashMap<String, UpdateInfo>>,
        event_tx: &broadcast::Sender<OtaEvent>,
        device_id: String,
        ip: String,
        board: String,
        app: String,
        firmware_manifest_uri: Option<String>,
    ) -> Result<()> {
        info!(
            "Starting firmware update for device {} ({}/{})",
            device_id, board, app
        );

        // Helper to check if cancelled
        let is_cancelled = || async {
            let updates = active_updates.read().await;
            updates
                .get(&device_id)
                .map(|u| matches!(u.state, UpdateState::Cancelled))
                .unwrap_or(false)
        };

        // Helper to update state
        let set_state = |state: UpdateState| async {
            let mut updates = active_updates.write().await;
            if let Some(info) = updates.get_mut(&device_id) {
                info.state = state.clone();
            }
            let _ = event_tx.send(OtaEvent {
                device_id: device_id.clone(),
                state,
            });
        };

        // 1. Fetch manifest to get download URL (requires explicit firmware_manifest_uri)
        let manifest = firmware_fetcher
            .get_manifest(&board, &app, firmware_manifest_uri.as_deref())
            .await?
            .ok_or_else(|| anyhow!("No firmware manifest found for {}/{} (firmware_manifest_uri not configured)", board, app))?;

        if is_cancelled().await {
            return Ok(());
        }

        // 2. Download firmware binary
        set_state(UpdateState::Downloading { progress: 0.0 }).await;
        info!(
            "Downloading firmware v{} from {}",
            manifest.latest.version, manifest.latest.url
        );

        let firmware_data = firmware_fetcher.download_firmware(&manifest.latest).await?;
        info!("Downloaded {} bytes", firmware_data.len());

        if is_cancelled().await {
            return Ok(());
        }

        // 3. Upload to device via MCUmgr
        set_state(UpdateState::Uploading { progress: 0.0 }).await;
        info!("Uploading firmware to device at {}:{}", ip, MCUMGR_PORT);

        // Create a temporary file for the firmware
        // The mcumgr-client upload functions expect a file path
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("firmware_{}.bin", device_id));
        tokio::fs::write(&temp_file, &firmware_data).await?;

        // Upload using mcumgr-client
        // Note: This is blocking, so we run it in a blocking task
        let temp_file_clone = temp_file.clone();
        let device_id_clone = device_id.clone();
        let event_tx_clone = event_tx.clone();
        let active_updates_clone = active_updates.clone();

        let ip_clone = ip.clone();
        let upload_result = tokio::task::spawn_blocking(move || {
            use mcumgr_client::{UdpTransport, UdpSpecs, upload_image_transport};

            // Create UDP transport
            let specs = UdpSpecs {
                host: ip_clone,
                port: MCUMGR_PORT,
                timeout_s: 10,
                mtu: 512,
            };
            let mut transport = UdpTransport::new(&specs)
                .map_err(|e| anyhow!("Failed to create transport: {}", e))?;

            // Upload with progress callback
            upload_image_transport(
                &mut transport,
                &temp_file_clone,
                0, // slot 0
                Some(|uploaded: u64, total: u64| {
                    let progress = uploaded as f32 / total as f32;
                    // Send progress update (best effort)
                    let _ = event_tx_clone.send(OtaEvent {
                        device_id: device_id_clone.clone(),
                        state: UpdateState::Uploading { progress },
                    });
                }),
            )?;

            Ok::<_, anyhow::Error>(())
        })
        .await??;

        // Clean up temp file
        let _ = tokio::fs::remove_file(&temp_file).await;

        if is_cancelled().await {
            return Ok(());
        }

        // 4. Mark image as pending test and reset
        set_state(UpdateState::Confirming).await;
        info!("Confirming firmware image");

        let ip_clone = ip.clone();
        let confirm_result = tokio::task::spawn_blocking(move || {
            use mcumgr_client::{UdpTransport, UdpSpecs, list_transport, test_transport};

            let specs = UdpSpecs {
                host: ip_clone,
                port: MCUMGR_PORT,
                timeout_s: 5,
                mtu: 1024,
            };
            let mut transport = UdpTransport::new(&specs)
                .map_err(|e| anyhow!("Failed to create transport: {}", e))?;

            // Get the hash of the uploaded image
            let image_list = list_transport(&mut transport)?;

            // Find the pending image (slot 1 typically)
            let pending_hash = image_list
                .images
                .iter()
                .find(|img| !img.confirmed && !img.active)
                .map(|img| img.hash.clone())
                .ok_or_else(|| anyhow!("No pending image found after upload"))?;

            // Mark as pending test
            test_transport(&mut transport, pending_hash, Some(false))?;

            Ok::<_, anyhow::Error>(())
        })
        .await??;

        if is_cancelled().await {
            return Ok(());
        }

        // 5. Reset device
        set_state(UpdateState::Rebooting).await;
        info!("Resetting device");

        let ip_clone = ip.clone();
        let reset_result = tokio::task::spawn_blocking(move || {
            use mcumgr_client::{UdpTransport, UdpSpecs, reset_transport};

            let specs = UdpSpecs {
                host: ip_clone,
                port: MCUMGR_PORT,
                timeout_s: 5,
                mtu: 1024,
            };
            let mut transport = UdpTransport::new(&specs)
                .map_err(|e| anyhow!("Failed to create transport: {}", e))?;

            reset_transport(&mut transport)?;

            Ok::<_, anyhow::Error>(())
        })
        .await??;

        // 6. Wait for device to come back and verify
        set_state(UpdateState::Verifying).await;
        info!("Waiting for device to reboot...");

        // Wait a bit for the device to reboot
        tokio::time::sleep(Duration::from_secs(5)).await;

        if is_cancelled().await {
            return Ok(());
        }

        // Try to verify the device came back with new firmware
        // Give it a few retries since reboot takes time
        let expected_mcuboot_hash = manifest.latest.mcuboot_hash.clone();
        let mut verified = false;

        for attempt in 0..10 {
            tokio::time::sleep(Duration::from_secs(2)).await;

            if is_cancelled().await {
                return Ok(());
            }

            let ip_clone = ip.clone();
            let expected_hash_clone = expected_mcuboot_hash.clone();

            let verify_result = tokio::task::spawn_blocking(move || {
                use mcumgr_client::{UdpTransport, UdpSpecs, list_transport};

                let specs = UdpSpecs {
                    host: ip_clone,
                    port: MCUMGR_PORT,
                    timeout_s: 2,
                    mtu: 1024,
                };
                let mut transport = UdpTransport::new(&specs)
                    .map_err(|e| anyhow!("Failed to create transport: {}", e))?;

                let image_list = list_transport(&mut transport)?;

                // Check if the active image is now the one we uploaded
                let active_image = image_list
                    .images
                    .iter()
                    .find(|img| img.active)
                    .ok_or_else(|| anyhow!("No active image found"))?;

                // Return both confirmed status and hash/version for verification
                let hash_hex = hex::encode(&active_image.hash);
                Ok::<_, anyhow::Error>((active_image.confirmed, hash_hex, active_image.version.clone()))
            })
            .await;

            match verify_result {
                Ok(Ok((confirmed, device_hash, _device_version))) => {
                    if confirmed {
                        // Verify by MCUboot hash
                        if device_hash.eq_ignore_ascii_case(&expected_hash_clone) {
                            info!("Device rebooted with correct firmware (hash verified)");
                            verified = true;
                            break;
                        } else {
                            warn!(
                                "Hash mismatch after update! Expected: {}, Got: {}",
                                &expected_hash_clone[..16], &device_hash[..16.min(device_hash.len())]
                            );
                        }
                    } else {
                        debug!("Device rebooted but firmware not yet confirmed (attempt {})", attempt + 1);
                    }
                }
                Ok(Err(e)) => {
                    debug!("Verification attempt {} failed: {}", attempt + 1, e);
                }
                Err(e) => {
                    debug!("Verification task failed: {}", e);
                }
            }
        }

        if !verified {
            warn!("Could not verify firmware update, but device may still be running new image");
        }

        // 7. Mark as complete
        set_state(UpdateState::Complete).await;
        info!("Firmware update completed for device {}", device_id);

        Ok(())
    }

    fn send_event(&self, device_id: &str, state: UpdateState) {
        let _ = self.event_tx.send(OtaEvent {
            device_id: device_id.to_string(),
            state,
        });
    }

    /// Upload a local firmware binary to a device (for development use)
    ///
    /// This skips the download step and uses a provided binary directly.
    /// The binary should be a valid MCUboot image (signed .bin file).
    pub async fn upload_local_firmware(
        &self,
        device_id: String,
        ip: String,
        firmware_data: Vec<u8>,
    ) -> Result<()> {
        // Check if already updating
        {
            let updates = self.active_updates.read().await;
            if let Some(info) = updates.get(&device_id) {
                if !info.state.is_terminal() {
                    return Err(anyhow!("Update already in progress for device {}", device_id));
                }
            }
        }

        // Initialize update state (skip downloading since we have the binary)
        {
            let mut updates = self.active_updates.write().await;
            updates.insert(
                device_id.clone(),
                UpdateInfo {
                    device_id: device_id.clone(),
                    ip: ip.clone(),
                    board: "local".to_string(),
                    app: "local".to_string(),
                    state: UpdateState::Uploading { progress: 0.0 },
                },
            );
        }

        self.send_event(&device_id, UpdateState::Uploading { progress: 0.0 });

        // Clone what we need for the spawned task
        let active_updates = self.active_updates.clone();
        let event_tx = self.event_tx.clone();

        // Spawn the upload task
        tokio::spawn(async move {
            let result = Self::run_local_upload(
                &active_updates,
                &event_tx,
                device_id.clone(),
                ip,
                firmware_data,
            )
            .await;

            if let Err(e) = result {
                error!("Local upload failed for device {}: {}", device_id, e);
                let mut updates = active_updates.write().await;
                if let Some(info) = updates.get_mut(&device_id) {
                    info.state = UpdateState::Failed {
                        error: e.to_string(),
                    };
                }
                let _ = event_tx.send(OtaEvent {
                    device_id: device_id.clone(),
                    state: UpdateState::Failed {
                        error: e.to_string(),
                    },
                });
            }
        });

        Ok(())
    }

    /// Run the local upload process (skips download, uses provided binary)
    async fn run_local_upload(
        active_updates: &Arc<RwLock<HashMap<String, UpdateInfo>>>,
        event_tx: &broadcast::Sender<OtaEvent>,
        device_id: String,
        ip: String,
        firmware_data: Vec<u8>,
    ) -> Result<()> {
        // Helper to check if cancelled
        let is_cancelled = || async {
            let updates = active_updates.read().await;
            updates
                .get(&device_id)
                .map(|u| matches!(u.state, UpdateState::Cancelled))
                .unwrap_or(false)
        };

        // Helper to update state
        let set_state = |state: UpdateState| async {
            let mut updates = active_updates.write().await;
            if let Some(info) = updates.get_mut(&device_id) {
                info.state = state.clone();
            }
            let _ = event_tx.send(OtaEvent {
                device_id: device_id.clone(),
                state,
            });
        };

        // Validate it's an MCUboot image
        if firmware_data.len() < 32 {
            anyhow::bail!("Binary too small to be MCUboot image");
        }
        let magic = u32::from_le_bytes([
            firmware_data[0],
            firmware_data[1],
            firmware_data[2],
            firmware_data[3],
        ]);
        if magic != 0x96f3b83d {
            anyhow::bail!(
                "Not an MCUboot image (magic=0x{:08x}, expected 0x96f3b83d)",
                magic
            );
        }

        info!(
            "Uploading {} bytes of local firmware to device {} at {}",
            firmware_data.len(),
            device_id,
            ip
        );

        if is_cancelled().await {
            return Ok(());
        }

        // 1. Write firmware to temp file for mcumgr-client
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("firmware_{}.bin", device_id));
        tokio::fs::write(&temp_file, &firmware_data).await?;

        // 2. Upload using mcumgr-client
        let temp_file_clone = temp_file.clone();
        let device_id_clone = device_id.clone();
        let event_tx_clone = event_tx.clone();
        let ip_clone = ip.clone();

        let _upload_result = tokio::task::spawn_blocking(move || {
            use mcumgr_client::{upload_image_transport, UdpSpecs, UdpTransport};

            let specs = UdpSpecs {
                host: ip_clone,
                port: MCUMGR_PORT,
                timeout_s: 10,
                mtu: 512,
            };
            let mut transport =
                UdpTransport::new(&specs).map_err(|e| anyhow!("Failed to create transport: {}", e))?;

            upload_image_transport(
                &mut transport,
                &temp_file_clone,
                0, // slot 0
                Some(|uploaded: u64, total: u64| {
                    let progress = uploaded as f32 / total as f32;
                    let _ = event_tx_clone.send(OtaEvent {
                        device_id: device_id_clone.clone(),
                        state: UpdateState::Uploading { progress },
                    });
                }),
            )?;

            Ok::<_, anyhow::Error>(())
        })
        .await??;

        // Clean up temp file
        let _ = tokio::fs::remove_file(&temp_file).await;

        if is_cancelled().await {
            return Ok(());
        }

        // 3. Confirm and reboot
        set_state(UpdateState::Confirming).await;
        info!("Confirming firmware image");

        let ip_clone = ip.clone();
        let _confirm_result = tokio::task::spawn_blocking(move || {
            use mcumgr_client::{list_transport, test_transport, UdpSpecs, UdpTransport};

            let specs = UdpSpecs {
                host: ip_clone,
                port: MCUMGR_PORT,
                timeout_s: 5,
                mtu: 1024,
            };
            let mut transport =
                UdpTransport::new(&specs).map_err(|e| anyhow!("Failed to create transport: {}", e))?;

            let image_list = list_transport(&mut transport)?;
            let pending_hash = image_list
                .images
                .iter()
                .find(|img| !img.confirmed && !img.active)
                .map(|img| img.hash.clone())
                .ok_or_else(|| anyhow!("No pending image found after upload"))?;

            test_transport(&mut transport, pending_hash, Some(false))?;

            Ok::<_, anyhow::Error>(())
        })
        .await??;

        if is_cancelled().await {
            return Ok(());
        }

        // 4. Reset device
        set_state(UpdateState::Rebooting).await;
        info!("Resetting device");

        let ip_clone = ip.clone();
        let _reset_result = tokio::task::spawn_blocking(move || {
            use mcumgr_client::{reset_transport, UdpSpecs, UdpTransport};

            let specs = UdpSpecs {
                host: ip_clone,
                port: MCUMGR_PORT,
                timeout_s: 5,
                mtu: 1024,
            };
            let mut transport =
                UdpTransport::new(&specs).map_err(|e| anyhow!("Failed to create transport: {}", e))?;

            reset_transport(&mut transport)?;

            Ok::<_, anyhow::Error>(())
        })
        .await??;

        // 5. Wait for device to come back
        set_state(UpdateState::Verifying).await;
        info!("Waiting for device to reboot...");
        tokio::time::sleep(Duration::from_secs(5)).await;

        if is_cancelled().await {
            return Ok(());
        }

        // For local uploads, we just verify the device comes back online
        // (we don't have an expected hash to compare against)
        let mut verified = false;
        for attempt in 0..10 {
            tokio::time::sleep(Duration::from_secs(2)).await;

            if is_cancelled().await {
                return Ok(());
            }

            let ip_clone = ip.clone();
            let verify_result = tokio::task::spawn_blocking(move || {
                use mcumgr_client::{list_transport, UdpSpecs, UdpTransport};

                let specs = UdpSpecs {
                    host: ip_clone,
                    port: MCUMGR_PORT,
                    timeout_s: 2,
                    mtu: 1024,
                };
                let mut transport =
                    UdpTransport::new(&specs).map_err(|e| anyhow!("Failed to create transport: {}", e))?;

                let image_list = list_transport(&mut transport)?;
                let active_image = image_list
                    .images
                    .iter()
                    .find(|img| img.active)
                    .ok_or_else(|| anyhow!("No active image found"))?;

                Ok::<_, anyhow::Error>(active_image.confirmed)
            })
            .await;

            match verify_result {
                Ok(Ok(confirmed)) => {
                    if confirmed {
                        info!("Device rebooted with new firmware (confirmed)");
                        verified = true;
                        break;
                    } else {
                        debug!(
                            "Device rebooted but firmware not yet confirmed (attempt {})",
                            attempt + 1
                        );
                    }
                }
                Ok(Err(e)) => {
                    debug!("Verification attempt {} failed: {}", attempt + 1, e);
                }
                Err(e) => {
                    debug!("Verification task failed: {}", e);
                }
            }
        }

        if !verified {
            warn!("Could not verify local firmware update, but device may still be running new image");
        }

        // 6. Mark as complete
        set_state(UpdateState::Complete).await;
        info!("Local firmware upload completed for device {}", device_id);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_state_is_terminal() {
        assert!(!UpdateState::Downloading { progress: 0.5 }.is_terminal());
        assert!(!UpdateState::Uploading { progress: 0.5 }.is_terminal());
        assert!(!UpdateState::Confirming.is_terminal());
        assert!(!UpdateState::Rebooting.is_terminal());
        assert!(!UpdateState::Verifying.is_terminal());
        assert!(UpdateState::Complete.is_terminal());
        assert!(UpdateState::Failed { error: "test".to_string() }.is_terminal());
        assert!(UpdateState::Cancelled.is_terminal());
    }
}
