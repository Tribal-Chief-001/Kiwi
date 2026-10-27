use crate::cli::Cli;
use crate::engine::probe::ClientHandle;
use crate::metrics::{
    BenchmarkReport, BufferbloatGrade, EdgeTelemetry, LatencyStats, ResourceProfile, ThroughputStats,
};
use crate::platform::detect_platform_info;
use crate::protocol::dns::{resolve_host, DnsCandidates};
use crate::protocol::h2::H2Client;
use crate::protocol::happy_eyeballs::connect_happy_eyeballs;
use crate::protocol::http1::Http1Client;
use crate::ui::{LiveUiState, TerminalRenderer, TestPhase};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::sleep;

pub async fn run_benchmark(cli: &Cli) -> Result<BenchmarkReport, Box<dyn std::error::Error>> {
    let bench_start = Instant::now();

    // 1. Link Layer Telemetry
    let platform_info = detect_platform_info(cli.interface.as_deref());

    if cli.plain && !cli.is_json_effective() {
        println!("🥝 KIWI: Starting network intelligence benchmark on {}...", platform_info.interface_name);
    }

    // 2. DNS Resolution via Happy Eyeballs candidates
    let candidates = resolve_host(&cli.host, 443, cli.ipv4_only, cli.ipv6_only).await?;

    // 3. Establish TCP + TLS connection (with 250ms race & 500ms TLS guard)
    let conn = connect_happy_eyeballs(&cli.host, &candidates, cli.interface.as_deref()).await?;
    let cold_start_ms = bench_start.elapsed().as_secs_f64() * 1000.0;

    let proto_desc = {
        let ip_family = if conn.is_ipv6 { "Dual-Stack IPv6" } else { "IPv4" };
        let alpn = conn
            .alpn
            .as_deref()
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .unwrap_or_else(|| "http/1.1".to_string());
        format!("{} ({}) · {} over TLS 1.3", ip_family, conn.peer_addr.ip(), alpn)
    };

    // 4. Initialize Client Protocol Engine (HTTP/2 or HTTP/1.1 based on negotiated ALPN)
    let is_h2 = conn.alpn.as_deref().map(|p| p == b"h2").unwrap_or(false);
    let mut client = if is_h2 {
        ClientHandle::H2(H2Client::new(conn.tls_stream, cli.host.clone()).await?)
    } else {
        ClientHandle::Http1(Box::new(Http1Client::new(conn.tls_stream, cli.host.clone())))
    };

    // 5. Initialize Live UI State & Terminal Renderer
    let is_json = cli.is_json_effective();
    let is_plain = cli.is_plain_effective();
    let mut renderer = TerminalRenderer::new(is_plain || is_json, cli.low_bandwidth);

    let mut ui_state = LiveUiState {
        phase: TestPhase::Warmup,
        current_mbps: 0.0,
        peak_mbps: 0.0,
        total_bytes: 0,
        elapsed_secs: 0.0,
        active_streams: 1,
        throughput_history: Vec::new(),
        idle_stats: None,
        live_loaded_ping_ms: None,
        edge_telemetry: EdgeTelemetry::default(),
        protocol_desc: proto_desc.clone(),
    };

    if !is_plain && !is_json {
        renderer.render_frame(&ui_state);
    }

    let probe_pending = Arc::new(AtomicBool::new(false));

    // 6. Phase 1: Baseline Unloaded Idle Latency
    ui_state.phase = TestPhase::IdleLatency;
    if !is_plain && !is_json {
        renderer.render_frame(&ui_state);
    }

    let idle_probes_count = cli.idle_probe_count();
    let (idle_stats, mut edge_telemetry_opt) = crate::engine::probe::run_idle_latency_test(
        &mut client,
        idle_probes_count,
        &probe_pending,
        &cli.host,
        &candidates,
        cli.interface.as_deref(),
    )
    .await;

    let mut edge_telemetry = edge_telemetry_opt.take().unwrap_or_default();
    ui_state.idle_stats = Some(idle_stats.clone());
    ui_state.edge_telemetry = edge_telemetry.clone();

    if cli.plain && !is_json {
        println!(
            "  ⚡ Baseline Idle Latency: {:.1} ms (Jitter: ±{:.1} ms, Loss: {:.1}%)",
            idle_stats.median_ms, idle_stats.jitter_ms, idle_stats.packet_loss_pct
        );
    }

    // 7. Phase 2: Saturated In-Band Download Test
    ui_state.phase = TestPhase::Download;
    ui_state.active_streams = cli.streams;
    ui_state.throughput_history.clear();

    let download_duration = Duration::from_secs_f64(cli.phase_duration_secs());
    let down_config = PhaseConfig {
        host: &cli.host,
        streams: cli.streams,
        duration: download_duration,
        is_download: true,
        suppress_ui: is_plain || is_json,
        probe_pending: probe_pending.clone(),
        candidates: &candidates,
        specified_iface: cli.interface.as_deref(),
    };
    let (download_stats, loaded_down_latency, down_telem) = run_phase_saturation(
        &mut client,
        down_config,
        &mut ui_state,
        &mut renderer,
    )
    .await;

    if let Some(t) = down_telem {
        merge_telemetry(&mut edge_telemetry, t);
        ui_state.edge_telemetry = edge_telemetry.clone();
    }

    if cli.plain && !is_json {
        println!(
            "  📥 Download Throughput: {:.1} Mbps (Peak: {:.1} Mbps, Loaded Latency: {:.1} ms)",
            download_stats.mbps, download_stats.peak_mbps, loaded_down_latency.p90_ms
        );
    }

    // 8. Phase 3: Saturated In-Band Upload Test
    // Verify client connection health before upload phase, re-establishing if closed
    let client_healthy = match &mut client {
        ClientHandle::H2(h2) => h2.send_request.clone().ready().await.is_ok(),
        ClientHandle::Http1(_) => true,
    };
    if !client_healthy
        && let Ok(conn) = connect_happy_eyeballs(&cli.host, &candidates, cli.interface.as_deref()).await
    {
        let is_h2 = conn.alpn.as_deref().map(|p| p == b"h2").unwrap_or(false);
        if is_h2 {
            if let Ok(new_h2) = H2Client::new(conn.tls_stream, cli.host.clone()).await {
                client = ClientHandle::H2(new_h2);
            }
        } else {
            client = ClientHandle::Http1(Box::new(Http1Client::new(conn.tls_stream, cli.host.clone())));
        }
    }

    ui_state.phase = TestPhase::Upload;
    ui_state.active_streams = cli.streams;
    ui_state.throughput_history.clear();

    let upload_duration = Duration::from_secs_f64(cli.phase_duration_secs());
    let up_config = PhaseConfig {
        host: &cli.host,
        streams: cli.streams,
        duration: upload_duration,
        is_download: false,
        suppress_ui: is_plain || is_json,
        probe_pending: probe_pending.clone(),
        candidates: &candidates,
        specified_iface: cli.interface.as_deref(),
    };
    let (upload_stats, loaded_up_latency, up_telem) = run_phase_saturation(
        &mut client,
        up_config,
        &mut ui_state,
        &mut renderer,
    )
    .await;

    if let Some(t) = up_telem {
        merge_telemetry(&mut edge_telemetry, t);
    }

    if cli.plain && !is_json {
        println!(
            "  📤 Upload Throughput: {:.1} Mbps (Peak: {:.1} Mbps, Loaded Latency: {:.1} ms)",
            upload_stats.mbps, upload_stats.peak_mbps, loaded_up_latency.p90_ms
        );
    }

    // 9. Compute Responsiveness, Apple RPM & Bufferbloat Grading
    let responsiveness = BufferbloatGrade::compute(
        idle_stats.median_ms,
        loaded_down_latency.p90_ms,
        Some(loaded_up_latency.p90_ms),
    );

    let total_elapsed = bench_start.elapsed().as_secs_f64();
    let current_rss = ResourceProfile::current_rss_mb();

    let report = BenchmarkReport {
        version: "1.0.0".to_string(),
        timestamp: chrono_or_now(),
        target_host: cli.host.clone(),
        protocol: proto_desc,
        edge: edge_telemetry,
        idle_latency: idle_stats,
        download: download_stats,
        loaded_down_latency,
        upload: upload_stats,
        loaded_up_latency,
        responsiveness,
        platform: platform_info,
        resources: ResourceProfile {
            cold_start_ms,
            peak_rss_mb: current_rss,
            duration_secs: total_elapsed,
        },
    };

    // 10. Output formatting
    ui_state.phase = TestPhase::Complete;
    if is_json {
        let json_str = serde_json::to_string_pretty(&report)?;
        println!("{}", json_str);
    } else {
        renderer.finish_and_print_receipt(&report);
    }

    Ok(report)
}

