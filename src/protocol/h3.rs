use crate::metrics::EdgeTelemetry;
use crate::protocol::dns::DnsCandidates;
use crate::protocol::telemetry::parse_edge_telemetry;
use crate::protocol::{PROBE_TIMEOUT, ProbePendingGuard};
use bytes::{Buf, Bytes};
use http::{Method, Request};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Represents an established HTTP/3 client multiplexer over Quinn QUIC transport.
#[derive(Clone)]
pub struct H3Client {
    pub send_request: h3::client::SendRequest<h3_quinn::OpenStreams, Bytes>,
    pub host: String,
    pub peer_addr: SocketAddr,
}

impl H3Client {
    /// Connects to host via HTTP/3 QUIC over UDP port 443 with 4.0s timeout guard.
    pub async fn connect(host: &str, candidates: &DnsCandidates) -> Result<Self, std::io::Error> {
        let peer_addr = candidates
            .ipv6
            .first()
            .copied()
            .or_else(|| candidates.ipv4.first().copied())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "No IP address candidates found for QUIC connection",
                )
            })?;

        let bind_addr: SocketAddr = if peer_addr.is_ipv6() {
            "[::]:0".parse().unwrap()
        } else {
            "0.0.0.0:0".parse().unwrap()
        };

        let mut root_store = rustls::RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let mut tls_config = rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        tls_config.alpn_protocols = vec![b"h3".to_vec()];

        let quic_crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        let mut client_config = quinn::ClientConfig::new(Arc::new(quic_crypto));

        let mut transport = quinn::TransportConfig::default();
        transport.max_idle_timeout(Some(Duration::from_secs(10).try_into().unwrap()));
        transport.initial_rtt(Duration::from_millis(50));
        client_config.transport_config(Arc::new(transport));

        let mut endpoint = quinn::Endpoint::client(bind_addr)?;
        endpoint.set_default_client_config(client_config);

        let connecting = endpoint.connect(peer_addr, host).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                format!("QUIC connection dispatch error: {:?}", e),
            )
        })?;

        let quinn_conn = tokio::time::timeout(Duration::from_secs(4), connecting)
            .await
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "QUIC handshake timed out after 4s (UDP port 443 may be filtered by firewall or ISP)",
                )
            })?
            .map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    format!("QUIC handshake failed: {:?}", e),
                )
            })?;

        let h3_conn = h3_quinn::Connection::new(quinn_conn);
        let (mut driver, send_request) = h3::client::new(h3_conn).await.map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                format!("HTTP/3 setup failed: {:?}", e),
            )
        })?;

        tokio::spawn(async move {
            let _ = driver.wait_idle().await;
        });

        Ok(H3Client {
            send_request,
            host: host.to_string(),
            peer_addr,
        })
    }

    /// Dispatch a single in-band latency probe to measure RTT over HTTP/3
    pub async fn dispatch_probe(
        &mut self,
        probe_pending: &AtomicBool,
    ) -> Result<(f64, Option<EdgeTelemetry>), std::io::Error> {
        tokio::time::timeout(PROBE_TIMEOUT, self.dispatch_probe_inner(probe_pending))
            .await
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "HTTP/3 probe request timed out under severe network latency/loss",
                )
            })?
    }

    async fn dispatch_probe_inner(
        &mut self,
        probe_pending: &AtomicBool,
    ) -> Result<(f64, Option<EdgeTelemetry>), std::io::Error> {
        let guard = ProbePendingGuard::activate(probe_pending);
        tokio::task::yield_now().await;

        let path = crate::protocol::download_path(&self.host, 0);
        let req = Request::builder()
            .method(Method::GET)
            .uri(format!("https://{}{}", self.host, path))
            .header("Host", &self.host)
            .header("User-Agent", "kiwi/1.0.0")
            .header("Accept", "*/*")
            .body(())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

        let start = Instant::now();
        let mut stream = self.send_request.send_request(req).await.map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::ConnectionReset, format!("{:?}", e))
        })?;
        let _ = stream.finish().await;

        drop(guard);

        let resp = stream
            .recv_response()
            .await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::TimedOut, format!("{:?}", e)))?;

        let status = resp.status();
        let telemetry = parse_edge_telemetry(resp.headers());

        while let Ok(Some(mut chunk)) = stream.recv_data().await {
            let _ = chunk.remaining();
            chunk.advance(chunk.remaining());
        }

        if !status.is_success() {
            return Err(std::io::Error::other(format!(
                "HTTP/3 probe failed with HTTP status {}",
                status
            )));
        }

        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        Ok((elapsed_ms, Some(telemetry)))
    }

    /// Run bulk download stream over HTTP/3 QUIC
    pub async fn run_download_stream(
        mut send_request: h3::client::SendRequest<h3_quinn::OpenStreams, Bytes>,
        host: String,
        bytes_to_fetch: u64,
        total_counter: Arc<AtomicU64>,
        stop_signal: Arc<AtomicBool>,
        probe_pending: Arc<AtomicBool>,
    ) -> Result<Option<EdgeTelemetry>, std::io::Error> {
        let mut telemetry = None;

        while !stop_signal.load(Ordering::Relaxed) {
            let path = crate::protocol::download_path(&host, bytes_to_fetch);
            let req = Request::builder()
                .method(Method::GET)
                .uri(format!("https://{}{}", host, path))
                .header("Host", &host)
                .header("User-Agent", "kiwi/1.0.0")
                .header("Accept", "*/*")
                .body(())
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

            let mut stream = match send_request.send_request(req).await {
                Ok(s) => s,
                Err(_) => break,
            };
            let _ = stream.finish().await;

            let resp = match stream.recv_response().await {
                Ok(r) => r,
                Err(_) => break,
            };

            let status = resp.status();
            let is_success = status.is_success();

            if status == http::StatusCode::TOO_MANY_REQUESTS {
                tokio::time::sleep(Duration::from_millis(100)).await;
            } else if !is_success {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }

            if telemetry.is_none() && is_success {
                telemetry = Some(parse_edge_telemetry(resp.headers()));
            }

            let mut local_batch: u64 = 0;

            while let Ok(Some(mut chunk)) = stream.recv_data().await {
                if stop_signal.load(Ordering::Relaxed) {
                    break;
                }
                let len = chunk.remaining();
                chunk.advance(len);

                if is_success {
                    local_batch += len as u64;
                    if local_batch >= 65536 {
                        total_counter.fetch_add(local_batch, Ordering::Relaxed);
                        local_batch = 0;
                    }

                    if probe_pending.load(Ordering::Relaxed) {
                        if local_batch > 0 {
                            total_counter.fetch_add(local_batch, Ordering::Relaxed);
                            local_batch = 0;
                        }
                        tokio::task::yield_now().await;
                    }
                }
            }

            if local_batch > 0 {
                total_counter.fetch_add(local_batch, Ordering::Relaxed);
            }
        }

        Ok(telemetry)
    }

    /// Run bulk upload stream over HTTP/3 QUIC
    pub async fn run_upload_stream(
        mut send_request: h3::client::SendRequest<h3_quinn::OpenStreams, Bytes>,
        host: String,
        total_counter: Arc<AtomicU64>,
        stop_signal: Arc<AtomicBool>,
        probe_pending: Arc<AtomicBool>,
    ) -> Result<Option<EdgeTelemetry>, std::io::Error> {
        let mut telemetry = None;
        let upload_chunk_slice = crate::protocol::h2::UPLOAD_BUFFER.clone();
        let chunk_size = 65536;

        while !stop_signal.load(Ordering::Relaxed) {
            let path = crate::protocol::upload_path(&host);
            let req = Request::builder()
                .method(Method::POST)
                .uri(format!("https://{}{}", host, path))
                .header("Host", &host)
                .header("User-Agent", "kiwi/1.0.0")
                .header("Content-Type", "application/octet-stream")
                .body(())
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

            let mut stream = match send_request.send_request(req).await {
                Ok(s) => s,
                Err(_) => break,
            };

            let mut local_batch: u64 = 0;
            let mut offset = 0;

            while !stop_signal.load(Ordering::Relaxed)
                && offset + chunk_size <= upload_chunk_slice.len()
            {
                let sub_slice = upload_chunk_slice.slice(offset..offset + chunk_size);
                if stream.send_data(sub_slice).await.is_err() {
                    break;
                }
                offset += chunk_size;
                local_batch += chunk_size as u64;

                if local_batch >= 65536 {
                    total_counter.fetch_add(local_batch, Ordering::Relaxed);
                    local_batch = 0;
                }

                if probe_pending.load(Ordering::Relaxed) {
                    if local_batch > 0 {
                        total_counter.fetch_add(local_batch, Ordering::Relaxed);
                        local_batch = 0;
                    }
                    tokio::task::yield_now().await;
                }
            }

            let _ = stream.finish().await;

            if local_batch > 0 {
                total_counter.fetch_add(local_batch, Ordering::Relaxed);
            }

            if let Ok(resp) = stream.recv_response().await {
                if resp.status() == http::StatusCode::TOO_MANY_REQUESTS {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                if telemetry.is_none() && resp.status().is_success() {
                    telemetry = Some(parse_edge_telemetry(resp.headers()));
                }
            }
        }

        Ok(telemetry)
    }
}
