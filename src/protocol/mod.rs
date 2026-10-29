pub mod dns;
pub mod h2;
pub mod h3;
pub mod happy_eyeballs;
pub mod http1;
pub mod stream;
pub mod telemetry;
pub mod tls;

pub use stream::AnyStream;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Maximum duration permitted for an individual in-band or idle latency probe.
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(1200);

/// RAII guard ensuring cooperative yield flag is reliably de-asserted on completion,
/// error, or timeout cancellation under severe packet loss.
pub struct ProbePendingGuard<'a> {
    flag: &'a AtomicBool,
}

impl<'a> ProbePendingGuard<'a> {
    pub fn activate(flag: &'a AtomicBool) -> Self {
        flag.store(true, Ordering::Release);
        Self { flag }
    }
}

impl<'a> Drop for ProbePendingGuard<'a> {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
}

/// Returns the download path for a given host and byte count.
/// Fastly / EdgeCompute endpoints use `/down?bytes=N`, whereas Cloudflare uses `/__down?bytes=N`.
pub fn download_path(host: &str, bytes: u64) -> String {
    if host.contains("fastly") || host.contains("edgecompute") {
        format!("/down?bytes={}", bytes)
    } else {
        format!("/__down?bytes={}", bytes)
    }
}

/// Returns the upload path for a given host.
/// Fastly / EdgeCompute endpoints use `/up`, whereas Cloudflare uses `/__up`.
pub fn upload_path(host: &str) -> &'static str {
    if host.contains("fastly") || host.contains("edgecompute") {
        "/up"
    } else {
        "/__up"
    }
}
