use crate::cli::Cli;
use crate::engine::probe::ClientHandle;
use crate::metrics::{
    BenchmarkReport, BufferbloatGrade, EdgeTelemetry, IspAuditReport, LatencyStats, ResourceProfile,
    ThroughputStats,
};
use crate::platform::detect_platform_info;
use crate::protocol::dns::{DnsCandidates, resolve_host};
use crate::protocol::h2::H2Client;
use crate::protocol::h3::H3Client;
use crate::protocol::happy_eyeballs::connect_happy_eyeballs;
use crate::protocol::http1::Http1Client;
use crate::ui::{LiveUiState, TerminalRenderer, TestPhase};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::sleep;

pub async fn run_benchmark(cli: &Cli) -> Result<BenchmarkReport, Box<dyn std::error::Error>> {
    let bench_start = Instant::now();

    // 1. Link Layer Telemetry
    let platform_info = detect_platform_info(cli.interface.as_deref());

    if cli.plain && !cli.is_json_effective() {
        println!(
            "🥝 KIWI: Starting network intelligence benchmark on {}...",
            platform_info.interface_name
        );
    }

    // 2. DNS Resolution via Happy Eyeballs candidates
    let port = cli.port.unwrap_or(if cli.http { 80 } else { 443 });
    let candidates = resolve_host(&cli.host, port, cli.ipv4_only, cli.ipv6_only).await?;

    // 3. Establish connection (HTTP/3 QUIC if --quic, else TCP + TLS with Happy Eyeballs)
    let (mut client, proto_desc, cold_start_ms) = if cli.quic {
        match H3Client::connect(&cli.host, &candidates).await {
            Ok(h3) => {
                let cold_start = bench_start.elapsed().as_secs_f64() * 1000.0;
                let ip_family = if h3.peer_addr.is_ipv6() {
                    "Dual-Stack IPv6"
                } else {
                    "IPv4"
                };
                let desc = format!("{} ({}) · HTTP/3 (QUIC/UDP)", ip_family, h3.peer_addr.ip());
                (ClientHandle::H3(h3), desc, cold_start)
            }
            Err(e) => {
                if cli.plain && !cli.is_json_effective() {
                    eprintln!(
                        "  [!] HTTP/3 QUIC connection to {} failed ({}), falling back to Happy Eyeballs TCP/TLS...",
                        cli.host, e
                    );
                }
                let conn = connect_happy_eyeballs(&cli.host, &candidates, cli.interface.as_deref()).await?;
                let cold_start = bench_start.elapsed().as_secs_f64() * 1000.0;
                let ip_family = if conn.is_ipv6 {
                    "Dual-Stack IPv6"
                } else {
                    "IPv4"
                };
                let desc = if conn.stream.is_tls() {
                    let alpn = conn
                        .alpn
                        .as_deref()
                        .map(|p| String::from_utf8_lossy(p).into_owned())
                        .unwrap_or_else(|| "http/1.1".to_string());
                    format!("{} ({}) · {} over TLS 1.3", ip_family, conn.peer_addr.ip(), alpn)
                } else {
                    format!("{} ({}) · HTTP/1.1 (LAN / Plain)", ip_family, conn.peer_addr.ip())
                };
                let is_h2 = conn.alpn.as_deref().map(|p| p == b"h2").unwrap_or(false);
                let client = if is_h2 {
                    ClientHandle::H2(H2Client::new(conn.stream, cli.host.clone()).await?)
                } else {
                    ClientHandle::Http1(Box::new(Http1Client::new(conn.stream, cli.host.clone())))
                };
                (client, desc, cold_start)
            }
        }
    } else {
        let conn = connect_happy_eyeballs(&cli.host, &candidates, cli.interface.as_deref()).await?;
        let cold_start = bench_start.elapsed().as_secs_f64() * 1000.0;
        let ip_family = if conn.is_ipv6 {
            "Dual-Stack IPv6"
        } else {
            "IPv4"
        };
        let desc = if conn.stream.is_tls() {
            let alpn = conn
                .alpn
                .as_deref()
                .map(|p| String::from_utf8_lossy(p).into_owned())
                .unwrap_or_else(|| "http/1.1".to_string());
            format!("{} ({}) · {} over TLS 1.3", ip_family, conn.peer_addr.ip(), alpn)
        } else {
            format!("{} ({}) · HTTP/1.1 (LAN / Plain)", ip_family, conn.peer_addr.ip())
        };
        let is_h2 = conn.alpn.as_deref().map(|p| p == b"h2").unwrap_or(false);
        let client = if is_h2 {
            ClientHandle::H2(H2Client::new(conn.stream, cli.host.clone()).await?)
        } else {
            ClientHandle::Http1(Box::new(Http1Client::new(conn.stream, cli.host.clone())))
        };
        (client, desc, cold_start)
    };

    // 4. Initialize Live UI State & Terminal Renderer
    let is_json = cli.is_json_effective();
    let is_plain = cli.is_plain_effective();
    let mut renderer = TerminalRenderer::new(is_plain || is_json, cli.low_bandwidth);

    let mut ui_state = LiveUiState {
        phase: TestPhase::IdleLatency,
        current_mbps: 0.0,
        peak_mbps: 0.0,
        down_mbps: 0.0,
        up_mbps: 0.0,
        total_bytes: 0,
        elapsed_secs: 0.0,
        active_streams: 1,
        throughput_history: Vec::new(),
        idle_stats: None,
        live_loaded_ping_ms: None,
        edge_telemetry: EdgeTelemetry::default(),
        protocol_desc: proto_desc.clone(),
    };

    let probe_pending = Arc::new(AtomicBool::new(false));

    // 6. Phase 1: Baseline Unloaded Idle Latency
    if !is_plain && !is_json {
        renderer.render_frame(&ui_state);
    }

    let idle_probes_count = cli.idle_probe_count();
    let mut running_idle_samples = Vec::with_capacity(idle_probes_count);
    let (idle_stats, mut edge_telemetry_opt) =
        crate::engine::probe::run_idle_latency_test_with_progress(
            &mut client,
            idle_probes_count,
            &probe_pending,
            &cli.host,
            &candidates,
            cli.interface.as_deref(),
            |probe_idx, rtt_ms, telem| {
                if !is_plain && !is_json {
                    running_idle_samples.push(rtt_ms);
                    ui_state.idle_stats = Some(LatencyStats::calculate(
                        running_idle_samples.clone(),
                        probe_idx,
                    ));
                    ui_state.live_loaded_ping_ms = Some(rtt_ms);
                    ui_state.elapsed_secs = bench_start.elapsed().as_secs_f64();
                    if let Some(t) = telem {
                        merge_telemetry(&mut ui_state.edge_telemetry, t.clone());
                    }
                    renderer.render_frame(&ui_state);
                }
            },
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

    // 7. Phase 2 & 3: Saturation Tests (Simultaneous Full-Duplex or Sequential)
    let (download_stats, upload_stats, loaded_down_latency, loaded_up_latency) = if cli.bidirectional {
        ui_state.phase = TestPhase::Bidirectional;
        ui_state.active_streams = cli.streams * 2;
        ui_state.throughput_history.clear();

        let bidi_duration = Duration::from_secs_f64(cli.phase_duration_secs());
        let bidi_config = BidiPhaseConfig {
            host: &cli.host,
            streams: cli.streams,
            connections: cli.connections.max(1),
            duration: bidi_duration,
            suppress_ui: is_plain || is_json,
            probe_pending: probe_pending.clone(),
            candidates: &candidates,
            specified_iface: cli.interface.as_deref(),
        };
        let (d_stats, u_stats, loaded_lat, bidi_telem) =
            run_bidirectional_saturation(&mut client, bidi_config, &mut ui_state, &mut renderer).await;

        if let Some(t) = bidi_telem {
            merge_telemetry(&mut edge_telemetry, t);
            ui_state.edge_telemetry = edge_telemetry.clone();
        }

        if cli.plain && !is_json {
            println!(
                "  ⚡ Full-Duplex Saturation: ↓{:.1} Mbps (Peak: {:.1}) / ↑{:.1} Mbps (Peak: {:.1}) · Loaded Latency: {:.1} ms",
                d_stats.mbps, d_stats.peak_mbps,
                u_stats.mbps, u_stats.peak_mbps,
                loaded_lat.p90_ms
            );
        }

        (d_stats, u_stats, loaded_lat.clone(), loaded_lat)
    } else {
        // Sequential Download Test
        ui_state.phase = TestPhase::Download;
        ui_state.active_streams = cli.streams;
        ui_state.throughput_history.clear();

        let download_duration = Duration::from_secs_f64(cli.phase_duration_secs());
        let down_config = PhaseConfig {
            host: &cli.host,
            streams: cli.streams,
            connections: cli.connections.max(1),
            duration: download_duration,
            is_download: true,
            suppress_ui: is_plain || is_json,
            probe_pending: probe_pending.clone(),
            candidates: &candidates,
            specified_iface: cli.interface.as_deref(),
        };
        let (down_stats, loaded_down, down_telem) =
            run_phase_saturation(&mut client, down_config, &mut ui_state, &mut renderer).await;

        if let Some(t) = down_telem {
            merge_telemetry(&mut edge_telemetry, t);
            ui_state.edge_telemetry = edge_telemetry.clone();
        }

        if cli.plain && !is_json {
            println!(
                "  📥 Download Throughput: {:.1} Mbps (Peak: {:.1} Mbps, Loaded Latency: {:.1} ms)",
                down_stats.mbps, down_stats.peak_mbps, loaded_down.p90_ms
            );
        }

        // Sequential Upload Test
        let client_healthy = match &mut client {
            ClientHandle::H2(h2) => h2.send_request.clone().ready().await.is_ok(),
            ClientHandle::Http1(_) => true,
            ClientHandle::H3(_) => true,
        };
        if !client_healthy
            && let Ok(conn) =
                connect_happy_eyeballs(&cli.host, &candidates, cli.interface.as_deref()).await
        {
            let is_h2 = conn.alpn.as_deref().map(|p| p == b"h2").unwrap_or(false);
            if is_h2 {
                if let Ok(new_h2) = H2Client::new(conn.stream, cli.host.clone()).await {
                    client = ClientHandle::H2(new_h2);
                }
            } else {
                client = ClientHandle::Http1(Box::new(Http1Client::new(
                    conn.stream,
                    cli.host.clone(),
                )));
            }
        }

        ui_state.phase = TestPhase::Upload;
        ui_state.active_streams = cli.streams;
        ui_state.throughput_history.clear();

        let upload_duration = Duration::from_secs_f64(cli.phase_duration_secs());
        let up_config = PhaseConfig {
            host: &cli.host,
            streams: cli.streams,
            connections: cli.connections.max(1),
            duration: upload_duration,
            is_download: false,
            suppress_ui: is_plain || is_json,
            probe_pending: probe_pending.clone(),
            candidates: &candidates,
            specified_iface: cli.interface.as_deref(),
        };
        let (up_stats, loaded_up, up_telem) =
            run_phase_saturation(&mut client, up_config, &mut ui_state, &mut renderer).await;

        if let Some(t) = up_telem {
            merge_telemetry(&mut edge_telemetry, t);
        }

        if cli.plain && !is_json {
            println!(
                "  📤 Upload Throughput: {:.1} Mbps (Peak: {:.1} Mbps, Loaded Latency: {:.1} ms)",
                up_stats.mbps, up_stats.peak_mbps, loaded_up.p90_ms
            );
        }

        (down_stats, up_stats, loaded_down, loaded_up)
    };

    // 8. Compute Responsiveness, Apple RPM & Bufferbloat Grading
    let responsiveness = BufferbloatGrade::compute(
        idle_stats.median_ms,
        loaded_down_latency.p90_ms,
        Some(loaded_up_latency.p90_ms),
    );

    // 9. Optional ISP Speedtest Neutrality Audit
    let isp_audit = if cli.audit_isp {
        ui_state.phase = TestPhase::AuditIsp;
        ui_state.throughput_history.clear();
        ui_state.current_mbps = 0.0;
        ui_state.peak_mbps = 0.0;
        if !is_plain && !is_json {
            renderer.render_frame(&ui_state);
        } else if cli.plain && !is_json {
            println!(
                "  ⚖️ Starting ISP Speedtest Neutrality Audit against {} (Transit) vs {} (On-Net)...",
                cli.audit_host, cli.host
            );
        }

        match run_audit_target(&cli.audit_host, cli, &mut renderer, &mut ui_state).await {
            Ok((audit_mbps, audit_idle_ms)) => {
                let audit_report = IspAuditReport::compute(
                    &cli.audit_host,
                    audit_mbps,
                    audit_idle_ms,
                    &cli.host,
                    download_stats.mbps,
                    idle_stats.median_ms,
                );
                if cli.plain && !is_json {
                    println!(
                        "  ⚖️ ISP Neutrality Audit: {} (Ratio: {:.2})\n     Transit: {:.1} Mbps vs On-Net: {:.1} Mbps",
                        audit_report.verdict, audit_report.neutrality_ratio, audit_report.anycast_download_mbps, audit_report.on_net_download_mbps
                    );
                }
                Some(audit_report)
            }
            Err(e) => {
                if cli.plain && !is_json {
                    eprintln!("  [!] ISP Neutrality Audit failed against {}: {}", cli.audit_host, e);
                }
                None
            }
        }
    } else {
        None
    };

    let total_elapsed = bench_start.elapsed().as_secs_f64();
    let current_rss = ResourceProfile::current_rss_mb();

    let report = BenchmarkReport {
        version: "1.0.0".to_string(),
        timestamp: chrono_or_now(),
        target_host: cli.host.clone(),
        protocol: proto_desc,
        is_bidirectional: cli.bidirectional,
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
        isp_audit,
    };

    // 10. Output formatting
    ui_state.phase = TestPhase::Complete;
    if cli.prometheus {
        let prom_str = crate::ui::prometheus::format_prometheus(&report);
        print!("{}", prom_str);
    } else if is_json {
        let json_str = serde_json::to_string_pretty(&report)?;
        println!("{}", json_str);
    } else {
        renderer.finish_and_print_receipt(&report);
    }

    if let Some(ref svg_path) = cli.export_svg {
        if let Err(e) = crate::ui::svg::export_svg(&report, svg_path) {
            eprintln!("[!] Failed to export SVG receipt to {}: {}", svg_path, e);
        } else if !cli.prometheus && !is_json {
            println!("  🎨 Vector SVG receipt card exported to: {}", svg_path);
        }
    }

    Ok(report)
}

fn chrono_or_now() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now();
    let secs = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
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
    if new_data.kernel_tcp.is_some() {
        target.kernel_tcp = new_data.kernel_tcp;
    }
}

