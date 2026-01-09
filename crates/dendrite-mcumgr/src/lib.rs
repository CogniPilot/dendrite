//! Dendrite MCUmgr - MCUmgr integration for device queries
//!
//! This crate wraps mcumgr-client to provide async device querying
//! for the Dendrite system.

pub mod query;
pub mod transport;

pub use query::{
    probe_device, query_device, query_result_to_device, DeviceQueryResult, QueryError,
    MCUMGR_PORT,
};
pub use transport::UdpTransportAsync;
