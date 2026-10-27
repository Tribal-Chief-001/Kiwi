use clap::Parser;
use http::{HeaderMap, HeaderValue};
use kiwi::cli::Cli;
use kiwi::metrics::{
    BenchmarkReport, BufferbloatGrade, EdgeTelemetry, LatencyStats, PlatformInfo, ResourceProfile, ThroughputStats,
};
use kiwi::platform::{is_virtual_interface, is_wireless_interface};
use kiwi::protocol::telemetry::parse_edge_telemetry;
use kiwi::ui::bento::render_bento_box;
use kiwi::ui::braille::{calculate_gauge_scale, render_braille_canvas, render_tachometer};

#[test]
fn test_cli_flags_and_modes() {
    let cli = Cli::parse_from(["kiwi", "--fast", "--plain", "-4"]);
    assert!(cli.fast);
    assert!(cli.plain);
    assert!(cli.ipv4_only);
    assert!(!cli.ipv6_only);
    assert_eq!(cli.phase_duration_secs(), 2.0);
    assert_eq!(cli.idle_probe_count(), 5);
    assert!(cli.is_plain_effective());

    let cli_deep = Cli::parse_from(["kiwi", "--deep", "--json", "-6", "--streams", "8"]);
    assert!(cli_deep.deep);
    assert!(cli_deep.json);
    assert!(cli_deep.ipv6_only);
    assert_eq!(cli_deep.streams, 8);
    assert_eq!(cli_deep.phase_duration_secs(), 10.0);
    assert_eq!(cli_deep.idle_probe_count(), 30);
    assert!(cli_deep.is_json_effective());
}

#[test]
fn test_latency_statistics_calculations() {
    // 1. Empty samples
    let empty = LatencyStats::calculate(vec![], 10);
    assert_eq!(empty.packet_loss_pct, 100.0);
    assert_eq!(empty.min_ms, 0.0);
    assert_eq!(empty.median_ms, 0.0);

    // 2. Single sample
    let single = LatencyStats::calculate(vec![25.0], 1);
    assert_eq!(single.packet_loss_pct, 0.0);
    assert_eq!(single.min_ms, 25.0);
    assert_eq!(single.median_ms, 25.0);
    assert_eq!(single.p90_ms, 25.0);
    assert_eq!(single.jitter_ms, 0.0);

    // 3. Multiple samples (odd count: 5 samples)
    let odd_samples = vec![10.0, 30.0, 20.0, 50.0, 40.0];
    let odd = LatencyStats::calculate(odd_samples, 5);
    assert_eq!(odd.min_ms, 10.0);
    assert_eq!(odd.median_ms, 30.0);
    assert_eq!(odd.p90_ms, 50.0);
    assert_eq!(odd.packet_loss_pct, 0.0);
    // Jitter calculation: |30-10| + |20-30| + |50-20| + |40-50| = 20 + 10 + 30 + 10 = 70 / 4 = 17.5
    assert!((odd.jitter_ms - 17.5).abs() < 1e-4);

    // 4. Partial packet loss: 7 samples out of 10 probes
    let partial = LatencyStats::calculate(vec![20.0, 22.0, 21.0, 23.0, 20.0, 22.0, 21.0], 10);
    assert_eq!(partial.packet_loss_pct, 30.0);
}

