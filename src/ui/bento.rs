use crate::metrics::BenchmarkReport;
use crate::ui::art::{
    BORDER_COLOR, BRIGHT_WHITE, CYBER_CYAN, DEEP_VIOLET, DIM_GRAY, ELECTRIC_AMBER, KIWI_EMERALD,
    NEON_MAGENTA, box_bot, box_row, box_sep, box_top, color_bold, color_fg, gradient_text,
    pad_to_width, render_cyber_kiwi_point_cloud, visible_width,
};
use crate::ui::braille::{calculate_gauge_scale, render_colored_tachometer};

/// Renders a sleek floating Bento-Box Receipt with razor-thin rounded micro-borders,
/// glowing TrueColor pill badges, dot-matrix separators, and an embedded
/// Ethereal Bioluminescent Cyber-Kiwi Mascot point-cloud.
pub fn render_bento_box(report: &BenchmarkReport, term_cols: usize) -> String {
    let width = if term_cols < 74 {
        term_cols.clamp(40, 73)
    } else {
        term_cols.clamp(74, 84)
    };
    let mut out = String::with_capacity(4096);

    out.push('\n');

    // ─── CARD 1: IDENTITY & TOPOLOGY WITH CYBER-KIWI POINT CLOUD ─────────────
    let brand_title = format!(
        "🥝 {}  {}",
        color_bold("KIWI BENCHMARK RECEIPT", KIWI_EMERALD),
        color_fg(&format!("[ v{} ]", report.version), DIM_GRAY)
    );
    out.push_str(&box_top(&brand_title, width));
    out.push('\n');

    let kiwi_art = render_cyber_kiwi_point_cloud();

    let client_asn = match report.edge.asn {
        Some(asn) => format!(" (AS{})", asn),
        None => String::new(),
    };
    let client_ip_str = if report.edge.client_ip.is_empty() {
        "Local Host".to_string()
    } else {
        report.edge.client_ip.clone()
    };
    let client_loc = if !report.edge.city.is_empty() {
        format!("{}, {}", report.edge.city, report.edge.country)
    } else {
        "Global Edge".to_string()
    };

    let colo_code = if !report.edge.colo.is_empty() {
        report.edge.colo.clone()
    } else {
        "CF-EDGE".to_string()
    };

    let edge_node_desc = format!("{} Anycast Fleet", colo_code);

    let proto_disp = {
        let mut p = report.protocol.clone();
        if p.len() > 30 {
            p = p.replace("Dual-Stack ", "").replace(" over TLS", " · TLS");
        }
        if let Some(open) = p.find('(')
            && let Some(close) = p.find(')')
            && close > open
            && p.len() > 30
        {
            p = format!("{}{}", p[..open].trim_end(), &p[close + 1..]);
        }
        p
    };

    let right_lines: Vec<String> = vec![
        color_bold("NETWORK INTELLIGENCE & IDENTITY", CYBER_CYAN),
        color_fg("· · · · · · · · · · · · · · · · · · · · · ·", (55, 65, 81)),
        format!(
            "{}  {}",
            color_fg("Client    ·", DIM_GRAY),
            color_bold(&format!("{}{}", client_ip_str, client_asn), BRIGHT_WHITE)
        ),
        format!(
            "{}  {}",
            color_fg("Location  ·", DIM_GRAY),
            color_fg(&client_loc, BRIGHT_WHITE)
        ),
        format!(
            "{}  {}",
            color_fg("Edge Node ·", DIM_GRAY),
            color_bold(&edge_node_desc, KIWI_EMERALD)
        ),
        format!(
            "{}  {}",
            color_fg("Transport ·", DIM_GRAY),
            color_fg(&proto_disp, BRIGHT_WHITE)
        ),
        format!(
            "{}  {}",
            color_fg("Target    ·", DIM_GRAY),
            color_fg(&report.target_host, DIM_GRAY)
        ),
        format!(
            "{}  {}",
            color_fg("Session   ·", DIM_GRAY),
            color_fg("Saturated Zero-Loss Run", CYBER_CYAN)
        ),
    ];

    let inner_width = width.saturating_sub(4);
    if width >= 74 {
        for i in 0..8 {
            let left = kiwi_art.get(i).cloned().unwrap_or_default();
            let right = right_lines.get(i).cloned().unwrap_or_default();

            let left_vis = visible_width(&left);
            let space_gap = 2;
            let right_target = inner_width.saturating_sub(left_vis + space_gap);
            let right_padded = pad_to_width(&right, right_target);

            let combined = format!("{}{}{}", left, " ".repeat(space_gap), right_padded);
            out.push_str(&box_row(&combined, width));
            out.push('\n');
        }
    } else {
        // Narrow viewport: center mascot on top, then separator, then metadata rows
        for line in &kiwi_art {
            let art_vis = visible_width(line);
            let pad_left = inner_width.saturating_sub(art_vis) / 2;
            let centered = format!("{}{}", " ".repeat(pad_left), line);
            out.push_str(&box_row(&centered, width));
            out.push('\n');
        }
        out.push_str(&box_sep(width));
        out.push('\n');
        for line in &right_lines {
            out.push_str(&box_row(line, width));
            out.push('\n');
        }
    }

    out.push_str(&box_bot(width));
    out.push('\n');

    // ─── CARD 2: BANDWIDTH VELOCITY ──────────────────────────────────────────
    let band_title = format!(
        "⚡ {}",
        gradient_text(
            "BANDWIDTH VELOCITY",
            &[(0.0, CYBER_CYAN), (1.0, KIWI_EMERALD)]
        )
    );
    out.push_str(&box_top(&band_title, width));
    out.push('\n');

    // Download Bandwidth
    let down_rating = rate_throughput(report.download.mbps);
    let down_badge = pill_badge(&format!("{:.1} Mbps", report.download.mbps), KIWI_EMERALD);
    let down_peak_badge = format!(
        "Peak: {}",
        color_fg(
            &format!("{:.1} Mbps", report.download.peak_mbps),
            BRIGHT_WHITE
        )
    );
    let down_rating_badge = format!("Rating: {}", pill_badge(down_rating, KIWI_EMERALD));

    let down_label = color_bold("📥 DOWNLOAD BANDWIDTH", CYBER_CYAN);
    out.push_str(&box_row(&format!("  {}", down_label), width));
    out.push('\n');
    if width >= 66 {
        out.push_str(&box_row(
            &format!(
                "     {}   {}   {}",
                down_badge, down_peak_badge, down_rating_badge
            ),
            width,
        ));
    } else {
        out.push_str(&box_row(
            &format!("     {}   {}", down_badge, down_peak_badge),
            width,
        ));
    }
    out.push('\n');

    let gauge_width = inner_width.saturating_sub(8).min(46);
    let down_scale = calculate_gauge_scale(report.download.peak_mbps.max(report.download.mbps));
    let down_tacho = render_colored_tachometer(report.download.mbps, down_scale, gauge_width);
    out.push_str(&box_row(&format!("     {}", down_tacho), width));
    out.push('\n');

    // Dot-matrix separator
    out.push_str(&box_sep(width));
    out.push('\n');

    // Upload Bandwidth
    let up_rating = rate_throughput(report.upload.mbps);
    let up_badge = pill_badge(&format!("{:.1} Mbps", report.upload.mbps), NEON_MAGENTA);
    let up_peak_badge = format!(
        "Peak: {}",
        color_fg(
            &format!("{:.1} Mbps", report.upload.peak_mbps),
            BRIGHT_WHITE
        )
    );
    let up_rating_badge = format!("Rating: {}", pill_badge(up_rating, NEON_MAGENTA));

    let up_label = color_bold("📤 UPLOAD BANDWIDTH", NEON_MAGENTA);
    out.push_str(&box_row(&format!("  {}", up_label), width));
    out.push('\n');
    if width >= 66 {
        out.push_str(&box_row(
            &format!(
                "     {}   {}   {}",
                up_badge, up_peak_badge, up_rating_badge
            ),
            width,
        ));
    } else {
        out.push_str(&box_row(
            &format!("     {}   {}", up_badge, up_peak_badge),
            width,
        ));
    }
    out.push('\n');

    let up_scale = calculate_gauge_scale(report.upload.peak_mbps.max(report.upload.mbps));
    let up_tacho = render_colored_tachometer(report.upload.mbps, up_scale, gauge_width);
    out.push_str(&box_row(&format!("     {}", up_tacho), width));
    out.push('\n');

    out.push_str(&box_bot(width));
    out.push('\n');

    // ─── CARD 3: LATENCY & BUFFERBLOAT DYNAMICS ─────────────────────────────
    let lat_title = format!(
        "🎯 {}",
        gradient_text(
            "LATENCY & RESPONSIVENESS DYNAMICS",
            &[(0.0, ELECTRIC_AMBER), (1.0, NEON_MAGENTA)]
        )
    );
    out.push_str(&box_top(&lat_title, width));
    out.push('\n');

    // Idle Latency
    let idle_label = color_bold("⚡ UNLOADED IDLE LATENCY", ELECTRIC_AMBER);
    let idle_val = pill_badge(
        &format!("{:.1} ms", report.idle_latency.median_ms),
        ELECTRIC_AMBER,
    );
    let idle_metrics = format!(
        "Jitter: ±{:.1} ms   Packet Loss: {:.1}%",
        report.idle_latency.jitter_ms, report.idle_latency.packet_loss_pct
    );
    out.push_str(&box_row(&format!("  {}", idle_label), width));
    out.push('\n');
    out.push_str(&box_row(
        &format!("     {}   {}", idle_val, color_fg(&idle_metrics, DIM_GRAY)),
        width,
    ));
    out.push('\n');

    // Loaded Working Latency
    let loaded_label = color_bold("🏋️ LOADED WORKING LATENCY", CYBER_CYAN);
    let loaded_val = pill_badge(
        &format!("{:.1} ms", report.responsiveness.working_rtt_ms),
        CYBER_CYAN,
    );
    let delta_str = format!("+{:.1} ms Delta", report.responsiveness.delta_ms);
    let delta_colored = color_fg(
        &delta_str,
        if report.responsiveness.delta_ms < 15.0 {
            KIWI_EMERALD
        } else {
            ELECTRIC_AMBER
        },
    );
    out.push_str(&box_row(&format!("  {}", loaded_label), width));
    out.push('\n');
    if width >= 66 {
        let loaded_metrics = format!(
            "Down: {:.1} ms · Up: {:.1} ms   [{}]",
            report.loaded_down_latency.p90_ms, report.loaded_up_latency.p90_ms, delta_colored
        );
        out.push_str(&box_row(
            &format!(
                "     {}   {}",
                loaded_val,
                color_fg(&loaded_metrics, DIM_GRAY)
            ),
            width,
        ));
    } else {
        out.push_str(&box_row(
            &format!("     {}   [{}]", loaded_val, delta_colored),
            width,
        ));
    }
    out.push('\n');

    // Dot-matrix separator
    out.push_str(&box_sep(width));
    out.push('\n');

    // Bufferbloat Responsiveness & Grade Badge
    let bloat_label = color_bold("🎯 BUFFERBLOAT RESPONSIVENESS", KIWI_EMERALD);
    let rpm_badge = pill_badge(&format!("{} RPM", report.responsiveness.rpm), KIWI_EMERALD);
    let (grade_color, grade_glow) = match report.responsiveness.grade.as_str() {
        "A+" => (KIWI_EMERALD, "ULTRA RESPONSIVE · ESPORTS GRADE"),
        "A" => (KIWI_EMERALD, "EXCELLENT · LOW LATENCY BUFFER"),
        "B" => (CYBER_CYAN, "GOOD · MINIMAL BUFFER DELAY"),
        "C" => (ELECTRIC_AMBER, "FAIR · MODERATE BLOAT"),
        _ => (NEON_MAGENTA, "POOR · HIGH WORKING LATENCY"),
    };

    let grade_pill = pill_badge(
        &format!("GRADE {}", report.responsiveness.grade),
        grade_color,
    );
    let grade_label_styled = color_fg(grade_glow, grade_color);

    out.push_str(&box_row(&format!("  {}", bloat_label), width));
    out.push('\n');
    if width >= 66 {
        out.push_str(&box_row(
            &format!(
                "     {}   {} ──► {}",
                rpm_badge, grade_pill, grade_label_styled
            ),
            width,
        ));
    } else {
        out.push_str(&box_row(
            &format!("     {}   {}", rpm_badge, grade_pill),
            width,
        ));
    }
    out.push('\n');

    out.push_str(&box_bot(width));
    out.push('\n');

    // ─── CARD 4: EDGE & HARDWARE TELEMETRY ──────────────────────────────────
    let telem_title = format!(
        "🖥️ {}",
        gradient_text(
            "EDGE & HARDWARE TELEMETRY",
            &[(0.0, DEEP_VIOLET), (1.0, CYBER_CYAN)]
        )
    );
    out.push_str(&box_top(&telem_title, width));
    out.push('\n');

    // Server Telemetry
    let cwnd_str = match report.edge.cf_l4_cwnd {
        Some(cwnd) => format!(" · cwnd: {}", cwnd),
        None => String::new(),
    };
    let retrans_str = match report.edge.cf_l4_retrans {
        Some(r) => format!(" · {} Retrans", r),
        None => String::new(),
    };
    let srv_desc = format!("{} Anycast Edge{}{}", colo_code, cwnd_str, retrans_str);
    out.push_str(&box_row(
        &format!(
            "  {}  {}",
            color_fg("Server Telemetry ·", DIM_GRAY),
            color_bold(&srv_desc, BRIGHT_WHITE)
        ),
        width,
    ));
    out.push('\n');

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
        Some(w) => format!(" — {}", color_fg(w, ELECTRIC_AMBER)),
        None => format!(" — {}", color_fg("Nominal capacity", KIWI_EMERALD)),
    };
    if width >= 66 {
        out.push_str(&box_row(
            &format!(
                "  {}  {}{}",
                color_fg("PHY Link Telemetry ·", DIM_GRAY),
                color_bold(&link_desc, BRIGHT_WHITE),
                warning_note
            ),
            width,
        ));
    } else {
        out.push_str(&box_row(
            &format!(
                "  {}  {}",
                color_fg("PHY Link ·", DIM_GRAY),
                color_bold(&link_desc, BRIGHT_WHITE)
            ),
            width,
        ));
    }
    out.push('\n');

    // Resource Profile
    let res_desc = format!(
        "Cold: {:.1}ms · Peak RSS: {:.1} MB · Duration: {:.1}s",
        report.resources.cold_start_ms,
        report.resources.peak_rss_mb,
        report.resources.duration_secs
    );
    out.push_str(&box_row(
        &format!(
            "  {}  {}",
            color_fg("Resource Profile ·", DIM_GRAY),
            color_fg(&res_desc, CYBER_CYAN)
        ),
        width,
    ));
    out.push('\n');

    out.push_str(&box_bot(width));
    out.push('\n');

    // ─── CARD 5: ISP SPEEDTEST NEUTRALITY AUDIT (if enabled) ────────────────
    if let Some(ref audit) = report.isp_audit {
        let audit_title = format!(
            "⚖️ {}",
            gradient_text(
                "ISP SPEEDTEST NEUTRALITY & ROUTING AUDIT",
                &[(0.0, ELECTRIC_AMBER), (1.0, KIWI_EMERALD)]
            )
        );
        out.push_str(&box_top(&audit_title, width));
        out.push('\n');

        let (verdict_color, verdict_badge_text) = if audit.is_throttled {
            (NEON_MAGENTA, "AGGRESSIVE SHAPING / CHEATING")
        } else if audit.neutrality_ratio >= 0.85 {
            (KIWI_EMERALD, "NEUTRAL · HONEST ROUTING")
        } else {
            (ELECTRIC_AMBER, "MILD PEERING SHAPING")
        };

        let verdict_pill = pill_badge(verdict_badge_text, verdict_color);
        let ratio_str = format!("Neutrality Ratio: {:.2}", audit.neutrality_ratio);

        out.push_str(&box_row(
            &format!("  {}   {}", verdict_pill, color_bold(&ratio_str, BRIGHT_WHITE)),
            width,
        ));
        out.push('\n');

        let speeds_str = format!(
            "Transit: {:.1} Mbps ({}) · On-Net: {:.1} Mbps ({})",
            audit.anycast_download_mbps, audit.anycast_host,
            audit.on_net_download_mbps, audit.on_net_host,
        );
        out.push_str(&box_row(&format!("  {}", color_fg(&speeds_str, DIM_GRAY)), width));
        out.push('\n');

        let analysis_wrapped = format!("Analysis: {}", audit.analysis);
        let max_len = width.saturating_sub(6);
        let display_analysis = if analysis_wrapped.len() > max_len {
            format!("{}...", &analysis_wrapped[..max_len.saturating_sub(3)])
        } else {
            analysis_wrapped
        };
        out.push_str(&box_row(
            &format!("  {}", color_fg(&display_analysis, BRIGHT_WHITE)),
            width,
        ));
        out.push('\n');

        out.push_str(&box_bot(width));
        out.push('\n');
    }

    out
}

/// Formats a glowing pill badge: `[ 526.4 Mbps ]` with subtle slate borders
fn pill_badge(content: &str, content_color: (u8, u8, u8)) -> String {
    let bracket_l = color_fg("[ ", BORDER_COLOR);
    let bracket_r = color_fg(" ]", BORDER_COLOR);
    let colored_content = color_bold(content, content_color);
    format!("{}{}{}", bracket_l, colored_content, bracket_r)
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
