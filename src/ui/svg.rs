use crate::metrics::BenchmarkReport;
use std::fs::File;
use std::io::Write;

/// Exports the benchmark receipt as a styled Vector SVG card suitable for GitHub READMEs, dashboards, and CI/CD artifacts.
pub fn export_svg(report: &BenchmarkReport, path: &str) -> Result<(), std::io::Error> {
    let svg_content = render_svg_card(report);
    let mut file = File::create(path)?;
    file.write_all(svg_content.as_bytes())?;
    Ok(())
}

pub fn render_svg_card(report: &BenchmarkReport) -> String {
    let (grade_color, grade_bg) = match report.responsiveness.grade.as_str() {
        "A+" => ("#10b981", "#064e3b"),
        "A" => ("#34d399", "#064e3b"),
        "B" => ("#06b6d4", "#164e63"),
        "C" => ("#f59e0b", "#78350f"),
        _ => ("#f43f5e", "#881337"),
    };

    let client_ip = if report.edge.client_ip.is_empty() {
        "Local Host".to_string()
    } else {
        report.edge.client_ip.clone()
    };

    let location = if !report.edge.city.is_empty() {
        format!("{}, {}", report.edge.city, report.edge.country)
    } else {
        "Global Edge".to_string()
    };

    let colo = if !report.edge.colo.is_empty() {
        report.edge.colo.clone()
    } else {
        "Anycast".to_string()
    };

    let mode_str = if report.is_bidirectional {
        "Full-Duplex Simultaneous Saturation"
    } else {
        "Sequential Saturated Run"
    };

    let audit_section = if let Some(ref audit) = report.isp_audit {
        let (verdict_color, verdict_bg) = if audit.is_throttled {
            ("#f43f5e", "#881337")
        } else {
            ("#10b981", "#064e3b")
        };
        format!(
            r###"
  <!-- ISP Neutrality Audit Card -->
  <g transform="translate(40, 480)">
    <rect width="720" height="85" rx="8" fill="#111827" stroke="#374151" stroke-width="1"/>
    <text x="20" y="28" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif" font-size="12" font-weight="700" fill="#9ca3af" letter-spacing="1">ISP SPEEDTEST NEUTRALITY AUDIT</text>
    <rect x="520" y="14" width="180" height="22" rx="4" fill="{verdict_bg}"/>
    <text x="610" y="29" font-family="monospace" font-size="10" font-weight="700" fill="{verdict_color}" text-anchor="middle">{verdict}</text>
    
    <text x="20" y="55" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif" font-size="12" fill="#d1d5db">Neutrality Ratio: <tspan font-family="monospace" font-weight="700" fill="#38bdf8">{ratio:.2}</tspan> · Anycast Transit: <tspan font-family="monospace" font-weight="700" fill="#10b981">{anycast:.1} Mbps</tspan> · On-Net Comparison: <tspan font-family="monospace" font-weight="700" fill="#f43f5e">{on_net:.1} Mbps</tspan></text>
    <text x="20" y="73" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif" font-size="11" fill="#9ca3af">{analysis_short}</text>
  </g>
"###,
            verdict_bg = verdict_bg,
            verdict = audit.verdict,
            verdict_color = verdict_color,
            ratio = audit.neutrality_ratio,
            anycast = audit.anycast_download_mbps,
            on_net = audit.on_net_download_mbps,
            analysis_short = if audit.analysis.len() > 95 {
                format!("{}...", &audit.analysis[..92])
            } else {
                audit.analysis.clone()
            }
        )
    } else {
        String::new()
    };

    let total_height = if report.isp_audit.is_some() { 600 } else { 500 };

    format!(
        r###"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 800 {total_height}" width="100%" height="100%">
  <defs>
    <linearGradient id="bgGrad" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#0b0f19"/>
      <stop offset="100%" stop-color="#111827"/>
    </linearGradient>
    <linearGradient id="downGrad" x1="0%" y1="0%" x2="100%" y2="0%">
      <stop offset="0%" stop-color="#06b6d4"/>
      <stop offset="100%" stop-color="#10b981"/>
    </linearGradient>
    <linearGradient id="upGrad" x1="0%" y1="0%" x2="100%" y2="0%">
      <stop offset="0%" stop-color="#8b5cf6"/>
      <stop offset="100%" stop-color="#ec4899"/>
    </linearGradient>
  </defs>

  <!-- Container Box -->
  <rect width="800" height="{total_height}" rx="16" fill="url(#bgGrad)" stroke="#1f2937" stroke-width="1.5"/>

  <!-- Header Section -->
  <g transform="translate(40, 45)">
    <text x="0" y="0" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif" font-size="22" font-weight="800" fill="#10b981">🥝 KIWI NETWORK RECEIPT</text>
    <rect x="290" y="-18" width="60" height="22" rx="4" fill="#064e3b"/>
    <text x="320" y="-3" font-family="monospace" font-size="11" font-weight="700" fill="#34d399" text-anchor="middle">v{version}</text>
    <text x="720" y="-2" font-family="monospace" font-size="12" fill="#6b7280" text-anchor="end">{mode}</text>
  </g>

  <!-- Divider -->
  <line x1="40" y1="65" x2="760" y2="65" stroke="#1f2937" stroke-width="1"/>

  <!-- Grid Row 1: Bandwidth Speed Badges -->
  <g transform="translate(40, 95)">
    <!-- Download Box -->
    <rect width="345" height="135" rx="10" fill="#111827" stroke="#1f2937" stroke-width="1"/>
    <text x="20" y="28" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif" font-size="12" font-weight="700" fill="#06b6d4" letter-spacing="1">📥 DOWNLOAD SPEED</text>
    <text x="20" y="78" font-family="monospace" font-size="38" font-weight="800" fill="url(#downGrad)">{down_mbps:.1}</text>
    <text x="190" y="72" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif" font-size="16" font-weight="600" fill="#9ca3af">Mbps</text>
    
    <rect x="20" y="98" width="305" height="6" rx="3" fill="#1f2937"/>
    <rect x="20" y="98" width="{down_bar_width}" height="6" rx="3" fill="url(#downGrad)"/>
    <text x="20" y="122" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif" font-size="11" fill="#9ca3af">Peak: {down_peak:.1} Mbps · Total: {down_mb:.1} MB</text>

    <!-- Upload Box -->
    <g transform="translate(375, 0)">
      <rect width="345" height="135" rx="10" fill="#111827" stroke="#1f2937" stroke-width="1"/>
      <text x="20" y="28" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif" font-size="12" font-weight="700" fill="#ec4899" letter-spacing="1">📤 UPLOAD SPEED</text>
      <text x="20" y="78" font-family="monospace" font-size="38" font-weight="800" fill="url(#upGrad)">{up_mbps:.1}</text>
      <text x="190" y="72" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif" font-size="16" font-weight="600" fill="#9ca3af">Mbps</text>
      
      <rect x="20" y="98" width="305" height="6" rx="3" fill="#1f2937"/>
      <rect x="20" y="98" width="{up_bar_width}" height="6" rx="3" fill="url(#upGrad)"/>
      <text x="20" y="122" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif" font-size="11" fill="#9ca3af">Peak: {up_peak:.1} Mbps · Total: {up_mb:.1} MB</text>
    </g>
  </g>

  <!-- Grid Row 2: Latency & Responsiveness -->
  <g transform="translate(40, 250)">
    <!-- Idle & Loaded Latencies -->
    <rect width="345" height="120" rx="10" fill="#111827" stroke="#1f2937" stroke-width="1"/>
    <text x="20" y="28" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif" font-size="12" font-weight="700" fill="#f59e0b" letter-spacing="1">⚡ LATENCY DYNAMICS</text>
    
    <text x="20" y="60" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif" font-size="13" fill="#9ca3af">Idle Ping:</text>
    <text x="110" y="60" font-family="monospace" font-size="15" font-weight="700" fill="#fbbf24">{idle_ms:.1} ms</text>
    <text x="200" y="60" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif" font-size="11" fill="#6b7280">(±{jitter:.1} ms)</text>
    
    <text x="20" y="92" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif" font-size="13" fill="#9ca3af">Loaded Ping:</text>
    <text x="110" y="92" font-family="monospace" font-size="15" font-weight="700" fill="#38bdf8">{working_ms:.1} ms</text>
    <text x="200" y="92" font-family="monospace" font-size="11" font-weight="700" fill="{grade_color}">+{delta_ms:.1} ms</text>

    <!-- Bufferbloat & Responsiveness Box -->
    <g transform="translate(375, 0)">
      <rect width="345" height="120" rx="10" fill="#111827" stroke="#1f2937" stroke-width="1"/>
      <text x="20" y="28" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif" font-size="12" font-weight="700" fill="#10b981" letter-spacing="1">🎯 BUFFERBLOAT RESPONSIVENESS</text>
      
      <text x="20" y="75" font-family="monospace" font-size="32" font-weight="800" fill="#10b981">{rpm}</text>
      <text x="135" y="70" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif" font-size="14" font-weight="600" fill="#9ca3af">RPM</text>
      
      <rect x="200" y="44" width="125" height="34" rx="6" fill="{grade_bg}"/>
      <text x="262" y="66" font-family="monospace" font-size="16" font-weight="800" fill="{grade_color}" text-anchor="middle">GRADE {grade}</text>
      <text x="20" y="104" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif" font-size="11" font-weight="600" fill="{grade_color}">{rating_label}</text>
    </g>
  </g>

  <!-- Network & Hardware Topology Footer -->
  <g transform="translate(40, 390)">
    <rect width="720" height="70" rx="8" fill="#111827" stroke="#1f2937" stroke-width="1"/>
    <text x="20" y="26" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif" font-size="12" fill="#9ca3af">Client: <tspan font-family="monospace" font-weight="700" fill="#e5e7eb">{client_ip}</tspan> ({location})</text>
    <text x="20" y="48" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif" font-size="12" fill="#9ca3af">Edge Node: <tspan font-family="monospace" font-weight="700" fill="#10b981">{colo} Anycast Fleet</tspan> · Interface: <tspan font-family="monospace" fill="#e5e7eb">{iface}</tspan> ({iface_type})</text>
    <text x="700" y="26" font-family="monospace" font-size="11" fill="#6b7280" text-anchor="end">Cold: {cold_ms:.1}ms · RSS: {rss_mb:.1}MB</text>
  </g>

  {audit_section}
</svg>
"###,
        total_height = total_height,
        version = report.version,
        mode = mode_str,
        down_mbps = report.download.mbps,
        down_peak = report.download.peak_mbps,
        down_mb = report.download.total_bytes as f64 / (1024.0 * 1024.0),
        down_bar_width = ((report.download.mbps
            / report.download.peak_mbps.max(report.download.mbps).max(1.0))
            * 305.0)
            .clamp(5.0, 305.0),
        up_mbps = report.upload.mbps,
        up_peak = report.upload.peak_mbps,
        up_mb = report.upload.total_bytes as f64 / (1024.0 * 1024.0),
        up_bar_width = ((report.upload.mbps
            / report.upload.peak_mbps.max(report.upload.mbps).max(1.0))
            * 305.0)
            .clamp(5.0, 305.0),
        idle_ms = report.idle_latency.median_ms,
        jitter = report.idle_latency.jitter_ms,
        working_ms = report.responsiveness.working_rtt_ms,
        delta_ms = report.responsiveness.delta_ms,
        rpm = report.responsiveness.rpm,
        grade = report.responsiveness.grade,
        grade_color = grade_color,
        grade_bg = grade_bg,
        rating_label = report.responsiveness.rating_label,
        client_ip = client_ip,
        location = location,
        colo = colo,
        iface = report.platform.interface_name,
        iface_type = report.platform.link_type,
        cold_ms = report.resources.cold_start_ms,
        rss_mb = report.resources.peak_rss_mb,
        audit_section = audit_section,
    )
}