#[test]
fn test_bufferbloat_grading_matrix() {
    // Grade A+: RPM >= 1200, delta <= 15ms
    let grade_a_plus = BufferbloatGrade::compute(20.0, 25.0, Some(27.0));
    // Working RTT = (25 + 27)/2 = 26ms -> RPM = 60000/26 = 2308, delta = 6ms
    assert_eq!(grade_a_plus.grade, "A+");
    assert_eq!(grade_a_plus.rating_label, "ULTRA RESPONSIVE");
    assert!(grade_a_plus.rpm >= 1200);

    // Grade A: RPM 800 - 1199, delta <= 35ms
    let grade_a = BufferbloatGrade::compute(25.0, 55.0, Some(55.0));
    // Working RTT = 55ms -> RPM = 60000/55 = 1091, delta = 30ms
    assert_eq!(grade_a.grade, "A");
    assert_eq!(grade_a.rating_label, "EXCELLENT");

    // Grade B: RPM 400 - 799, delta <= 80ms
    let grade_b = BufferbloatGrade::compute(30.0, 95.0, Some(105.0));
    // Working RTT = 100ms -> RPM = 600, delta = 70ms
    assert_eq!(grade_b.grade, "B");
    assert_eq!(grade_b.rating_label, "ACCEPTABLE");

    // Grade C: RPM 150 - 399, delta <= 200ms
    let grade_c = BufferbloatGrade::compute(30.0, 200.0, Some(220.0));
    // Working RTT = 210ms -> RPM = 286, delta = 180ms
    assert_eq!(grade_c.grade, "C");
    assert_eq!(grade_c.rating_label, "NOTICEABLE BUFFERBLOAT");

    // Grade F: RPM < 150 or delta > 200ms
    let grade_f = BufferbloatGrade::compute(30.0, 500.0, Some(550.0));
    assert_eq!(grade_f.grade, "F");
    assert_eq!(grade_f.rating_label, "SEVERE BUFFERBLOAT");

    // Edge case: Loaded latency lower than idle (jitter artifact) -> delta clamped to 0.0
    let jitter_artifact = BufferbloatGrade::compute(40.0, 35.0, Some(35.0));
    assert_eq!(jitter_artifact.delta_ms, 0.0);
}

#[test]
fn test_edge_telemetry_parser() {
    let mut headers = HeaderMap::new();
    headers.insert("colo", HeaderValue::from_static("BOM"));
    headers.insert("asn", HeaderValue::from_static("55836"));
    headers.insert("city", HeaderValue::from_static("Bhopal"));
    headers.insert("country", HeaderValue::from_static("IN"));
    headers.insert("cf-meta-ip", HeaderValue::from_static("2409:40c4:19:8727::1"));
    headers.insert(
        "server-timing",
        HeaderValue::from_static(
            r#"cfL4;desc="?proto=TCP&rtt=42899&min_rtt=39859&rtt_var=16481&sent=5&recv=7&lost=0&retrans=2&sent_bytes=2858&recv_bytes=720&delivery_rate=73129&cwnd=53&unsent_bytes=0""#,
        ),
    );

    let telem = parse_edge_telemetry(&headers);
    assert_eq!(telem.colo, "BOM");
    assert_eq!(telem.asn, Some(55836));
    assert_eq!(telem.city, "Bhopal");
    assert_eq!(telem.country, "IN");
    assert_eq!(telem.client_ip, "2409:40c4:19:8727::1");
    assert_eq!(telem.cf_l4_proto.as_deref(), Some("TCP"));
    assert_eq!(telem.cf_l4_rtt_us, Some(42899));
    assert_eq!(telem.cf_l4_min_rtt_us, Some(39859));
    assert_eq!(telem.cf_l4_cwnd, Some(53));
    assert_eq!(telem.cf_l4_retrans, Some(2));
    assert_eq!(telem.cf_l4_delivery_rate, Some(73129));
}

#[test]
fn test_braille_canvas_and_tachometer() {
    // Braille canvas with empty history
    let (top_empty, bot_empty) = render_braille_canvas(&[], 20);
    assert_eq!(top_empty.chars().count(), 20);
    assert_eq!(bot_empty.chars().count(), 20);
    assert!(top_empty.chars().all(|c| c == ' '));

    // Braille canvas with varying throughput waveform
    let history = vec![10.0, 50.0, 100.0, 300.0, 550.0, 400.0, 200.0, 100.0, 50.0, 10.0];
    let (top_wave, bot_wave) = render_braille_canvas(&history, 15);
    assert_eq!(top_wave.chars().count(), 15);
    assert_eq!(bot_wave.chars().count(), 15);
    // Must contain braille pattern symbols
    assert!(bot_wave.chars().any(|c| (c as u32) >= 0x2800 && (c as u32) <= 0x28FF));

    // Tachometer gauge
    let tacho_0 = render_tachometer(0.0, 100.0, 10);
    assert_eq!(tacho_0, "[░░░░░░░░░░]");
    let tacho_50 = render_tachometer(50.0, 100.0, 10);
    assert_eq!(tacho_50, "[█████░░░░░]");
    let tacho_100 = render_tachometer(100.0, 100.0, 10);
    assert_eq!(tacho_100, "[██████████]");
    let tacho_over = render_tachometer(150.0, 100.0, 10);
    assert_eq!(tacho_over, "[██████████]");

    // Gauge auto-scaling
    assert_eq!(calculate_gauge_scale(30.0), 50.0);
    assert_eq!(calculate_gauge_scale(85.0), 100.0);
    assert_eq!(calculate_gauge_scale(220.0), 250.0);
    assert_eq!(calculate_gauge_scale(420.0), 500.0);
    assert_eq!(calculate_gauge_scale(900.0), 1000.0);
    assert_eq!(calculate_gauge_scale(2200.0), 2500.0);
    assert_eq!(calculate_gauge_scale(4500.0), 5000.0);
    assert_eq!(calculate_gauge_scale(8500.0), 10000.0);
}

