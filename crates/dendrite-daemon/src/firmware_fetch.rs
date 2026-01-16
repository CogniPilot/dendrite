//! Remote firmware manifest fetching with caching
//!
//! This module handles:
//! 1. Fetching firmware manifests from explicitly configured URIs
//! 2. Caching manifests with TTL to avoid excessive network requests
//! 3. Downloading firmware binaries for OTA updates
//!
//! Note: There is no default firmware URL. Each device must have
//! `firmware_manifest_uri` set in its `<software>` element in HCDF.

use anyhow::{Context, Result};
use dendrite_core::{FirmwareManifest, FirmwareRelease};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Cache TTL for firmware manifests (5 minutes)
const MANIFEST_CACHE_TTL: Duration = Duration::from_secs(300);

/// Cached manifest entry
struct CachedManifest {
    manifest: FirmwareManifest,
    fetched_at: Instant,
}

/// Firmware manifest fetcher with in-memory caching
pub struct FirmwareFetcher {
    /// HTTP client
    client: reqwest::Client,
    /// Manifest cache: (board, app) -> cached manifest
    cache: Arc<RwLock<HashMap<(String, String), CachedManifest>>>,
}

impl FirmwareFetcher {
    /// Create a new firmware fetcher
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self {
            client,
            cache: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Construct manifest URL from a base URI
    ///
    /// If firmware_manifest_uri is "https://firmware.cognipilot.org/spinali/cerebri",
    /// returns "https://firmware.cognipilot.org/spinali/cerebri/latest.json"
    pub fn construct_manifest_url(firmware_manifest_uri: &str) -> String {
        let base = firmware_manifest_uri.trim_end_matches('/');
        format!("{}/latest.json", base)
    }

    /// Get firmware manifest for a device
    ///
    /// Requires an explicit firmware_manifest_uri - there is no default fallback.
    /// Returns None if no URI is provided.
    ///
    /// Uses in-memory cache with TTL to avoid excessive network requests.
    pub async fn get_manifest(
        &self,
        board: &str,
        app: &str,
        firmware_manifest_uri: Option<&str>,
    ) -> Result<Option<FirmwareManifest>> {
        // Require explicit URI - no default fallback
        let uri = match firmware_manifest_uri {
            Some(uri) => uri,
            None => {
                debug!(
                    board = %board,
                    app = %app,
                    "No firmware_manifest_uri configured, skipping firmware check"
                );
                return Ok(None);
            }
        };

        let key = (board.to_string(), app.to_string());

        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.get(&key) {
                if cached.fetched_at.elapsed() < MANIFEST_CACHE_TTL {
                    debug!(
                        board = %board,
                        app = %app,
                        age_secs = cached.fetched_at.elapsed().as_secs(),
                        "Using cached firmware manifest"
                    );
                    return Ok(Some(cached.manifest.clone()));
                }
            }
        }

        // Fetch from remote using explicit URI
        let url = Self::construct_manifest_url(uri);
        debug!(url = %url, "Fetching firmware manifest");

        let response = match self.client.get(&url).send().await {
            Ok(resp) => resp,
            Err(e) => {
                warn!(url = %url, error = %e, "Failed to fetch firmware manifest");
                return Ok(None);
            }
        };

        if !response.status().is_success() {
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                debug!(
                    board = %board,
                    app = %app,
                    "No firmware manifest available (404)"
                );
            } else {
                warn!(
                    url = %url,
                    status = %response.status(),
                    "Firmware manifest fetch failed"
                );
            }
            return Ok(None);
        }

        let manifest: FirmwareManifest = match response.json().await {
            Ok(m) => m,
            Err(e) => {
                warn!(url = %url, error = %e, "Failed to parse firmware manifest");
                return Ok(None);
            }
        };

        info!(
            board = %board,
            app = %app,
            version = %manifest.latest.version,
            "Fetched firmware manifest"
        );

        // Update cache
        {
            let mut cache = self.cache.write().await;
            cache.insert(key, CachedManifest {
                manifest: manifest.clone(),
                fetched_at: Instant::now(),
            });
        }

