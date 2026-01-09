//! HCDF Fragment Database - Maps board/app combinations to device templates
//!
//! Fragments provide default model paths, descriptions, and other metadata
//! for discovered devices based on their board type and running application.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FragmentError {
    #[error("Failed to read fragment index: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Failed to parse fragment index: {0}")]
    ParseError(#[from] toml::de::Error),
    #[error("Failed to serialize fragment index: {0}")]
    SerializeError(#[from] toml::ser::Error),
    #[error("No matching fragment found for board={0}, app={1}")]
    NoMatch(String, String),
}

/// A single fragment entry in the database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fragment {
    /// Board type to match (e.g., "mr_mcxn_t1")
    pub board: String,
    /// Application name to match (e.g., "optical-flow"), or "*" for wildcard
    #[serde(default = "default_wildcard")]
    pub app: String,
    /// Path to 3D model file (relative to models directory)
    pub model: String,
    /// Human-readable description
    #[serde(default)]
    pub description: Option<String>,
    /// Default mass in kg
    #[serde(default)]
    pub mass: Option<f64>,
    /// Match priority (higher = preferred when multiple matches)
    #[serde(default)]
    pub priority: i32,
}

fn default_wildcard() -> String {
    "*".to_string()
}

/// The fragment database index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentIndex {
    /// Version of the fragment index format
    #[serde(default = "default_version")]
    pub version: String,
    /// List of fragment entries
    #[serde(default)]
    pub fragment: Vec<Fragment>,
}

fn default_version() -> String {
    "1.0".to_string()
}

impl Default for FragmentIndex {
    fn default() -> Self {
        Self {
            version: default_version(),
            fragment: Vec::new(),
        }
    }
}

impl FragmentIndex {
    /// Load fragment index from a TOML file
    pub fn from_file(path: &Path) -> Result<Self, FragmentError> {
        let content = std::fs::read_to_string(path)?;
        let index: FragmentIndex = toml::from_str(&content)?;
        Ok(index)
    }

    /// Load fragment index from a TOML string
    pub fn from_toml(content: &str) -> Result<Self, FragmentError> {
        let index: FragmentIndex = toml::from_str(content)?;
        Ok(index)
    }

    /// Find the best matching fragment for a board/app combination
    ///
    /// Matching priority:
    /// 1. Exact board + exact app match (highest priority value wins)
    /// 2. Exact board + wildcard app
    /// 3. No match
    pub fn find_match(&self, board: &str, app: &str) -> Option<&Fragment> {
        let mut best_match: Option<&Fragment> = None;
        let mut best_score = i32::MIN;

        for fragment in &self.fragment {
            // Check board match (case-insensitive)
            if !fragment.board.eq_ignore_ascii_case(board) {
                continue;
            }

            // Calculate match score
            let is_exact_app = fragment.app.eq_ignore_ascii_case(app);
            let is_wildcard = fragment.app == "*";

            if !is_exact_app && !is_wildcard {
                continue;
            }

            // Exact app matches get +1000 bonus over wildcards
            let score = fragment.priority + if is_exact_app { 1000 } else { 0 };

            if score > best_score {
                best_score = score;
                best_match = Some(fragment);
            }
        }

        best_match
    }

    /// Get the model path for a board/app combination
    pub fn get_model(&self, board: &str, app: &str) -> Option<String> {
        self.find_match(board, app).map(|f| f.model.clone())
    }

    /// Add a new fragment entry
    pub fn add(&mut self, fragment: Fragment) {
        self.fragment.push(fragment);
    }

    /// Save the index to a TOML file
    pub fn to_file(&self, path: &Path) -> Result<(), FragmentError> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

/// Fragment database with in-memory cache
#[derive(Debug, Clone)]
pub struct FragmentDatabase {
    index: FragmentIndex,
    /// Cache of board+app -> model path lookups
    cache: HashMap<(String, String), Option<String>>,
}

impl FragmentDatabase {
    /// Create a new fragment database from an index
    pub fn new(index: FragmentIndex) -> Self {
        Self {
            index,
            cache: HashMap::new(),
        }
    }

    /// Load from a file
    pub fn from_file(path: &Path) -> Result<Self, FragmentError> {
        let index = FragmentIndex::from_file(path)?;
        Ok(Self::new(index))
    }

    /// Create an empty database
    pub fn empty() -> Self {
        Self::new(FragmentIndex::default())
    }

    /// Get the model path for a device, using cache
    pub fn get_model(&mut self, board: &str, app: &str) -> Option<String> {
        let key = (board.to_lowercase(), app.to_lowercase());

        if let Some(cached) = self.cache.get(&key) {
            return cached.clone();
        }

        let result = self.index.get_model(board, app);
        self.cache.insert(key, result.clone());
        result
    }

    /// Find the full fragment for a device
    pub fn find_fragment(&self, board: &str, app: &str) -> Option<&Fragment> {
        self.index.find_match(board, app)
    }

    /// Get the underlying index
    pub fn index(&self) -> &FragmentIndex {
        &self.index
    }

    /// Clear the lookup cache
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Reload the index from a file
    pub fn reload(&mut self, path: &Path) -> Result<(), FragmentError> {
        self.index = FragmentIndex::from_file(path)?;
        self.cache.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fragment_matching() {
        let toml = r#"
version = "1.0"

[[fragment]]
board = "mr_mcxn_t1"
app = "optical-flow"
model = "optical_flow.glb"
description = "Optical flow sensor"
priority = 10

[[fragment]]
board = "mr_mcxn_t1"
app = "*"
model = "mcnt1hub.glb"
description = "Default MR-MCXN-T1 model"
priority = 0

[[fragment]]
board = "navq95"
app = "*"
model = "navq95.glb"
"#;

        let index = FragmentIndex::from_toml(toml).unwrap();

        // Exact match should win
        let m = index.find_match("mr_mcxn_t1", "optical-flow").unwrap();
        assert_eq!(m.model, "optical_flow.glb");

        // Wildcard should match unknown app
        let m = index.find_match("mr_mcxn_t1", "unknown-app").unwrap();
        assert_eq!(m.model, "mcnt1hub.glb");

        // Different board
        let m = index.find_match("navq95", "anything").unwrap();
        assert_eq!(m.model, "navq95.glb");

        // No match for unknown board
        assert!(index.find_match("unknown_board", "app").is_none());
    }

    #[test]
    fn test_case_insensitive() {
        let toml = r#"
[[fragment]]
board = "MR_MCXN_T1"
app = "Optical-Flow"
model = "optical_flow.glb"
"#;

        let index = FragmentIndex::from_toml(toml).unwrap();

        // Should match regardless of case
        assert!(index.find_match("mr_mcxn_t1", "optical-flow").is_some());
        assert!(index.find_match("MR_MCXN_T1", "OPTICAL-FLOW").is_some());
    }

    #[test]
    fn test_database_cache() {
        let toml = r#"
[[fragment]]
board = "test"
app = "*"
model = "test.glb"
"#;

        let index = FragmentIndex::from_toml(toml).unwrap();
        let mut db = FragmentDatabase::new(index);

        // First lookup populates cache
        let result1 = db.get_model("test", "app1");
        assert_eq!(result1, Some("test.glb".to_string()));

        // Second lookup uses cache
        let result2 = db.get_model("test", "app1");
        assert_eq!(result2, Some("test.glb".to_string()));
    }
}