pub struct BidiPhaseConfig<'a> {
    pub host: &'a str,
    pub streams: usize,
    pub connections: usize,
    pub duration: Duration,
    pub suppress_ui: bool,
    pub probe_pending: Arc<AtomicBool>,
    pub candidates: &'a DnsCandidates,
    pub specified_iface: Option<&'a str>,
}

async fn run_audit_target(
    audit_host: &str,
    cli: &Cli,
    renderer: &mut TerminalRenderer,
    ui_state: &mut LiveUiState,
) -> Result<(f64, f64), Box<dyn std::error::Error>> {
    let audit_candidates = resolve_host(audit_host, 443, cli.ipv4_only, cli.ipv6_only).await?;
    let conn = connect_happy_eyeballs(audit_host, &audit_candidates, cli.interface.as_deref()).await?;
    let is_h2 = conn.alpn.as_deref().map(|p| p == b"h2").unwrap_or(false);
    let mut audit_client = if is_h2 {
        ClientHandle::H2(H2Client::new(conn.stream, audit_host.to_string()).await?)
    } else {
        ClientHandle::Http1(Box::new(Http1Client::new(conn.stream, audit_host.to_string())))
    };

    let probe_pending = Arc::new(AtomicBool::new(false));
    let (idle_stats, _) = crate::engine::probe::run_idle_latency_test_with_progress(
        &mut audit_client,
        3,
        &probe_pending,
        audit_host,
        &audit_candidates,
        cli.interface.as_deref(),
        |_, _, _| {},
    )
    .await;

    let audit_duration = Duration::from_secs(3);
    let config = PhaseConfig {
        host: audit_host,
        streams: cli.streams.min(4),
        connections: 1,
        duration: audit_duration,
        is_download: true,
        suppress_ui: cli.is_plain_effective() || cli.is_json_effective(),
        probe_pending,
        candidates: &audit_candidates,
        specified_iface: cli.interface.as_deref(),
    };

    let (down_stats, _, _) =
        run_phase_saturation(&mut audit_client, config, ui_state, renderer).await;
    Ok((down_stats.mbps, idle_stats.median_ms))
}

