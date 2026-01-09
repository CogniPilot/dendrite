//! Async UDP transport wrapper for mcumgr-client

use anyhow::Result;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::timeout;
use tracing::{debug, trace};

/// Async UDP transport for MCUmgr protocol
pub struct UdpTransportAsync {
    socket: UdpSocket,
    target: SocketAddr,
    timeout_ms: u64,
    mtu: usize,
    seq: u8,
}

impl UdpTransportAsync {
    /// Create a new async UDP transport
    pub async fn new(host: &str, port: u16, timeout_ms: u64) -> Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        let target: SocketAddr = format!("{}:{}", host, port).parse()?;

        Ok(Self {
            socket,
            target,
            timeout_ms,
            mtu: 1024,
            seq: 0,
        })
    }

    /// Get next sequence number
    fn next_seq(&mut self) -> u8 {
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1);
        seq
    }

    /// Encode SMP v2 header
    fn encode_header(&self, op: u8, group: u16, id: u8, body_len: u16, seq: u8) -> [u8; 8] {
        let mut header = [0u8; 8];
        // Byte 0: Res(3) | Ver(2) | OP(3) - Version 1 (SMP v2)
        header[0] = (1 << 3) | (op & 0x07);
        // Byte 1: Flags
        header[1] = 0;
        // Bytes 2-3: Length (big-endian)
        header[2] = (body_len >> 8) as u8;
        header[3] = body_len as u8;
        // Bytes 4-5: Group ID (big-endian)
        header[4] = (group >> 8) as u8;
        header[5] = group as u8;
        // Byte 6: Sequence
        header[6] = seq;
        // Byte 7: Command ID
        header[7] = id;
        header
    }

    /// Decode SMP v2 header
    fn decode_header(&self, data: &[u8]) -> Option<(u8, u16, u8, u16, u8)> {
        if data.len() < 8 {
            return None;
        }
        let op = data[0] & 0x07;
        let body_len = ((data[2] as u16) << 8) | (data[3] as u16);
        let group = ((data[4] as u16) << 8) | (data[5] as u16);
        let seq = data[6];
        let id = data[7];
        Some((op, group, id, body_len, seq))
    }

    /// Send request and receive response
    pub async fn transceive(
        &mut self,
        op: u8,
        group: u16,
        id: u8,
        body: &[u8],
    ) -> Result<Vec<u8>> {
        let seq = self.next_seq();
        let header = self.encode_header(op, group, id, body.len() as u16, seq);

        // Build packet
        let mut packet = Vec::with_capacity(8 + body.len());
        packet.extend_from_slice(&header);
        packet.extend_from_slice(body);

        trace!(
            target = %self.target,
            op = op,
            group = group,
            id = id,
            seq = seq,
            body_len = body.len(),
            "Sending MCUmgr request"
        );

        // Send packet
        self.socket.send_to(&packet, self.target).await?;

        // Receive response with timeout
        let mut buf = vec![0u8; self.mtu];
        let duration = Duration::from_millis(self.timeout_ms);

        let (len, _addr) = timeout(duration, self.socket.recv_from(&mut buf)).await??;

        if len < 8 {
            anyhow::bail!("Response too short: {} bytes", len);
        }

        // Parse response header
        let (resp_op, resp_group, resp_id, resp_len, resp_seq) =
            self.decode_header(&buf).ok_or_else(|| anyhow::anyhow!("Invalid response header"))?;

        debug!(
            op = resp_op,
            group = resp_group,
            id = resp_id,
            seq = resp_seq,
            body_len = resp_len,
            "Received MCUmgr response"
        );

        // Validate response matches request
        if resp_seq != seq {
            anyhow::bail!(
                "Sequence mismatch: expected {}, got {}",
                seq,
                resp_seq
            );
        }

        // Extract body
        let body_start = 8;
        let body_end = body_start + resp_len as usize;
        if body_end > len {
            anyhow::bail!(
                "Response body truncated: expected {} bytes, got {}",
                resp_len,
                len - body_start
            );
        }

        Ok(buf[body_start..body_end].to_vec())
    }

    /// Check if device is reachable (echo test)
    pub async fn ping(&mut self) -> Result<bool> {
        // Echo request: group=0 (Default), id=0 (Echo), op=2 (Write)
        let body = serde_cbor::to_vec(&EchoReq { d: "ping" })?;

        match self.transceive(2, 0, 0, &body).await {
            Ok(resp_body) => {
                let resp: EchoRsp = serde_cbor::from_slice(&resp_body)?;
                Ok(resp.r == "ping")
            }
            Err(_) => Ok(false),
        }
    }
}

#[derive(serde::Serialize)]
struct EchoReq<'a> {
    d: &'a str,
}

#[derive(serde::Deserialize)]
struct EchoRsp {
    r: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_encoding() {
        let transport = UdpTransportAsync {
            socket: unsafe { std::mem::zeroed() }, // Just for testing header encoding
            target: "127.0.0.1:1337".parse().unwrap(),
            timeout_ms: 5000,
            mtu: 1024,
            seq: 0,
        };

        let header = transport.encode_header(2, 0, 0, 10, 5);
        // op=2, version=1: (1 << 3) | 2 = 10
        assert_eq!(header[0], 10);
        // flags=0
        assert_eq!(header[1], 0);
        // length=10 big-endian
        assert_eq!(header[2], 0);
        assert_eq!(header[3], 10);
        // group=0
        assert_eq!(header[4], 0);
        assert_eq!(header[5], 0);
        // seq=5
        assert_eq!(header[6], 5);
        // id=0
        assert_eq!(header[7], 0);
    }
}
