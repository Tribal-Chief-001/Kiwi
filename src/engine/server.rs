use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Runs a high-performance zero-allocation embedded speedtest daemon for private LAN / air-gapped benchmarking.
pub async fn run_server(port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let bind_addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;
    let listener = TcpListener::bind(bind_addr).await?;

    println!(
        "🥝 KIWI DAEMON: Embedded speedtest engine running on http://{}",
        bind_addr
    );
    println!("   Parity: iPerf3 / LibreSpeed private benchmark server");
    println!("   Endpoints: GET /down?bytes=N, POST /up, GET /health");
    println!("   To test: kiwi --host <this-ip> --port {} --plain", port);
    println!("   (Press Ctrl+C to stop)");

    let payload = Arc::new(vec![0xAAu8; 65536]); // Pre-allocated 64KB chunk buffer

    loop {
        let (socket, peer_addr) = listener.accept().await?;
        let payload_ref = payload.clone();
        tokio::spawn(async move {
            let _ = handle_client(socket, peer_addr, payload_ref).await;
        });
    }
}

async fn handle_client(
    mut stream: TcpStream,
    _peer: SocketAddr,
    chunk_buffer: Arc<Vec<u8>>,
) -> Result<(), std::io::Error> {
    let mut header_buf = vec![0u8; 8192];
    loop {
        // Read headers with an idle timeout of 30 seconds
        let n = match tokio::time::timeout(Duration::from_secs(30), stream.read(&mut header_buf))
            .await
        {
            Ok(Ok(n)) if n > 0 => n,
            _ => break,
        };

        // Find end of HTTP headers: \r\n\r\n (standard) or \n\n (loose)
        let header_end_offset = header_buf[..n]
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|pos| pos + 4)
            .or_else(|| {
                header_buf[..n]
                    .windows(2)
                    .position(|w| w == b"\n\n")
                    .map(|pos| pos + 2)
            });

        let header_end = match header_end_offset {
            Some(pos) => pos,
            None => {
                // Incomplete or non-HTTP payload
                break;
            }
        };

        let header_str = String::from_utf8_lossy(&header_buf[..header_end]);
        let first_line = header_str.lines().next().unwrap_or("");
        let is_conn_close = header_str.to_lowercase().contains("connection: close");

        if first_line.starts_with("GET /down") || first_line.starts_with("GET /__down") {
            // Parse requested bytes, capped at 1 GB per request
            let bytes_to_send: u64 = if let Some(query_start) = first_line.find("bytes=") {
                let rest = &first_line[query_start + 6..];
                let num_str = rest
                    .split(|c: char| !c.is_ascii_digit())
                    .next()
                    .unwrap_or("0");
                num_str
                    .parse::<u64>()
                    .unwrap_or(10_000_000)
                    .min(1_000_000_000)
            } else {
                10_000_000
            };

            let conn_hdr = if is_conn_close { "close" } else { "keep-alive" };
            let resp_header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
                bytes_to_send, conn_hdr
            );
            if stream.write_all(resp_header.as_bytes()).await.is_err() {
                break;
            }

            let mut remaining = bytes_to_send;
            while remaining > 0 {
                let to_write = (remaining as usize).min(chunk_buffer.len());
                if stream.write_all(&chunk_buffer[..to_write]).await.is_err() {
                    break;
                }
                remaining -= to_write as u64;
            }
            let _ = stream.flush().await;

            if is_conn_close {
                break;
            }
        } else if first_line.starts_with("POST /up") || first_line.starts_with("POST /__up") {
            // Parse Content-Length to drain
            let content_length: u64 = header_str
                .lines()
                .find(|l| l.to_lowercase().starts_with("content-length:"))
                .and_then(|l| l.split(':').nth(1))
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);

            // Account for any body bytes already read into header_buf past the headers
            let body_already_read = (n - header_end) as u64;
            let mut drained = body_already_read.min(content_length);

            // Read and discard remaining body payload
            let mut drain_buf = [0u8; 16384];
            while drained < content_length {
                let to_read = (content_length - drained).min(drain_buf.len() as u64) as usize;
                let bytes_read = match tokio::time::timeout(
                    Duration::from_secs(15),
                    stream.read(&mut drain_buf[..to_read]),
                )
                .await
                {
                    Ok(Ok(b)) if b > 0 => b,
                    _ => break,
                };
                drained += bytes_read as u64;
            }

            let conn_hdr = if is_conn_close { "close" } else { "keep-alive" };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\nOK",
                conn_hdr
            );
            if stream.write_all(resp.as_bytes()).await.is_err() {
                break;
            }
            let _ = stream.flush().await;

            if is_conn_close {
                break;
            }
        } else {
            let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 27\r\nAccess-Control-Allow-Origin: *\r\n\r\n🥝 Kiwi Server Running 1.0\n";
            let _ = stream.write_all(resp.as_bytes()).await;
            let _ = stream.flush().await;
            break;
        }
    }
    Ok(())
}
