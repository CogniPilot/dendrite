//! Remote HCDF fragment fetching with SHA-based caching
//!
//! This module handles:
//! 1. Querying devices for their HCDF URL + SHA via MCUmgr
//! 2. Constructing fallback URLs from board/app names
//! 3. Fetching and caching remote HCDF files
//! 4. Fetching and caching GLB model files with SHA verification
//! 5. SHA verification to avoid re-downloading unchanged files

use anyhow::{Context, Result};
use dendrite_core::{FragmentCache, sha256_hex};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Base URL for the HCDF models repository
pub const HCDF_BASE_URL: &str = "https://hcdf.cognipilot.org";

/// HCDF fetcher with caching
pub struct HcdfFetcher {
    /// HTTP client
    client: reqwest::Client,
    /// Fragment cache for HCDF files and models
    cache: Arc<RwLock<FragmentCache>>,
}

impl HcdfFetcher {
    /// Create a new fetcher with the given cache directory
    pub fn new(cache_dir: PathBuf) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        let cache = FragmentCache::new(cache_dir)
            .context("Failed to create fragment cache")?;

        Ok(Self {
            client,
            cache: Arc::new(RwLock::new(cache)),
        })
    }

    /// Construct the HCDF URL from board and app names
    ///
    /// URL pattern: https://hcdf.cognipilot.org/{board}/{app}/{app}.hcdf
    pub fn construct_url(board: &str, app: &str) -> String {
        format!("{}/{}/{}/{}.hcdf", HCDF_BASE_URL, board, app, app)
    }

    /// Get the cache directory path
    pub async fn cache_dir(&self) -> PathBuf {
        self.cache.read().await.base_dir.clone()
    }

    /// Get the models directory path (for serving via HTTP)
    pub async fn models_dir(&self) -> PathBuf {
        self.cache.read().await.models_dir()
    }

    /// Fetch HCDF for a device, using cache when possible
    ///
    /// # Arguments
    /// * `board` - Board name (e.g., "mr_mcxn_t1")
    /// * `app` - Application name (e.g., "optical-flow")
    /// * `device_url` - URL reported by device via MCUmgr (optional)
    /// * `device_sha` - SHA reported by device via MCUmgr (optional)
    ///
    /// # Returns
    /// The HCDF XML content as a string, or None if fetch failed
    pub async fn fetch_hcdf(
        &self,
        board: &str,
        app: &str,
        device_url: Option<&str>,
        device_sha: Option<&str>,
    ) -> Result<Option<String>> {
        // Check if we have a cached version matching the device SHA
        if let Some(sha) = device_sha {
            let cache = self.cache.read().await;
            if cache.has_hcdf(sha) {
                info!(
                    sha = %sha,
                    "Using cached HCDF (SHA match)"
                );
                match cache.read_hcdf(sha) {
                    Ok(content) => return Ok(Some(content)),
                    Err(e) => {
                        warn!(sha = %sha, error = %e, "Failed to read cached HCDF");
                        // Fall through to fetch
                    }
                }
            }
        }

        // Determine URL to fetch from
        let url = device_url
            .map(|u| u.to_string())
            .unwrap_or_else(|| Self::construct_url(board, app));

        info!(url = %url, board = %board, app = %app, "Fetching remote HCDF");

        // Fetch the HCDF file
        let response = match self.client.get(&url).send().await {
            Ok(resp) => resp,
            Err(e) => {
                warn!(url = %url, error = %e, "Failed to fetch HCDF, trying cache fallback");
                // Try cache fallback
                let cache = self.cache.read().await;
                if let Ok(content) = cache.read_hcdf_by_board_app(board, app) {
                    info!(board = %board, app = %app, "Using cached HCDF fallback (offline)");
                    return Ok(Some(content));
                }
                return Ok(None);
            }
        };

        if !response.status().is_success() {
            warn!(
                url = %url,
                status = %response.status(),
                "HCDF fetch returned non-success status, trying cache fallback"
            );
            // Try cache fallback
            let cache = self.cache.read().await;
            if let Ok(content) = cache.read_hcdf_by_board_app(board, app) {
                info!(board = %board, app = %app, "Using cached HCDF fallback (server error)");
                return Ok(Some(content));
            }
            return Ok(None);
        }

        let content = response.text().await
            .context("Failed to read HCDF response body")?;

        // Compute SHA256 of the content
        let computed_sha = sha256_hex(content.as_bytes());
        let short_sha = &computed_sha[..8];

        // Verify SHA if device provided one
        if let Some(expected_sha) = device_sha {
            if !computed_sha.starts_with(expected_sha) && !expected_sha.starts_with(short_sha) {
                warn!(
                    expected = %expected_sha,
                    computed = %computed_sha,
                    "HCDF SHA mismatch - content may have changed"
                );
                // Continue anyway, but log the mismatch
            }
        }

        // Cache the content
        {
            let mut cache = self.cache.write().await;
            match cache.store_hcdf(&url, &computed_sha, board, app, content.as_bytes()) {
                Ok(path) => {
                    info!(
                        url = %url,
                        sha = %short_sha,
                        board = %board,
                        app = %app,
                        path = %path.display(),
                        "Cached remote HCDF"
                    );
                }
                Err(e) => {
                    warn!(error = %e, "Failed to cache HCDF");
                }
            }
        }

        Ok(Some(content))
    }

    /// Fetch and cache a model file, returning the local path
    ///
    /// # Arguments
    /// * `model_url` - Full URL to the model file
    /// * `expected_sha` - Expected SHA256 hash (optional, from HCDF `<model sha="...">`)
    /// * `hcdf_sha` - SHA of the parent HCDF (for linking in cache manifest)
    ///
    /// # Returns
    /// The relative path to the cached model (e.g., "models/fbf4836d-name.glb")
    pub async fn fetch_model(
        &self,
        model_url: &str,
        expected_sha: Option<&str>,
        hcdf_sha: &str,
    ) -> Result<Option<String>> {
        // Extract model name from URL
        let model_name = model_url
            .rsplit('/')
            .next()
            .unwrap_or("model.glb");

        // If we have an expected SHA, check cache first
        if let Some(sha) = expected_sha {
            let cache = self.cache.read().await;
            if cache.has_model(sha) {
                if let Some(path) = cache.manifest.get_model_path(sha) {
                    info!(
                        model = %model_name,
                        sha = %&sha[..8.min(sha.len())],
                        "Using cached model (SHA match)"
                    );
                    return Ok(Some(path.to_string()));
                }
            }
        }

        info!(url = %model_url, model = %model_name, "Fetching remote model");

        // Fetch the model file
        let response = match self.client.get(model_url).send().await {
            Ok(resp) => resp,
            Err(e) => {
                warn!(url = %model_url, error = %e, "Failed to fetch model");
                return Ok(None);
            }
        };

        if !response.status().is_success() {
            warn!(
                url = %model_url,
                status = %response.status(),
                "Model fetch returned non-success status"
            );
            return Ok(None);
        }

        let content = response.bytes().await
            .context("Failed to read model response body")?;

        // Compute SHA256 of the content
        let computed_sha = sha256_hex(&content);
        let short_sha = &computed_sha[..8];

        // Verify SHA if expected
        if let Some(expected) = expected_sha {
            // Compare using prefix matching (expected might be truncated)
            let expected_prefix = &expected[..expected.len().min(8)];
            if !computed_sha.starts_with(expected_prefix) && !expected.starts_with(short_sha) {
                warn!(
                    model = %model_name,
                    expected = %expected,
                    computed = %computed_sha,
                    "Model SHA mismatch - content may have changed"
                );
                // Continue anyway
            }
        }

        // Store in cache
        let relative_path = {
            let mut cache = self.cache.write().await;

            // Check again if model exists (another thread might have cached it)
            if cache.has_model(&computed_sha) {
                if let Some(path) = cache.manifest.get_model_path(&computed_sha) {
                    debug!(
                        model = %model_name,
                        sha = %short_sha,
                        "Model already cached by another request"
                    );
                    return Ok(Some(path.to_string()));
                }
            }

            match cache.store_model(hcdf_sha, model_name, &computed_sha, model_url, &content) {
                Ok(path) => {
                    // Get the actual filename that was stored
                    let cached_name = path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(model_name);
                    info!(
                        model = %model_name,
                        sha = %short_sha,
                        path = %path.display(),
                        "Cached remote model"
                    );
                    // Return relative path for use in model_path
                    format!("models/{}", cached_name)
                }
                Err(e) => {
                    warn!(error = %e, "Failed to cache model");
                    return Ok(None);
                }
            }
        };

        Ok(Some(relative_path))
    }

    /// Fetch HCDF using only board/app (fallback URL construction)
    pub async fn fetch_hcdf_by_board_app(
        &self,
        board: &str,
        app: &str,
    ) -> Result<Option<String>> {
        self.fetch_hcdf(board, app, None, None).await
    }

    /// Get cache statistics
    pub async fn cache_stats(&self) -> (usize, usize, PathBuf) {
        let cache = self.cache.read().await;
        let hcdf_count = cache.manifest.hcdf.len();
        let model_count = cache.manifest.models_by_sha.len();
        (hcdf_count, model_count, cache.base_dir.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_construct_url() {
        assert_eq!(
            HcdfFetcher::construct_url("mr_mcxn_t1", "optical-flow"),
            "https://hcdf.cognipilot.org/mr_mcxn_t1/optical-flow/optical-flow.hcdf"
        );
    }
}
