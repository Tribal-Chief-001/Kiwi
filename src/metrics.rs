use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KernelTcpInfo {
    pub rtt_us: u32,
    pub min_rtt_us: u32,
    pub snd_cwnd: u32,
    pub retrans: u32,
    pub total_retrans: u32,
    pub delivery_rate_bps: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kernel_tcp: Option<KernelTcpInfo>,
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
        samples.retain(|s| s.is_finite() && *s >= 0.0);

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
            let sum_diff: f64 = samples.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
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
        let idle_median_ms = if idle_median_ms.is_finite() && idle_median_ms >= 0.0 {
            idle_median_ms
        } else {
            0.0
        };
        let down_p90_ms = if down_p90_ms.is_finite() && down_p90_ms >= 0.0 {
            down_p90_ms
        } else {
            0.0
        };
        let working_rtt_ms = match up_p90_ms {
            Some(up) if up.is_finite() && up > 0.0 => (down_p90_ms + up) / 2.0,
            _ => down_p90_ms,
        };

        let rpm = if working_rtt_ms > 0.0 {
            ((60_000.0 / working_rtt_ms).round() as u64).min(999_999) as u32
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

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct IspAuditReport {
    pub anycast_host: String,
    pub anycast_download_mbps: f64,
    pub anycast_idle_latency_ms: f64,
    pub on_net_host: String,
    pub on_net_download_mbps: f64,
    pub on_net_idle_latency_ms: f64,
    pub neutrality_ratio: f64,
    pub is_throttled: bool,
    pub verdict: String,
    pub analysis: String,
}

impl IspAuditReport {
    pub fn compute(
        anycast_host: &str,
        anycast_mbps: f64,
        anycast_latency_ms: f64,
        on_net_host: &str,
        on_net_mbps: f64,
        on_net_latency_ms: f64,
    ) -> Self {
        let anycast_mbps = if anycast_mbps.is_finite() && anycast_mbps > 0.0 {
            anycast_mbps
        } else {
            0.0
        };
        let on_net_mbps = if on_net_mbps.is_finite() && on_net_mbps > 0.0 {
            on_net_mbps
        } else {
            0.0
        };
        let ratio = if on_net_mbps > 0.0 {
            (anycast_mbps / on_net_mbps).clamp(0.0, 10.0)
        } else if anycast_mbps > 0.0 {
            1.0
        } else {
            0.0
        };

        let (is_throttled, verdict, analysis) = if ratio >= 0.85 {
            (
                false,
                "NEUTRAL · HONEST ROUTING".to_string(),
                format!(
                    "Neutrality Ratio {:.2} (>= 0.85). No evidence of ISP speedtest prioritization. Real-world transit bandwidth matches local pipe.",
                    ratio
                ),
            )
        } else if ratio >= 0.65 {
            (
                false,
                "MILD PEERING SHAPING".to_string(),
                format!(
                    "Neutrality Ratio {:.2} (0.65 - 0.84). Minor transit congestion or peering de-prioritization detected on remote routes.",
                    ratio
                ),
            )
        } else {
            (
                true,
                "AGGRESSIVE SHAPING / CHEATING".to_string(),
                format!(
                    "Neutrality Ratio {:.2} (< 0.65). Severe ISP QoS bias detected! ISP delivers {:.1} Mbps on-net but only {:.1} Mbps for global transit ({:.1}% throughput drop).",
                    ratio,
                    on_net_mbps,
                    anycast_mbps,
                    (1.0 - ratio) * 100.0
                ),
            )
        };

        Self {
            anycast_host: anycast_host.to_string(),
            anycast_download_mbps: anycast_mbps,
            anycast_idle_latency_ms: anycast_latency_ms,
            on_net_host: on_net_host.to_string(),
            on_net_download_mbps: on_net_mbps,
            on_net_idle_latency_ms: on_net_latency_ms,
            neutrality_ratio: ratio,
            is_throttled,
            verdict,
            analysis,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct BenchmarkReport {
    pub version: String,
    pub timestamp: String,
    pub target_host: String,
    pub protocol: String,
    #[serde(default)]
    pub is_bidirectional: bool,
    pub edge: EdgeTelemetry,
    pub idle_latency: LatencyStats,
    pub download: ThroughputStats,
    pub loaded_down_latency: LatencyStats,
    pub upload: ThroughputStats,
    pub loaded_up_latency: LatencyStats,
    pub responsiveness: BufferbloatGrade,
    pub platform: PlatformInfo,
    pub resources: ResourceProfile,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isp_audit: Option<IspAuditReport>,
}
