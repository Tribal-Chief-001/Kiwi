pub mod bento;
pub mod braille;
pub mod layout;

use crate::metrics::{
    BenchmarkReport, BufferbloatGrade, EdgeTelemetry, LatencyStats, ResourceProfile,
};
use crate::ui::braille::{calculate_gauge_scale, render_braille_canvas, render_tachometer};
use crate::ui::layout::{LayoutMode, TerminalLayout};
use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestPhase {
    Warmup,
    IdleLatency,
    Download,
    Upload,
    Complete,
}

pub struct LiveUiState {
    pub phase: TestPhase,
    pub current_mbps: f64,
    pub peak_mbps: f64,
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
        let mut buffer = String::new();
        let mut line_count = 0;

        match layout.mode {
            LayoutMode::Full => {
                let term_cols = layout.cols as usize;
                let width = term_cols.clamp(80, 86);
                let sep = "─".repeat(width);

                // Line 1: Header
                buffer.push_str(
                    "\x1b[1;32m🥝 KIWI ꘡ ULTRA-FAST EDGE NETWORK INTELLIGENCE [v1.0.0]\x1b[0m\n",
                );
                line_count += 1;

                // Line 2: Separator
                buffer.push_str(&format!("\x1b[38;5;242m{}\x1b[0m\n", sep));
                line_count += 1;

                // Line 3: Route & Proto
                let client_desc = if !state.edge_telemetry.client_ip.is_empty() {
                    let asn = state
                        .edge_telemetry
                        .asn
                        .map(|a| format!("AS{}", a))
                        .unwrap_or_default();
                    format!("{} {}", asn, state.edge_telemetry.city)
                } else {
                    "Client Edge Route".to_string()
                };
                let edge_desc = if !state.edge_telemetry.colo.is_empty() {
                    format!("{} Anycast Edge", state.edge_telemetry.colo)
                } else {
                    "Cloudflare Fleet".to_string()
                };
                buffer.push_str(&format!(
                    "  \x1b[1mROUTE\x1b[0m: {} ──► {} │ \x1b[1mPROTO\x1b[0m: {}\n",
                    client_desc.trim(),
                    edge_desc,
                    state.protocol_desc
                ));
                line_count += 1;

                // Line 4: Separator
                buffer.push_str(&format!("\x1b[38;5;242m{}\x1b[0m\n", sep));
                line_count += 1;

                // Line 5: Section Title
                let phase_title = match state.phase {
                    TestPhase::Warmup => "CONNECTING & WARMING UP...",
                    TestPhase::IdleLatency => "MEASURING BASELINE IDLE LATENCY...",
                    TestPhase::Download => "DOWNLOAD THROUGHPUT",
                    TestPhase::Upload => "UPLOAD THROUGHPUT",
                    TestPhase::Complete => "BENCHMARK FINISHING...",
                };
                let fps_str = if layout.is_ssh { "10 FPS" } else { "20 FPS" };
                buffer.push_str(&format!(
                    "  \x1b[1m{:<40} REAL-TIME VELOCITY WAVEFORM ({})\x1b[0m\n",
                    phase_title, fps_str
                ));
                line_count += 1;

                // Line 6 & 7: Speed + Braille Waveform
                let braille_width = 36.min(width.saturating_sub(44));
                let (top_wave, bot_wave) =
                    render_braille_canvas(&state.throughput_history, braille_width);

                let dir_symbol = match state.phase {
                    TestPhase::Download => "\x1b[36m▲\x1b[0m",
                    TestPhase::Upload => "\x1b[35m▼\x1b[0m",
                    _ => "\x1b[33m•\x1b[0m",
                };
                buffer.push_str(&format!(
                    "  {} \x1b[1m{:>6.1} Mbps\x1b[0m                            \x1b[36m{}\x1b[0m\n",
                    dir_symbol, state.current_mbps, top_wave
                ));
                line_count += 1;

                let transferred_mb = state.total_bytes as f64 / (1024.0 * 1024.0);
                buffer.push_str(&format!(
                    "  Peak: {:.1} Mbps · Transferred: {:.1} MB      \x1b[36m{}\x1b[0m\n",
                    state.peak_mbps, transferred_mb, bot_wave
                ));
                line_count += 1;

                // Line 8: Empty
                buffer.push('\n');
                line_count += 1;

                // Line 9 & 10: Tachometer
                let scale = calculate_gauge_scale(state.peak_mbps.max(state.current_mbps));
                let gauge_bar = render_tachometer(state.current_mbps, scale, 42);
                buffer.push_str("  \x1b[1mDYNAMIC TACHOMETER GAUGE:\x1b[0m\n");
                line_count += 1;
                buffer.push_str(&format!(
                    "  \x1b[32m{}\x1b[0m  {:.1} / {:.0} Mbps\n",
                    gauge_bar, state.current_mbps, scale
                ));
                line_count += 1;

                // Line 11: Empty
                buffer.push('\n');
                line_count += 1;

                // Line 12..15: Bufferbloat Monitor
                buffer.push_str("  \x1b[1mIETF RESPONSIVENESS & BUFFERBLOAT MONITOR:\x1b[0m\n");
                line_count += 1;

                let idle_desc = match &state.idle_stats {
                    Some(s) => format!(
                        "{:.1} ms  (Jitter: ±{:.1} ms · Loss: {:.1}%)",
                        s.median_ms, s.jitter_ms, s.packet_loss_pct
                    ),
                    None => "Probing...".to_string(),
                };
                buffer.push_str(&format!("  Baseline (Idle) Ping  :  {}\n", idle_desc));
                line_count += 1;

                let (loaded_desc, grade_desc) = match (state.live_loaded_ping_ms, &state.idle_stats)
                {
                    (Some(loaded), Some(idle)) => {
                        let delta = (loaded - idle.median_ms).max(0.0);
                        let grade = BufferbloatGrade::compute(idle.median_ms, loaded, None);
                        (
                            format!("{:.1} ms  [ +{:.1} ms Bufferbloat Delta ]", loaded, delta),
                            format!(
                                "{:>5} RPM  ──►  [ GRADE {} : {} ]",
                                grade.rpm, grade.grade, grade.rating_label
                            ),
                        )
                    }
                    _ => ("Pending...".to_string(), "Computing...".to_string()),
                };
                buffer.push_str(&format!("  Working (Loaded) Ping :  {}\n", loaded_desc));
                line_count += 1;
                buffer.push_str(&format!("  Responsiveness Score  :  {}\n", grade_desc));
                line_count += 1;

                // Line 16: Empty
                buffer.push('\n');
                line_count += 1;

                // Line 17: Footer
                let rss_mb = ResourceProfile::current_rss_mb();
                let cwnd_val = state.edge_telemetry.cf_l4_cwnd.unwrap_or(0);
                buffer.push_str(&format!(
                    "  Active Streams: {} Multiplexed │ Edge cwnd: {} │ Memory RSS: {:.1} MB\n",
                    state.active_streams, cwnd_val, rss_mb
                ));
                line_count += 1;
            }
            LayoutMode::Compact => {
                let phase_name = match state.phase {
                    TestPhase::Warmup => "Warmup",
                    TestPhase::IdleLatency => "Idle Ping",
                    TestPhase::Download => "Download",
                    TestPhase::Upload => "Upload",
                    TestPhase::Complete => "Done",
                };

                buffer.push_str(&format!(
                    "🥝 \x1b[1mKIWI\x1b[0m [Phase: \x1b[36m{}\x1b[0m] \x1b[1;32m{:.1} Mbps\x1b[0m (Peak: {:.1} Mbps)\n",
                    phase_name, state.current_mbps, state.peak_mbps
                ));
                line_count += 1;

                let scale = calculate_gauge_scale(state.peak_mbps.max(state.current_mbps));
                let gauge_bar = render_tachometer(state.current_mbps, scale, 30);
                buffer.push_str(&format!(
                    "  {} {:.1}/{:.0} Mbps\n",
                    gauge_bar, state.current_mbps, scale
                ));
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
                buffer.push_str(&format!(
                    "  Idle: {} │ Loaded: {} │ Streams: {}\n",
                    idle_str, working_str, state.active_streams
                ));
                line_count += 1;

                buffer.push_str(&format!(
                    "  Colo: {} │ RSS: {:.1} MB │ {}\n",
                    if state.edge_telemetry.colo.is_empty() {
                        "Edge"
                    } else {
                        &state.edge_telemetry.colo
                    },
                    ResourceProfile::current_rss_mb(),
                    state.protocol_desc
                ));
                line_count += 1;
            }
            LayoutMode::UltraDense => {
                let p_str = match state.phase {
                    TestPhase::Download => format!("Down: {:.1}M", state.current_mbps),
                    TestPhase::Upload => format!("Up: {:.1}M", state.current_mbps),
                    TestPhase::IdleLatency => "Ping".to_string(),
                    _ => "Init".to_string(),
                };
                let ping = state.live_loaded_ping_ms.unwrap_or(0.0);
                buffer.push_str(&format!(
                    "🥝 [{}] [Ping: {:.1}ms] [Streams: {}]\n",
                    p_str, ping, state.active_streams
                ));
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
            // \x1b[?2025h: begin synchronized update
            // \x1b[{N}A: move cursor up N lines
            // \r: carriage return
            let _ = write!(stdout, "\x1b[?2025h\x1b[{}A\r", self.last_rendered_lines);
        } else {
            let _ = write!(stdout, "\x1b[?2025h");
        }

        // Write each line prefixed with clear-line \x1b[2K
        for line in buffer.lines() {
            let _ = writeln!(stdout, "\x1b[2K{}", line);
        }

        // \x1b[?2025l: end synchronized update
        let _ = write!(stdout, "\x1b[?2025l");
        let _ = stdout.flush();

        self.last_rendered_lines = line_count;
    }

    /// Clear the live HUD and print the permanent Bento-box receipt
    pub fn finish_and_print_receipt(&mut self, report: &BenchmarkReport) {
        if !self.plain_mode && self.last_rendered_lines > 0 {
            let mut stdout = std::io::stdout().lock();
            // Move up and clear all transient lines
            let _ = write!(stdout, "\x1b[{}A\r", self.last_rendered_lines);
            for _ in 0..self.last_rendered_lines {
                let _ = writeln!(stdout, "\x1b[2K");
            }
            let _ = write!(stdout, "\x1b[{}A\r", self.last_rendered_lines);
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
