use crate::metrics::BenchmarkReport;

pub fn render_bento_box(report: &BenchmarkReport, term_cols: usize) -> String {
    let width = term_cols.clamp(50, 86);
    let double_border = "═".repeat(width);
    let single_border = "─".repeat(width);

    let mut out = String::new();

    out.push_str("\x1b[1;32m🥝 KIWI BENCHMARK RECEIPT\x1b[0m\n");
    out.push_str(&format!("\x1b[38;5;242m{}\x1b[0m\n", double_border));

    // Client line
    let client_asn = match report.edge.asn {
        Some(asn) => format!(" (AS{})", asn),
        None => String::new(),
    };
    let client_loc = if !report.edge.city.is_empty() {
        format!(" · {}, {}", report.edge.city, report.edge.country)
    } else {
        String::new()
    };
    out.push_str(&format!(
        "  \x1b[1mClient\x1b[0m       : {}{}{}\n",
        if report.edge.client_ip.is_empty() {
            "Local Host"
        } else {
            &report.edge.client_ip
        },
        client_asn,
        client_loc
    ));

    // Edge node line
    let colo_str = if !report.edge.colo.is_empty() {
        format!(
            "Anycast Fleet [{} Colocation · {}, {}]",
            report.edge.colo, report.edge.city, report.edge.country
        )
    } else {
        report.target_host.clone()
    };
    out.push_str(&format!("  \x1b[1mEdge Node\x1b[0m    : {}\n", colo_str));

    // Transport line
    out.push_str(&format!(
        "  \x1b[1mTransport\x1b[0m    : {}\n",
        report.protocol
    ));

    out.push_str(&format!("\x1b[38;5;242m{}\x1b[0m\n", single_border));

    // Table Header
    out.push_str("  \x1b[1mMETRIC                        MEASUREMENT         PEAK / DRIFT       RATING\x1b[0m\n");
    out.push_str(&format!("\x1b[38;5;242m{}\x1b[0m\n", single_border));

    // Download Bandwidth
    let down_rating = rate_throughput(report.download.mbps);
    out.push_str(&format!(
        "  \x1b[36m📥 DOWNLOAD BANDWIDTH\x1b[0m         {:>6.1} Mbps          Peak: {:>5.1} Mbps   \x1b[1;32m{}\x1b[0m\n",
        report.download.mbps, report.download.peak_mbps, down_rating
    ));

    // Upload Bandwidth
    let up_rating = rate_throughput(report.upload.mbps);
    out.push_str(&format!(
        "  \x1b[35m📤 UPLOAD BANDWIDTH\x1b[0m           {:>6.1} Mbps          Peak: {:>5.1} Mbps   \x1b[1;32m{}\x1b[0m\n",
        report.upload.mbps, report.upload.peak_mbps, up_rating
    ));

    // Idle Latency
    out.push_str(&format!(
        "  \x1b[33m⚡ UNLOADED IDLE LATENCY\x1b[0m       {:>6.1} ms            Jitter: ±{:>4.1} ms  {:.1}% Loss\n",
        report.idle_latency.median_ms, report.idle_latency.jitter_ms, report.idle_latency.packet_loss_pct
    ));

    // Loaded Latency
    out.push_str(&format!(
        "  \x1b[34m🏋️ LOADED WORKING LATENCY\x1b[0m      {:>6.1} ms            Down: {:>4.1}ms      Up: {:>4.1}ms\n",
        report.responsiveness.working_rtt_ms, report.loaded_down_latency.p90_ms, report.loaded_up_latency.p90_ms
    ));

    // Bufferbloat Responsiveness
    let grade_color = match report.responsiveness.grade.as_str() {
        "A+" => "\x1b[1;36m",
        "A" => "\x1b[1;32m",
        "B" => "\x1b[1;33m",
        "C" => "\x1b[1;38;5;208m",
        _ => "\x1b[1;31m",
    };
    out.push_str(&format!(
        "  \x1b[32m🎯 BUFFERBLOAT RESPONSIVENESS\x1b[0m  {:>6} RPM           +{:.1}ms Delta      {}GRADE {}\x1b[0m\n",
        report.responsiveness.rpm, report.responsiveness.delta_ms, grade_color, report.responsiveness.grade
    ));

    out.push_str(&format!("\x1b[38;5;242m{}\x1b[0m\n", single_border));

    // Server Telemetry
    let cwnd_str = match report.edge.cf_l4_cwnd {
        Some(cwnd) => format!(" · cwnd: {}", cwnd),
        None => String::new(),
    };
    let retrans_str = match report.edge.cf_l4_retrans {
        Some(r) => format!(" · {} Server Retransmissions", r),
        None => String::new(),
    };
    out.push_str(&format!(
        "  \x1b[1mServer Telemetry\x1b[0m   : {} Anycast Edge{}{}\n",
        if report.edge.colo.is_empty() {
            "Cloudflare"
        } else {
            &report.edge.colo
        },
        cwnd_str,
        retrans_str
    ));

    // PHY Link Telemetry
    let link_desc = match report.platform.link_speed_mbps {
        Some(s) => format!(
            "{} ({} Mbps {})",
            report.platform.interface_name, s, report.platform.link_type
        ),
        None => format!(
            "{} ({})",
            report.platform.interface_name, report.platform.link_type
        ),
    };
    let warning_note = match &report.platform.bottleneck_warning {
        Some(w) => format!(" — \x1b[33m{}\x1b[0m", w),
        None => " — Local link capacity nominal".to_string(),
    };
    out.push_str(&format!(
        "  \x1b[1mPHY Link Telemetry\x1b[0m : {}{}\n",
        link_desc, warning_note
    ));

    // Resource Profile
    out.push_str(&format!(
        "  \x1b[1mResource Profile\x1b[0m   : Cold Start: {:.1}ms │ Peak RSS: {:.1} MB │ Duration: {:.1}s\n",
        report.resources.cold_start_ms, report.resources.peak_rss_mb, report.resources.duration_secs
    ));

    out.push_str(&format!("\x1b[38;5;242m{}\x1b[0m\n", double_border));

    out
}

fn rate_throughput(mbps: f64) -> &'static str {
    if mbps >= 300.0 {
        "EXCELLENT"
    } else if mbps >= 100.0 {
        "GOOD"
    } else if mbps >= 25.0 {
        "FAIR"
    } else {
        "LOW"
    }
}