#[test]
fn test_platform_interface_classification() {
    assert!(is_virtual_interface("vethb01b3e2"));
    assert!(is_virtual_interface("docker0"));
    assert!(is_virtual_interface("br-0a2abbe5a7a3"));
    assert!(is_virtual_interface("tun0"));
    assert!(is_virtual_interface("wg0"));
    assert!(is_virtual_interface("tailscale0"));
    assert!(!is_virtual_interface("eth0"));
    assert!(!is_virtual_interface("en0"));
    assert!(!is_virtual_interface("wlp0s20f3"));

    assert!(is_wireless_interface("wlp0s20f3"));
    assert!(is_wireless_interface("wlan0"));
    assert!(!is_wireless_interface("lo"));
}

#[test]
fn test_bento_box_receipt_rendering() {
    let report = BenchmarkReport {
        version: "1.0.0".to_string(),
        timestamp: "Unix(1793094558)".to_string(),
        target_host: "speed.cloudflare.com".to_string(),
        protocol: "Dual-Stack IPv6 · h2 over TLS 1.3".to_string(),
        edge: EdgeTelemetry {
            colo: "BOM".to_string(),
            asn: Some(55836),
            city: "Bhopal".to_string(),
            country: "IN".to_string(),
            client_ip: "2409:40c4:19:8727::1".to_string(),
            cf_l4_proto: Some("TCP".to_string()),
            cf_l4_rtt_us: Some(42899),
            cf_l4_min_rtt_us: Some(39859),
            cf_l4_cwnd: Some(53),
            cf_l4_retrans: Some(0),
            cf_l4_delivery_rate: Some(73129),
        },
        idle_latency: LatencyStats {
            min_ms: 28.4,
            median_ms: 28.4,
            p90_ms: 29.5,
            jitter_ms: 0.9,
            samples: vec![28.4, 29.5],
            packet_loss_pct: 0.0,
        },
        download: ThroughputStats {
            bps: 526_400_000.0,
            mbps: 526.4,
            peak_mbps: 541.2,
            total_bytes: 384_000_000,
            duration_secs: 5.0,
            history: vec![500.0, 526.4],
        },
        loaded_down_latency: LatencyStats {
            min_ms: 32.0,
            median_ms: 34.8,
            p90_ms: 34.8,
            jitter_ms: 1.2,
            samples: vec![34.8],
            packet_loss_pct: 0.0,
        },
        upload: ThroughputStats {
            bps: 139_800_000.0,
            mbps: 139.8,
            peak_mbps: 145.0,
            total_bytes: 100_000_000,
            duration_secs: 5.0,
            history: vec![130.0, 139.8],
        },
        loaded_up_latency: LatencyStats {
            min_ms: 35.0,
            median_ms: 37.6,
            p90_ms: 37.6,
            jitter_ms: 1.5,
            samples: vec![37.6],
            packet_loss_pct: 0.0,
        },
        responsiveness: BufferbloatGrade {
            grade: "A+".to_string(),
            rpm: 1657,
            working_rtt_ms: 36.2,
            delta_ms: 7.8,
            rating_label: "ULTRA RESPONSIVE".to_string(),
        },
        platform: PlatformInfo {
            interface_name: "wlp0s20f3".to_string(),
            link_speed_mbps: Some(866),
            link_type: "Wi-Fi 6".to_string(),
            is_virtual: false,
            bottleneck_warning: None,
        },
        resources: ResourceProfile {
            cold_start_ms: 3.2,
            peak_rss_mb: 3.6,
            duration_secs: 6.4,
        },
    };

    let bento = render_bento_box(&report, 86);
    assert!(bento.contains("KIWI BENCHMARK RECEIPT"));
    assert!(bento.contains("DOWNLOAD BANDWIDTH"));
    assert!(bento.contains("526.4 Mbps"));
    assert!(bento.contains("UPLOAD BANDWIDTH"));
    assert!(bento.contains("139.8 Mbps"));
    assert!(bento.contains("GRADE A+"));
    assert!(bento.contains("1657 RPM"));
    assert!(bento.contains("BOM"));
    assert!(bento.contains("wlp0s20f3"));
}

