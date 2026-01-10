//! HCDF Fragment Database - Maps board/app combinations to device templates
//!
//! Fragments provide composite visuals (multiple 3D models), reference frames,
//! descriptions, and other metadata for discovered devices based on their
//! board type and running application.
//!
//! The fragment system uses a TOML index that references HCDF XML files
//! containing the full fragment definitions with visuals and frames.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::hcdf::{Comp, Frame, Hcdf, Visual};

#[derive(Error, Debug)]
pub enum FragmentError {
    #[error("Failed to read fragment index: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Failed to parse fragment index: {0}")]
    ParseError(#[from] toml::de::Error),
    #[error("Failed to serialize fragment index: {0}")]
    SerializeError(#[from] toml::ser::Error),
    #[error("Failed to parse HCDF file: {0}")]
    HcdfError(#[from] crate::hcdf::HcdfError),
    #[error("No matching fragment found for board={0}, app={1}")]
    NoMatch(String, String),
    #[error("HCDF file has no comp element: {0}")]
    NoComp(String),
}

/// A fragment index entry - maps board/app to an HCDF file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentIndexEntry {
    /// Board type to match (e.g., "mr_mcxn_t1")
    pub board: String,
    /// Application name to match (e.g., "optical-flow"), or "*" for wildcard
    #[serde(default = "default_wildcard")]
    pub app: String,
    /// Path to HCDF file containing the fragment definition
    pub hcdf: String,
}

/// A loaded fragment with all its visuals and frames
#[derive(Debug, Clone)]
pub struct Fragment {
    /// Board type this fragment matches
    pub board: String,
    /// Application name this fragment matches ("*" for wildcard)
    pub app: String,
    /// Component name from the HCDF file
    pub name: String,
    /// Human-readable description
    pub description: Option<String>,
    /// Default mass in kg
    pub mass: Option<f64>,
    /// Multiple visual elements with individual poses
    pub visuals: Vec<Visual>,
    /// Reference frames for this component
    pub frames: Vec<Frame>,
    /// Path to the source HCDF file
    pub hcdf_path: PathBuf,
}

fn default_wildcard() -> String {
    "*".to_string()
}

/// The fragment database index (TOML format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentIndex {
    /// Version of the fragment index format
    #[serde(default = "default_version")]
    pub version: String,
    /// List of fragment entries mapping board/app to HCDF files
    #[serde(default)]
    pub fragment: Vec<FragmentIndexEntry>,
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

    /// Find the best matching index entry for a board/app combination
    ///
    /// Matching rules (simplified):
    /// 1. Exact board + exact app match takes precedence
    /// 2. Exact board + wildcard app as fallback
    /// 3. No match
    pub fn find_entry(&self, board: &str, app: &str) -> Option<&FragmentIndexEntry> {
        let mut exact_match: Option<&FragmentIndexEntry> = None;
        let mut wildcard_match: Option<&FragmentIndexEntry> = None;

        for entry in &self.fragment {
            // Check board match (case-insensitive)
            if !entry.board.eq_ignore_ascii_case(board) {
                continue;
            }

            let is_exact_app = entry.app.eq_ignore_ascii_case(app);
            let is_wildcard = entry.app == "*";

            if is_exact_app {
                exact_match = Some(entry);
                break; // Exact match found, stop searching
            } else if is_wildcard && wildcard_match.is_none() {
                wildcard_match = Some(entry);
            }
        }

        exact_match.or(wildcard_match)
    }

    /// Add a new fragment entry
    pub fn add(&mut self, entry: FragmentIndexEntry) {
        self.fragment.push(entry);
    }

    /// Save the index to a TOML file
    pub fn to_file(&self, path: &Path) -> Result<(), FragmentError> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

/// Fragment database with loaded HCDF fragments
#[derive(Debug, Clone)]
pub struct FragmentDatabase {
    /// The TOML index mapping board/app to HCDF files
    index: FragmentIndex,
    /// Base directory for resolving relative HCDF paths
    base_dir: PathBuf,
    /// Loaded fragments keyed by HCDF path
    fragments: HashMap<PathBuf, Fragment>,
    /// Cache of board+app -> fragment lookup
    lookup_cache: HashMap<(String, String), Option<PathBuf>>,
}

impl FragmentDatabase {
    /// Create a new fragment database from an index
    pub fn new(index: FragmentIndex, base_dir: PathBuf) -> Self {
        Self {
            index,
            base_dir,
            fragments: HashMap::new(),
            lookup_cache: HashMap::new(),
        }
    }

    /// Load from an index file and its directory
    pub fn from_file(path: &Path) -> Result<Self, FragmentError> {
        let index = FragmentIndex::from_file(path)?;
        let base_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let mut db = Self::new(index, base_dir);
        db.load_all_fragments()?;
        Ok(db)
    }

    /// Create an empty database
    pub fn empty() -> Self {
        Self::new(FragmentIndex::default(), PathBuf::new())
    }

    /// Load all HCDF fragments referenced by the index
    pub fn load_all_fragments(&mut self) -> Result<(), FragmentError> {
        for entry in &self.index.fragment.clone() {
            let hcdf_path = self.base_dir.join(&entry.hcdf);
            if !self.fragments.contains_key(&hcdf_path) {
                if let Ok(fragment) = self.load_fragment_file(&hcdf_path, entry) {
                    self.fragments.insert(hcdf_path, fragment);
                }
            }
        }
        Ok(())
    }

    /// Load a single HCDF file into a Fragment
    fn load_fragment_file(&self, path: &Path, entry: &FragmentIndexEntry) -> Result<Fragment, FragmentError> {
        let hcdf = Hcdf::from_file(path)?;

        // Get the first comp element (fragments should have exactly one)
        let comp = hcdf.comp.into_iter().next()
            .or_else(|| {
                // Fall back to mcu if no comp
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
            })
            .ok_or_else(|| FragmentError::NoComp(path.display().to_string()))?;

        Ok(Fragment {
            board: entry.board.clone(),
            app: entry.app.clone(),
            name: comp.name,
            description: comp.description,
            mass: comp.mass,
            visuals: comp.visual,
            frames: comp.frame,
            hcdf_path: path.to_path_buf(),
        })
    }

    /// Find the fragment for a board/app combination
    pub fn find_fragment(&mut self, board: &str, app: &str) -> Option<&Fragment> {
        let key = (board.to_lowercase(), app.to_lowercase());

        // Check lookup cache first
        if let Some(cached_path) = self.lookup_cache.get(&key) {
            return cached_path.as_ref().and_then(|p| self.fragments.get(p));
        }

        // Find entry in index
        let entry = self.index.find_entry(board, app)?;
        let hcdf_path = self.base_dir.join(&entry.hcdf);

        // Load fragment if not already loaded
        if !self.fragments.contains_key(&hcdf_path) {
            if let Ok(fragment) = self.load_fragment_file(&hcdf_path, entry) {
                self.fragments.insert(hcdf_path.clone(), fragment);
            }
        }

        // Cache the lookup result
        let result_path = if self.fragments.contains_key(&hcdf_path) {
            Some(hcdf_path.clone())
        } else {
            None
        };
        self.lookup_cache.insert(key.clone(), result_path);

        self.fragments.get(&hcdf_path)
    }

    /// Get the first model path from a fragment's visuals (for backwards compatibility)
    pub fn get_model(&mut self, board: &str, app: &str) -> Option<String> {
        self.find_fragment(board, app)
            .and_then(|f| f.visuals.first())
            .and_then(|v| v.model.as_ref())
            .map(|m| m.href.clone())
    }

    /// Get all visuals for a board/app combination
    pub fn get_visuals(&mut self, board: &str, app: &str) -> Vec<Visual> {
        self.find_fragment(board, app)
            .map(|f| f.visuals.clone())
            .unwrap_or_default()
    }

    /// Get all frames for a board/app combination
    pub fn get_frames(&mut self, board: &str, app: &str) -> Vec<Frame> {
        self.find_fragment(board, app)
            .map(|f| f.frames.clone())
            .unwrap_or_default()
    }

    /// Get the underlying index
    pub fn index(&self) -> &FragmentIndex {
        &self.index
    }

    /// Clear the lookup cache
    pub fn clear_cache(&mut self) {
        self.lookup_cache.clear();
    }

    /// Reload the database from a file
    pub fn reload(&mut self, path: &Path) -> Result<(), FragmentError> {
        self.index = FragmentIndex::from_file(path)?;
        self.base_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        self.fragments.clear();
        self.lookup_cache.clear();
        self.load_all_fragments()?;
        Ok(())
    }

    /// Add a fragment from an HCDF content string (for remote loading)
    pub fn add_fragment_from_hcdf(
        &mut self,
        board: &str,
        app: &str,
        hcdf_content: &str,
        source_path: PathBuf,
    ) -> Result<(), FragmentError> {
        let hcdf = Hcdf::from_xml(hcdf_content)?;

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
            })
            .ok_or_else(|| FragmentError::NoComp(source_path.display().to_string()))?;

        let fragment = Fragment {
            board: board.to_string(),
            app: app.to_string(),
            name: comp.name,
            description: comp.description,
            mass: comp.mass,
            visuals: comp.visual,
            frames: comp.frame,
            hcdf_path: source_path.clone(),
        };

        self.fragments.insert(source_path.clone(), fragment);
        self.lookup_cache.insert(
            (board.to_lowercase(), app.to_lowercase()),
            Some(source_path),
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_index_entry_matching() {
        let toml = r#"
version = "1.0"

[[fragment]]
board = "mr_mcxn_t1"
app = "optical-flow"
hcdf = "optical_flow.hcdf"

[[fragment]]
board = "mr_mcxn_t1"
app = "*"
hcdf = "mcnt1hub.hcdf"

[[fragment]]
board = "navq95"
app = "*"
hcdf = "navq95.hcdf"
"#;

        let index = FragmentIndex::from_toml(toml).unwrap();

        // Exact match should win
        let entry = index.find_entry("mr_mcxn_t1", "optical-flow").unwrap();
        assert_eq!(entry.hcdf, "optical_flow.hcdf");

        // Wildcard should match unknown app
        let entry = index.find_entry("mr_mcxn_t1", "unknown-app").unwrap();
        assert_eq!(entry.hcdf, "mcnt1hub.hcdf");

        // Different board
        let entry = index.find_entry("navq95", "anything").unwrap();
        assert_eq!(entry.hcdf, "navq95.hcdf");

        // No match for unknown board
        assert!(index.find_entry("unknown_board", "app").is_none());
    }

    #[test]
    fn test_case_insensitive() {
        let toml = r#"
[[fragment]]
board = "MR_MCXN_T1"
app = "Optical-Flow"
hcdf = "optical_flow.hcdf"
"#;

        let index = FragmentIndex::from_toml(toml).unwrap();

        // Should match regardless of case
        assert!(index.find_entry("mr_mcxn_t1", "optical-flow").is_some());
        assert!(index.find_entry("MR_MCXN_T1", "OPTICAL-FLOW").is_some());
    }

    #[test]
    fn test_fragment_from_hcdf() {
        let hcdf_xml = r#"<?xml version="1.0"?>
<hcdf version="1.2">
    <comp name="test-assembly" role="sensor">
        <description>Test component</description>
        <visual name="board">
            <pose>0 0 0 0 0 0</pose>
            <model href="models/board.glb"/>
        </visual>
        <visual name="sensor">
            <pose>0 0 -0.005 3.14159 0 0</pose>
            <model href="models/sensor.glb" sha="abc123"/>
        </visual>
        <frame name="sensor_frame">
            <description>Sensor reference frame</description>
            <pose>0 0 -0.005 3.14159 0 0</pose>
        </frame>
    </comp>
</hcdf>"#;

        let mut db = FragmentDatabase::empty();
        db.add_fragment_from_hcdf(
            "test_board",
            "test_app",
            hcdf_xml,
            PathBuf::from("/test/test.hcdf"),
        ).unwrap();

        let fragment = db.find_fragment("test_board", "test_app").unwrap();
        assert_eq!(fragment.name, "test-assembly");
        assert_eq!(fragment.description, Some("Test component".to_string()));
        assert_eq!(fragment.visuals.len(), 2);
        assert_eq!(fragment.frames.len(), 1);

        // Check visuals
        assert_eq!(fragment.visuals[0].name, "board");
        assert_eq!(fragment.visuals[1].name, "sensor");
        assert_eq!(fragment.visuals[1].model.as_ref().unwrap().sha, Some("abc123".to_string()));

        // Check frames
        assert_eq!(fragment.frames[0].name, "sensor_frame");
        assert_eq!(fragment.frames[0].description, Some("Sensor reference frame".to_string()));
    }
}