async fn run_bidirectional_saturation(
    client: &mut ClientHandle,
    config: BidiPhaseConfig<'_>,
    ui_state: &mut LiveUiState,
    renderer: &mut TerminalRenderer,
) -> (ThroughputStats, ThroughputStats, LatencyStats, Option<EdgeTelemetry>) {
    let down_bytes = Arc::new(AtomicU64::new(0));
    let up_bytes = Arc::new(AtomicU64::new(0));
    let stop_signal = Arc::new(AtomicBool::new(false));
    let loaded_samples = Arc::new(Mutex::new(Vec::new()));
    let captured_telemetry = Arc::new(Mutex::new(None));

    let mut stream_handles = Vec::with_capacity(config.streams * 2);

    match client {
        ClientHandle::H2(h2) => {
            let mut conn_pool = vec![h2.send_request.clone()];
            if config.connections > 1 {
                for _ in 1..config.connections {
                    if let Ok(conn) = connect_happy_eyeballs(
                        config.host,
                        config.candidates,
                        config.specified_iface,
                    )
                    .await
                        && let Ok(extra_h2) =
                            H2Client::new(conn.stream, config.host.to_string()).await
                    {
                        conn_pool.push(extra_h2.send_request);
                    }
                }
            }

            for i in 0..config.streams {
                let send_req = conn_pool[i % conn_pool.len()].clone();
                let host_clone = config.host.to_string();
                let candidates_clone = config.candidates.clone();
                let iface_clone = config.specified_iface.map(|s| s.to_string());
                let bytes_counter = down_bytes.clone();
                let stop_flag = stop_signal.clone();
                let probe_flag = config.probe_pending.clone();
                let telem_target = captured_telemetry.clone();

                let handle = tokio::spawn(async move {
                    let mut current_send_req = send_req;
                    while !stop_flag.load(Ordering::Relaxed) {
                        let res = H2Client::run_download_stream(
                            current_send_req.clone(),
                            host_clone.clone(),
                            5_000_000,
                            bytes_counter.clone(),
                            stop_flag.clone(),
                            probe_flag.clone(),
                        )
                        .await;
                        if let Ok(Some(t)) = res {
                            let mut guard = telem_target.lock().await;
                            if guard.is_none() {
                                *guard = Some(t);
                            }
                        }

                        if !stop_flag.load(Ordering::Relaxed) {
                            if let Ok(conn) = connect_happy_eyeballs(
                                &host_clone,
                                &candidates_clone,
                                iface_clone.as_deref(),
                            )
                            .await
                            {
                                if let Ok(new_h2) =
                                    H2Client::new(conn.stream, host_clone.clone()).await
                                {
                                    current_send_req = new_h2.send_request;
                                }
                            } else {
                                sleep(Duration::from_millis(50)).await;
                            }
                        }
                    }
                });
                stream_handles.push(handle);
            }

            for i in 0..config.streams {
                let send_req = conn_pool[i % conn_pool.len()].clone();
                let host_clone = config.host.to_string();
                let candidates_clone = config.candidates.clone();
                let iface_clone = config.specified_iface.map(|s| s.to_string());
                let bytes_counter = up_bytes.clone();
                let stop_flag = stop_signal.clone();
                let probe_flag = config.probe_pending.clone();
                let telem_target = captured_telemetry.clone();

                let handle = tokio::spawn(async move {
                    let mut current_send_req = send_req;
                    while !stop_flag.load(Ordering::Relaxed) {
                        let res = H2Client::run_upload_stream(
                            current_send_req.clone(),
                            host_clone.clone(),
                            bytes_counter.clone(),
                            stop_flag.clone(),
                            probe_flag.clone(),
                        )
                        .await;
                        if let Ok(Some(t)) = res {
                            let mut guard = telem_target.lock().await;
                            if guard.is_none() {
                                *guard = Some(t);
                            }
                        }

                        if !stop_flag.load(Ordering::Relaxed) {
                            if let Ok(conn) = connect_happy_eyeballs(
                                &host_clone,
                                &candidates_clone,
                                iface_clone.as_deref(),
                            )
                            .await
                            {
                                if let Ok(new_h2) =
                                    H2Client::new(conn.stream, host_clone.clone()).await
                                {
                                    current_send_req = new_h2.send_request;
                                }
                            } else {
                                sleep(Duration::from_millis(50)).await;
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
                let bytes_counter = down_bytes.clone();
                let stop_flag = stop_signal.clone();
                let probe_flag = config.probe_pending.clone();
                let telem_target = captured_telemetry.clone();

                let handle = tokio::spawn(async move {
                    while !stop_flag.load(Ordering::Relaxed) {
                        if let Ok(conn) = connect_happy_eyeballs(
                            &host_clone,
                            &candidates_clone,
                            iface_clone.as_deref(),
                        )
                        .await
                        {
                            let h1_stream = Http1Client::new(conn.stream, host_clone.clone());
                            let res = h1_stream
                                .run_download_stream(
                                    5_000_000,
                                    bytes_counter.clone(),
                                    stop_flag.clone(),
                                    probe_flag.clone(),
                                )
                                .await;
                            if let Ok(Some(t)) = res {
                                let mut guard = telem_target.lock().await;
                                if guard.is_none() {
                                    *guard = Some(t);
                                }
                            }
                        } else {
                            sleep(Duration::from_millis(50)).await;
                        }
                    }
                });
                stream_handles.push(handle);
            }

            for _ in 0..config.streams {
                let host_clone = config.host.to_string();
                let candidates_clone = config.candidates.clone();
                let iface_clone = config.specified_iface.map(|s| s.to_string());
                let bytes_counter = up_bytes.clone();
                let stop_flag = stop_signal.clone();
                let probe_flag = config.probe_pending.clone();
                let telem_target = captured_telemetry.clone();

                let handle = tokio::spawn(async move {
                    while !stop_flag.load(Ordering::Relaxed) {
                        if let Ok(conn) = connect_happy_eyeballs(
                            &host_clone,
                            &candidates_clone,
                            iface_clone.as_deref(),
                        )
                        .await
                        {
                            let h1_stream = Http1Client::new(conn.stream, host_clone.clone());
                            let res = h1_stream
                                .run_upload_stream(
                                    bytes_counter.clone(),
                                    stop_flag.clone(),
                                    probe_flag.clone(),
                                )
                                .await;
                            if let Ok(Some(t)) = res {
                                let mut guard = telem_target.lock().await;
                                if guard.is_none() {
                                    *guard = Some(t);
                                }
                            }
                        } else {
                            sleep(Duration::from_millis(50)).await;
                        }
                    }
                });
                stream_handles.push(handle);
            }
        }
        ClientHandle::H3(h3) => {
            let mut conn_pool = vec![h3.send_request.clone()];
            if config.connections > 1 {
                for _ in 1..config.connections {
                    if let Ok(extra_h3) = H3Client::connect(config.host, config.candidates).await {
                        conn_pool.push(extra_h3.send_request);
                    }
                }
            }

            for i in 0..config.streams {
                let send_req = conn_pool[i % conn_pool.len()].clone();
                let host_clone = config.host.to_string();
                let candidates_clone = config.candidates.clone();
                let bytes_counter = down_bytes.clone();
                let stop_flag = stop_signal.clone();
                let probe_flag = config.probe_pending.clone();
                let telem_target = captured_telemetry.clone();

                let handle = tokio::spawn(async move {
                    let mut current_send_req = send_req;
                    while !stop_flag.load(Ordering::Relaxed) {
                        let res = H3Client::run_download_stream(
                            current_send_req.clone(),
                            host_clone.clone(),
                            5_000_000,
                            bytes_counter.clone(),
                            stop_flag.clone(),
                            probe_flag.clone(),
                        )
                        .await;
                        if let Ok(Some(t)) = res {
                            let mut guard = telem_target.lock().await;
                            if guard.is_none() {
                                *guard = Some(t);
                            }
                        }

                        if !stop_flag.load(Ordering::Relaxed) {
                            if let Ok(new_h3) =
                                H3Client::connect(&host_clone, &candidates_clone).await
                            {
                                current_send_req = new_h3.send_request;
                            } else {
                                sleep(Duration::from_millis(50)).await;
                            }
                        }
                    }
                });
                stream_handles.push(handle);
            }

            for i in 0..config.streams {
                let send_req = conn_pool[i % conn_pool.len()].clone();
                let host_clone = config.host.to_string();
                let candidates_clone = config.candidates.clone();
                let bytes_counter = up_bytes.clone();
                let stop_flag = stop_signal.clone();
                let probe_flag = config.probe_pending.clone();
                let telem_target = captured_telemetry.clone();

                let handle = tokio::spawn(async move {
                    let mut current_send_req = send_req;
                    while !stop_flag.load(Ordering::Relaxed) {
                        let res = H3Client::run_upload_stream(
                            current_send_req.clone(),
                            host_clone.clone(),
                            bytes_counter.clone(),
                            stop_flag.clone(),
                            probe_flag.clone(),
                        )
                        .await;
                        if let Ok(Some(t)) = res {
                            let mut guard = telem_target.lock().await;
                            if guard.is_none() {
                                *guard = Some(t);
                            }
                        }

                        if !stop_flag.load(Ordering::Relaxed) {
                            if let Ok(new_h3) =
                                H3Client::connect(&host_clone, &candidates_clone).await
                            {
                                current_send_req = new_h3.send_request;
                            } else {
                                sleep(Duration::from_millis(50)).await;
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
                raw_fd: h2.raw_fd,
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
                            if let Ok(conn) = connect_happy_eyeballs(
                                &host_probe,
                                &candidates_probe,
                                iface_probe.as_deref(),
                            )
                            .await
                                && let Ok(new_h2) =
                                    H2Client::new(conn.stream, host_probe.clone()).await
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
                        if let Ok(conn) = connect_happy_eyeballs(
                            &host_probe,
                            &candidates_probe,
                            iface_probe.as_deref(),
                        )
                        .await
                        {
                            h1_probe_opt =
                                Some(Http1Client::new(conn.stream, host_probe.clone()));
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
                                if let Ok(new_conn) = connect_happy_eyeballs(
                                    &host_probe,
                                    &candidates_probe,
                                    iface_probe.as_deref(),
                                )
                                .await
                                {
                                    h1_probe_opt = Some(Http1Client::new(
                                        new_conn.stream,
                                        host_probe.clone(),
                                    ));
                                }
                            }
                        }
                    }
                }
            })
        }
        ClientHandle::H3(h3) => {
            let mut probe_client = h3.clone();
            tokio::spawn(async move {
                while !probe_stop.load(Ordering::Relaxed) {
                    sleep(Duration::from_millis(150)).await;
                    if probe_stop.load(Ordering::Relaxed) {
                        break;
                    }
                    attempted_counter.fetch_add(1, Ordering::Relaxed);
                    if let Ok((rtt, _)) = probe_client.dispatch_probe(&probe_flag).await {
                        let mut guard = probe_samples_target.lock().await;
                        guard.push(rtt);
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
    let mut last_down = 0u64;
    let mut last_up = 0u64;
    let mut peak_mbps = 0.0f64;
    let mut peak_down_mbps = 0.0f64;
    let mut peak_up_mbps = 0.0f64;

    while phase_start.elapsed() < config.duration {
        sleep(tick_interval).await;
        let now = Instant::now();
        let delta_secs = (now - last_tick).as_secs_f64();
        last_tick = now;

        let cur_down = down_bytes.load(Ordering::Relaxed);
        let cur_up = up_bytes.load(Ordering::Relaxed);
        let delta_down = cur_down.saturating_sub(last_down);
        let delta_up = cur_up.saturating_sub(last_up);
        last_down = cur_down;
        last_up = cur_up;

        let down_mbps = if delta_secs > 0.0 {
            (delta_down as f64 * 8.0) / (delta_secs * 1_000_000.0)
        } else {
            0.0
        };
        let up_mbps = if delta_secs > 0.0 {
            (delta_up as f64 * 8.0) / (delta_secs * 1_000_000.0)
        } else {
            0.0
        };
        let combined_mbps = down_mbps + up_mbps;

        if combined_mbps > peak_mbps {
            peak_mbps = combined_mbps;
        }
        if down_mbps > peak_down_mbps {
            peak_down_mbps = down_mbps;
        }
        if up_mbps > peak_up_mbps {
            peak_up_mbps = up_mbps;
        }

        ui_state.throughput_history.push(combined_mbps);

        let latest_ping = {
            let guard = loaded_samples.lock().await;
            guard.last().copied()
        };

        if let Some(ktcp) = client.read_kernel_tcp_info() {
            ui_state.edge_telemetry.kernel_tcp = Some(ktcp);
        }

        ui_state.current_mbps = combined_mbps;
        ui_state.peak_mbps = peak_mbps;
        ui_state.down_mbps = down_mbps;
        ui_state.up_mbps = up_mbps;
        ui_state.total_bytes = cur_down + cur_up;
        ui_state.elapsed_secs = phase_start.elapsed().as_secs_f64();
        ui_state.live_loaded_ping_ms = latest_ping;

        if !config.suppress_ui {
            renderer.render_frame(ui_state);
        }
    }

    stop_signal.store(true, Ordering::Release);
    if tokio::time::timeout(Duration::from_millis(800), &mut probe_handle)
        .await
        .is_err()
    {
        probe_handle.abort();
    }
    for mut h in stream_handles {
        if tokio::time::timeout(Duration::from_millis(800), &mut h)
            .await
            .is_err()
        {
            h.abort();
        }
    }
    config.probe_pending.store(false, Ordering::Release);

    let total_down = down_bytes.load(Ordering::Relaxed);
    let total_up = up_bytes.load(Ordering::Relaxed);
    let total_secs = phase_start.elapsed().as_secs_f64();

    let avg_down_mbps = if total_secs > 0.0 {
        (total_down as f64 * 8.0) / (total_secs * 1_000_000.0)
    } else {
        0.0
    };
    let avg_up_mbps = if total_secs > 0.0 {
        (total_up as f64 * 8.0) / (total_secs * 1_000_000.0)
    } else {
        0.0
    };

    let final_samples = {
        let guard = loaded_samples.lock().await;
        guard.clone()
    };
    let total_attempted =
        (attempted_probes.load(Ordering::Relaxed) as usize).max(final_samples.len());
    let latency_stats = LatencyStats::calculate(final_samples, total_attempted);

    let telem = {
        let guard = captured_telemetry.lock().await;
        guard.clone()
    };

    let d_stats = ThroughputStats {
        bps: avg_down_mbps * 1_000_000.0,
        mbps: avg_down_mbps,
        peak_mbps: peak_down_mbps,
        total_bytes: total_down,
        duration_secs: total_secs,
        history: ui_state.throughput_history.clone(),
    };

    let u_stats = ThroughputStats {
        bps: avg_up_mbps * 1_000_000.0,
        mbps: avg_up_mbps,
        peak_mbps: peak_up_mbps,
        total_bytes: total_up,
        duration_secs: total_secs,
        history: ui_state.throughput_history.clone(),
    };

    (d_stats, u_stats, latency_stats, telem)
}

struct PhaseConfig<'a> {
    host: &'a str,
    streams: usize,
    connections: usize,
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
            let mut conn_pool = vec![h2.send_request.clone()];
            if config.connections > 1 {
                for _ in 1..config.connections {
                    if let Ok(conn) = connect_happy_eyeballs(
                        config.host,
                        config.candidates,
                        config.specified_iface,
                    )
                    .await
                        && let Ok(extra_h2) =
                            H2Client::new(conn.stream, config.host.to_string()).await
                    {
                        conn_pool.push(extra_h2.send_request);
                    }
                }
            }

            for i in 0..config.streams {
                let send_req = conn_pool[i % conn_pool.len()].clone();
                let host_clone = config.host.to_string();
                let candidates_clone = config.candidates.clone();
                let iface_clone = config.specified_iface.map(|s| s.to_string());
                let bytes_counter = total_bytes.clone();
                let stop_flag = stop_signal.clone();
                let probe_flag = config.probe_pending.clone();
                let telem_target = captured_telemetry.clone();
                let is_down = config.is_download;

                let handle = tokio::spawn(async move {
                    let mut current_send_req = send_req;
                    while !stop_flag.load(Ordering::Relaxed) {
                        if is_down {
                            let res = H2Client::run_download_stream(
                                current_send_req.clone(),
                                host_clone.clone(),
                                5_000_000,
                                bytes_counter.clone(),
                                stop_flag.clone(),
                                probe_flag.clone(),
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
                                current_send_req.clone(),
                                host_clone.clone(),
                                bytes_counter.clone(),
                                stop_flag.clone(),
                                probe_flag.clone(),
                            )
                            .await;
                            if let Ok(Some(t)) = res {
                                let mut guard = telem_target.lock().await;
                                if guard.is_none() {
                                    *guard = Some(t);
                                }
                            }
                        }

                        if !stop_flag.load(Ordering::Relaxed) {
                            // Seamless reconnect if connection dropped abruptly mid-test
                            if let Ok(conn) = connect_happy_eyeballs(
                                &host_clone,
                                &candidates_clone,
                                iface_clone.as_deref(),
                            )
                            .await
                            {
                                if let Ok(new_h2) =
                                    H2Client::new(conn.stream, host_clone.clone()).await
                                {
                                    current_send_req = new_h2.send_request;
                                }
                            } else {
                                sleep(Duration::from_millis(50)).await;
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
                    while !stop_flag.load(Ordering::Relaxed) {
                        if let Ok(conn) = connect_happy_eyeballs(
                            &host_clone,
                            &candidates_clone,
                            iface_clone.as_deref(),
                        )
                        .await
                        {
                            let h1_stream = Http1Client::new(conn.stream, host_clone.clone());
                            if is_down {
                                let res = h1_stream
                                    .run_download_stream(
                                        5_000_000,
                                        bytes_counter.clone(),
                                        stop_flag.clone(),
                                        probe_flag.clone(),
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
                                        bytes_counter.clone(),
                                        stop_flag.clone(),
                                        probe_flag.clone(),
                                    )
                                    .await;
                                if let Ok(Some(t)) = res {
                                    let mut guard = telem_target.lock().await;
                                    if guard.is_none() {
                                        *guard = Some(t);
                                    }
                                }
                            }
                        } else {
                            sleep(Duration::from_millis(50)).await;
                        }
                    }
                });
                stream_handles.push(handle);
            }
        }
        ClientHandle::H3(h3) => {
            let mut conn_pool = vec![h3.send_request.clone()];
            if config.connections > 1 {
                for _ in 1..config.connections {
                    if let Ok(extra_h3) = H3Client::connect(config.host, config.candidates).await {
                        conn_pool.push(extra_h3.send_request);
                    }
                }
            }

            for i in 0..config.streams {
                let send_req = conn_pool[i % conn_pool.len()].clone();
                let host_clone = config.host.to_string();
                let candidates_clone = config.candidates.clone();
                let bytes_counter = total_bytes.clone();
                let stop_flag = stop_signal.clone();
                let probe_flag = config.probe_pending.clone();
                let telem_target = captured_telemetry.clone();
                let is_down = config.is_download;

                let handle = tokio::spawn(async move {
                    let mut current_send_req = send_req;
                    while !stop_flag.load(Ordering::Relaxed) {
                        if is_down {
                            let res = H3Client::run_download_stream(
                                current_send_req.clone(),
                                host_clone.clone(),
                                5_000_000,
                                bytes_counter.clone(),
                                stop_flag.clone(),
                                probe_flag.clone(),
                            )
                            .await;
                            if let Ok(Some(t)) = res {
                                let mut guard = telem_target.lock().await;
                                if guard.is_none() {
                                    *guard = Some(t);
                                }
                            }
                        } else {
                            let res = H3Client::run_upload_stream(
                                current_send_req.clone(),
                                host_clone.clone(),
                                bytes_counter.clone(),
                                stop_flag.clone(),
                                probe_flag.clone(),
                            )
                            .await;
                            if let Ok(Some(t)) = res {
                                let mut guard = telem_target.lock().await;
                                if guard.is_none() {
                                    *guard = Some(t);
                                }
                            }
                        }

                        if !stop_flag.load(Ordering::Relaxed) {
                            if let Ok(new_h3) =
                                H3Client::connect(&host_clone, &candidates_clone).await
                            {
                                current_send_req = new_h3.send_request;
                            } else {
                                sleep(Duration::from_millis(50)).await;
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
                raw_fd: h2.raw_fd,
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
                            if let Ok(conn) = connect_happy_eyeballs(
                                &host_probe,
                                &candidates_probe,
                                iface_probe.as_deref(),
                            )
                            .await
                                && let Ok(new_h2) =
                                    H2Client::new(conn.stream, host_probe.clone()).await
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
                        if let Ok(conn) = connect_happy_eyeballs(
                            &host_probe,
                            &candidates_probe,
                            iface_probe.as_deref(),
                        )
                        .await
                        {
                            h1_probe_opt =
                                Some(Http1Client::new(conn.stream, host_probe.clone()));
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
                                if let Ok(new_conn) = connect_happy_eyeballs(
                                    &host_probe,
                                    &candidates_probe,
                                    iface_probe.as_deref(),
                                )
                                .await
                                {
                                    h1_probe_opt = Some(Http1Client::new(
                                        new_conn.stream,
                                        host_probe.clone(),
                                    ));
                                }
                            }
                        }
                    }
                }
            })
        }
        ClientHandle::H3(h3) => {
            let mut probe_client = h3.clone();
            let host_probe = config.host.to_string();
            let candidates_probe = config.candidates.clone();

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
                            if let Ok(new_h3) =
                                H3Client::connect(&host_probe, &candidates_probe).await
                            {
                                probe_client = new_h3;
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

        ui_state.throughput_history.push(current_mbps);

        // Fetch latest probe RTT
        let latest_ping = {
            let guard = loaded_samples.lock().await;
            guard.last().copied()
        };

        if config.is_download {
            ui_state.down_mbps = current_mbps;
        } else {
            ui_state.up_mbps = current_mbps;
        }
        ui_state.current_mbps = current_mbps;
        ui_state.peak_mbps = peak_mbps;
        ui_state.total_bytes = current_bytes;
        ui_state.elapsed_secs = phase_start.elapsed().as_secs_f64();
        ui_state.live_loaded_ping_ms = latest_ping;

        if !config.suppress_ui {
            renderer.render_frame(ui_state);
        }
    }

    // Stop all workers with timeout safety guards and explicit task abort to prevent task leakage
    stop_signal.store(true, Ordering::Release);
    if tokio::time::timeout(Duration::from_millis(800), &mut probe_handle)
        .await
        .is_err()
    {
        probe_handle.abort();
    }
    for mut h in stream_handles {
        if tokio::time::timeout(Duration::from_millis(800), &mut h)
            .await
            .is_err()
        {
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
    let total_attempted =
        (attempted_probes.load(Ordering::Relaxed) as usize).max(final_samples.len());
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
        history: ui_state.throughput_history.clone(),
    };

    (stats, latency_stats, telem)
}
