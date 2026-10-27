use clap::Parser;
use std::io::IsTerminal;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "kiwi",
    author = "Kiwi Engineering Team",
    version = "1.0.0",
    about = "🥝 State-of-the-Art Zero-Footprint 2026 CLI Network Intelligence Engine"
)]
pub struct Cli {
    /// Output machine-readable JSON format
    #[arg(long)]
    pub json: bool,

    /// Ultra-fast 2-second sanity check mode
    #[arg(long)]
    pub fast: bool,

    /// Deep diagnostic mode (10s saturation, extended bufferbloat & TCP audit)
    #[arg(long)]
    pub deep: bool,

    /// Headless non-interactive plain-text output (no Braille or ANSI overwriting)
    #[arg(long)]
    pub plain: bool,

    /// Target specific network interface (e.g. wlp0s20f3, eth0)
    #[arg(short = 'I', long = "interface")]
    pub interface: Option<String>,

    /// Force IPv4 stack only (disables Happy Eyeballs IPv6)
    #[arg(short = '4')]
    pub ipv4_only: bool,

    /// Force IPv6 stack only (disables Happy Eyeballs IPv4)
    #[arg(short = '6')]
    pub ipv6_only: bool,

    /// Low-bandwidth / SSH mode (throttles UI to 10 FPS, simpler glyphs)
    #[arg(long)]
    pub low_bandwidth: bool,

    /// HTTP/3 QUIC over UDP mode
    #[arg(long)]
    pub quic: bool,

    /// Custom target host (defaults to speed.cloudflare.com)
    #[arg(long, default_value = "speed.cloudflare.com")]
    pub host: String,

    /// Number of concurrent multiplexed streams (default: 4)
    #[arg(long, default_value = "4")]
    pub streams: usize,
}

impl Cli {
    /// Determines whether output should be formatted as JSON automatically
    pub fn is_json_effective(&self) -> bool {
        if self.json {
            return true;
        }
        // If not connected to a TTY and user hasn't explicitly asked for plain output,
        // automatically switch to JSON for pipelines (e.g., `kiwi | jq`).
        if !std::io::stdout().is_terminal() && !self.plain {
            return true;
        }
        false
    }

    /// Determines whether ANSI cursor overwriting should be disabled
    pub fn is_plain_effective(&self) -> bool {
        self.plain || !std::io::stdout().is_terminal()
    }

    /// Test phase duration in seconds
    pub fn phase_duration_secs(&self) -> f64 {
        if self.fast {
            2.0
        } else if self.deep {
            10.0
        } else {
            5.0
        }
    }

    /// Number of idle latency micro-probes
    pub fn idle_probe_count(&self) -> usize {
        if self.fast {
            5
        } else if self.deep {
            30
        } else {
            15
        }
    }
}
