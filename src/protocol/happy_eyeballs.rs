use crate::platform::apply_socket_optimizations;
use crate::protocol::dns::DnsCandidates;
use crate::protocol::tls::get_tls_connector;
use rustls::pki_types::ServerName;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::sleep;
use tokio_rustls::client::TlsStream;

const CONNECTION_ATTEMPT_DELAY: Duration = Duration::from_millis(250);
const TLS_HANDSHAKE_GUARD_DURATION: Duration = Duration::from_millis(500);

pub struct EstablishedConnection {
    pub tls_stream: TlsStream<TcpStream>,
    pub peer_addr: SocketAddr,
    pub is_ipv6: bool,
    pub alpn: Option<Vec<u8>>,
}

/// Connects to a target host using Happy Eyeballs v2 (RFC 8305) with 500ms TLS Handshake Guard.
pub async fn connect_happy_eyeballs(
    host: &str,
    candidates: &DnsCandidates,
    specified_iface: Option<&str>,
) -> Result<EstablishedConnection, std::io::Error> {
    let server_name: ServerName<'static> = ServerName::try_from(host.to_string()).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("Invalid server name: {:?}", e),
        )
    })?;

    // Attempt race between IPv6 and IPv4
    let (tcp_stream, peer_addr, is_ipv6) = race_tcp_connection(candidates, specified_iface).await?;

    let connector = get_tls_connector();

    if is_ipv6 && !candidates.ipv4.is_empty() {
        // 500ms TLS Handshake Guard for IPv6 to defeat PMTUD blackholes
        match tokio::time::timeout(
            TLS_HANDSHAKE_GUARD_DURATION,
            connector.connect(server_name.clone(), tcp_stream),
        )
        .await
        {
            Ok(Ok(tls_stream)) => {
                let alpn = tls_stream.get_ref().1.alpn_protocol().map(|p| p.to_vec());
                Ok(EstablishedConnection {
                    tls_stream,
                    peer_addr,
                    is_ipv6: true,
                    alpn,
                })
            }
            Ok(Err(_)) | Err(_) => {
                // IPv6 TLS stalled or timed out! Fall back immediately to IPv4 candidate.
                let v4_addr = candidates.ipv4[0];
                let v4_tcp = connect_single(v4_addr, specified_iface).await?;
                let v4_tls = connector.connect(server_name, v4_tcp).await?;
                let alpn = v4_tls.get_ref().1.alpn_protocol().map(|p| p.to_vec());
                Ok(EstablishedConnection {
                    tls_stream: v4_tls,
                    peer_addr: v4_addr,
                    is_ipv6: false,
                    alpn,
                })
            }
        }
    } else {
        // Direct TLS handshake (IPv4 or IPv6 without fallback)
        let tls_stream = connector.connect(server_name, tcp_stream).await?;
        let alpn = tls_stream.get_ref().1.alpn_protocol().map(|p| p.to_vec());
        Ok(EstablishedConnection {
            tls_stream,
            peer_addr,
            is_ipv6,
            alpn,
        })
    }
}

async fn race_tcp_connection(
    candidates: &DnsCandidates,
    specified_iface: Option<&str>,
) -> Result<(TcpStream, SocketAddr, bool), std::io::Error> {
    if candidates.ipv6.is_empty() && candidates.ipv4.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "No address candidates provided",
        ));
    }

    // If only IPv4 exists
    if candidates.ipv6.is_empty() {
        let addr = candidates.ipv4[0];
        let stream = connect_single(addr, specified_iface).await?;
        return Ok((stream, addr, false));
    }

    // If only IPv6 exists
    if candidates.ipv4.is_empty() {
        let addr = candidates.ipv6[0];
        let stream = connect_single(addr, specified_iface).await?;
        return Ok((stream, addr, true));
    }

    // Both IPv6 and IPv4 exist: Happy Eyeballs v2 250ms staggered race
    let v6_addr = candidates.ipv6[0];
    let v4_addr = candidates.ipv4[0];
    let iface_v6 = specified_iface.map(|s| s.to_string());
    let iface_v4 = specified_iface.map(|s| s.to_string());

    tokio::select! {
        res_v6 = connect_single(v6_addr, iface_v6.as_deref()) => {
            match res_v6 {
                Ok(stream) => Ok((stream, v6_addr, true)),
                Err(_) => {
                    // v6 failed fast, connect v4 immediately
                    let stream = connect_single(v4_addr, iface_v4.as_deref()).await?;
                    Ok((stream, v4_addr, false))
                }
            }
        }
        _ = sleep(CONNECTION_ATTEMPT_DELAY) => {
            // 250ms expired! Race IPv4 concurrently with the still-running IPv6 attempt
            tokio::select! {
                res_v6 = connect_single(v6_addr, iface_v6.as_deref()) => {
                    match res_v6 {
                        Ok(stream) => Ok((stream, v6_addr, true)),
                        Err(_) => {
                            let stream = connect_single(v4_addr, iface_v4.as_deref()).await?;
                            Ok((stream, v4_addr, false))
                        }
                    }
                }
                res_v4 = connect_single(v4_addr, iface_v4.as_deref()) => {
                    match res_v4 {
                        Ok(stream) => Ok((stream, v4_addr, false)),
                        Err(_) => {
                            // If v4 fails, wait for v6
                            let stream = connect_single(v6_addr, iface_v6.as_deref()).await?;
                            Ok((stream, v6_addr, true))
                        }
                    }
                }
            }
        }
    }
}

async fn connect_single(
    addr: SocketAddr,
    specified_iface: Option<&str>,
) -> Result<TcpStream, std::io::Error> {
    let stream = TcpStream::connect(addr).await?;
    let _ = stream.set_nodelay(true);
    apply_socket_optimizations(&stream, specified_iface);
    Ok(stream)
}
