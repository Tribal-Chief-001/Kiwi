use crate::metrics::BenchmarkReport;

/// Formats a complete BenchmarkReport into standard Prometheus exposition format (text 0.0.4).
pub fn format_prometheus(report: &BenchmarkReport) -> String {
    let mut out = String::with_capacity(2048);
    let host = &report.target_host;
    let iface = &report.platform.interface_name;

    // Download metrics
    out.push_str(
        "# HELP kiwi_download_mbps Saturated download throughput in megabits per second\n",
    );
    out.push_str("# TYPE kiwi_download_mbps gauge\n");
    out.push_str(&format!(
        "kiwi_download_mbps{{target=\"{}\",interface=\"{}\"}} {:.2}\n",
        host, iface, report.download.mbps
    ));

    out.push_str(
        "# HELP kiwi_download_peak_mbps Peak download throughput in megabits per second\n",
    );
    out.push_str("# TYPE kiwi_download_peak_mbps gauge\n");
    out.push_str(&format!(
        "kiwi_download_peak_mbps{{target=\"{}\"}} {:.2}\n",
        host, report.download.peak_mbps
    ));

    out.push_str(
        "# HELP kiwi_download_bytes_total Total bytes transferred during download phase\n",
    );
    out.push_str("# TYPE kiwi_download_bytes_total counter\n");
    out.push_str(&format!(
        "kiwi_download_bytes_total{{target=\"{}\"}} {}\n",
        host, report.download.total_bytes
    ));

    // Upload metrics
    out.push_str("# HELP kiwi_upload_mbps Saturated upload throughput in megabits per second\n");
    out.push_str("# TYPE kiwi_upload_mbps gauge\n");
    out.push_str(&format!(
        "kiwi_upload_mbps{{target=\"{}\",interface=\"{}\"}} {:.2}\n",
        host, iface, report.upload.mbps
    ));

    out.push_str("# HELP kiwi_upload_peak_mbps Peak upload throughput in megabits per second\n");
    out.push_str("# TYPE kiwi_upload_peak_mbps gauge\n");
    out.push_str(&format!(
        "kiwi_upload_peak_mbps{{target=\"{}\"}} {:.2}\n",
        host, report.upload.peak_mbps
    ));

    out.push_str("# HELP kiwi_upload_bytes_total Total bytes transferred during upload phase\n");
    out.push_str("# TYPE kiwi_upload_bytes_total counter\n");
    out.push_str(&format!(
        "kiwi_upload_bytes_total{{target=\"{}\"}} {}\n",
        host, report.upload.total_bytes
    ));

    // Baseline Idle Latency
    out.push_str("# HELP kiwi_idle_latency_ms Baseline unloaded idle latency in milliseconds\n");
    out.push_str("# TYPE kiwi_idle_latency_ms gauge\n");
    out.push_str(&format!(
        "kiwi_idle_latency_ms{{target=\"{}\"}} {:.2}\n",
        host, report.idle_latency.median_ms
    ));

    out.push_str("# HELP kiwi_idle_jitter_ms Baseline unloaded idle jitter in milliseconds\n");
    out.push_str("# TYPE kiwi_idle_jitter_ms gauge\n");
    out.push_str(&format!(
        "kiwi_idle_jitter_ms{{target=\"{}\"}} {:.2}\n",
        host, report.idle_latency.jitter_ms
    ));

    out.push_str("# HELP kiwi_idle_packet_loss_percent Baseline idle packet loss percentage\n");
    out.push_str("# TYPE kiwi_idle_packet_loss_percent gauge\n");
    out.push_str(&format!(
        "kiwi_idle_packet_loss_percent{{target=\"{}\"}} {:.2}\n",
        host, report.idle_latency.packet_loss_pct
    ));

    // Loaded Working Latencies
    out.push_str(
        "# HELP kiwi_loaded_down_latency_ms Loaded download P90 latency in milliseconds\n",
    );
    out.push_str("# TYPE kiwi_loaded_down_latency_ms gauge\n");
    out.push_str(&format!(
        "kiwi_loaded_down_latency_ms{{target=\"{}\"}} {:.2}\n",
        host, report.loaded_down_latency.p90_ms
    ));

    out.push_str("# HELP kiwi_loaded_up_latency_ms Loaded upload P90 latency in milliseconds\n");
    out.push_str("# TYPE kiwi_loaded_up_latency_ms gauge\n");
    out.push_str(&format!(
        "kiwi_loaded_up_latency_ms{{target=\"{}\"}} {:.2}\n",
        host, report.loaded_up_latency.p90_ms
    ));

    // Apple RPM Responsiveness & Bufferbloat Grade
    out.push_str(
        "# HELP kiwi_bufferbloat_rpm Apple Round-trips Per Minute responsiveness under load\n",
    );
    out.push_str("# TYPE kiwi_bufferbloat_rpm gauge\n");
    out.push_str(&format!(
        "kiwi_bufferbloat_rpm{{grade=\"{}\",rating=\"{}\"}} {}\n",
        report.responsiveness.grade, report.responsiveness.rating_label, report.responsiveness.rpm
    ));

    out.push_str(
        "# HELP kiwi_bufferbloat_delta_ms Bufferbloat working latency delta in milliseconds\n",
    );
    out.push_str("# TYPE kiwi_bufferbloat_delta_ms gauge\n");
    out.push_str(&format!(
        "kiwi_bufferbloat_delta_ms{{target=\"{}\"}} {:.2}\n",
        host, report.responsiveness.delta_ms
    ));

    out.push_str("# HELP kiwi_responsiveness_grade Bufferbloat letter grade\n");
    out.push_str("# TYPE kiwi_responsiveness_grade gauge\n");
    out.push_str(&format!(
        "kiwi_responsiveness_grade{{grade=\"{}\",rating=\"{}\"}} 1\n",
        report.responsiveness.grade, report.responsiveness.rating_label
    ));

    // Full-duplex indicator
    out.push_str("# HELP kiwi_is_bidirectional Flag indicating if test was run in simultaneous full-duplex mode\n");
    out.push_str("# TYPE kiwi_is_bidirectional gauge\n");
    out.push_str(&format!(
        "kiwi_is_bidirectional {}\n",
        if report.is_bidirectional { 1 } else { 0 }
    ));

    // TCP Kernel Telemetry
    if let Some(cwnd) = report.edge.cf_l4_cwnd {
        out.push_str("# HELP kiwi_tcp_cwnd_packets TCP congestion window size in packets\n");
        out.push_str("# TYPE kiwi_tcp_cwnd_packets gauge\n");
        out.push_str(&format!("kiwi_tcp_cwnd_packets {}\n", cwnd));
    }
    if let Some(retrans) = report.edge.cf_l4_retrans {
        out.push_str("# HELP kiwi_tcp_retransmissions TCP retransmitted segments count\n");
        out.push_str("# TYPE kiwi_tcp_retransmissions gauge\n");
        out.push_str(&format!("kiwi_tcp_retransmissions {}\n", retrans));
    }
    if let Some(rtt) = report.edge.cf_l4_rtt_us {
        out.push_str(
            "# HELP kiwi_tcp_rtt_microseconds TCP smoothed round-trip time in microseconds\n",
        );
        out.push_str("# TYPE kiwi_tcp_rtt_microseconds gauge\n");
        out.push_str(&format!("kiwi_tcp_rtt_microseconds {}\n", rtt));
    }
    if let Some(min_rtt) = report.edge.cf_l4_min_rtt_us {
        out.push_str(
            "# HELP kiwi_tcp_min_rtt_microseconds TCP minimum round-trip time in microseconds\n",
        );
        out.push_str("# TYPE kiwi_tcp_min_rtt_microseconds gauge\n");
        out.push_str(&format!("kiwi_tcp_min_rtt_microseconds {}\n", min_rtt));
    }

    if let Some(ktcp) = report.edge.kernel_tcp {
        out.push_str("# HELP kiwi_kernel_tcp_cwnd Congestion window sampled directly from OS kernel socket\n");
        out.push_str("# TYPE kiwi_kernel_tcp_cwnd gauge\n");
        out.push_str(&format!("kiwi_kernel_tcp_cwnd {}\n", ktcp.snd_cwnd));

        out.push_str("# HELP kiwi_kernel_tcp_rtt_microseconds Smoothed RTT sampled directly from OS kernel socket\n");
        out.push_str("# TYPE kiwi_kernel_tcp_rtt_microseconds gauge\n");
        out.push_str(&format!(
            "kiwi_kernel_tcp_rtt_microseconds {}\n",
            ktcp.rtt_us
        ));

        out.push_str("# HELP kiwi_kernel_tcp_retransmissions Retransmissions sampled directly from OS kernel socket\n");
        out.push_str("# TYPE kiwi_kernel_tcp_retransmissions gauge\n");
        out.push_str(&format!(
            "kiwi_kernel_tcp_retransmissions {}\n",
            ktcp.retrans
        ));
    }

    // ISP Neutrality Audit
    if let Some(ref audit) = report.isp_audit {
        out.push_str(
            "# HELP kiwi_isp_neutrality_ratio ISP speedtest neutrality ratio (Anycast / On-Net)\n",
        );
        out.push_str("# TYPE kiwi_isp_neutrality_ratio gauge\n");
        out.push_str(&format!(
            "kiwi_isp_neutrality_ratio{{verdict=\"{}\"}} {:.4}\n",
            audit.verdict, audit.neutrality_ratio
        ));

        out.push_str("# HELP kiwi_isp_on_net_download_mbps On-net comparison throughput in megabits per second\n");
        out.push_str("# TYPE kiwi_isp_on_net_download_mbps gauge\n");
        out.push_str(&format!(
            "kiwi_isp_on_net_download_mbps{{host=\"{}\"}} {:.2}\n",
            audit.on_net_host, audit.on_net_download_mbps
        ));

        out.push_str("# HELP kiwi_isp_anycast_download_mbps Anycast transit throughput in megabits per second\n");
        out.push_str("# TYPE kiwi_isp_anycast_download_mbps gauge\n");
        out.push_str(&format!(
            "kiwi_isp_anycast_download_mbps{{host=\"{}\"}} {:.2}\n",
            audit.anycast_host, audit.anycast_download_mbps
        ));

        out.push_str("# HELP kiwi_isp_is_throttled Flag indicating whether ISP throttling or QoS shaping was detected\n");
        out.push_str("# TYPE kiwi_isp_is_throttled gauge\n");
        out.push_str(&format!(
            "kiwi_isp_is_throttled {}\n",
            if audit.is_throttled { 1 } else { 0 }
        ));
    }

    // Process & System Footprint
    out.push_str(
        "# HELP kiwi_cold_start_seconds Process cold start initialization time in seconds\n",
    );
    out.push_str("# TYPE kiwi_cold_start_seconds gauge\n");
    out.push_str(&format!(
        "kiwi_cold_start_seconds {:.6}\n",
        report.resources.cold_start_ms / 1000.0
    ));

    out.push_str("# HELP kiwi_peak_rss_bytes Process peak resident set size in bytes\n");
    out.push_str("# TYPE kiwi_peak_rss_bytes gauge\n");
    out.push_str(&format!(
        "kiwi_peak_rss_bytes {:.0}\n",
        report.resources.peak_rss_mb * 1024.0 * 1024.0
    ));

    out
}
