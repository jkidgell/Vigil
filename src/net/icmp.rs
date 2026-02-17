// ICMP ping executor using surge-ping, which supports unprivileged ICMP on Linux
// via IPPROTO_ICMP dgram sockets (no root required).
//
// Linux prerequisite: the kernel must allow ICMP sockets for unprivileged users.
// Check with: sysctl net.ipv4.ping_group_range
// Enable with: sudo sysctl -w net.ipv4.ping_group_range="0 65535"
// Most modern Linux distros already permit this.

use crate::net::{PollExecutor, PollOutcome};
use async_trait::async_trait;
use std::net::IpAddr;
use std::time::{Duration, Instant};
use surge_ping::{Client, Config, PingIdentifier, PingSequence, ICMP};
use tokio::net::lookup_host;

/// Polls a node by sending an ICMP echo request (ping).
pub struct IcmpExecutor;

impl IcmpExecutor {
    pub fn new() -> Self {
        Self
    }

    /// Resolve a hostname or IP string to an IpAddr.
    async fn resolve(address: &str) -> Option<IpAddr> {
        // If it parses directly as an IP, use it
        if let Ok(ip) = address.parse::<IpAddr>() {
            return Some(ip);
        }
        // Otherwise resolve via DNS
        let target = format!("{}:0", address);
        if let Ok(mut addrs) = lookup_host(target).await {
            return addrs.next().map(|s| s.ip());
        }
        None
    }
}

impl Default for IcmpExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PollExecutor for IcmpExecutor {
    async fn execute(&self, address: &str, timeout_duration: Duration) -> PollOutcome {
        let ip = match Self::resolve(address).await {
            Some(ip) => ip,
            None => {
                return PollOutcome {
                    success: false,
                    latency: None,
                    error: Some(format!("Failed to resolve address: {}", address)),
                }
            }
        };

        // Select IPv4 or IPv6 ICMP type
        let icmp_kind = match ip {
            IpAddr::V4(_) => ICMP::V4,
            IpAddr::V6(_) => ICMP::V6,
        };

        let config = Config::builder().kind(icmp_kind).build();
        let client = match Client::new(&config) {
            Ok(c) => c,
            Err(e) => {
                return PollOutcome {
                    success: false,
                    latency: None,
                    error: Some(format!("Failed to create ICMP client: {}", e)),
                }
            }
        };

        let payload = [0u8; 56]; // Standard ping payload size
        let mut pinger = client.pinger(ip, PingIdentifier(rand_id())).await;
        pinger.timeout(timeout_duration);

        let start = Instant::now();
        match pinger.ping(PingSequence(0), &payload).await {
            Ok((_packet, rtt)) => PollOutcome {
                success: true,
                latency: Some(rtt),
                error: None,
            },
            Err(e) => PollOutcome {
                success: false,
                latency: Some(start.elapsed()),
                error: Some(e.to_string()),
            },
        }
    }
}

/// Generate a pseudo-random 16-bit identifier for the ICMP ping session.
fn rand_id() -> u16 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (nanos ^ (std::process::id() as u32)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-check test. Real ICMP requires kernel permission:
    ///   sudo sysctl -w net.ipv4.ping_group_range="0 65535"
    /// or run as root. Skipped in CI unless permissions are set.
    #[test]
    fn test_icmp_executor_compiles() {
        let _executor = IcmpExecutor::new();
    }

    #[tokio::test]
    async fn test_icmp_resolve_ip() {
        let ip = IcmpExecutor::resolve("127.0.0.1").await;
        assert!(ip.is_some());
        assert_eq!(ip.unwrap(), IpAddr::V4("127.0.0.1".parse().unwrap()));
    }

    #[tokio::test]
    async fn test_icmp_resolve_invalid() {
        let ip = IcmpExecutor::resolve("not-a-valid-hostname-xyz.invalid").await;
        assert!(ip.is_none());
    }
}
