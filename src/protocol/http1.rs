use crate::metrics::EdgeTelemetry;
use crate::protocol::telemetry::parse_edge_telemetry;
use crate::protocol::{PROBE_TIMEOUT, ProbePendingGuard};
use http::HeaderMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

pub struct Http1Client {
    pub stream: TlsStream<TcpStream>,
    pub host: String,
    pub read_buf: Vec<u8>,
}

impl Http1Client {
    pub fn new(stream: TlsStream<TcpStream>, host: String) -> Self {
        Http1Client {
            stream,
            host,
            read_buf: Vec::with_capacity(4096),
        }
    }

    /// Dispatch a single probe to measure RTT over HTTP/1.1 keep-alive connection
    pub async fn dispatch_probe(
        &mut self,
        probe_pending: &AtomicBool,
    ) -> Result<(f64, Option<EdgeTelemetry>), std::io::Error> {
        tokio::time::timeout(PROBE_TIMEOUT, self.dispatch_probe_inner(probe_pending))
            .await
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "HTTP/1.1 probe request timed out under severe network latency/loss",
                )
            })?
    }

    async fn dispatch_probe_inner(
        &mut self,
        probe_pending: &AtomicBool,
    ) -> Result<(f64, Option<EdgeTelemetry>), std::io::Error> {
        let guard = ProbePendingGuard::activate(probe_pending);
        tokio::task::yield_now().await;

        let req = format!(
            "GET /__down?bytes=0 HTTP/1.1\r\nHost: {}\r\nUser-Agent: kiwi/1.0.0\r\nAccept: */*\r\nConnection: keep-alive\r\n\r\n",
            self.host
        );

        let start = Instant::now();
        self.stream.write_all(req.as_bytes()).await?;
        self.stream.flush().await?;

        // Probe placed on wire; release yield
        drop(guard);

        let (headers, content_length) = self.read_response_headers().await?;
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

        let telemetry = parse_edge_telemetry(&headers);

        // Discard any response body if content_length > 0
        let mut remaining = content_length;
        if !self.read_buf.is_empty() {
            let buffered = self.read_buf.len() as u64;
            let take = buffered.min(remaining);
            remaining -= take;
            self.read_buf.drain(..take as usize);
        }
        let mut sink = [0u8; 4096];
        while remaining > 0 {
            let to_read = remaining.min(sink.len() as u64) as usize;
            let n = self.stream.read(&mut sink[..to_read]).await?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "Socket closed prematurely while reading probe response body",
                ));
            }
            remaining -= n as u64;
        }

        Ok((elapsed_ms, Some(telemetry)))
    }

    /// Run bulk download stream using L1d stack sink [u8; 16384] and task-local batching
    pub async fn run_download_stream(
        mut self,
        bytes_per_req: u64,
        total_counter: Arc<AtomicU64>,
        stop_signal: Arc<AtomicBool>,
        probe_pending: Arc<AtomicBool>,
    ) -> Result<Option<EdgeTelemetry>, std::io::Error> {
        let mut telemetry = None;
        let mut sink = [0u8; 16384];

        while !stop_signal.load(Ordering::Relaxed) {
            let req = format!(
                "GET /__down?bytes={} HTTP/1.1\r\nHost: {}\r\nUser-Agent: kiwi/1.0.0\r\nAccept: */*\r\nConnection: keep-alive\r\n\r\n",
                bytes_per_req, self.host
            );

            if self.stream.write_all(req.as_bytes()).await.is_err() {
                break;
            }
            if self.stream.flush().await.is_err() {
                break;
            }

            let (headers, content_length) = match self.read_response_headers().await {
                Ok(res) => res,
                Err(_) => break,
            };

            if telemetry.is_none() {
                telemetry = Some(parse_edge_telemetry(&headers));
            }

            let mut remaining = content_length;
            let mut local_batch: u64 = 0;

            if !self.read_buf.is_empty() {
                let buffered = self.read_buf.len() as u64;
                let take = buffered.min(remaining);
                remaining -= take;
                local_batch += take;
                self.read_buf.drain(..take as usize);
            }

            while remaining > 0 && !stop_signal.load(Ordering::Relaxed) {
                let to_read = remaining.min(sink.len() as u64) as usize;
                let n = match self.stream.read(&mut sink[..to_read]).await {
                    Ok(0) => break, // EOF
                    Ok(n) => n,
                    Err(_) => break,
                };

                remaining -= n as u64;
                local_batch += n as u64;

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

            if local_batch > 0 {
                total_counter.fetch_add(local_batch, Ordering::Relaxed);
            }

            if remaining > 0 {
                // Stopped before consuming whole body, break keep-alive
                break;
            }
        }

        Ok(telemetry)
    }

    /// Run bulk upload stream posting pre-seeded static slices
    pub async fn run_upload_stream(
        mut self,
        total_counter: Arc<AtomicU64>,
        stop_signal: Arc<AtomicBool>,
        probe_pending: Arc<AtomicBool>,
    ) -> Result<Option<EdgeTelemetry>, std::io::Error> {
        let mut telemetry = None;
        let upload_data = &crate::protocol::h2::UPLOAD_BUFFER;
        let req_size = 131072; // 128 KB per upload chunk request
        let chunk_size = 32768; // 32 KB write slices

        while !stop_signal.load(Ordering::Relaxed) {
            let req_header = format!(
                "POST /__up HTTP/1.1\r\nHost: {}\r\nUser-Agent: kiwi/1.0.0\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                self.host, req_size
            );

            if self.stream.write_all(req_header.as_bytes()).await.is_err() {
                break;
            }

            let mut offset = 0;
            let mut local_batch: u64 = 0;

            while offset < req_size && !stop_signal.load(Ordering::Relaxed) {
                let end = (offset + chunk_size).min(req_size);
                let slice = &upload_data[offset..end];

                if self.stream.write_all(slice).await.is_err() {
                    break;
                }
                let len = slice.len();
                offset += len;
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

            let _ = self.stream.flush().await;

            if local_batch > 0 {
                total_counter.fetch_add(local_batch, Ordering::Relaxed);
            }

            if offset < req_size {
                // If cancelled before finishing Content-Length, drop socket and exit cleanly
                break;
            }

            let (headers, content_length) = match self.read_response_headers().await {
                Ok(res) => res,
                Err(_) => break,
            };

            if telemetry.is_none() {
                telemetry = Some(parse_edge_telemetry(&headers));
            }

            // Drain upload response body
            let mut remaining = content_length;
            if !self.read_buf.is_empty() {
                let buffered = self.read_buf.len() as u64;
                let take = buffered.min(remaining);
                remaining -= take;
                self.read_buf.drain(..take as usize);
            }
            let mut sink = [0u8; 1024];
            while remaining > 0 {
                let to_read = remaining.min(sink.len() as u64) as usize;
                let n = match self.stream.read(&mut sink[..to_read]).await {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break,
                };
                remaining -= n as u64;
            }
        }

        Ok(telemetry)
    }

    /// Read HTTP/1.1 response header block until \r\n\r\n and return parsed HeaderMap & Content-Length
    async fn read_response_headers(&mut self) -> Result<(HeaderMap, u64), std::io::Error> {
        let mut buf = [0u8; 1024];
        let mut header_bytes = Vec::with_capacity(2048);

        // If leftover in read_buf, drain it
        if !self.read_buf.is_empty() {
            header_bytes.extend_from_slice(&self.read_buf);
            self.read_buf.clear();
        }

        let delimiter = b"\r\n\r\n";
        let mut split_pos = None;

        while split_pos.is_none() {
            if let Some(pos) = header_bytes.windows(4).position(|w| w == delimiter) {
                split_pos = Some(pos);
                break;
            }

            let n = self.stream.read(&mut buf).await?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "EOF while reading HTTP response headers",
                ));
            }
            header_bytes.extend_from_slice(&buf[..n]);
        }

        let pos = split_pos.unwrap();
        let header_slice = &header_bytes[..pos];
        let body_start = pos + 4;

        // Save any leftover body bytes into read_buf
        if body_start < header_bytes.len() {
            self.read_buf.extend_from_slice(&header_bytes[body_start..]);
        }

        let header_text = std::str::from_utf8(header_slice)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let mut headers = HeaderMap::new();
        let mut content_length = 0u64;

        for line in header_text.lines().skip(1) {
            if let Some((k, v)) = line.split_once(':') {
                let key = k.trim();
                let val = v.trim();

                if key.eq_ignore_ascii_case("content-length") {
                    content_length = val.parse().unwrap_or(0);
                }

                if let (Ok(h_key), Ok(h_val)) = (
                    http::header::HeaderName::from_bytes(key.as_bytes()),
                    http::HeaderValue::from_str(val),
                ) {
                    headers.append(h_key, h_val);
                }
            }
        }

        // If read_buf already has bytes, account for them in content_length
        if !self.read_buf.is_empty() {
            // Already buffered in read_buf
        }

        Ok((headers, content_length))
    }
}
