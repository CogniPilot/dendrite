//! Dendrite Core - Core types, HCDF parsing, and device registry
//!
//! This crate provides the foundational types for the Dendrite system:
//! - HCDF (Hardware Configuration Descriptive Format) parsing and serialization
//! - Device registry types for tracking discovered hardware
//! - Topology graph for parent/child device relationships
//! - Fragment database for board/app to model mapping
//! - Cache management for remote HCDF files and models

pub mod cache;
pub mod device;
pub mod fragment;
pub mod hcdf;
pub mod topology;

pub use cache::{CacheError, CacheManifest, CachedHcdf, CachedModel, FragmentCache, sha256_hex};
pub use device::{Device, DeviceAxisAlign, DeviceFov, DeviceFrame, DeviceGeometry, DeviceId, DeviceInfo, DevicePort, DeviceSensor, DeviceStatus, DeviceVisual, FirmwareInfo};
pub use fragment::{Fragment, FragmentDatabase, FragmentError, FragmentIndex, FragmentIndexEntry};
pub use hcdf::{Comp, Frame, Hcdf, HcdfError, ModelRef, Pose, Visual, parse_pose_string};
pub use topology::{Topology, TopologyNode};
