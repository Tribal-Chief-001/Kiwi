# 🥝 Kiwi: Zero-Footprint 2026 CLI Network Intelligence Engine

[![Pure Rust](https://img.shields.io/badge/Language-Rust_2024-orange.svg)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Zero C Dependencies](https://img.shields.io/badge/C_Deps-Zero-success.svg)]()
[![Peak RSS](https://img.shields.io/badge/RSS-<6MB-brightgreen.svg)]()

> **Kiwi** is a high-performance, single-binary network intelligence utility built for modern 2026 systems. Engineered from the ground up in pure Rust with zero C/OpenSSL dynamic dependencies, Kiwi measures real-world network responsiveness, throughput, and bufferbloat directly in-place without ever hijacking your shell or launching alternate screens.

---

## ⚡ Core Philosophy & Architecture

1. **In-Place Execution ("Alag Se Terminal Na Khole")**:
   - Executes directly inside your active shell session.
   - Never switches to alternate screen buffers (`smcup`/`rmcup`).
   - Uses relative ANSI cursor rewinds for smooth 20 FPS (10 FPS under SSH) Unicode Braille waveforms (`⡀⣀⣠⣤⣶⣿`) and tachometer gauges.
   - Upon completion, live transient gauges cleanly collapse into a permanent, compact **Bento-box receipt** leaving terminal scrollback 100% pristine.
2. **SOTA 2026 Measurement Paradigms**:
   - **IETF IPPM Responsiveness (`draft-ietf-ippm-responsiveness`)**: Measures true bufferbloat via in-band multiplexed micro-probes during saturated load to defeat ICMP QoS priority cheating.
   - **Apple RPM (Round-trips Per Minute)**: Calculates working RTT and assigns bufferbloat grades (A+ to F).
   - **Happy Eyeballs v2 (RFC 8305)**: Concurrent IPv6/IPv4 DNS resolution with a 250ms TCP SYN race and a 500ms TLS Handshake Guard to defeat PMTUD blackholes.
   - **Cloudflare Edge Telemetry**: Parses `server-timing: cfL4` descriptors for server-observed RTT, cwnd, retransmissions, client IP, ASN, and edge colocation.
3. **Sub-6MB Peak RSS & Zero Hot-Loop Allocations**:
   - Single-threaded Tokio runtime driver.
   - L1d cache-aligned 16 KB stack ring sinks matching RFC 8446 max TLS 1.3 records.
   - Dynamic BDP flow-control scaling (1 MB stream / 2 MB connection window) with immediate `WINDOW_UPDATE` frames.
   - Static pre-seeded pseudo-random upload slices to prevent middlebox compression bias.
   - Task-local register batch accumulation flushing to atomic byte counters every 64 KB.
4. **Systems & Signal Safety**:
   - Global atomic `PROBE_PENDING` cooperative yield ensures bulk traffic tasks yield execution to latency probes.
   - Synchronous panic hook (`std::panic::set_hook`) and OS signal handlers (`SIGINT`/`SIGTERM`) guarantee cursor restoration even under `panic = "abort"`.
   - Windows 1.0 ms timer precision via `timeBeginPeriod(1)`.

---

## 🚀 Installation & Build

```bash
# Clone the repository
git clone https://github.com/Tribal-Chief-001/Kiwi.git
cd Kiwi

# Build optimized release binary
cargo build --release

# Run unit and integration tests
cargo test
```

---

## 💻 CLI Usage & Ergonomics

```bash
# 1. Standard instant test (Default: Auto BDP, in-place HUD)
kiwi

# 2. Ultra-fast burst mode (2-second quick sanity check)
kiwi --fast

# 3. Deep diagnostic mode (10s saturation, full bufferbloat & TCP audit)
kiwi --deep

# 4. Headless machine-readable JSON output (ideal for monitoring & CI/CD)
kiwi --json

# 5. Non-interactive plain text mode (no ANSI rewinds, simple stdout)
kiwi --plain

# 6. Target specific network interface
kiwi -I wlp0s20f3

# 7. Force IPv4 or IPv6 stack only (overrides Happy Eyeballs)
kiwi -4
kiwi -6

# 8. Low-bandwidth / SSH mode (throttles to 10 FPS, ASCII tachometers)
kiwi --low-bandwidth
```

### Automatic Pipeline Detection

When stdout is redirected or piped (e.g. `kiwi | jq .`), Kiwi automatically switches to JSON output mode without ANSI escape sequences.

---

## 📊 Bento-Box Benchmark Receipt

```text
🥝 KIWI BENCHMARK RECEIPT
════════════════════════════════════════════════════════════════════════════════
  Client       : 2409:40c4:19:8727:693c:4842:ec7:9ed1 (AS55836) · Bhopal, IN
  Edge Node    : Anycast Fleet [BOM Colocation · Bhopal, IN]
  Transport    : Dual-Stack IPv6 (2606:4700:7::da) · http/1.1 over TLS 1.3
────────────────────────────────────────────────────────────────────────────────
  METRIC                        MEASUREMENT         PEAK / DRIFT       RATING
────────────────────────────────────────────────────────────────────────────────
  📥 DOWNLOAD BANDWIDTH          123.7 Mbps          Peak: 299.0 Mbps   GOOD
  📤 UPLOAD BANDWIDTH             11.3 Mbps          Peak:  82.4 Mbps   LOW
  ⚡ UNLOADED IDLE LATENCY         82.7 ms            Jitter: ±15.1 ms  0.0% Loss
  🏋️ LOADED WORKING LATENCY       247.3 ms            Down: 189.7ms      Up: 304.8ms
  🎯 BUFFERBLOAT RESPONSIVENESS     243 RPM           +164.5ms Delta      GRADE C
────────────────────────────────────────────────────────────────────────────────
  Server Telemetry   : BOM Anycast Edge · cwnd: 51 · 0 Server Retransmissions
  PHY Link Telemetry : wlp0s20f3 (Wi-Fi) — Local link capacity nominal
  Resource Profile   : Cold Start: 90.3ms │ Peak RSS: 5.9 MB │ Duration: 5.2s
════════════════════════════════════════════════════════════════════════════════
```

---

## 🛠️ Verification & Quality Gates

- `cargo check`: 0 errors, 0 warnings
- `cargo clippy -- -D warnings`: 100% clean
- `cargo test`: 10/10 tests passing
- Stripped binary size: ~2.5 MB
- Memory RSS: strictly under 6 MB
