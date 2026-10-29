use crate::metrics::{EdgeTelemetry, LatencyStats};
use crate::protocol::dns::DnsCandidates;
use crate::protocol::h2::H2Client;
use crate::protocol::happy_eyeballs::connect_happy_eyeballs;
use crate::protocol::http1::Http1Client;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tokio::time::sleep;

use crate::metrics::KernelTcpInfo;
use crate::protocol::h3::H3Client;

pub enum ClientHandle {
    H2(H2Client),
    Http1(Box<Http1Client>),
    H3(H3Client),
}

impl ClientHandle {
    pub async fn dispatch_probe(
        &mut self,
        probe_pending: &AtomicBool,
    ) -> Result<(f64, Option<EdgeTelemetry>), std::io::Error> {
        match self {
            ClientHandle::H2(h2) => h2.dispatch_probe(probe_pending).await,
            ClientHandle::Http1(h1) => h1.dispatch_probe(probe_pending).await,
            ClientHandle::H3(h3) => h3.dispatch_probe(probe_pending).await,
        }
    }

    pub fn raw_fd(&self) -> Option<i32> {
        match self {
            ClientHandle::H2(h2) => h2.raw_fd,
            ClientHandle::Http1(h1) => h1.raw_fd,
            ClientHandle::H3(_) => None,
        }
    }

    pub fn read_kernel_tcp_info(&self) -> Option<KernelTcpInfo> {
        #[cfg(unix)]
        {
            self.raw_fd().and_then(crate::platform::read_tcp_info)
        }
        #[cfg(not(unix))]
        {
            None
        }
    }
}

/// Run baseline unloaded idle latency test via sequential micro-queries.
pub async fn run_idle_latency_test(
    client: &mut ClientHandle,
    probe_count: usize,
    probe_pending: &AtomicBool,
    host: &str,
    candidates: &DnsCandidates,
    specified_iface: Option<&str>,
) -> (LatencyStats, Option<EdgeTelemetry>) {
    run_idle_latency_test_with_progress(
        client,
        probe_count,
        probe_pending,
        host,
        candidates,
        specified_iface,
        |_, _, _| (),
    )
    .await
}

/// Run baseline unloaded idle latency test with live per-probe progress updates.
pub async fn run_idle_latency_test_with_progress(
    client: &mut ClientHandle,
    probe_count: usize,
    probe_pending: &AtomicBool,
    host: &str,
    candidates: &DnsCandidates,
    specified_iface: Option<&str>,
    mut on_probe: impl FnMut(usize, f64, &Option<EdgeTelemetry>),
) -> (LatencyStats, Option<EdgeTelemetry>) {
    let mut samples = Vec::with_capacity(probe_count);
    let mut edge_telemetry = None;

    for i in 0..probe_count {
        match client.dispatch_probe(probe_pending).await {
            Ok((rtt_ms, telemetry)) => {
                samples.push(rtt_ms);
                if edge_telemetry.is_none() && telemetry.is_some() {
                    edge_telemetry = telemetry;
                }
                if let Some(ref mut telem) = edge_telemetry
                    && let Some(kinfo) = client.read_kernel_tcp_info()
                {
                    telem.kernel_tcp = Some(kinfo);
                    if telem.cf_l4_cwnd.is_none() && kinfo.snd_cwnd > 0 {
                        telem.cf_l4_cwnd = Some(kinfo.snd_cwnd);
                    }
                    if telem.cf_l4_rtt_us.is_none() && kinfo.rtt_us > 0 {
                        telem.cf_l4_rtt_us = Some(kinfo.rtt_us as u64);
                    }
                    if telem.cf_l4_min_rtt_us.is_none() && kinfo.min_rtt_us > 0 {
                        telem.cf_l4_min_rtt_us = Some(kinfo.min_rtt_us as u64);
                    }
                    if telem.cf_l4_retrans.is_none() {
                        telem.cf_l4_retrans = Some(kinfo.total_retrans.max(kinfo.retrans));
                    }
                }
                on_probe(i + 1, rtt_ms, &edge_telemetry);
            }
            Err(_) => {
                // If idle probe dropped or timed out under severe packet loss,
                // re-establish connection so subsequent idle probes have a fresh socket.
                match client {
                    ClientHandle::H3(_) => {
                        if let Ok(h3) = H3Client::connect(host, candidates).await {
                            *client = ClientHandle::H3(h3);
                        }
                    }
                    _ => {
                        if let Ok(conn) = connect_happy_eyeballs(host, candidates, specified_iface).await {
                            let is_h2 = conn.alpn.as_deref().map(|p| p == b"h2").unwrap_or(false);
                            if is_h2 {
                                if let Ok(h2) = H2Client::new(conn.stream, host.to_string()).await {
                                    *client = ClientHandle::H2(h2);
                                }
                            } else {
                                *client = ClientHandle::Http1(Box::new(Http1Client::new(
                                    conn.stream,
                                    host.to_string(),
                                )));
                            }
                        }
                    }
                }
            }
        }
        // Micro pacing between idle probes
        sleep(Duration::from_millis(50)).await;
    }

    let stats = LatencyStats::calculate(samples, probe_count);
    (stats, edge_telemetry)
}
