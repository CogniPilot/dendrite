//! HCDF and model caching with SHA-based deduplication
//!
//! This module provides caching for remotely-fetched HCDF files and their
//! associated GLB models. Files are stored by their SHA256 hash to enable:
//! - Skipping downloads when the device-reported SHA matches a cached file
//! - Reusing unchanged models when HCDF files are updated
//! - Efficient storage with no duplicate model files
//!
//! HCDF files are stored as: `{board}/{app}/{sha}-{app}.hcdf`
//! with a symlink: `{board}/{app}/{app}.hcdf` -> `{sha}-{app}.hcdf`
//!
//! Model files are stored with SHA-prefixed names: `models/{short_sha}-{name}.glb`
//! This allows multiple versions of the same logical model to coexist and
//! enables instant cache lookups by SHA.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CacheError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("SHA mismatch: expected {expected}, got {actual}")]
    ShaMismatch { expected: String, actual: String },
    #[error("URL not in cache: {0}")]
    NotCached(String),
}

/// Cache manifest entry for a single HCDF file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedHcdf {
    /// Original URL this was fetched from
    pub url: String,
    /// SHA256 hash of the HCDF content
    pub sha: String,
    /// Board name (e.g., "mr_mcxn_t1")
    #[serde(default)]
    pub board: String,
    /// App name (e.g., "optical-flow")
    #[serde(default)]
    pub app: String,
    /// Local file path (relative to cache directory)
    pub path: String,
    /// When this was fetched (ISO 8601)
    pub fetched_at: String,
    /// Model files referenced by this HCDF, with their SHAs
    pub models: HashMap<String, CachedModel>,
}

/// Cache manifest entry for a model file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedModel {
    /// Original href from the HCDF (may be relative or absolute URL)
    pub href: String,
    /// SHA256 hash of the model content (full)
    pub sha: String,
    /// Short SHA (first 8 characters) used in filename
    pub short_sha: String,
    /// Original model name (without SHA prefix)
    pub name: String,
    /// Local file path (relative to cache directory): models/{short_sha}-{name}
    pub path: String,
}

/// The cache manifest tracks all cached HCDF files and their models
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CacheManifest {
    /// Version of the manifest format
    #[serde(default = "default_version")]
    pub version: String,
    /// HCDF entries keyed by their SHA
    pub hcdf: HashMap<String, CachedHcdf>,
    /// Model entries keyed by their SHA (for cross-HCDF deduplication)
    pub models_by_sha: HashMap<String, String>, // SHA -> relative path
    /// Index from board/app to latest HCDF SHA (for fallback lookups)
    #[serde(default)]
    pub latest_by_board_app: HashMap<String, String>, // "{board}/{app}" -> SHA
}

fn default_version() -> String {
    "1.0".to_string()
}

impl CacheManifest {
    /// Create a new empty cache manifest
    pub fn new() -> Self {
        Self {
            version: default_version(),
            hcdf: HashMap::new(),
            models_by_sha: HashMap::new(),
            latest_by_board_app: HashMap::new(),
        }
    }

    /// Load manifest from a file
    pub fn from_file(path: &Path) -> Result<Self, CacheError> {
        let content = std::fs::read_to_string(path)?;
        let manifest: CacheManifest = serde_json::from_str(&content)?;
        Ok(manifest)
    }

    /// Load manifest or create new if file doesn't exist
    pub fn load_or_create(path: &Path) -> Result<Self, CacheError> {
        if path.exists() {
            Self::from_file(path)
        } else {
            Ok(Self::new())
        }
    }

    /// Save manifest to a file
    pub fn save(&self, path: &Path) -> Result<(), CacheError> {
        let content = serde_json::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Check if we have an HCDF cached with the given SHA
    pub fn has_hcdf(&self, sha: &str) -> bool {
        self.hcdf.contains_key(sha)
    }

    /// Get the cached HCDF entry by SHA
    pub fn get_hcdf(&self, sha: &str) -> Option<&CachedHcdf> {
        self.hcdf.get(sha)
    }

    /// Check if we have a model cached with the given SHA
    pub fn has_model(&self, sha: &str) -> bool {
        self.models_by_sha.contains_key(sha)
    }

    /// Get the path to a cached model by SHA
    pub fn get_model_path(&self, sha: &str) -> Option<&str> {
        self.models_by_sha.get(sha).map(|s| s.as_str())
    }

    /// Add or update an HCDF entry
    pub fn add_hcdf(&mut self, entry: CachedHcdf) {
        // Also add all models to the global model index
        for (_, model) in &entry.models {
            if !self.models_by_sha.contains_key(&model.sha) {
                self.models_by_sha.insert(model.sha.clone(), model.path.clone());
            }
        }
        // Update board/app index to point to this (latest) version
        if !entry.board.is_empty() && !entry.app.is_empty() {
            let key = format!("{}/{}", entry.board, entry.app);
            self.latest_by_board_app.insert(key, entry.sha.clone());
        }
        self.hcdf.insert(entry.sha.clone(), entry);
    }

    /// Get the latest cached HCDF SHA for a board/app combination
    pub fn get_latest_sha(&self, board: &str, app: &str) -> Option<&str> {
        let key = format!("{}/{}", board, app);
        self.latest_by_board_app.get(&key).map(|s| s.as_str())
    }

    /// Get the latest cached HCDF entry for a board/app combination
    pub fn get_latest_hcdf(&self, board: &str, app: &str) -> Option<&CachedHcdf> {
        self.get_latest_sha(board, app)
            .and_then(|sha| self.hcdf.get(sha))
    }

    /// Get the local path for an HCDF file by its SHA
    pub fn get_hcdf_path(&self, sha: &str) -> Option<String> {
        self.hcdf.get(sha).map(|e| e.path.clone())
    }
}

/// Cache directory manager
#[derive(Debug, Clone)]
pub struct FragmentCache {
    /// Base directory for the cache
    pub base_dir: PathBuf,
    /// Path to the manifest file
    pub manifest_path: PathBuf,
    /// The cache manifest
    pub manifest: CacheManifest,
}

impl FragmentCache {
    /// Create a new fragment cache at the given directory
    pub fn new(base_dir: PathBuf) -> Result<Self, CacheError> {
        std::fs::create_dir_all(&base_dir)?;

        let manifest_path = base_dir.join("manifest.json");
        let manifest = CacheManifest::load_or_create(&manifest_path)?;

        Ok(Self {
            base_dir,
            manifest_path,
            manifest,
        })
    }

