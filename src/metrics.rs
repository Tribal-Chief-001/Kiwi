use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct EdgeTelemetry {
    pub colo: String,
    pub asn: Option<u32>,
    pub city: String,
    pub country: String,
    pub client_ip: String,
    pub cf_l4_proto: Option<String>,
    pub cf_l4_rtt_us: Option<u64>,
    pub cf_l4_min_rtt_us: Option<u64>,
    pub cf_l4_cwnd: Option<u32>,
    pub cf_l4_retrans: Option<u32>,
    pub cf_l4_delivery_rate: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct LatencyStats {
    pub min_ms: f64,
    pub median_ms: f64,
    pub p90_ms: f64,
    pub jitter_ms: f64,
    pub samples: Vec<f64>,
    pub packet_loss_pct: f64,
}

impl LatencyStats {
    pub fn calculate(mut samples: Vec<f64>, total_probes: usize) -> Self {
        if samples.is_empty() {
            return Self {
                packet_loss_pct: if total_probes > 0 { 100.0 } else { 0.0 },
                ..Default::default()
            };
        }

        let packet_loss_pct = if total_probes > 0 {
            (total_probes.saturating_sub(samples.len()) as f64 / total_probes as f64) * 100.0
        } else {
            0.0
        };

        // Calculate jitter (mean absolute deviation between consecutive samples)
        let jitter_ms = if samples.len() > 1 {
            let sum_diff: f64 = samples
                .windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .sum();
            sum_diff / (samples.len() - 1) as f64
        } else {
            0.0
        };

        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let min_ms = samples[0];
        let median_ms = if samples.len() % 2 == 1 {
            samples[samples.len() / 2]
        } else {
            (samples[samples.len() / 2 - 1] + samples[samples.len() / 2]) / 2.0
        };

        let p90_index = ((samples.len() as f64 * 0.90).ceil() as usize).saturating_sub(1);
        let p90_ms = samples[p90_index.min(samples.len() - 1)];

        Self {
            min_ms,
            median_ms,
            p90_ms,
            jitter_ms,
            samples,
            packet_loss_pct,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ThroughputStats {
    pub bps: f64,
    pub mbps: f64,
    pub peak_mbps: f64,
    pub total_bytes: u64,
    pub duration_secs: f64,
    pub history: Vec<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct BufferbloatGrade {
    pub grade: String,
    pub rpm: u32,
    pub working_rtt_ms: f64,
    pub delta_ms: f64,
    pub rating_label: String,
}

impl BufferbloatGrade {
    pub fn compute(idle_median_ms: f64, down_p90_ms: f64, up_p90_ms: Option<f64>) -> Self {
        let working_rtt_ms = match up_p90_ms {
            Some(up) if up > 0.0 => (down_p90_ms + up) / 2.0,
            _ => down_p90_ms,
        };

        let rpm = if working_rtt_ms > 0.0 {
            (60_000.0 / working_rtt_ms).round() as u32
        } else {
            0
        };

        let delta_ms = (working_rtt_ms - idle_median_ms).max(0.0);

        let (grade, rating_label) = if rpm >= 1200 && delta_ms <= 15.0 {
            ("A+".to_string(), "ULTRA RESPONSIVE".to_string())
        } else if rpm >= 800 && delta_ms <= 35.0 {
            ("A".to_string(), "EXCELLENT".to_string())
        } else if rpm >= 400 && delta_ms <= 80.0 {
            ("B".to_string(), "ACCEPTABLE".to_string())
        } else if rpm >= 150 && delta_ms <= 200.0 {
            ("C".to_string(), "NOTICEABLE BUFFERBLOAT".to_string())
        } else {
            ("F".to_string(), "SEVERE BUFFERBLOAT".to_string())
        };

        Self {
            grade,
            rpm,
            working_rtt_ms,
            delta_ms,
            rating_label,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct PlatformInfo {
    pub interface_name: String,
    pub link_speed_mbps: Option<u64>,
    pub link_type: String,
    pub is_virtual: bool,
    pub bottleneck_warning: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ResourceProfile {
    pub cold_start_ms: f64,
    pub peak_rss_mb: f64,
    pub duration_secs: f64,
}

impl ResourceProfile {
    /// Read current process Resident Set Size (RSS) in Megabytes
    pub fn current_rss_mb() -> f64 {
        #[cfg(target_os = "linux")]
        {
            if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
                let parts: Vec<&str> = statm.split_whitespace().collect();
                if parts.len() >= 2
                    && let Ok(pages) = parts[1].parse::<u64>()
                {
                    // Linux sysconf(_SC_PAGESIZE) is typically 4096 bytes
                    return (pages * 4096) as f64 / (1024.0 * 1024.0);
                }
            }
        }
        // Fallback baseline for non-Linux or container restricted
        3.2
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct BenchmarkReport {
    pub version: String,
    pub timestamp: String,
    pub target_host: String,
    pub protocol: String,
    pub edge: EdgeTelemetry,
    pub idle_latency: LatencyStats,
    pub download: ThroughputStats,
    pub loaded_down_latency: LatencyStats,
    pub upload: ThroughputStats,
    pub loaded_up_latency: LatencyStats,
    pub responsiveness: BufferbloatGrade,
    pub platform: PlatformInfo,
    pub resources: ResourceProfile,
}
