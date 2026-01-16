//! Firmware types for version checking and OTA updates
//!
//! This module provides types for:
//! - Firmware manifest from upstream repository (firmware.cognipilot.org)
//! - Version comparison using semver (primary) with date fallback
//! - Post-update verification using image hash comparison

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Firmware manifest from upstream repository
/// Fetched from: https://firmware.cognipilot.org/{board}/{app}/latest.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirmwareManifest {
    /// Board name (e.g., "mr_mcxn_t1")
    pub board: String,
    /// Application name (e.g., "optical-flow")
    pub app: String,
    /// Latest available release
    pub latest: FirmwareRelease,
    /// Previous releases (for rollback)
    #[serde(default)]
    pub previous: Vec<FirmwareRelease>,
}

/// A specific firmware release
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirmwareRelease {
    /// Version string (semver format preferred)
    pub version: String,
    /// Release date
    pub date: DateTime<Utc>,
    /// MCUboot image hash (SHA256 over image header + protected TLVs + payload)
    /// This is what MCUmgr returns from image_state and is used for:
    /// 1. Download verification (computed on downloaded binary)
    /// 2. Post-update verification (compared against device report)
    pub mcuboot_hash: String,
    /// Binary file size in bytes
    pub size: u64,
    /// Download URL for the binary
    pub url: String,
    /// Optional changelog/release notes
    #[serde(default)]
    pub changelog: Option<String>,
}

/// Result of firmware version comparison
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FirmwareStatus {
    /// Device firmware is up to date
    UpToDate,
    /// Newer firmware is available
    UpdateAvailable {
        /// The latest available release
        latest_version: String,
        /// Changelog for the latest release
        changelog: Option<String>,
    },
    /// Couldn't determine firmware status (version unparseable, no manifest, etc.)
    Unknown,
    /// Firmware checking is disabled
    CheckDisabled,
}

impl Default for FirmwareStatus {
    fn default() -> Self {
        Self::CheckDisabled
    }
}

/// Compare device firmware version against upstream manifest
///
/// Uses semver comparison as primary method, falls back to date comparison
/// if version strings cannot be parsed. Does NOT use SHA for comparison
/// (SHA is only used for post-update verification).
pub fn compare_versions(
    device_version: Option<&str>,
    device_date: Option<DateTime<Utc>>,
    manifest: &FirmwareManifest,
) -> FirmwareStatus {
    // Try semver comparison first (primary method)
    if let Some(device_ver_str) = device_version {
        // Strip any prefix like "v" or trailing info like "-dirty"
        let clean_device_ver = clean_version_string(device_ver_str);
        let clean_latest_ver = clean_version_string(&manifest.latest.version);

        if let (Ok(device_ver), Ok(latest_ver)) = (
            semver::Version::parse(&clean_device_ver),
            semver::Version::parse(&clean_latest_ver),
        ) {
            if device_ver < latest_ver {
                return FirmwareStatus::UpdateAvailable {
                    latest_version: manifest.latest.version.clone(),
                    changelog: manifest.latest.changelog.clone(),
                };
            }
            return FirmwareStatus::UpToDate;
        }
    }

    // Fallback: date comparison (when version strings aren't valid semver)
    if let Some(device_build_date) = device_date {
        if device_build_date < manifest.latest.date {
            return FirmwareStatus::UpdateAvailable {
                latest_version: manifest.latest.version.clone(),
                changelog: manifest.latest.changelog.clone(),
            };
        }
        return FirmwareStatus::UpToDate;
    }

    // Cannot determine - version unparseable and no build date
    FirmwareStatus::Unknown
}

/// Clean a version string for semver parsing
/// Removes common prefixes (v, V) and suffixes (-dirty, +build)
fn clean_version_string(version: &str) -> String {
    let mut v = version.trim();

    // Remove common prefixes
    if v.starts_with('v') || v.starts_with('V') {
        v = &v[1..];
    }

    // For semver parsing, we need to handle pre-release and build metadata
    // but strip things that aren't valid semver
    // "1.2.3-dirty" is valid semver pre-release, keep it
    // "1.2.3+abc123" is valid semver build metadata, keep it

    v.to_string()
}

