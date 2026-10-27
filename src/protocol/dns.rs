use std::net::SocketAddr;
use tokio::net::lookup_host;

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

/// Concurrently resolve DNS candidates for target host and port, separating IPv6 and IPv4.
pub async fn resolve_host(
    host: &str,
    port: u16,
    ipv4_only: bool,
    ipv6_only: bool,
) -> Result<DnsCandidates, std::io::Error> {
    let addr_str = format!("{}:{}", host, port);
    let addrs = lookup_host(&addr_str).await?;

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
