use crate::metrics::{EdgeTelemetry, LatencyStats};
use crate::protocol::h2::H2Client;
use crate::protocol::http1::Http1Client;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tokio::time::sleep;

pub enum ClientHandle {
    H2(H2Client),
    Http1(Box<Http1Client>),
}

impl ClientHandle {
    pub async fn dispatch_probe(
        &mut self,
        probe_pending: &AtomicBool,
    ) -> Result<(f64, Option<EdgeTelemetry>), std::io::Error> {
        match self {
            ClientHandle::H2(h2) => h2.dispatch_probe(probe_pending).await,
            ClientHandle::Http1(h1) => h1.dispatch_probe(probe_pending).await,
        }
    }
}

/// Run baseline unloaded idle latency test via sequential micro-queries.
pub async fn run_idle_latency_test(
    client: &mut ClientHandle,
    probe_count: usize,
    probe_pending: &AtomicBool,
) -> (LatencyStats, Option<EdgeTelemetry>) {
    let mut samples = Vec::with_capacity(probe_count);
    let mut edge_telemetry = None;

    for _ in 0..probe_count {
        if let Ok((rtt_ms, telemetry)) = client.dispatch_probe(probe_pending).await {
            samples.push(rtt_ms);
            if edge_telemetry.is_none() && telemetry.is_some() {
                edge_telemetry = telemetry;
            }
        }
        // Micro pacing between idle probes
        sleep(Duration::from_millis(50)).await;
    }

    let stats = LatencyStats::calculate(samples, probe_count);
    (stats, edge_telemetry)
}
