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

#[derive(Debug, Clone)]
pub struct HttpResponseMeta {
    pub status_code: u16,
    pub headers: HeaderMap,
    pub content_length: Option<u64>,
    pub is_chunked: bool,
    pub connection_close: bool,
}

pub struct Http1Client {
    pub stream: TlsStream<TcpStream>,
    pub host: String,
    pub read_buf: Vec<u8>,
    pub raw_fd: Option<i32>,
}

impl Http1Client {
    pub fn new(stream: TlsStream<TcpStream>, host: String) -> Self {
        #[cfg(unix)]
        let raw_fd = {
            use std::os::unix::io::AsRawFd;
            Some(stream.get_ref().0.as_raw_fd())
        };
        #[cfg(not(unix))]
        let raw_fd = None;

        Http1Client {
            stream,
            host,
            read_buf: Vec::with_capacity(4096),
            raw_fd,
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

        let path = crate::protocol::download_path(&self.host, 0);
        let req = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: kiwi/1.0.0\r\nAccept: */*\r\nConnection: keep-alive\r\n\r\n",
            path, self.host
        );

        let start = Instant::now();
        self.stream.write_all(req.as_bytes()).await?;
        self.stream.flush().await?;

        // Probe placed on wire; release yield
        drop(guard);

        let meta = self.read_response_headers().await?;
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

        let telemetry = parse_edge_telemetry(&meta.headers);

        // Always drain response body cleanly to preserve keep-alive framing
        self.drain_body(meta.is_chunked, meta.content_length)
            .await?;

        if meta.status_code < 200 || meta.status_code >= 300 {
            return Err(std::io::Error::other(format!(
                "HTTP/1.1 probe failed with HTTP status {}",
                meta.status_code
            )));
        }

        Ok((elapsed_ms, Some(telemetry)))
    }

    /// Run bulk download stream supporting both Content-Length and Chunked Transfer Encoding
    pub async fn run_download_stream(
        mut self,
        bytes_per_req: u64,
        total_counter: Arc<AtomicU64>,
        stop_signal: Arc<AtomicBool>,
        probe_pending: Arc<AtomicBool>,
    ) -> Result<Option<EdgeTelemetry>, std::io::Error> {
        let mut telemetry = None;
        let mut sink = [0u8; 16384];

        let path = crate::protocol::download_path(&self.host, bytes_per_req);
        while !stop_signal.load(Ordering::Relaxed) {
            let req = format!(
                "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: kiwi/1.0.0\r\nAccept: */*\r\nConnection: keep-alive\r\n\r\n",
                path, self.host
            );

            if self.stream.write_all(req.as_bytes()).await.is_err() {
                break;
            }
            if self.stream.flush().await.is_err() {
                break;
            }

            let meta = match self.read_response_headers().await {
                Ok(res) => res,
                Err(_) => break,
            };

            let is_success = meta.status_code >= 200 && meta.status_code < 300;

            if meta.status_code == 429 {
                // Rate limited: back off and drain body without counting
                let _ = self.drain_body(meta.is_chunked, meta.content_length).await;
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                if meta.connection_close {
                    break;
                }
                continue;
            }

            if !is_success {
                // Server error / unexpected status: back off slightly and drain body
                let _ = self.drain_body(meta.is_chunked, meta.content_length).await;
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                if meta.connection_close {
                    break;
                }
                continue;
            }

            if telemetry.is_none() {
                telemetry = Some(parse_edge_telemetry(&meta.headers));
            }

            let mut local_batch: u64 = 0;

            if meta.is_chunked {
                // Decode Chunked Transfer Encoding
                loop {
                    if stop_signal.load(Ordering::Relaxed) {
                        break;
                    }
                    let chunk_size = match self.read_chunk_size().await {
                        Ok(sz) => sz,
                        Err(_) => break,
                    };
                    if chunk_size == 0 {
                        // Read trailing CRLF and optional trailers
                        let _ = self.drain_chunked_trailers().await;
                        break;
                    }

                    let mut remaining = chunk_size as u64;
                    // Drain chunk payload
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
                            Ok(0) => break,
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

                    if remaining > 0 {
                        break;
                    }

                    // Read chunk trailing CRLF
                    let _ = self.read_exact_bytes(2).await;
                }

                if meta.connection_close {
                    break;
                }
            } else if let Some(mut remaining) = meta.content_length {
                // Fixed Content-Length
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

                if remaining > 0 || meta.connection_close {
                    // Stopped before consuming whole body, break keep-alive
                    break;
                }
            } else {
                // Undefined Content-Length and not chunked: read until EOF or stop
                while !stop_signal.load(Ordering::Relaxed) {
                    let n = match self.stream.read(&mut sink).await {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(_) => break,
                    };
                    local_batch += n as u64;
                    if local_batch >= 65536 {
                        total_counter.fetch_add(local_batch, Ordering::Relaxed);
                        local_batch = 0;
                    }
                }
                break;
            }

            if local_batch > 0 {
                total_counter.fetch_add(local_batch, Ordering::Relaxed);
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

        let path = crate::protocol::upload_path(&self.host);
        while !stop_signal.load(Ordering::Relaxed) {
            let req_header = format!(
                "POST {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: kiwi/1.0.0\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                path, self.host, req_size
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

            let meta = match self.read_response_headers().await {
                Ok(res) => res,
                Err(_) => break,
            };

            let is_success = meta.status_code >= 200 && meta.status_code < 300;

            if meta.status_code == 429 {
                let _ = self.drain_body(meta.is_chunked, meta.content_length).await;
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                if meta.connection_close {
                    break;
                }
                continue;
            }

            if !is_success {
                let _ = self.drain_body(meta.is_chunked, meta.content_length).await;
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                if meta.connection_close {
                    break;
                }
                continue;
            }

            if telemetry.is_none() && is_success {
                telemetry = Some(parse_edge_telemetry(&meta.headers));
            }

            // Drain upload response body
            if self
                .drain_body(meta.is_chunked, meta.content_length)
                .await
                .is_err()
                || meta.connection_close
            {
                break;
            }
        }

        Ok(telemetry)
    }

    /// Read HTTP/1.1 response header block until \r\n\r\n and return parsed HttpResponseMeta
    pub async fn read_response_headers(&mut self) -> Result<HttpResponseMeta, std::io::Error> {
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

        let status_line = header_text
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        let status_code: u16 = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(200);

        let mut headers = HeaderMap::new();
        let mut content_length = None;
        let mut is_chunked = false;
        let mut connection_close = false;

        let after_status = header_text
            .lines()
            .skip_while(|l| l.trim().is_empty())
            .skip(1);

        for line in after_status {
            if let Some((k, v)) = line.split_once(':') {
                let key = k.trim();
                let val = v.trim();

                if key.eq_ignore_ascii_case("content-length") {
                    content_length = val.parse().ok();
                } else if key.eq_ignore_ascii_case("transfer-encoding")
                    && val.to_lowercase().contains("chunked")
                {
                    is_chunked = true;
                } else if key.eq_ignore_ascii_case("connection")
                    && val.to_lowercase().contains("close")
                {
                    connection_close = true;
                }

                if let (Ok(h_key), Ok(h_val)) = (
                    http::header::HeaderName::from_bytes(key.as_bytes()),
                    http::HeaderValue::from_str(val),
                ) {
                    headers.append(h_key, h_val);
                }
            }
        }

        Ok(HttpResponseMeta {
            status_code,
            headers,
            content_length,
            is_chunked,
            connection_close,
        })
    }

    async fn fill_buf_until_crlf(&mut self) -> Result<usize, std::io::Error> {
        let mut temp = [0u8; 512];
        while !self.read_buf.windows(2).any(|w| w == b"\r\n") {
            let n = self.stream.read(&mut temp).await?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "EOF while searching for CRLF in chunked stream",
                ));
            }
            self.read_buf.extend_from_slice(&temp[..n]);
        }
        let pos = self.read_buf.windows(2).position(|w| w == b"\r\n").unwrap();
        Ok(pos)
    }

    async fn read_chunk_size(&mut self) -> Result<usize, std::io::Error> {
        let pos = self.fill_buf_until_crlf().await?;
        let line_bytes: Vec<u8> = self.read_buf.drain(..pos + 2).collect();
        let line_str = std::str::from_utf8(&line_bytes[..pos])
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let hex_part = line_str.split(';').next().unwrap_or("").trim();
        usize::from_str_radix(hex_part, 16)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    async fn read_exact_bytes(&mut self, mut count: usize) -> Result<Vec<u8>, std::io::Error> {
        let mut out = Vec::with_capacity(count);
        if !self.read_buf.is_empty() {
            let take = self.read_buf.len().min(count);
            out.extend(self.read_buf.drain(..take));
            count -= take;
        }
        let mut temp = [0u8; 512];
        while count > 0 {
            let to_read = count.min(temp.len());
            let n = self.stream.read(&mut temp[..to_read]).await?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "EOF while reading exact bytes",
                ));
            }
            out.extend_from_slice(&temp[..n]);
            count -= n;
        }
        Ok(out)
    }

    async fn drain_chunked_trailers(&mut self) -> Result<(), std::io::Error> {
        loop {
            let pos = self.fill_buf_until_crlf().await?;
            self.read_buf.drain(..pos + 2);
            if pos == 0 {
                // Empty CRLF line marks the end of trailer-section
                break;
            }
        }
        Ok(())
    }

    async fn drain_body(
        &mut self,
        is_chunked: bool,
        content_length: Option<u64>,
    ) -> Result<(), std::io::Error> {
        if is_chunked {
            loop {
                let sz = self.read_chunk_size().await?;
                if sz == 0 {
                    self.drain_chunked_trailers().await?;
                    break;
                }
                let _ = self.read_exact_bytes(sz).await?;
                let _ = self.read_exact_bytes(2).await?; // Trailing CRLF
            }
        } else if let Some(mut remaining) = content_length {
            if !self.read_buf.is_empty() {
                let take = (self.read_buf.len() as u64).min(remaining);
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
                        "EOF while draining HTTP response body",
                    ));
                }
                remaining -= n as u64;
            }
        }
        Ok(())
    }
}