    /// Get the path where an HCDF file should be stored
    pub fn hcdf_path(&self, sha: &str) -> PathBuf {
        self.base_dir.join(format!("{}.hcdf", sha))
    }

    /// Get the models directory (flat structure for all models)
    pub fn models_dir(&self) -> PathBuf {
        self.base_dir.join("models")
    }

    /// Get the path where a model file should be stored
    /// Uses format: models/{short_sha}-{name}
    pub fn model_path(&self, sha: &str, model_name: &str) -> PathBuf {
        let short_sha = &sha[..8.min(sha.len())];
        self.models_dir().join(format!("{}-{}", short_sha, model_name))
    }

    /// Get short SHA (first 8 characters) from a full SHA
    pub fn short_sha(sha: &str) -> String {
        sha[..8.min(sha.len())].to_string()
    }

    /// Check if we have a cached HCDF with the given SHA
    pub fn has_hcdf(&self, sha: &str) -> bool {
        if let Some(entry) = self.manifest.get_hcdf(sha) {
            self.base_dir.join(&entry.path).exists()
        } else {
            false
        }
    }

    /// Check if we have a cached model with the given SHA
    pub fn has_model(&self, sha: &str) -> bool {
        if let Some(path) = self.manifest.get_model_path(sha) {
            self.base_dir.join(path).exists()
        } else {
            false
        }
    }

    /// Store an HCDF file in the cache
    ///
    /// Files are stored as: `{board}/{app}/{short_sha}-{app}.hcdf`
    /// with a symlink: `{board}/{app}/{app}.hcdf` -> `{short_sha}-{app}.hcdf`
    pub fn store_hcdf(
        &mut self,
        url: &str,
        sha: &str,
        board: &str,
        app: &str,
        content: &[u8],
    ) -> Result<PathBuf, CacheError> {
        let short_sha = Self::short_sha(sha);

        // Create directory structure: {board}/{app}/
        let dir = self.base_dir.join(board).join(app);
        std::fs::create_dir_all(&dir)?;

        // Store with SHA-prefixed name
        let sha_filename = format!("{}-{}.hcdf", short_sha, app);
        let path = dir.join(&sha_filename);
        std::fs::write(&path, content)?;

        // Create/update symlink for latest version
        let symlink_name = format!("{}.hcdf", app);
        let symlink_path = dir.join(&symlink_name);

        // Remove existing symlink if present
        if symlink_path.exists() || symlink_path.is_symlink() {
            let _ = std::fs::remove_file(&symlink_path);
        }

        // Create symlink (relative, just the filename)
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let _ = symlink(&sha_filename, &symlink_path);
        }

        let relative_path = format!("{}/{}/{}", board, app, sha_filename);
        let entry = CachedHcdf {
            url: url.to_string(),
            sha: sha.to_string(),
            board: board.to_string(),
            app: app.to_string(),
            path: relative_path,
            fetched_at: chrono::Utc::now().to_rfc3339(),
            models: HashMap::new(),
        };

        self.manifest.add_hcdf(entry);
        self.manifest.save(&self.manifest_path)?;

        Ok(path)
    }

