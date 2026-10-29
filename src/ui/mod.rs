pub mod art;
pub mod bento;
pub mod braille;
pub mod layout;
pub mod prometheus;
pub mod svg;

use crate::metrics::{
    BenchmarkReport, BufferbloatGrade, EdgeTelemetry, LatencyStats, ResourceProfile,
};
use crate::ui::art::{
    BRIGHT_WHITE, CYBER_CYAN, DIM_GRAY, ELECTRIC_AMBER, KIWI_EMERALD, NEON_MAGENTA, box_bot,
    box_row, box_sep, box_top, color_bold, color_fg, pad_to_width,
};
use crate::ui::braille::{
    calculate_gauge_scale, render_colored_tachometer, render_phosphor_canvas,
};
use crate::ui::layout::{LayoutMode, TerminalLayout};
use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestPhase {
    Warmup,
    IdleLatency,
    Download,
    Upload,
    Bidirectional,
    AuditIsp,
    Complete,
}

pub struct LiveUiState {
    pub phase: TestPhase,
    pub current_mbps: f64,
    pub peak_mbps: f64,
    pub down_mbps: f64,
    pub up_mbps: f64,
    pub total_bytes: u64,
    pub elapsed_secs: f64,
    pub active_streams: usize,
    pub throughput_history: Vec<f64>,
    pub idle_stats: Option<LatencyStats>,
    pub live_loaded_ping_ms: Option<f64>,
    pub edge_telemetry: EdgeTelemetry,
    pub protocol_desc: String,
}

pub struct TerminalRenderer {
    last_rendered_lines: usize,
    plain_mode: bool,
    low_bandwidth: bool,
}

impl TerminalRenderer {
    pub fn new(plain_mode: bool, low_bandwidth: bool) -> Self {
        if !plain_mode {
            crate::safety::hide_cursor();
        }
        TerminalRenderer {
            last_rendered_lines: 0,
            plain_mode,
            low_bandwidth,
        }
    }