        Ok(Some(manifest))
    }

    /// Download firmware binary from a release URL
    ///
    /// Returns the binary data after verifying size and MCUboot hash.
    pub async fn download_firmware(&self, release: &FirmwareRelease) -> Result<Vec<u8>> {
        info!(
            version = %release.version,
            url = %release.url,
            size = release.size,
            "Downloading firmware binary"
        );

        let response = self.client.get(&release.url).send().await
            .context("Failed to download firmware")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Firmware download failed with status {}",
                response.status()
            );
        }

        let data = response.bytes().await
            .context("Failed to read firmware response body")?
            .to_vec();

        // Verify size
        if data.len() as u64 != release.size {
            anyhow::bail!(
                "Firmware size mismatch: expected {} bytes, got {} bytes",
                release.size,
                data.len()
            );
        }

        // Verify MCUboot hash (same hash used for post-update verification)
        let computed_hash = compute_mcuboot_hash(&data)?;
        if !computed_hash.eq_ignore_ascii_case(&release.mcuboot_hash) {
            anyhow::bail!(
                "Firmware MCUboot hash mismatch: expected {}, got {}",
                release.mcuboot_hash,
                computed_hash
            );
        }

        info!(
            version = %release.version,
            size = data.len(),
            mcuboot_hash = %&computed_hash[..16],
            "Firmware download verified"
        );

        Ok(data)
    }

    /// Clear the manifest cache
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
        info!("Cleared firmware manifest cache");
    }

    /// Get cache statistics
    pub async fn cache_stats(&self) -> usize {
        self.cache.read().await.len()
    }
}

/// Compute MCUboot image hash from binary data
///
/// MCUboot computes the image hash as SHA256 over:
/// - Image header (first hdr_size bytes, typically 32)
/// - Protected TLVs (protect_tlv_size bytes)
/// - Image payload (img_size bytes)
///
/// This excludes the trailing TLV area with signature.
fn compute_mcuboot_hash(data: &[u8]) -> Result<String> {
    use sha2::{Sha256, Digest};

    // MCUboot image header structure (first 32 bytes):
    // struct image_header {
    //     uint32_t ih_magic;           // 0x96f3b83d
    //     uint32_t ih_load_addr;
    //     uint16_t ih_hdr_size;        // Header size
    //     uint16_t ih_protect_tlv_size; // Protected TLV area size
    //     uint32_t ih_img_size;        // Image payload size
    //     uint32_t ih_flags;
    //     struct image_version ih_ver; // 8 bytes
    //     uint32_t _pad1;
    // };

    if data.len() < 32 {
        anyhow::bail!("Binary too small to be MCUboot image ({} bytes)", data.len());
    }

    let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if magic != 0x96f3b83d {
        anyhow::bail!("Not an MCUboot image (magic=0x{:08x}, expected 0x96f3b83d)", magic);
    }

    let hdr_size = u16::from_le_bytes([data[8], data[9]]) as usize;
    let protect_tlv_size = u16::from_le_bytes([data[10], data[11]]) as usize;
    let img_size = u32::from_le_bytes([data[12], data[13], data[14], data[15]]) as usize;

    // Hash covers: header + protected TLVs + payload
    let hash_size = hdr_size + protect_tlv_size + img_size;

    if data.len() < hash_size {
        anyhow::bail!(
            "Binary truncated: need {} bytes for hash, have {} bytes",
            hash_size,
            data.len()
        );
    }

    let mut hasher = Sha256::new();
    hasher.update(&data[..hash_size]);
    let result = hasher.finalize();

    Ok(hex::encode(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_construct_manifest_url() {
        // Standard case
        assert_eq!(
            FirmwareFetcher::construct_manifest_url("https://firmware.cognipilot.org/spinali/cerebri"),
            "https://firmware.cognipilot.org/spinali/cerebri/latest.json"
        );

        // With trailing slash
        assert_eq!(
            FirmwareFetcher::construct_manifest_url("https://firmware.cognipilot.org/spinali/cerebri/"),
            "https://firmware.cognipilot.org/spinali/cerebri/latest.json"
        );

        // Custom domain
        assert_eq!(
            FirmwareFetcher::construct_manifest_url("https://custom.example.com/firmware/myboard/myapp"),
            "https://custom.example.com/firmware/myboard/myapp/latest.json"
        );
    }

    #[test]
    fn test_compute_mcuboot_hash_invalid_magic() {
        let data = vec![0u8; 100]; // All zeros, wrong magic
        let result = compute_mcuboot_hash(&data);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Not an MCUboot image"));
    }

    #[test]
    fn test_compute_mcuboot_hash_too_small() {
        let data = vec![0u8; 16]; // Too small
        let result = compute_mcuboot_hash(&data);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too small"));
    }
}
