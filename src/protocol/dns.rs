use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::lookup_host;
use tokio::time::timeout;

const DNS_RESOLUTION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct DnsCandidates {
    pub ipv6: Vec<SocketAddr>,
    pub ipv4: Vec<SocketAddr>,
}

impl DnsCandidates {
    pub fn is_empty(&self) -> bool {
        self.ipv6.is_empty() && self.ipv4.is_empty()
    }
}

/// Concurrently resolve DNS candidates for target host and port, separating IPv6 and IPv4 with 5s timeout.
pub async fn resolve_host(
    host: &str,
    port: u16,
    ipv4_only: bool,
    ipv6_only: bool,
) -> Result<DnsCandidates, std::io::Error> {
    let clean_host = host.trim().trim_start_matches('[').trim_end_matches(']');
    let addrs = timeout(DNS_RESOLUTION_TIMEOUT, lookup_host((clean_host, port)))
        .await
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!(
                    "DNS resolution timed out for {} after {:?}",
                    host, DNS_RESOLUTION_TIMEOUT
                ),
            )
        })??;

    let mut ipv6 = Vec::new();
    let mut ipv4 = Vec::new();

    for addr in addrs {
        match addr {
            SocketAddr::V6(_) => {
                if !ipv4_only {
                    ipv6.push(addr);
                }
            }
            SocketAddr::V4(_) => {
                if !ipv6_only {
                    ipv4.push(addr);
                }
            }
        }
    }

    let candidates = DnsCandidates { ipv6, ipv4 };

    if candidates.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AddrNotAvailable,
            format!(
                "No valid IP addresses resolved for {} (ipv4_only={}, ipv6_only={})",
                host, ipv4_only, ipv6_only
            ),
        ));
    }

    Ok(candidates)
}
