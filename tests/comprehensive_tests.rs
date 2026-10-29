use clap::Parser;
use http::{HeaderMap, HeaderValue};
use kiwi::cli::Cli;
use kiwi::metrics::{
    BenchmarkReport, BufferbloatGrade, EdgeTelemetry, IspAuditReport, KernelTcpInfo, LatencyStats,
    PlatformInfo, ResourceProfile, ThroughputStats,
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
    headers.insert(
        "cf-meta-ip",
        HeaderValue::from_static("2409:40c4:19:8727::1"),
    );
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
    let history = vec![
        10.0, 50.0, 100.0, 300.0, 550.0, 400.0, 200.0, 100.0, 50.0, 10.0,
    ];
    let (top_wave, bot_wave) = render_braille_canvas(&history, 15);
    assert_eq!(top_wave.chars().count(), 15);
    assert_eq!(bot_wave.chars().count(), 15);
    // Must contain braille pattern symbols
    assert!(
        bot_wave
            .chars()
            .any(|c| (c as u32) >= 0x2800 && (c as u32) <= 0x28FF)
    );

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
    assert!(is_wireless_interface("Wi-Fi"));
    assert!(is_wireless_interface("Wireless Network Connection"));
    assert!(!is_wireless_interface("lo"));
    assert!(!is_wireless_interface("eth0"));
}