fn chrono_or_now() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now();
    let secs = now.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs();
    format!("Unix({})", secs)
}

fn merge_telemetry(target: &mut EdgeTelemetry, new_data: EdgeTelemetry) {
    if target.colo.is_empty() && !new_data.colo.is_empty() {
        target.colo = new_data.colo;
    }
    if target.asn.is_none() && new_data.asn.is_some() {
        target.asn = new_data.asn;
    }
    if target.city.is_empty() && !new_data.city.is_empty() {
        target.city = new_data.city;
    }
    if target.country.is_empty() && !new_data.country.is_empty() {
        target.country = new_data.country;
    }
    if target.client_ip.is_empty() && !new_data.client_ip.is_empty() {
        target.client_ip = new_data.client_ip;
    }
    if new_data.cf_l4_cwnd.is_some() {
        target.cf_l4_cwnd = new_data.cf_l4_cwnd;
    }
    if new_data.cf_l4_retrans.is_some() {
        target.cf_l4_retrans = new_data.cf_l4_retrans;
    }
    if new_data.cf_l4_rtt_us.is_some() {
        target.cf_l4_rtt_us = new_data.cf_l4_rtt_us;
    }
    if new_data.cf_l4_min_rtt_us.is_some() {
        target.cf_l4_min_rtt_us = new_data.cf_l4_min_rtt_us;
    }
    if new_data.cf_l4_delivery_rate.is_some() {
        target.cf_l4_delivery_rate = new_data.cf_l4_delivery_rate;
    }
}