/// Verify that a flashed image matches the expected binary
///
/// Used for post-update verification to confirm the correct firmware was flashed.
pub fn verify_image_hash(device_hash: Option<&str>, expected_sha: &str) -> bool {
    device_hash
        .map(|h| h.eq_ignore_ascii_case(expected_sha))
        .unwrap_or(false)
}

/// OTA update state tracking
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdateState {
    /// Downloading firmware binary from repository
    Downloading { progress: f32 },
    /// Uploading firmware to device via MCUmgr
    Uploading { progress: f32 },
    /// Confirming the new image
    Confirming,
    /// Rebooting the device
    Rebooting,
    /// Verifying the new image hash
    Verifying,
    /// Update completed successfully
    Complete,
    /// Update failed
    Failed { error: String },
}

impl Default for UpdateState {
    fn default() -> Self {
        Self::Downloading { progress: 0.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_manifest(version: &str, date: &str) -> FirmwareManifest {
        FirmwareManifest {
            board: "test_board".to_string(),
            app: "test_app".to_string(),
            latest: FirmwareRelease {
                version: version.to_string(),
                date: date.parse().unwrap(),
                mcuboot_hash: "abc123def456".to_string(),
                size: 1000,
                url: "https://example.com/test.bin".to_string(),
                changelog: Some("Test release".to_string()),
            },
            previous: vec![],
        }
    }

    #[test]
    fn test_semver_comparison_up_to_date() {
        let manifest = make_manifest("1.2.3", "2026-01-10T12:00:00Z");
        let status = compare_versions(Some("1.2.3"), None, &manifest);
        assert_eq!(status, FirmwareStatus::UpToDate);
    }

    #[test]
    fn test_semver_comparison_newer_available() {
        let manifest = make_manifest("2.0.0", "2026-01-10T12:00:00Z");
        let status = compare_versions(Some("1.2.3"), None, &manifest);
        assert!(matches!(status, FirmwareStatus::UpdateAvailable { .. }));
    }

    #[test]
    fn test_semver_with_v_prefix() {
        let manifest = make_manifest("v1.2.3", "2026-01-10T12:00:00Z");
        let status = compare_versions(Some("v1.2.3"), None, &manifest);
        assert_eq!(status, FirmwareStatus::UpToDate);
    }

    #[test]
    fn test_semver_with_dirty_suffix() {
        let manifest = make_manifest("1.2.3", "2026-01-10T12:00:00Z");
        let status = compare_versions(Some("1.2.3-dirty"), None, &manifest);
        // -dirty is a pre-release version, so 1.2.3-dirty < 1.2.3
        assert!(matches!(status, FirmwareStatus::UpdateAvailable { .. }));
    }

    #[test]
    fn test_date_fallback_up_to_date() {
        let manifest = make_manifest("not-semver", "2026-01-10T12:00:00Z");
        let device_date: DateTime<Utc> = "2026-01-15T12:00:00Z".parse().unwrap();
        let status = compare_versions(Some("also-not-semver"), Some(device_date), &manifest);
        assert_eq!(status, FirmwareStatus::UpToDate);
    }

    #[test]
    fn test_date_fallback_update_available() {
        let manifest = make_manifest("not-semver", "2026-01-10T12:00:00Z");
        let device_date: DateTime<Utc> = "2026-01-01T12:00:00Z".parse().unwrap();
        let status = compare_versions(Some("also-not-semver"), Some(device_date), &manifest);
        assert!(matches!(status, FirmwareStatus::UpdateAvailable { .. }));
    }

    #[test]
    fn test_unknown_when_no_info() {
        let manifest = make_manifest("not-semver", "2026-01-10T12:00:00Z");
        let status = compare_versions(Some("also-not-semver"), None, &manifest);
        assert_eq!(status, FirmwareStatus::Unknown);
    }

    #[test]
    fn test_verify_image_hash_match() {
        assert!(verify_image_hash(Some("abc123def456"), "abc123def456"));
        assert!(verify_image_hash(Some("ABC123DEF456"), "abc123def456")); // Case insensitive
    }

    #[test]
    fn test_verify_image_hash_mismatch() {
        assert!(!verify_image_hash(Some("abc123"), "def456"));
        assert!(!verify_image_hash(None, "abc123"));
    }
}