#[test]
fn test_bento_box_receipt_rendering() {
    let report = BenchmarkReport {
        version: "1.0.0".to_string(),
        timestamp: "Unix(1793094558)".to_string(),
        target_host: "speed.cloudflare.com".to_string(),
        protocol: "Dual-Stack IPv6 · h2 over TLS 1.3".to_string(),
        is_bidirectional: false,
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
            kernel_tcp: None,
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
        isp_audit: None,
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

    // Narrow terminal responsive bento rendering (60 cols)
    let bento_narrow = render_bento_box(&report, 60);
    assert!(bento_narrow.contains("KIWI BENCHMARK RECEIPT"));
    assert!(bento_narrow.contains("DOWNLOAD BANDWIDTH"));
    assert!(bento_narrow.contains("526.4 Mbps"));
    assert!(bento_narrow.contains("UPLOAD BANDWIDTH"));
    assert!(bento_narrow.contains("GRADE A+"));

    // Verify no line exceeds the target width
    for line in bento_narrow.lines() {
        if !line.is_empty() {
            assert!(
                kiwi::ui::art::visible_width(line) <= 60,
                "Line '{}' width {} exceeded 60",
                line,
                kiwi::ui::art::visible_width(line)
            );
        }
    }
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
    use kiwi::platform::{WirelessStats, evaluate_bottlenecks};

    // 1. Half-duplex physical link
    let half_dup = evaluate_bottlenecks(
        false,
        false,
        Some(1000),
        Some("half"),
        Some("up"),
        Some(1),
        None,
    );
    assert!(half_dup.is_some());
    assert!(half_dup.unwrap().contains("Half-duplex"));

    // 2. Degraded 100 Mbps Ethernet port
    let degraded_100 = evaluate_bottlenecks(
        false,
        false,
        Some(100),
        Some("full"),
        Some("up"),
        Some(1),
        None,
    );
    assert!(degraded_100.is_some());
    assert!(degraded_100.unwrap().contains("100 Mbps link detected"));

    // 3. Severe 10 Mbps Ethernet port
    let severe_10 = evaluate_bottlenecks(
        false,
        false,
        Some(10),
        Some("full"),
        Some("up"),
        Some(1),
        None,
    );
    assert!(severe_10.is_some());
    assert!(severe_10.unwrap().contains("Severe 10 Mbps bottleneck"));

    // 4. Physical carrier lost
    let carrier_lost = evaluate_bottlenecks(
        false,
        false,
        Some(1000),
        Some("full"),
        Some("down"),
        Some(0),
        None,
    );
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
    let healthy_eth = evaluate_bottlenecks(
        false,
        false,
        Some(1000),
        Some("full"),
        Some("up"),
        Some(1),
        None,
    );
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
    assert!(is_virtual_interface("utun3"));
    assert!(is_virtual_interface("vEthernet (WSL)"));
}

#[test]
fn test_loaded_packet_loss_calculation() {
    // 8 samples collected out of 10 attempted probes -> 20.0% packet loss
    let samples = vec![30.0, 32.0, 31.0, 33.0, 34.0, 32.0, 31.0, 35.0];
    let stats = LatencyStats::calculate(samples, 10);
    assert_eq!(stats.samples.len(), 8);
    assert!((stats.packet_loss_pct - 20.0).abs() < 1e-4);
}

#[test]
fn test_latency_stats_underflow_and_zero_probes_edge_cases() {
    // 1. total_probes < samples.len() (must not overflow/panic)
    let samples = vec![20.0, 22.0, 21.0];
    let stats = LatencyStats::calculate(samples, 2);
    assert_eq!(stats.packet_loss_pct, 0.0);
    assert_eq!(stats.samples.len(), 3);

    // 2. total_probes == 0 with empty samples (must report 0.0% loss without division by zero/overflow)
    let empty_zero = LatencyStats::calculate(vec![], 0);
    assert_eq!(empty_zero.packet_loss_pct, 0.0);
    assert_eq!(empty_zero.min_ms, 0.0);
}

#[cfg(target_os = "linux")]
#[test]
fn test_linux_bottleneck_carrier_down_override() {
    use kiwi::platform::evaluate_bottlenecks;

    // Carrier signal present (1), but administrative state is explicitly 'down'
    let warning = evaluate_bottlenecks(
        false,
        false,
        Some(1000),
        Some("full"),
        Some("down"),
        Some(1),
        None,
    );
    assert!(warning.is_some());
    assert!(warning.unwrap().contains("operational state is 'down'"));
}

#[cfg(target_os = "linux")]
#[test]
fn test_linux_default_route_ipv4_tie_break() {
    use kiwi::platform::parse_default_route_ipv4;

    // 1. Dual-homed system with identical metric (100):
    // docker0 (virtual bridge) listed first, eth0 (physical Gigabit) listed second.
    // parse_default_route_ipv4 must select physical eth0 over virtual docker0!
    let mock_routes_virtual_tie = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\tMTU\tWindow\tIRTT\n\
docker0\t00000000\t010011AC\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n";
    assert_eq!(
        parse_default_route_ipv4(mock_routes_virtual_tie),
        Some("eth0".to_string())
    );

    // 2. Dual-homed system with identical metric (100):
    // wlan0 (Wi-Fi) listed first, eth0 (Ethernet) listed second.
    // Wired physical Ethernet must be prioritized over Wi-Fi when metrics tie!
    let mock_routes_wifi_tie = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\tMTU\tWindow\tIRTT\n\
wlan0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n";
    assert_eq!(
        parse_default_route_ipv4(mock_routes_wifi_tie),
        Some("eth0".to_string())
    );

    // 3. Metric priority beats interface type:
    // wlan0 metric 50, eth0 metric 100 -> lower metric wlan0 wins!
    let mock_routes_metric_priority = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\tMTU\tWindow\tIRTT\n\
eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
wlan0\t00000000\t0101A8C0\t0003\t0\t0\t50\t00000000\t0\t0\t0\n";
    assert_eq!(
        parse_default_route_ipv4(mock_routes_metric_priority),
        Some("wlan0".to_string())
    );

    // 4. Inactive route (Flags = 0002 without RTF_UP 0x0001) must be ignored
    let mock_routes_down = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\tMTU\tWindow\tIRTT\n\
eth0\t00000000\t0101A8C0\t0000\t0\t0\t10\t00000000\t0\t0\t0\n\
eth1\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n";
    assert_eq!(
        parse_default_route_ipv4(mock_routes_down),
        Some("eth1".to_string())
    );
}

#[cfg(target_os = "linux")]
#[test]
fn test_linux_default_route_ipv6_tie_break_and_flags() {
    use kiwi::platform::parse_default_route_ipv6;

    // 1. Dual-homed IPv6 default routes with identical metric:
    // docker0 listed first, eth0 listed second -> eth0 wins!
    let mock_ipv6 = "\
00000000000000000000000000000000 00 00000000000000000000000000000000 00 fe800000000000000000000000000001 00000400 00000001 00000000 08000003 docker0\n\
00000000000000000000000000000000 00 00000000000000000000000000000000 00 fe800000000000000000000000000002 00000400 00000001 00000000 08000003 eth0\n";
    assert_eq!(
        parse_default_route_ipv6(mock_ipv6),
        Some("eth0".to_string())
    );

    // 2. Route with RTF_REJECT (0x0200) must be skipped even with metric 0
    let mock_ipv6_reject = "\
00000000000000000000000000000000 00 00000000000000000000000000000000 00 00000000000000000000000000000000 00000000 00000001 00000000 00200200 eth0\n\
00000000000000000000000000000000 00 00000000000000000000000000000000 00 fe800000000000000000000000000002 00000400 00000001 00000000 08000003 eth1\n";
    assert_eq!(
        parse_default_route_ipv6(mock_ipv6_reject),
        Some("eth1".to_string())
    );
}

#[test]
fn test_cli_ipv4_and_ipv6_mutual_exclusivity() {
    // Both -4 and -6 passed concurrently must fail argument parsing
    let res = Cli::try_parse_from(["kiwi", "-4", "-6"]);
    assert!(
        res.is_err(),
        "Cli parsing should fail when both -4 and -6 are supplied"
    );
}

#[test]
fn test_bufferbloat_extreme_edge_cases() {
    // 1. Zero/sub-millisecond working RTT: RPM must be clamped cleanly without u32 overflow
    let near_zero = BufferbloatGrade::compute(0.0, 0.0001, Some(0.0001));
    assert_eq!(near_zero.rpm, 999_999);
    assert_eq!(near_zero.grade, "A+");

    // 2. NaN values in idle or working latency must default cleanly without panics
    let nan_case = BufferbloatGrade::compute(f64::NAN, f64::NAN, Some(f64::NAN));
    assert_eq!(nan_case.rpm, 0);
    assert_eq!(nan_case.delta_ms, 0.0);
    assert_eq!(nan_case.grade, "F");

    // 3. Negative working RTT (impossible clock skew) sanitized
    let neg_case = BufferbloatGrade::compute(10.0, -5.0, Some(-10.0));
    assert_eq!(neg_case.rpm, 0);
    assert_eq!(neg_case.delta_ms, 0.0);

    // 4. Extremely high loaded latency (100 seconds = 0.6 RPM -> rounds to 1 RPM)
    let slow = BufferbloatGrade::compute(20.0, 100_000.0, Some(100_000.0));
    assert!(slow.rpm <= 1);
    assert_eq!(slow.grade, "F");
    assert_eq!(slow.rating_label, "SEVERE BUFFERBLOAT");
}

#[test]
fn test_latency_stats_nan_and_negative_filtering() {
    // 1. Vector containing NaN and negative values must be filtered out
    let dirty_samples = vec![20.0, f64::NAN, -5.0, 25.0, f64::INFINITY, 30.0];
    let stats = LatencyStats::calculate(dirty_samples, 6);
    assert_eq!(stats.samples.len(), 3);
    assert_eq!(stats.min_ms, 20.0);
    assert_eq!(stats.median_ms, 25.0);
    assert_eq!(stats.p90_ms, 30.0);
    // 3 valid samples out of 6 total probes -> 50% packet loss
    assert_eq!(stats.packet_loss_pct, 50.0);

    // 2. All NaN samples -> 100% packet loss
    let all_nan = vec![f64::NAN, f64::NAN];
    let nan_stats = LatencyStats::calculate(all_nan, 2);
    assert_eq!(nan_stats.packet_loss_pct, 100.0);
    assert_eq!(nan_stats.samples.len(), 0);

    // 3. Exactly identical samples -> jitter must be exactly 0.0
    let identical = vec![15.5, 15.5, 15.5, 15.5];
    let id_stats = LatencyStats::calculate(identical, 4);
    assert_eq!(id_stats.jitter_ms, 0.0);
}

#[test]
fn test_gauge_scale_extreme_values() {
    // 1. Zero peak Mbps
    assert_eq!(calculate_gauge_scale(0.0), 50.0);

    // 2. NaN and negative numbers default to safe minimum scale
    assert_eq!(calculate_gauge_scale(f64::NAN), 50.0);
    assert_eq!(calculate_gauge_scale(-100.0), 50.0);

    // 3. 10 Gbps (10,000 Mbps) -> 11,000 Mbps
    assert_eq!(calculate_gauge_scale(10_000.0), 11_000.0);

    // 4. 100 Gbps (100,000 Mbps) -> 101,000 Mbps
    assert_eq!(calculate_gauge_scale(100_000.0), 101_000.0);
}

#[test]
fn test_bento_box_responsive_narrow_and_ultrawide() {
    let report = BenchmarkReport {
        version: "1.0.0".to_string(),
        timestamp: "Unix(1793094558)".to_string(),
        target_host: "speed.cloudflare.com".to_string(),
        protocol: "Dual-Stack IPv6 · h2 over TLS 1.3".to_string(),
        is_bidirectional: false,
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
            kernel_tcp: None,
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
        isp_audit: None,
    };

    // Test extreme narrow viewport: 40 columns (mobile Termux split pane)
    let bento_40 = render_bento_box(&report, 40);
    for line in bento_40.lines() {
        if !line.is_empty() {
            assert!(
                kiwi::ui::art::visible_width(line) <= 40,
                "Line '{}' exceeded 40 columns (width={})",
                line,
                kiwi::ui::art::visible_width(line)
            );
        }
    }

    // Test compact viewport: 55 columns
    let bento_55 = render_bento_box(&report, 55);
    for line in bento_55.lines() {
        if !line.is_empty() {
            assert!(
                kiwi::ui::art::visible_width(line) <= 55,
                "Line '{}' exceeded 55 columns (width={})",
                line,
                kiwi::ui::art::visible_width(line)
            );
        }
    }

    // Test ultrawide viewport: 240 columns
    let bento_240 = render_bento_box(&report, 240);
    assert!(bento_240.contains("KIWI BENCHMARK RECEIPT"));
    for line in bento_240.lines() {
        if !line.is_empty() {
            assert!(
                kiwi::ui::art::visible_width(line) <= 84,
                "Line exceeded max 84 columns on ultrawide (width={})",
                kiwi::ui::art::visible_width(line)
            );
        }
    }
}

#[test]
fn test_isp_speedtest_neutrality_audit_grading() {
    // 1. Honest routing: ratio >= 0.85
    let honest = IspAuditReport::compute("speed.fastly.com", 480.0, 18.0, "speed.cloudflare.com", 500.0, 16.0);
    assert!(!honest.is_throttled);
    assert_eq!(honest.verdict, "NEUTRAL · HONEST ROUTING");
    assert!(honest.neutrality_ratio >= 0.85);

    // 2. Mild shaping: ratio between 0.65 and 0.84
    let mild = IspAuditReport::compute("speed.fastly.com", 350.0, 25.0, "speed.cloudflare.com", 500.0, 15.0);
    assert!(!mild.is_throttled);
    assert_eq!(mild.verdict, "MILD PEERING SHAPING");
    assert!(mild.neutrality_ratio >= 0.65 && mild.neutrality_ratio < 0.85);

    // 3. Aggressive shaping / cheating: ratio < 0.65
    let cheated = IspAuditReport::compute("speed.fastly.com", 150.0, 45.0, "speed.cloudflare.com", 600.0, 12.0);
    assert!(cheated.is_throttled);
    assert_eq!(cheated.verdict, "AGGRESSIVE SHAPING / CHEATING");
    assert!(cheated.neutrality_ratio < 0.65);
    assert!(cheated.analysis.contains("Severe ISP QoS bias"));
}

#[test]
fn test_prometheus_metrics_exposition() {
    let report = BenchmarkReport {
        version: "1.0.0".to_string(),
        timestamp: "Unix(1793094558)".to_string(),
        target_host: "speed.cloudflare.com".to_string(),
        protocol: "IPv4 (104.16.1.1) · h2 over TLS 1.3".to_string(),
        is_bidirectional: false,
        edge: EdgeTelemetry {
            colo: "FRA".to_string(),
            asn: Some(13335),
            city: "Frankfurt".to_string(),
            country: "DE".to_string(),
            client_ip: "198.51.100.1".to_string(),
            cf_l4_proto: Some("TCP".to_string()),
            cf_l4_rtt_us: Some(15000),
            cf_l4_min_rtt_us: Some(14000),
            cf_l4_cwnd: Some(42),
            cf_l4_retrans: Some(1),
            cf_l4_delivery_rate: Some(120000),
            kernel_tcp: Some(KernelTcpInfo {
                rtt_us: 15200,
                min_rtt_us: 14100,
                snd_cwnd: 45,
                retrans: 1,
                total_retrans: 2,
                delivery_rate_bps: Some(950_000_000),
            }),
        },
        idle_latency: LatencyStats {
            min_ms: 14.0,
            median_ms: 15.0,
            p90_ms: 16.0,
            jitter_ms: 0.5,
            samples: vec![14.0, 15.0, 16.0],
            packet_loss_pct: 0.0,
        },
        download: ThroughputStats {
            bps: 850_000_000.0,
            mbps: 850.0,
            peak_mbps: 910.0,
            total_bytes: 500_000_000,
            duration_secs: 5.0,
            history: vec![800.0, 850.0],
        },
        loaded_down_latency: LatencyStats {
            min_ms: 16.0,
            median_ms: 18.0,
            p90_ms: 19.5,
            jitter_ms: 0.8,
            samples: vec![18.0, 19.5],
            packet_loss_pct: 0.0,
        },
        upload: ThroughputStats {
            bps: 200_000_000.0,
            mbps: 200.0,
            peak_mbps: 215.0,
            total_bytes: 125_000_000,
            duration_secs: 5.0,
            history: vec![190.0, 200.0],
        },
        loaded_up_latency: LatencyStats {
            min_ms: 18.0,
            median_ms: 20.0,
            p90_ms: 22.0,
            jitter_ms: 1.1,
            samples: vec![20.0, 22.0],
            packet_loss_pct: 0.0,
        },
        responsiveness: BufferbloatGrade {
            grade: "A+".to_string(),
            rpm: 2100,
            working_rtt_ms: 19.5,
            delta_ms: 4.5,
            rating_label: "ULTRA RESPONSIVE".to_string(),
        },
        platform: PlatformInfo {
            interface_name: "eth0".to_string(),
            link_speed_mbps: Some(1000),
            link_type: "Gigabit Ethernet".to_string(),
            is_virtual: false,
            bottleneck_warning: None,
        },
        resources: ResourceProfile {
            cold_start_ms: 2.8,
            peak_rss_mb: 3.4,
            duration_secs: 10.2,
        },
        isp_audit: None,
    };

    let prom = kiwi::ui::prometheus::format_prometheus(&report);
    assert!(prom.contains("kiwi_download_mbps{"));
    assert!(prom.contains("850.00"));
    assert!(prom.contains("kiwi_upload_mbps{"));
    assert!(prom.contains("200.00"));
    assert!(prom.contains("kiwi_idle_latency_ms{"));
    assert!(prom.contains("15.00"));
    assert!(prom.contains("kiwi_bufferbloat_rpm{"));
    assert!(prom.contains("2100"));
    assert!(prom.contains("kiwi_responsiveness_grade{"));
    assert!(prom.contains("grade=\"A+\""));
    assert!(prom.contains("kiwi_kernel_tcp_cwnd 45"));
}

#[test]
fn test_vector_svg_receipt_export() {
    let report = BenchmarkReport {
        version: "1.0.0".to_string(),
        timestamp: "Unix(1793094558)".to_string(),
        target_host: "speed.cloudflare.com".to_string(),
        protocol: "Dual-Stack IPv6 · h2 over TLS 1.3".to_string(),
        is_bidirectional: true,
        edge: EdgeTelemetry {
            colo: "LHR".to_string(),
            asn: Some(13335),
            city: "London".to_string(),
            country: "GB".to_string(),
            client_ip: "2001:db8::1".to_string(),
            cf_l4_proto: Some("TCP".to_string()),
            cf_l4_rtt_us: Some(12000),
            cf_l4_min_rtt_us: Some(11000),
            cf_l4_cwnd: Some(60),
            cf_l4_retrans: Some(0),
            cf_l4_delivery_rate: Some(150000),
            kernel_tcp: None,
        },
        idle_latency: LatencyStats {
            min_ms: 11.0,
            median_ms: 12.0,
            p90_ms: 12.8,
            jitter_ms: 0.3,
            samples: vec![11.0, 12.0, 12.8],
            packet_loss_pct: 0.0,
        },
        download: ThroughputStats {
            bps: 950_000_000.0,
            mbps: 950.0,
            peak_mbps: 980.0,
            total_bytes: 600_000_000,
            duration_secs: 5.0,
            history: vec![900.0, 950.0],
        },
        loaded_down_latency: LatencyStats {
            min_ms: 13.0,
            median_ms: 14.5,
            p90_ms: 15.0,
            jitter_ms: 0.5,
            samples: vec![14.5, 15.0],
            packet_loss_pct: 0.0,
        },
        upload: ThroughputStats {
            bps: 450_000_000.0,
            mbps: 450.0,
            peak_mbps: 470.0,
            total_bytes: 300_000_000,
            duration_secs: 5.0,
            history: vec![420.0, 450.0],
        },
        loaded_up_latency: LatencyStats {
            min_ms: 14.0,
            median_ms: 16.0,
            p90_ms: 17.0,
            jitter_ms: 0.7,
            samples: vec![16.0, 17.0],
            packet_loss_pct: 0.0,
        },
        responsiveness: BufferbloatGrade {
            grade: "A+".to_string(),
            rpm: 2500,
            working_rtt_ms: 15.0,
            delta_ms: 3.0,
            rating_label: "ULTRA RESPONSIVE".to_string(),
        },
        platform: PlatformInfo {
            interface_name: "eth0".to_string(),
            link_speed_mbps: Some(1000),
            link_type: "Gigabit Ethernet".to_string(),
            is_virtual: false,
            bottleneck_warning: None,
        },
        resources: ResourceProfile {
            cold_start_ms: 2.1,
            peak_rss_mb: 3.2,
            duration_secs: 5.4,
        },
        isp_audit: Some(IspAuditReport::compute(
            "speed.fastly.com",
            930.0,
            12.5,
            "speed.cloudflare.com",
            950.0,
            12.0,
        )),
    };

    let svg_str = kiwi::ui::svg::render_svg_card(&report);
    assert!(svg_str.starts_with("<svg"));
    assert!(svg_str.trim().ends_with("</svg>"));
    assert!(svg_str.contains("KIWI NETWORK RECEIPT"));
    assert!(svg_str.contains("950.0"));
    assert!(svg_str.contains("450.0"));
    assert!(svg_str.contains("Peak: 980.0 Mbps"));
    assert!(svg_str.contains("Peak: 470.0 Mbps"));
    assert!(svg_str.contains("GRADE A+"));
    assert!(svg_str.contains("Full-Duplex Simultaneous Saturation"));
    assert!(svg_str.contains("NEUTRAL · HONEST ROUTING"));
}

#[test]
fn test_bidirectional_and_quic_cli_parsing() {
    let cli = Cli::parse_from(["kiwi", "--bidirectional", "--quic", "--prometheus", "--audit-isp"]);
    assert!(cli.bidirectional);
    assert!(cli.quic);
    assert!(cli.prometheus);
    assert!(cli.audit_isp);
    assert!(!cli.is_json_effective());
    assert!(cli.is_plain_effective());

    let cli_alias = Cli::parse_from(["kiwi", "--simultaneous"]);
    assert!(cli_alias.bidirectional);
}
