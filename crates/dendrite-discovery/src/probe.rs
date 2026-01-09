//! MCUmgr port probing for device verification

use anyhow::Result;
use dendrite_mcumgr::{probe_device, query_device, DeviceQueryResult, MCUMGR_PORT};
use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;
use tokio::task::JoinSet;
use tracing::{debug, info, trace};

/// Probe timeout in milliseconds
const PROBE_TIMEOUT_MS: u64 = 1000;

/// Probe multiple IP addresses for MCUmgr devices
pub async fn probe_hosts(hosts: &[Ipv4Addr], port: u16) -> Vec<Ipv4Addr> {
    let mut tasks = JoinSet::new();

    for &host in hosts {
        tasks.spawn(async move {
            let ip = IpAddr::V4(host);
            if probe_device(ip, port, PROBE_TIMEOUT_MS).await {
                Some(host)
            } else {
                None
            }
        });
    }

    let mut mcumgr_hosts = Vec::new();
    while let Some(result) = tasks.join_next().await {
        if let Ok(Some(ip)) = result {
            info!(ip = %ip, "Found MCUmgr device");
            mcumgr_hosts.push(ip);
        }
    }

    debug!(
        "Probed {} hosts, found {} MCUmgr devices",
        hosts.len(),
        mcumgr_hosts.len()
    );
    mcumgr_hosts
}

/// Query multiple devices for full information
pub async fn query_hosts(
    hosts: &[Ipv4Addr],
    port: u16,
) -> Vec<(Ipv4Addr, DeviceQueryResult)> {
    let mut tasks = JoinSet::new();

    for &host in hosts {
        tasks.spawn(async move {
            let ip = IpAddr::V4(host);
            match query_device(ip, port).await {
                Ok(result) => Some((host, result)),
                Err(e) => {
                    debug!(ip = %host, error = %e, "Failed to query device");
                    None
                }
            }
        });
    }

    let mut results = Vec::new();
    while let Some(result) = tasks.join_next().await {
        if let Ok(Some((ip, query_result))) = result {
            results.push((ip, query_result));
        }
    }

    results
}

/// Probe a single host with retries
pub async fn probe_with_retry(ip: Ipv4Addr, port: u16, retries: u32) -> bool {
    for attempt in 0..retries {
        if probe_device(IpAddr::V4(ip), port, PROBE_TIMEOUT_MS).await {
            return true;
        }
        if attempt < retries - 1 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    false
}