    /// Render a single frame of the live transient UI in-place
    pub fn render_frame(&mut self, state: &LiveUiState) {
        if self.plain_mode {
            // In plain/headless mode, no in-place overwrites
            return;
        }

        let layout = TerminalLayout::current(self.low_bandwidth);
        let mut buffer = String::with_capacity(2048);
        let mut line_count = 0;

        match layout.mode {
            LayoutMode::Full => {
                let term_cols = layout.cols as usize;
                let width = term_cols.clamp(74, 82);

                // Line 1: Header Top Border
                let brand = format!(
                    "🥝 {}  {}",
                    color_bold("KIWI", KIWI_EMERALD),
                    color_fg("꘡ EDGE NETWORK INTELLIGENCE [v1.0.0]", CYBER_CYAN)
                );
                buffer.push_str(&box_top(&brand, width));
                buffer.push('\n');
                line_count += 1;

                // Line 2: Route & Proto
                let client_desc = if !state.edge_telemetry.client_ip.is_empty() {
                    let asn = state
                        .edge_telemetry
                        .asn
                        .map(|a| format!("AS{}", a))
                        .unwrap_or_default();
                    format!("{} {}", asn, state.edge_telemetry.city)
                } else {
                    "Client Route".to_string()
                };
                let edge_desc = if !state.edge_telemetry.colo.is_empty() {
                    format!("{} Edge", state.edge_telemetry.colo)
                } else {
                    "Cloudflare Anycast".to_string()
                };
                let proto_clean = {
                    let p = &state.protocol_desc;
                    let is_v6 = p.contains("IPv6") || p.contains("v6");
                    let proto_name = if p.contains("h2") { "h2" } else { "http/1.1" };
                    let family = if is_v6 { "IPv6" } else { "IPv4" };
                    format!("{} · {} · TLS 1.3", family, proto_name)
                };
                let route_str = format!(
                    "{} {} ──► {} │ {}",
                    color_fg("ROUTE:", DIM_GRAY),
                    color_bold(client_desc.trim(), BRIGHT_WHITE),
                    color_bold(&edge_desc, KIWI_EMERALD),
                    color_fg(&proto_clean, DIM_GRAY),
                );
                buffer.push_str(&box_row(&route_str, width));
                buffer.push('\n');
                line_count += 1;

                // Line 3: Separator
                buffer.push_str(&box_sep(width));
                buffer.push('\n');
                line_count += 1;

                // Line 4: Section Title & Direction Pulse
                let (phase_title, phase_color) = match state.phase {
                    TestPhase::Warmup => ("CONNECTING & WARMING UP...", CYBER_CYAN),
                    TestPhase::IdleLatency => {
                        ("MEASURING BASELINE IDLE LATENCY...", ELECTRIC_AMBER)
                    }
                    TestPhase::Download => ("SATURATED DOWNLOAD THROUGHPUT", CYBER_CYAN),
                    TestPhase::Upload => ("SATURATED UPLOAD THROUGHPUT", NEON_MAGENTA),
                    TestPhase::Bidirectional => {
                        ("FULL-DUPLEX SIMULTANEOUS SATURATION", KIWI_EMERALD)
                    }
                    TestPhase::AuditIsp => ("ISP ROUTING NEUTRALITY AUDIT", ELECTRIC_AMBER),
                    TestPhase::Complete => ("BENCHMARK FINALIZING...", KIWI_EMERALD),
                };
                let fps_str = if layout.is_ssh { "10 FPS" } else { "20 FPS" };
                let dir_symbol = match state.phase {
                    TestPhase::Download => "▲",
                    TestPhase::Upload => "▼",
                    TestPhase::Bidirectional => "▲▼",
                    TestPhase::AuditIsp => "⚖",
                    _ => "⚡",
                };
                let title_line = format!(
                    "{} {}  {}",
                    color_bold(dir_symbol, phase_color),
                    color_bold(phase_title, BRIGHT_WHITE),
                    color_fg(&format!("({} LIVE)", fps_str), DIM_GRAY),
                );
                buffer.push_str(&box_row(&title_line, width));
                buffer.push('\n');
                line_count += 1;

                // Line 5 & 6: Speed + Phosphor Oscilloscope Waveform
                let braille_width = 28.min(width.saturating_sub(44));
                let (top_wave, bot_wave) =
                    render_phosphor_canvas(&state.throughput_history, braille_width);

                let speed_badge = if state.phase == TestPhase::Bidirectional {
                    format!(
                        "{} · {}",
                        color_bold(&format!("↓{:>5.1}M", state.down_mbps), CYBER_CYAN),
                        color_bold(&format!("↑{:>5.1}M", state.up_mbps), NEON_MAGENTA),
                    )
                } else {
                    format!(
                        "{} {}",
                        color_bold(&format!("{:>6.1} Mbps", state.current_mbps), phase_color),
                        color_fg(&format!("(Peak: {:>5.1})", state.peak_mbps), DIM_GRAY),
                    )
                };
                let transferred_mb = state.total_bytes as f64 / (1024.0 * 1024.0);
                let transferred_str = format!("Transferred: {:>5.1} MB", transferred_mb);

                let inner_width = width.saturating_sub(4);
                let left_col_width = inner_width.saturating_sub(braille_width + 2);
                let left_row_1 = pad_to_width(&speed_badge, left_col_width);
                let left_row_2 =
                    pad_to_width(&color_fg(&transferred_str, DIM_GRAY), left_col_width);

                let wave_row_1 = format!("{}  {}", left_row_1, top_wave);
                buffer.push_str(&box_row(&wave_row_1, width));
                buffer.push('\n');
                line_count += 1;

                let wave_row_2 = format!("{}  {}", left_row_2, bot_wave);
                buffer.push_str(&box_row(&wave_row_2, width));
                buffer.push('\n');
                line_count += 1;

                // Line 7: Dynamic Tachometer Gauge
                let scale = calculate_gauge_scale(state.peak_mbps.max(state.current_mbps));
                let gauge_bar = render_colored_tachometer(state.current_mbps, scale, 34);
                let tacho_line = format!(
                    "{} {}  {:.1} / {:.0} Mbps",
                    color_fg("GAUGE:", DIM_GRAY),
                    gauge_bar,
                    state.current_mbps,
                    scale
                );
                buffer.push_str(&box_row(&tacho_line, width));
                buffer.push('\n');
                line_count += 1;

                // Line 8: Separator
                buffer.push_str(&box_sep(width));
                buffer.push('\n');
                line_count += 1;

                // Line 9: Baseline Idle Ping
                let idle_desc = match &state.idle_stats {
                    Some(s) => format!(
                        "{} (Jitter: ±{:.1} ms · Loss: {:.1}%)",
                        color_bold(&format!("{:.1} ms", s.median_ms), ELECTRIC_AMBER),
                        s.jitter_ms,
                        s.packet_loss_pct
                    ),
                    None => color_fg("Probing active...", DIM_GRAY),
                };
                let idle_line = format!(
                    "{}  {}",
                    color_fg("Baseline (Idle) Ping  ·", DIM_GRAY),
                    idle_desc
                );
                buffer.push_str(&box_row(&idle_line, width));
                buffer.push('\n');
                line_count += 1;

                // Line 10: Working (Loaded) Ping
                let (loaded_desc, grade_desc) = match (state.live_loaded_ping_ms, &state.idle_stats)
                {
                    (Some(loaded), Some(idle)) => {
                        let delta = (loaded - idle.median_ms).max(0.0);
                        let grade = BufferbloatGrade::compute(idle.median_ms, loaded, None);
                        (
                            format!(
                                "{} [ +{:.1} ms Delta ]",
                                color_bold(&format!("{:.1} ms", loaded), CYBER_CYAN),
                                delta
                            ),
                            format!(
                                "{} ──► [ GRADE {} : {} ]",
                                color_bold(&format!("{:>5} RPM", grade.rpm), KIWI_EMERALD),
                                grade.grade,
                                grade.rating_label
                            ),
                        )
                    }
                    _ => (
                        color_fg("Pending saturation...", DIM_GRAY),
                        color_fg("Computing...", DIM_GRAY),
                    ),
                };
                let loaded_line = format!(
                    "{}  {}",
                    color_fg("Working (Loaded) Ping ·", DIM_GRAY),
                    loaded_desc
                );
                buffer.push_str(&box_row(&loaded_line, width));
                buffer.push('\n');
                line_count += 1;

                // Line 11: Responsiveness Score
                let resp_line = format!(
                    "{}  {}",
                    color_fg("Responsiveness Score  ·", DIM_GRAY),
                    grade_desc
                );
                buffer.push_str(&box_row(&resp_line, width));
                buffer.push('\n');
                line_count += 1;

                // Line 12: Separator
                buffer.push_str(&box_sep(width));
                buffer.push('\n');
                line_count += 1;

                // Line 13: Footer Telemetry
                let rss_mb = ResourceProfile::current_rss_mb();
                let cwnd_val = state.edge_telemetry.cf_l4_cwnd.unwrap_or(0);
                let footer_line = format!(
                    "Streams: {} Multiplexed │ Edge cwnd: {} │ RSS: {:.1} MB │ Elapsed: {:.1}s",
                    state.active_streams, cwnd_val, rss_mb, state.elapsed_secs
                );
                buffer.push_str(&box_row(&color_fg(&footer_line, DIM_GRAY), width));
                buffer.push('\n');
                line_count += 1;

                // Line 14: Box Bottom
                buffer.push_str(&box_bot(width));
                buffer.push('\n');
                line_count += 1;
            }
            LayoutMode::Compact => {
                let width = (layout.cols as usize).clamp(50, 78);
                let phase_name = match state.phase {
                    TestPhase::Warmup => "Warmup",
                    TestPhase::IdleLatency => "Idle Ping",
                    TestPhase::Download => "Download",
                    TestPhase::Upload => "Upload",
                    TestPhase::Bidirectional => "Simultaneous",
                    TestPhase::AuditIsp => "ISP Audit",
                    TestPhase::Complete => "Done",
                };

                let brand = format!("🥝 KIWI [{}]", phase_name);
                buffer.push_str(&box_top(&brand, width));
                buffer.push('\n');
                line_count += 1;

                let spd = if state.phase == TestPhase::Bidirectional {
                    format!(
                        "{} ↓{:.1}M ↑{:.1}M (Agg: {:.1}M)",
                        color_bold("SPEED:", KIWI_EMERALD),
                        state.down_mbps,
                        state.up_mbps,
                        state.current_mbps
                    )
                } else {
                    format!(
                        "{} {:.1} Mbps  (Peak: {:.1} Mbps)",
                        color_bold("SPEED:", KIWI_EMERALD),
                        state.current_mbps,
                        state.peak_mbps
                    )
                };
                buffer.push_str(&box_row(&spd, width));
                buffer.push('\n');
                line_count += 1;

                let scale = calculate_gauge_scale(state.peak_mbps.max(state.current_mbps));
                let gauge_width = width.saturating_sub(24).min(30);
                let gauge_bar = render_colored_tachometer(state.current_mbps, scale, gauge_width);
                let gauge_line =
                    format!("{} {:.1}/{:.0} Mbps", gauge_bar, state.current_mbps, scale);
                buffer.push_str(&box_row(&gauge_line, width));
                buffer.push('\n');
                line_count += 1;

                let idle_str = state
                    .idle_stats
                    .as_ref()
                    .map(|s| format!("{:.1}ms", s.median_ms))
                    .unwrap_or_else(|| "--".to_string());
                let working_str = state
                    .live_loaded_ping_ms
                    .map(|p| format!("{:.1}ms", p))
                    .unwrap_or_else(|| "--".to_string());
                let lat_line = format!(
                    "Idle: {} │ Loaded: {} │ Streams: {}",
                    idle_str, working_str, state.active_streams
                );
                buffer.push_str(&box_row(&color_fg(&lat_line, DIM_GRAY), width));
                buffer.push('\n');
                line_count += 1;

                buffer.push_str(&box_bot(width));
                buffer.push('\n');
                line_count += 1;
            }
            LayoutMode::UltraDense => {
                let p_str = match state.phase {
                    TestPhase::Download => format!("Down: {:.1}M", state.current_mbps),
                    TestPhase::Upload => format!("Up: {:.1}M", state.current_mbps),
                    TestPhase::Bidirectional => {
                        format!("↓{:.1}M ↑{:.1}M", state.down_mbps, state.up_mbps)
                    }
                    TestPhase::AuditIsp => format!("Audit: {:.1}M", state.current_mbps),
                    TestPhase::IdleLatency => "Ping".to_string(),
                    _ => "Init".to_string(),
                };
                let ping = state.live_loaded_ping_ms.unwrap_or(0.0);
                let term_cols = layout.cols as usize;

                let line = if term_cols < 46 {
                    format!("🥝 {} · {:.0}ms", color_bold(&p_str, KIWI_EMERALD), ping)
                } else {
                    format!(
                        "🥝 [{}] [Ping: {}] [Streams: {}]",
                        color_bold(&p_str, KIWI_EMERALD),
                        color_bold(&format!("{:.1}ms", ping), CYBER_CYAN),
                        state.active_streams
                    )
                };
                let safe_line = pad_to_width(&line, term_cols.min(art::visible_width(&line)));
                buffer.push_str(&safe_line);
                buffer.push('\n');
                line_count += 1;
            }
        }

        self.apply_buffer(&buffer, line_count);
    }