    /// Store a model file in the cache
    /// If model_name already has a SHA prefix (8 hex chars followed by dash), use as-is
    /// Otherwise store as: models/{short_sha}-{name}
    pub fn store_model(
        &mut self,
        hcdf_sha: &str,
        model_name: &str,
        model_sha: &str,
        href: &str,
        content: &[u8],
    ) -> Result<PathBuf, CacheError> {
        // Create flat models directory
        let models_dir = self.models_dir();
        std::fs::create_dir_all(&models_dir)?;

        let short_sha = Self::short_sha(model_sha);

        // Check if filename already has a SHA prefix (8 hex chars + dash)
        let has_sha_prefix = model_name.len() > 9
            && model_name.chars().nth(8) == Some('-')
            && model_name[..8].chars().all(|c| c.is_ascii_hexdigit());

        // Use original name if already SHA-prefixed, otherwise add SHA prefix
        let cached_name = if has_sha_prefix {
            model_name.to_string()
        } else {
            format!("{}-{}", short_sha, model_name)
        };

        let path = models_dir.join(&cached_name);
        std::fs::write(&path, content)?;

        let relative_path = format!("models/{}", cached_name);

        // Add to global model index
        self.manifest.models_by_sha.insert(model_sha.to_string(), relative_path.clone());

        // Add to the HCDF's model list
        if let Some(hcdf_entry) = self.manifest.hcdf.get_mut(hcdf_sha) {
            hcdf_entry.models.insert(
                model_name.to_string(),
                CachedModel {
                    href: href.to_string(),
                    sha: model_sha.to_string(),
                    short_sha: short_sha.clone(),
                    name: model_name.to_string(),
                    path: relative_path,
                },
            );
        }

        self.manifest.save(&self.manifest_path)?;

        Ok(path)
    }

    /// Get the absolute path to a cached model by its SHA
    pub fn get_cached_model_path(&self, sha: &str) -> Option<PathBuf> {
        self.manifest
            .get_model_path(sha)
            .map(|p| self.base_dir.join(p))
    }

    /// Get the absolute path to a cached HCDF by its SHA
    pub fn get_cached_hcdf_path(&self, sha: &str) -> Option<PathBuf> {
        self.manifest
            .get_hcdf_path(sha)
            .map(|p| self.base_dir.join(p))
    }

    /// Read a cached HCDF file content by SHA
    pub fn read_hcdf(&self, sha: &str) -> Result<String, CacheError> {
        let path = self.get_cached_hcdf_path(sha)
            .ok_or_else(|| CacheError::NotCached(sha.to_string()))?;
        Ok(std::fs::read_to_string(path)?)
    }

    /// Read the latest cached HCDF for a board/app (follows symlink)
    pub fn read_hcdf_by_board_app(&self, board: &str, app: &str) -> Result<String, CacheError> {
        // First try the manifest index
        if let Some(sha) = self.manifest.get_latest_sha(board, app) {
            if let Ok(content) = self.read_hcdf(sha) {
                return Ok(content);
            }
        }

        // Fallback: try reading via symlink directly
        let symlink_path = self.base_dir.join(board).join(app).join(format!("{}.hcdf", app));
        if symlink_path.exists() {
            return Ok(std::fs::read_to_string(symlink_path)?);
        }

        Err(CacheError::NotCached(format!("{}/{}", board, app)))
    }

    /// Check if we have any cached HCDF for a board/app
    pub fn has_hcdf_for_board_app(&self, board: &str, app: &str) -> bool {
        // Check manifest index
        if self.manifest.get_latest_sha(board, app).is_some() {
            return true;
        }
        // Check for symlink
        let symlink_path = self.base_dir.join(board).join(app).join(format!("{}.hcdf", app));
        symlink_path.exists()
    }

    /// Get the latest HCDF entry for a board/app
    pub fn get_latest_hcdf(&self, board: &str, app: &str) -> Option<&CachedHcdf> {
        self.manifest.get_latest_hcdf(board, app)
    }
}

/// Compute SHA256 hash of data and return as hex string
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    hex::encode(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_cache_manifest() {
        let mut manifest = CacheManifest::new();

        let entry = CachedHcdf {
            url: "https://hcdf.cognipilot.org/spinali/v1.0.hcdf".to_string(),
            sha: "abc123".to_string(),
            board: "spinali".to_string(),
            app: "default".to_string(),
            path: "abc123.hcdf".to_string(),
            fetched_at: "2026-01-10T12:00:00Z".to_string(),
            models: HashMap::new(),
        };

        manifest.add_hcdf(entry);

        assert!(manifest.has_hcdf("abc123"));
        assert!(!manifest.has_hcdf("xyz789"));
    }

    #[test]
    fn test_fragment_cache() {
        let temp_dir = TempDir::new().unwrap();
        let mut cache = FragmentCache::new(temp_dir.path().to_path_buf()).unwrap();

        // Store an HCDF
        let content = b"<hcdf>test</hcdf>";
        let sha = sha256_hex(content);
        cache.store_hcdf("https://example.com/test.hcdf", &sha, "test_board", "test_app", content).unwrap();

        assert!(cache.has_hcdf(&sha));

        // Read it back by SHA
        let read_content = cache.read_hcdf(&sha).unwrap();
        assert_eq!(read_content, "<hcdf>test</hcdf>");

        // Read it back by board/app
        let read_content2 = cache.read_hcdf_by_board_app("test_board", "test_app").unwrap();
        assert_eq!(read_content2, "<hcdf>test</hcdf>");
    }

    #[test]
    fn test_sha256() {
        let data = b"hello world";
        let hash = sha256_hex(data);
        assert_eq!(hash, "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9");
    }
}
