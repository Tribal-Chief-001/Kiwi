pub mod dns;
pub mod h2;
pub mod happy_eyeballs;
pub mod http1;
pub mod telemetry;
pub mod tls;

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