#[test]
fn test_probe_pending_guard_raii() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let flag = AtomicBool::new(false);
    assert!(!flag.load(Ordering::Relaxed));

    {
        let _guard = kiwi::protocol::ProbePendingGuard::activate(&flag);
        assert!(flag.load(Ordering::Relaxed));
    }

    // Must be automatically restored on drop
    assert!(!flag.load(Ordering::Relaxed));
}

#[cfg(target_os = "linux")]
#[test]
fn test_linux_bottleneck_evaluation() {
    use kiwi::platform::{evaluate_bottlenecks, WirelessStats};

    // 1. Half-duplex physical link
    let half_dup = evaluate_bottlenecks(false, false, Some(1000), Some("half"), Some("up"), Some(1), None);
    assert!(half_dup.is_some());
    assert!(half_dup.unwrap().contains("Half-duplex"));

    // 2. Degraded 100 Mbps Ethernet port
    let degraded_100 = evaluate_bottlenecks(false, false, Some(100), Some("full"), Some("up"), Some(1), None);
    assert!(degraded_100.is_some());
    assert!(degraded_100.unwrap().contains("100 Mbps link detected"));

    // 3. Severe 10 Mbps Ethernet port
    let severe_10 = evaluate_bottlenecks(false, false, Some(10), Some("full"), Some("up"), Some(1), None);
    assert!(severe_10.is_some());
    assert!(severe_10.unwrap().contains("Severe 10 Mbps bottleneck"));

    // 4. Physical carrier lost
    let carrier_lost = evaluate_bottlenecks(false, false, Some(1000), Some("full"), Some("down"), Some(0), None);
    assert!(carrier_lost.is_some());
    assert!(carrier_lost.unwrap().contains("carrier signal lost"));

    // 5. Weak Wi-Fi signal (-82 dBm)
    let weak_wifi = evaluate_bottlenecks(
        false,
        true,
        None,
        None,
        Some("up"),
        Some(1),
        Some(WirelessStats {
            link_quality: Some(25),
            signal_level_dbm: Some(-82),
        }),
    );
    assert!(weak_wifi.is_some());
    assert!(weak_wifi.unwrap().contains("Weak Wi-Fi signal (-82 dBm)"));

    // 6. Healthy link (Gigabit Ethernet)
    let healthy_eth = evaluate_bottlenecks(false, false, Some(1000), Some("full"), Some("up"), Some(1), None);
    assert!(healthy_eth.is_none());

    // 7. Healthy Wi-Fi (-42 dBm, link quality 68/70)
    let healthy_wifi = evaluate_bottlenecks(
        false,
        true,
        None,
        None,
        Some("up"),
        Some(1),
        Some(WirelessStats {
            link_quality: Some(68),
            signal_level_dbm: Some(-42),
        }),
    );
    assert!(healthy_wifi.is_none());
}

#[test]
fn test_extended_virtual_interface_prefixes() {
    assert!(is_virtual_interface("tap0"));
    assert!(is_virtual_interface("cni0"));
    assert!(is_virtual_interface("flannel.1"));
    assert!(is_virtual_interface("kube-bridge"));
    assert!(is_virtual_interface("podman0"));
}

#[test]
fn test_loaded_packet_loss_calculation() {
    // 8 samples collected out of 10 attempted probes -> 20.0% packet loss
    let samples = vec![30.0, 32.0, 31.0, 33.0, 34.0, 32.0, 31.0, 35.0];
    let stats = LatencyStats::calculate(samples, 10);
    assert_eq!(stats.samples.len(), 8);
    assert!((stats.packet_loss_pct - 20.0).abs() < 1e-4);
}
