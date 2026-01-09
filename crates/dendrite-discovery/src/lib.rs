//! Dendrite Discovery - Network discovery for T1 ethernet devices
//!
//! This crate provides multiple discovery methods:
//! - LLDP (Link Layer Discovery Protocol) for physical port detection
//! - ARP scanning for subnet enumeration
//! - MCUmgr port probing for device verification

pub mod arp;
pub mod lldp;
pub mod probe;
pub mod scanner;

pub use scanner::{
    DeviceOverride, DiscoveryEvent, DiscoveryScanner, ParentConfig, ScannerConfig,
};
