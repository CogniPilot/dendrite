//! Configuration loading and validation

use anyhow::Result;
use dendrite_discovery::{ScannerConfig, ParentConfig, DeviceOverride};
use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;
use std::path::Path;
use tracing::info;

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub discovery: DiscoveryConfig,
    #[serde(default)]
    pub parent: Option<ParentDeviceConfig>,
    #[serde(default)]
    pub models: ModelsConfig,
    #[serde(default)]
    pub hcdf: HcdfConfig,
    #[serde(default)]
    pub fragments: FragmentsConfig,
    #[serde(default, rename = "device_override")]
    pub device_overrides: Vec<DeviceOverrideConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// Bind address for web server
    #[serde(default = "default_bind")]
    pub bind: String,
    /// Full discovery scan interval in seconds (discovers new devices)
    #[serde(default = "default_interval")]
    pub discovery_interval_secs: u64,
    /// Heartbeat interval in seconds (lightweight status check)
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval_secs: u64,
    /// TLS configuration (optional - enables HTTPS when present)
    #[serde(default)]
    pub tls: Option<TlsConfig>,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            discovery_interval_secs: default_interval(),
            heartbeat_interval_secs: default_heartbeat_interval(),
            tls: None,
        }
    }
}

/// TLS/HTTPS configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Path to certificate file (PEM format)
    pub cert: String,
    /// Path to private key file (PEM format)
    pub key: String,
}

fn default_bind() -> String {
    "0.0.0.0:8080".to_string()
}

fn default_interval() -> u64 {
    60  // Full scan every 60 seconds
}

fn default_heartbeat_interval() -> u64 {
    2  // Lightweight ARP/ping check every 2 seconds
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryConfig {
    /// Subnet to scan
    #[serde(default = "default_subnet")]
    pub subnet: Ipv4Addr,
    /// Subnet prefix length
    #[serde(default = "default_prefix")]
    pub prefix_len: u8,
    /// MCUmgr port
    #[serde(default = "default_mcumgr_port")]
    pub mcumgr_port: u16,
    /// Use LLDP for port detection
    #[serde(default = "default_true")]
    pub use_lldp: bool,
    /// Use ARP scanning
    #[serde(default = "default_true")]
    pub use_arp: bool,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            subnet: default_subnet(),
            prefix_len: default_prefix(),
            mcumgr_port: default_mcumgr_port(),
            use_lldp: true,
            use_arp: true,
        }
    }
}

fn default_subnet() -> Ipv4Addr {
    Ipv4Addr::new(192, 168, 186, 0)
}

fn default_prefix() -> u8 {
    24
}

fn default_mcumgr_port() -> u16 {
    1337
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParentDeviceConfig {
    /// Parent device name
    pub name: String,
    /// Board type
    pub board: String,
    /// Number of T1 ports
    #[serde(default = "default_ports")]
    pub ports: u8,
    /// Parent IP address (optional)
    pub ip: Option<Ipv4Addr>,
}

fn default_ports() -> u8 {
    6
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsConfig {
    /// Path to 3D model files
    #[serde(default = "default_models_path")]
    pub path: String,
}

impl Default for ModelsConfig {
    fn default() -> Self {
        Self {
            path: default_models_path(),
        }
    }
}

fn default_models_path() -> String {
    "./assets/models".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HcdfConfig {
    /// Path to HCDF file
    #[serde(default = "default_hcdf_path")]
    pub path: String,
    /// Auto-save interval in seconds (0 to disable)
    #[serde(default)]
    pub autosave_interval_secs: u64,
}

impl Default for HcdfConfig {
    fn default() -> Self {
        Self {
            path: default_hcdf_path(),
            autosave_interval_secs: 0,
        }
    }
}

fn default_hcdf_path() -> String {
    "./dendrite.hcdf".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentsConfig {
    /// Path to fragments index file
    #[serde(default = "default_fragments_path")]
    pub path: String,
}

impl Default for FragmentsConfig {
    fn default() -> Self {
        Self {
            path: default_fragments_path(),
        }
    }
}

fn default_fragments_path() -> String {
    "./fragments/index.toml".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceOverrideConfig {
    /// Hardware ID to match
    pub hwid: String,
    /// Override name
    pub name: Option<String>,
    /// Override port number
    pub port: Option<u8>,
    /// Override model path
    pub model_path: Option<String>,
}

impl Config {
    /// Convert to ScannerConfig
    pub fn to_scanner_config(&self) -> ScannerConfig {
        ScannerConfig {
            subnet: self.discovery.subnet,
            prefix_len: self.discovery.prefix_len,
            mcumgr_port: self.discovery.mcumgr_port,
            interval_secs: self.daemon.discovery_interval_secs,
            heartbeat_interval_secs: self.daemon.heartbeat_interval_secs,
            use_lldp: self.discovery.use_lldp,
            use_arp: self.discovery.use_arp,
            parent: self.parent.as_ref().map(|p| ParentConfig {
                name: p.name.clone(),
                board: p.board.clone(),
                ports: p.ports,
                ip: p.ip,
            }),
            overrides: self
                .device_overrides
                .iter()
                .map(|o| DeviceOverride {
                    hwid: o.hwid.clone(),
                    name: o.name.clone(),
                    port: o.port,
                    model_path: o.model_path.clone(),
                })
                .collect(),
        }
    }
}

/// Load configuration from file
pub fn load_config(path: &Path) -> Result<Config> {
    if path.exists() {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        info!(path = %path.display(), "Loaded configuration");
        Ok(config)
    } else {
        info!(
            path = %path.display(),
            "Configuration file not found, using defaults"
        );
        Ok(Config {
            daemon: DaemonConfig::default(),
            discovery: DiscoveryConfig::default(),
            parent: None,
            models: ModelsConfig::default(),
            hcdf: HcdfConfig::default(),
            fragments: FragmentsConfig::default(),
            device_overrides: Vec::new(),
        })
    }
}

/// Save default configuration to file
pub fn save_default_config(path: &Path) -> Result<()> {
    let config = Config {
        daemon: DaemonConfig::default(),
        discovery: DiscoveryConfig::default(),
        parent: Some(ParentDeviceConfig {
            name: "navq95".to_string(),
            board: "imx95-navq".to_string(),
            ports: 6,
            ip: None,
        }),
        models: ModelsConfig::default(),
        hcdf: HcdfConfig::default(),
        fragments: FragmentsConfig::default(),
        device_overrides: vec![DeviceOverrideConfig {
            hwid: "0x12345678".to_string(),
            name: Some("spinali-front-left".to_string()),
            port: Some(2),
            model_path: Some("models/spinali.glb".to_string()),
        }],
    };

    let content = toml::to_string_pretty(&config)?;
    std::fs::write(path, content)?;
    Ok(())
}
