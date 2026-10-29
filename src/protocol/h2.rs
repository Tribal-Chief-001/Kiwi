use crate::metrics::EdgeTelemetry;
use crate::protocol::telemetry::parse_edge_telemetry;
use crate::protocol::{PROBE_TIMEOUT, ProbePendingGuard};
use bytes::Bytes;
use h2::client::{Builder, SendRequest};
use http::{Method, Request};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Instant;

pub const UPLOAD_BUFFER_SIZE: usize = 1048576; // 1 MB

pub static UPLOAD_BUFFER: LazyLock<Bytes> = LazyLock::new(|| {
    let mut data = vec![0u8; UPLOAD_BUFFER_SIZE];
    let mut state: u64 = 0x853c49e6748fea9b;
    for chunk in data.as_chunks_mut::<8>().0 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        chunk.copy_from_slice(&state.to_le_bytes());
    }
    Bytes::from(data)
});

use crate::protocol::AnyStream;

/// Represents an established HTTP/2 client multiplexer.
#[derive(Clone)]
pub struct H2Client {
    pub send_request: SendRequest<Bytes>,
    pub host: String,
    pub raw_fd: Option<i32>,
}

impl H2Client {
    pub async fn new(stream: AnyStream, host: String) -> Result<Self, std::io::Error> {
        let raw_fd = stream.raw_fd();

        let mut builder = Builder::new();
        // Dynamic BDP window scaling: 1MB stream, 2MB connection window
        builder.initial_window_size(1024 * 1024);
        builder.initial_connection_window_size(2048 * 1024);
        builder.max_concurrent_streams(128);

        let (send_request, connection) = builder.handshake(stream).await.map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                format!("H2 handshake failed: {:?}", e),
            )
        })?;

        // Drive the connection state machine on Tokio background
        tokio::spawn(async move {
            let _ = connection.await;
        });

        Ok(H2Client {
            send_request,
            host,
            raw_fd,
        })
    }

    /// Dispatch a single in-band latency probe to measure RTT without bulk stream distortion.
    pub async fn dispatch_probe(
        &mut self,
        probe_pending: &AtomicBool,
    ) -> Result<(f64, Option<EdgeTelemetry>), std::io::Error> {
        tokio::time::timeout(PROBE_TIMEOUT, self.dispatch_probe_inner(probe_pending))
            .await
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Probe request timed out under severe network latency/loss",
                )
            })?
    }

    async fn dispatch_probe_inner(
        &mut self,
        probe_pending: &AtomicBool,
    ) -> Result<(f64, Option<EdgeTelemetry>), std::io::Error> {
        // Assert cooperative yield flag with RAII auto-release
        let guard = ProbePendingGuard::activate(probe_pending);
        tokio::task::yield_now().await;

        let path = crate::protocol::download_path(&self.host, 0);
        let request = Request::builder()
            .method(Method::GET)
            .uri(format!("https://{}{}", self.host, path))
            .header("Host", &self.host)
            .header("User-Agent", "kiwi/1.0.0")
            .header("Accept", "*/*")
            .body(())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

        let start = Instant::now();
        let mut ready_client = self.send_request.clone().ready().await.map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                format!("Client not ready: {:?}", e),
            )
        })?;

        let (response_future, _) = ready_client.send_request(request, true).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                format!("Send request failed: {:?}", e),
            )
        })?;

        // De-assert probe pending once request is queued onto the wire
        drop(guard);

        let response = response_future.await.map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("Probe response error: {:?}", e),
            )
        })?;

        let status = response.status();
        let edge_telemetry = parse_edge_telemetry(response.headers());
        let mut body = response.into_body();

        while let Some(chunk_res) = body.data().await {
            let chunk = chunk_res.map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    format!("Chunk error: {:?}", e),
                )
            })?;
            let len = chunk.len();
            let _ = body.flow_control().release_capacity(len);
        }

        if !status.is_success() {
            return Err(std::io::Error::other(format!(
                "Probe failed with HTTP status {}",
                status
            )));
        }

        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        Ok((elapsed_ms, Some(edge_telemetry)))
    }

    /// Run a bulk download stream using L1d-resident stack sink and immediate WINDOW_UPDATE.
    pub async fn run_download_stream(
        send_request: SendRequest<Bytes>,
        host: String,
        bytes_to_fetch: u64,
        total_counter: Arc<AtomicU64>,
        stop_signal: Arc<AtomicBool>,
        probe_pending: Arc<AtomicBool>,
    ) -> Result<Option<EdgeTelemetry>, std::io::Error> {
        let mut telemetry = None;

        while !stop_signal.load(Ordering::Relaxed) {
            let mut ready = match send_request.clone().ready().await {
                Ok(r) => r,
                Err(_) => break,
            };

            let path = crate::protocol::download_path(&host, bytes_to_fetch);
            let request = Request::builder()
                .method(Method::GET)
                .uri(format!("https://{}{}", host, path))
                .header("Host", &host)
                .header("User-Agent", "kiwi/1.0.0")
                .header("Accept", "*/*")
                .body(())
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

            let (response_fut, _) = match ready.send_request(request, true) {
                Ok(pair) => pair,
                Err(_) => break,
            };

            let response = match response_fut.await {
                Ok(r) => r,
                Err(_) => break,
            };

            let status = response.status();
            let is_success = status.is_success();

            if status == http::StatusCode::TOO_MANY_REQUESTS {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            } else if !is_success {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }

            if telemetry.is_none() && is_success {
                telemetry = Some(parse_edge_telemetry(response.headers()));
            }

            let mut body = response.into_body();
            let mut local_batch: u64 = 0;

            while let Some(chunk_res) = body.data().await {
                if stop_signal.load(Ordering::Relaxed) {
                    break;
                }

                let chunk = match chunk_res {
                    Ok(c) => c,
                    Err(_) => break,
                };

                let len = chunk.len();
                // Immediate WINDOW_UPDATE flow control frame
                let _ = body.flow_control().release_capacity(len);

                if is_success {
                    local_batch += len as u64;
                    if local_batch >= 65536 {
                        total_counter.fetch_add(local_batch, Ordering::Relaxed);
                        local_batch = 0;
                    }

                    // Cooperative yield if in-band probe is dispatched
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

    /// Run a bulk upload stream posting pre-seeded static slices.
    pub async fn run_upload_stream(
        send_request: SendRequest<Bytes>,
        host: String,
        total_counter: Arc<AtomicU64>,
        stop_signal: Arc<AtomicBool>,
        probe_pending: Arc<AtomicBool>,
    ) -> Result<Option<EdgeTelemetry>, std::io::Error> {
        let mut telemetry = None;
        let upload_chunk_slice = UPLOAD_BUFFER.clone();
        let chunk_size = 65536; // 64 KB sub-slice

        while !stop_signal.load(Ordering::Relaxed) {
            let mut ready = match send_request.clone().ready().await {
                Ok(r) => r,
                Err(_) => break,
            };

            let path = crate::protocol::upload_path(&host);
            let request = Request::builder()
                .method(Method::POST)
                .uri(format!("https://{}{}", host, path))
                .header("Host", &host)
                .header("User-Agent", "kiwi/1.0.0")
                .header("Content-Type", "application/octet-stream")
                .body(())
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

            let (response_fut, mut send_stream) = match ready.send_request(request, false) {
                Ok(pair) => pair,
                Err(_) => break,
            };

            let mut local_batch: u64 = 0;
            let mut offset = 0;

            // Stream upload chunks
            while !stop_signal.load(Ordering::Relaxed)
                && offset + chunk_size <= upload_chunk_slice.len()
            {
                // Reserve send capacity
                send_stream.reserve_capacity(chunk_size);
                match send_stream.capacity() {
                    0 => {
                        // Wait for flow control capacity
                        tokio::task::yield_now().await;
                    }
                    cap => {
                        let to_send = chunk_size.min(cap);
                        let sub_slice = upload_chunk_slice.slice(offset..offset + to_send);
                        if send_stream.send_data(sub_slice, false).is_err() {
                            break;
                        }
                        offset += to_send;
                        local_batch += to_send as u64;

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
            }

            let _ = send_stream.send_data(Bytes::new(), true);

            if local_batch > 0 {
                total_counter.fetch_add(local_batch, Ordering::Relaxed);
            }

            if let Ok(response) = response_fut.await {
                if response.status() == http::StatusCode::TOO_MANY_REQUESTS {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                if telemetry.is_none() && response.status().is_success() {
                    telemetry = Some(parse_edge_telemetry(response.headers()));
                }
            }
        }

        Ok(telemetry)
    }
}
