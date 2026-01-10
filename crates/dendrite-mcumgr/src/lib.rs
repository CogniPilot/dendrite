//! Dendrite MCUmgr - MCUmgr integration for device queries
//!
//! This crate wraps mcumgr-client to provide async device querying
//! for the Dendrite system.

pub mod query;
pub mod transport;

pub use query::{
    probe_device, query_device, query_hcdf_info, query_result_to_device,
    hcdf_group, DeviceQueryResult, HcdfInfoResponse, QueryError,
    MCUMGR_PORT,
};
pub use transport::UdpTransportAsync;