    /// Rewind cursor in-place and output buffer
    fn apply_buffer(&mut self, buffer: &str, line_count: usize) {
        let mut stdout = std::io::stdout().lock();

        // If we rendered lines previously, move cursor up
        if self.last_rendered_lines > 0 {
            let _ = write!(stdout, "\x1b[?2025h\x1b[{}A\r", self.last_rendered_lines);
        } else {
            let _ = write!(stdout, "\x1b[?2025h");
        }

        // Write each line prefixed with clear-line \x1b[2K
        for line in buffer.lines() {
            let _ = writeln!(stdout, "\x1b[2K{}", line);
        }

        // \x1b[J: clear any leftover trailing lines, \x1b[?2025l: end synchronized update
        let _ = write!(stdout, "\x1b[J\x1b[?2025l");
        let _ = stdout.flush();

        self.last_rendered_lines = line_count;
    }

    /// Clear the live HUD and print the permanent Bento-box receipt
    pub fn finish_and_print_receipt(&mut self, report: &BenchmarkReport) {
        if !self.plain_mode && self.last_rendered_lines > 0 {
            let mut stdout = std::io::stdout().lock();
            // Move up to the first transient line and erase everything down to the end of screen
            let _ = write!(stdout, "\x1b[{}A\r\x1b[J", self.last_rendered_lines);
            let _ = stdout.flush();
            self.last_rendered_lines = 0;
        }

        crate::safety::restore_terminal();

        let (cols, _) = crossterm::terminal::size().unwrap_or((80, 24));
        let receipt = bento::render_bento_box(report, cols as usize);
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{}", receipt);
        let _ = stdout.flush();
    }
}
