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
const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

#[cfg(unix)]
use std::os::unix::io::AsRawFd;

pub struct EstablishedConnection {
    pub tls_stream: TlsStream<TcpStream>,
    pub peer_addr: SocketAddr,
    pub is_ipv6: bool,
    pub alpn: Option<Vec<u8>>,
    pub raw_fd: Option<i32>,
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
                #[cfg(unix)]
                let raw_fd = Some(tls_stream.get_ref().0.as_raw_fd());
                #[cfg(not(unix))]
                let raw_fd = None;
                Ok(EstablishedConnection {
                    tls_stream,
                    peer_addr,
                    is_ipv6: true,
                    alpn,
                    raw_fd,
                })
            }
            Ok(Err(_)) | Err(_) => {
                // IPv6 TLS stalled or timed out! Fall back immediately to IPv4 candidate.
                let (v4_tcp, v4_addr) =
                    connect_any_candidate(&candidates.ipv4, specified_iface).await?;
                let v4_tls = tokio::time::timeout(
                    TLS_HANDSHAKE_TIMEOUT,
                    connector.connect(server_name, v4_tcp),
                )
                .await
                .map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "IPv4 fallback TLS handshake timed out after 5s",
                    )
                })??;
                let alpn = v4_tls.get_ref().1.alpn_protocol().map(|p| p.to_vec());
                #[cfg(unix)]
                let raw_fd = Some(v4_tls.get_ref().0.as_raw_fd());
                #[cfg(not(unix))]
                let raw_fd = None;
                Ok(EstablishedConnection {
                    tls_stream: v4_tls,
                    peer_addr: v4_addr,
                    is_ipv6: false,
                    alpn,
                    raw_fd,
                })
            }
        }
    } else {
        // Direct TLS handshake (IPv4 or IPv6 without fallback) with 5s timeout guard
        let tls_stream = tokio::time::timeout(
            TLS_HANDSHAKE_TIMEOUT,
            connector.connect(server_name, tcp_stream),
        )
        .await
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "TLS handshake timed out after 5s",
            )
        })??;
        let alpn = tls_stream.get_ref().1.alpn_protocol().map(|p| p.to_vec());
        #[cfg(unix)]
        let raw_fd = Some(tls_stream.get_ref().0.as_raw_fd());
        #[cfg(not(unix))]
        let raw_fd = None;
        Ok(EstablishedConnection {
            tls_stream,
            peer_addr,
            is_ipv6,
            alpn,
            raw_fd,
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
        let (stream, addr) = connect_any_candidate(&candidates.ipv4, specified_iface).await?;
        return Ok((stream, addr, false));
    }

    // If only IPv6 exists
    if candidates.ipv4.is_empty() {
        let (stream, addr) = connect_any_candidate(&candidates.ipv6, specified_iface).await?;
        return Ok((stream, addr, true));
    }

    // Both IPv6 and IPv4 exist: RFC 8305 Happy Eyeballs v2 staggered race.
    // 1. Initiate IPv6 connection
    let v6_fut = connect_any_candidate(&candidates.ipv6, specified_iface);
    tokio::pin!(v6_fut);

    // 2. Race IPv6 against the 250ms Connection Attempt Delay
    tokio::select! {
        res_v6 = &mut v6_fut => {
            match res_v6 {
                Ok((stream, addr)) => Ok((stream, addr, true)),
                Err(_) => {
                    // v6 failed fast within 250ms, connect v4 immediately
                    let (stream, addr) = connect_any_candidate(&candidates.ipv4, specified_iface).await?;
                    Ok((stream, addr, false))
                }
            }
        }
        _ = sleep(CONNECTION_ATTEMPT_DELAY) => {
            // 250ms expired! Start IPv4 concurrently WITHOUT cancelling the in-flight IPv6 attempt
            let v4_fut = connect_any_candidate(&candidates.ipv4, specified_iface);
            tokio::pin!(v4_fut);

            tokio::select! {
                res_v6 = &mut v6_fut => {
                    match res_v6 {
                        Ok((stream, addr)) => Ok((stream, addr, true)),
                        Err(_) => {
                            // v6 errored; await in-flight v4
                            let (stream, addr) = v4_fut.await?;
                            Ok((stream, addr, false))
                        }
                    }
                }
                res_v4 = &mut v4_fut => {
                    match res_v4 {
                        Ok((stream, addr)) => Ok((stream, addr, false)),
                        Err(v4_err) => {
                            // v4 errored; await in-flight v6
                            match v6_fut.await {
                                Ok((stream, addr)) => Ok((stream, addr, true)),
                                Err(v6_err) => {
                                    Err(std::io::Error::new(
                                        std::io::ErrorKind::ConnectionRefused,
                                        format!(
                                            "Happy Eyeballs failed on both IPv6 ({}) and IPv4 ({})",
                                            v6_err, v4_err
                                        ),
                                    ))
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

async fn connect_any_candidate(
    candidates: &[SocketAddr],
    specified_iface: Option<&str>,
) -> Result<(TcpStream, SocketAddr), std::io::Error> {
    let mut last_err = None;
    for &addr in candidates {
        match connect_single(addr, specified_iface).await {
            Ok(stream) => return Ok((stream, addr)),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "No address candidates reachable",
        )
    }))
}

async fn connect_single(
    addr: SocketAddr,
    specified_iface: Option<&str>,
) -> Result<TcpStream, std::io::Error> {
    let stream = tokio::time::timeout(TCP_CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("TCP connection to {} timed out after 5s", addr),
            )
        })??;
    let _ = stream.set_nodelay(true);
    apply_socket_optimizations(&stream, specified_iface);
    Ok(stream)
}