struct PhaseConfig<'a> {
    host: &'a str,
    streams: usize,
    duration: Duration,
    is_download: bool,
    suppress_ui: bool,
    probe_pending: Arc<AtomicBool>,
    candidates: &'a DnsCandidates,
    specified_iface: Option<&'a str>,
}

async fn run_phase_saturation(
    client: &mut ClientHandle,
    config: PhaseConfig<'_>,
    ui_state: &mut LiveUiState,
    renderer: &mut TerminalRenderer,
) -> (ThroughputStats, LatencyStats, Option<EdgeTelemetry>) {
    let total_bytes = Arc::new(AtomicU64::new(0));
    let stop_signal = Arc::new(AtomicBool::new(false));
    let loaded_samples = Arc::new(Mutex::new(Vec::new()));
    let captured_telemetry = Arc::new(Mutex::new(None));

    // Spawn bulk streams
    let mut stream_handles = Vec::with_capacity(config.streams);

    match client {
        ClientHandle::H2(h2) => {
            for _ in 0..config.streams {
                let send_req = h2.send_request.clone();
                let host_clone = config.host.to_string();
                let bytes_counter = total_bytes.clone();
                let stop_flag = stop_signal.clone();
                let probe_flag = config.probe_pending.clone();
                let telem_target = captured_telemetry.clone();
                let is_down = config.is_download;

                let handle = tokio::spawn(async move {
                    if is_down {
                        let res = H2Client::run_download_stream(
                            send_req,
                            host_clone,
                            25_000_000,
                            bytes_counter,
                            stop_flag,
                            probe_flag,
                        )
                        .await;
                        if let Ok(Some(t)) = res {
                            let mut guard = telem_target.lock().await;
                            if guard.is_none() {
                                *guard = Some(t);
                            }
                        }
                    } else {
                        let res = H2Client::run_upload_stream(
                            send_req,
                            host_clone,
                            bytes_counter,
                            stop_flag,
                            probe_flag,
                        )
                        .await;
                        if let Ok(Some(t)) = res {
                            let mut guard = telem_target.lock().await;
                            if guard.is_none() {
                                *guard = Some(t);
                            }
                        }
                    }
                });
                stream_handles.push(handle);
            }
        }
        ClientHandle::Http1(_) => {
            for _ in 0..config.streams {
                let host_clone = config.host.to_string();
                let candidates_clone = config.candidates.clone();
                let iface_clone = config.specified_iface.map(|s| s.to_string());
                let bytes_counter = total_bytes.clone();
                let stop_flag = stop_signal.clone();
                let probe_flag = config.probe_pending.clone();
                let telem_target = captured_telemetry.clone();
                let is_down = config.is_download;

                let handle = tokio::spawn(async move {
                    if let Ok(conn) = connect_happy_eyeballs(&host_clone, &candidates_clone, iface_clone.as_deref()).await {
                        let h1_stream = Http1Client::new(conn.tls_stream, host_clone);
                        if is_down {
                            let res = h1_stream
                                .run_download_stream(
                                    25_000_000,
                                    bytes_counter,
                                    stop_flag,
                                    probe_flag,
                                )
                                .await;
                            if let Ok(Some(t)) = res {
                                let mut guard = telem_target.lock().await;
                                if guard.is_none() {
                                    *guard = Some(t);
                                }
                            }
                        } else {
                            let res = h1_stream
                                .run_upload_stream(
                                    bytes_counter,
                                    stop_flag,
                                    probe_flag,
                                )
                                .await;
                            if let Ok(Some(t)) = res {
                                let mut guard = telem_target.lock().await;
                                if guard.is_none() {
                                    *guard = Some(t);
                                }
                            }
                        }
                    }
                });
                stream_handles.push(handle);
            }
        }
    }

    // In-band latency probe loop
    let probe_stop = stop_signal.clone();
    let probe_samples_target = loaded_samples.clone();
    let probe_flag = config.probe_pending.clone();
    let attempted_probes = Arc::new(AtomicU64::new(0));
    let attempted_counter = attempted_probes.clone();

    let mut probe_handle = match client {
        ClientHandle::H2(h2) => {
            let mut probe_client = H2Client {
                send_request: h2.send_request.clone(),
                host: config.host.to_string(),
            };
            let host_probe = config.host.to_string();
            let candidates_probe = config.candidates.clone();
            let iface_probe = config.specified_iface.map(|s| s.to_string());

            tokio::spawn(async move {
                while !probe_stop.load(Ordering::Relaxed) {
                    sleep(Duration::from_millis(150)).await;
                    if probe_stop.load(Ordering::Relaxed) {
                        break;
                    }
                    attempted_counter.fetch_add(1, Ordering::Relaxed);
                    match probe_client.dispatch_probe(&probe_flag).await {
                        Ok((rtt, _)) => {
                            let mut guard = probe_samples_target.lock().await;
                            guard.push(rtt);
                        }
                        Err(_) => {
                            // If H2 connection dropped or desynchronized under loss, reconnect
                            if let Ok(conn) = connect_happy_eyeballs(&host_probe, &candidates_probe, iface_probe.as_deref()).await
                                && let Ok(new_h2) = H2Client::new(conn.tls_stream, host_probe.clone()).await
                            {
                                probe_client = new_h2;
                            }
                        }
                    }
                }
            })
        }
        ClientHandle::Http1(_) => {
            let host_probe = config.host.to_string();
            let candidates_probe = config.candidates.clone();
            let iface_probe = config.specified_iface.map(|s| s.to_string());
            tokio::spawn(async move {
                let mut h1_probe_opt: Option<Http1Client> = None;
                while !probe_stop.load(Ordering::Relaxed) {
                    sleep(Duration::from_millis(150)).await;
                    if probe_stop.load(Ordering::Relaxed) {
                        break;
                    }
                    if h1_probe_opt.is_none() {
                        if let Ok(conn) = connect_happy_eyeballs(&host_probe, &candidates_probe, iface_probe.as_deref()).await {
                            h1_probe_opt = Some(Http1Client::new(conn.tls_stream, host_probe.clone()));
                        } else {
                            attempted_counter.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                    }

                    if let Some(mut h1) = h1_probe_opt.take() {
                        attempted_counter.fetch_add(1, Ordering::Relaxed);
                        match h1.dispatch_probe(&probe_flag).await {
                            Ok((rtt, _)) => {
                                let mut guard = probe_samples_target.lock().await;
                                guard.push(rtt);
                                h1_probe_opt = Some(h1);
                            }
                            Err(_) => {
                                // Dead or desynchronized socket is dropped immediately.
                                // Attempt reconnect for subsequent probes.
                                if let Ok(new_conn) = connect_happy_eyeballs(&host_probe, &candidates_probe, iface_probe.as_deref()).await {
                                    h1_probe_opt = Some(Http1Client::new(new_conn.tls_stream, host_probe.clone()));
                                }
                            }
                        }
                    }
                }
            })
        }
    };

    // UI Tick loop (dynamic 10 FPS for SSH, 20 FPS for local)
    let layout = crate::ui::layout::TerminalLayout::current(false);
    let tick_interval = Duration::from_millis(1000 / layout.fps);
    let phase_start = Instant::now();
    let mut last_tick = phase_start;
    let mut last_bytes = 0u64;
    let mut peak_mbps = 0.0f64;
    let mut throughput_history = Vec::new();

    while phase_start.elapsed() < config.duration {
        sleep(tick_interval).await;
        let now = Instant::now();
        let delta_secs = (now - last_tick).as_secs_f64();
        last_tick = now;

        let current_bytes = total_bytes.load(Ordering::Relaxed);
        let delta_bytes = current_bytes.saturating_sub(last_bytes);
        last_bytes = current_bytes;

        let current_mbps = if delta_secs > 0.0 {
            (delta_bytes as f64 * 8.0) / (delta_secs * 1_000_000.0)
        } else {
            0.0
        };

        if current_mbps > peak_mbps {
            peak_mbps = current_mbps;
        }

        throughput_history.push(current_mbps);

        // Fetch latest probe RTT
        let latest_ping = {
            let guard = loaded_samples.lock().await;
            guard.last().copied()
        };

        ui_state.current_mbps = current_mbps;
        ui_state.peak_mbps = peak_mbps;
        ui_state.total_bytes = current_bytes;
        ui_state.elapsed_secs = phase_start.elapsed().as_secs_f64();
        ui_state.throughput_history = throughput_history.clone();
        ui_state.live_loaded_ping_ms = latest_ping;

        if !config.suppress_ui {
            renderer.render_frame(ui_state);
        }
    }

    // Stop all workers with timeout safety guards and explicit task abort to prevent task leakage
    stop_signal.store(true, Ordering::Release);
    if tokio::time::timeout(Duration::from_millis(800), &mut probe_handle).await.is_err() {
        probe_handle.abort();
    }
    for mut h in stream_handles {
        if tokio::time::timeout(Duration::from_millis(800), &mut h).await.is_err() {
            h.abort();
        }
    }
    config.probe_pending.store(false, Ordering::Release);

    let total_transferred = total_bytes.load(Ordering::Relaxed);
    let total_secs = phase_start.elapsed().as_secs_f64();
    let avg_bps = if total_secs > 0.0 {
        (total_transferred as f64 * 8.0) / total_secs
    } else {
        0.0
    };
    let avg_mbps = avg_bps / 1_000_000.0;

    let final_samples = {
        let guard = loaded_samples.lock().await;
        guard.clone()
    };
    let total_attempted = (attempted_probes.load(Ordering::Relaxed) as usize).max(final_samples.len());
    let latency_stats = LatencyStats::calculate(final_samples, total_attempted);

    let telem = {
        let guard = captured_telemetry.lock().await;
        guard.clone()
    };

    let stats = ThroughputStats {
        bps: avg_bps,
        mbps: avg_mbps,
        peak_mbps,
        total_bytes: total_transferred,
        duration_secs: total_secs,
        history: throughput_history,
    };

    (stats, latency_stats, telem)
}
