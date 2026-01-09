//! Dendrite Core - Core types, HCDF parsing, and device registry
//!
//! This crate provides the foundational types for the Dendrite system:
//! - HCDF (Hardware Configuration Descriptive Format) parsing and serialization
//! - Device registry types for tracking discovered hardware
//! - Topology graph for parent/child device relationships
//! - Fragment database for board/app to model mapping

pub mod device;
pub mod fragment;
pub mod hcdf;
pub mod topology;

pub use device::{Device, DeviceId, DeviceInfo, DeviceStatus, FirmwareInfo};
pub use fragment::{Fragment, FragmentDatabase, FragmentError, FragmentIndex};
pub use hcdf::{Hcdf, HcdfError};
pub use topology::{Topology, TopologyNode};
