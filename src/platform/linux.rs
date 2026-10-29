use crate::metrics::{KernelTcpInfo, PlatformInfo};
use std::fs;
use std::os::unix::io::{AsRawFd, RawFd};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WirelessStats {
    pub link_quality: Option<u32>,
    pub signal_level_dbm: Option<i32>,
}

pub fn detect_platform_info(specified_iface: Option<&str>) -> PlatformInfo {
    let iface = match specified_iface {
        Some(name) => name.to_string(),
        None => detect_default_interface().unwrap_or_else(|| "default".to_string()),
    };

    let is_virtual = is_virtual_interface(&iface);
    let is_wireless = is_wireless_interface(&iface);
    let speed_mbps = read_link_speed(&iface);
    let duplex = read_link_duplex(&iface);
    let operstate = read_operstate(&iface);
    let carrier = read_carrier(&iface);
    let wireless_stats = if is_wireless {
        read_wireless_stats(&iface)
    } else {
        None
    };

    let link_type = if is_virtual {
        "Virtual Overlay".to_string()
    } else if is_wireless {
        "Wi-Fi".to_string()
    } else if speed_mbps.is_some() {
        "Ethernet".to_string()
    } else {
        "Link".to_string()
    };

    let bottleneck_warning = evaluate_bottlenecks(
        is_virtual,
        is_wireless,
        speed_mbps,
        duplex.as_deref(),
        operstate.as_deref(),
        carrier,
        wireless_stats,
    );

    PlatformInfo {
        interface_name: iface,
        link_speed_mbps: speed_mbps,
        link_type,
        is_virtual,
        bottleneck_warning,
    }
}

pub fn evaluate_bottlenecks(
    is_virtual: bool,
    is_wireless: bool,
    speed_mbps: Option<u64>,
    duplex: Option<&str>,
    operstate: Option<&str>,
    carrier: Option<u32>,
    wireless_stats: Option<WirelessStats>,
) -> Option<String> {
    let mut warnings = Vec::new();

    // Check link carrier / operational state
    if let Some(c) = carrier
        && c == 0
    {
        warnings.push("Physical carrier signal lost (port unplugged or link down)".to_string());
    } else if let Some(state) = operstate
        && (state == "down" || state == "lowerlayerdown")
    {
        warnings.push(format!("Interface operational state is '{}'", state));
    }

    if is_wireless {
        if let Some(wstats) = wireless_stats {
            if let Some(rssi) = wstats.signal_level_dbm
                && rssi <= -75
            {
                warnings.push(format!(
                    "Weak Wi-Fi signal ({} dBm) - risk of packet loss, rate rollback, and jitter spikes",
                    rssi
                ));
            }
            if let Some(quality) = wstats.link_quality
                && quality < 30
                && warnings.is_empty()
            {
                warnings.push(format!(
                    "Degraded Wi-Fi link quality ({}/70) - RF interference or retry elevated",
                    quality
                ));
            }
        }
    } else if !is_virtual {
        // Physical Ethernet link diagnostic checks
        if let Some(d) = duplex
            && d.eq_ignore_ascii_case("half")
        {
            warnings.push(
                "Half-duplex link negotiation detected (collision domain / bad cable pair)"
                    .to_string(),
            );
        }

        if let Some(speed) = speed_mbps {
            if speed == 10 {
                warnings.push("Severe 10 Mbps bottleneck detected on physical port (damaged cable or 10BASE-T negotiation)".to_string());
            } else if speed == 100 {
                warnings.push("Degraded 100 Mbps link detected on physical port (check cable/negotiation - Gigabit capability limited)".to_string());
            } else if speed < 10 {
                warnings.push(format!("Low PHY link speed: {} Mbps", speed));
            }
        }
    }

    if warnings.is_empty() {
        None
    } else {
        Some(warnings.join(" │ "))
    }
}

pub fn detect_default_interface() -> Option<String> {
    // 1. Check IPv4 routing table (/proc/net/route)
    if let Some(iface) = detect_default_interface_ipv4() {
        return Some(iface);
    }
    // 2. Check IPv6 routing table (/proc/net/ipv6_route)
    if let Some(iface) = detect_default_interface_ipv6() {
        return Some(iface);
    }
    // 3. Fallback: scan /sys/class/net for first UP physical interface
    detect_first_up_interface()
}

pub fn parse_default_route_ipv4(routes: &str) -> Option<String> {
    let mut best_iface = None;
    let mut best_priority = None;

    for line in routes.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // fields: [Iface, Destination, Gateway, Flags, RefCnt, Use, Metric, Mask, MTU, Window, IRTT]
        if fields.len() >= 7 && fields[1] == "00000000" {
            let flags = u32::from_str_radix(fields[3], 16).unwrap_or(0);
            // Require RTF_UP (0x0001)
            if flags & 0x0001 == 0 {
                continue;
            }
            let iface = fields[0];
            let metric = fields[6].parse::<u32>().unwrap_or(u32::MAX);
            let is_virt = is_virtual_interface(iface);
            let is_wifi = is_wireless_interface(iface);

            // Priority tuple: (metric, is_virtual, is_wireless)
            // 1. Lower metric wins
            // 2. Physical interface wins over virtual bridge/overlay
            // 3. Wired Ethernet wins over Wi-Fi
            let priority = (metric, is_virt, is_wifi);
            if best_priority.is_none() || Some(priority) < best_priority {
                best_priority = Some(priority);
                best_iface = Some(iface.to_string());
            }
        }
    }
    best_iface
}

pub fn detect_default_interface_ipv4() -> Option<String> {
    let routes = fs::read_to_string("/proc/net/route").ok()?;
    parse_default_route_ipv4(&routes)
}

pub fn parse_default_route_ipv6(routes: &str) -> Option<String> {
    let mut best_iface = None;
    let mut best_priority = None;

    for line in routes.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // Destination prefix is 32 hex chars (128 bits), followed by prefix length
        // Default route has destination 00000000000000000000000000000000 and prefix length 00
        if fields.len() >= 10
            && fields[0] == "00000000000000000000000000000000"
            && fields[1] == "00"
        {
            let iface = fields[fields.len() - 1];
            if iface == "lo" {
                continue;
            }
            // Check flags (fields[8] in hex): require RTF_UP (0x0001) and exclude RTF_REJECT (0x0200)
            let flags = u32::from_str_radix(fields[8], 16).unwrap_or(0);
            if flags & 0x0001 == 0 || flags & 0x0200 != 0 {
                continue;
            }

            let metric = u32::from_str_radix(fields[5], 16).unwrap_or(u32::MAX);
            let is_virt = is_virtual_interface(iface);
            let is_wifi = is_wireless_interface(iface);

            let priority = (metric, is_virt, is_wifi);
            if best_priority.is_none() || Some(priority) < best_priority {
                best_priority = Some(priority);
                best_iface = Some(iface.to_string());
            }
        }
    }
    best_iface
}

pub fn detect_default_interface_ipv6() -> Option<String> {
    let routes = fs::read_to_string("/proc/net/ipv6_route").ok()?;
    parse_default_route_ipv6(&routes)
}

pub fn detect_first_up_interface() -> Option<String> {
    if let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if name == "lo" || is_virtual_interface(&name) {
                continue;
            }
            if let Some(state) = read_operstate(&name)
                && state == "up"
            {
                return Some(name.to_string());
            }
        }
    }
    None
}

pub fn is_virtual_interface(name: &str) -> bool {
    super::is_virtual_interface(name)
}

pub fn is_wireless_interface(name: &str) -> bool {
    // Check sysfs wireless directory
    let sysfs_path = format!("/sys/class/net/{}/wireless", name);
    if fs::metadata(sysfs_path).is_ok() {
        return true;
    }
    // Check sysfs phy80211 link (modern nl80211 devices without WEXT)
    let phy_path = format!("/sys/class/net/{}/phy80211", name);
    if fs::metadata(phy_path).is_ok() {
        return true;
    }
    // Check /proc/net/wireless
    if let Ok(wireless) = fs::read_to_string("/proc/net/wireless")
        && wireless.lines().any(|l| l.contains(name))
    {
        return true;
    }
    let lower = name.to_lowercase();
    lower.starts_with("wl")
        || lower.contains("wi-fi")
        || lower.contains("wireless")
        || lower.contains("wlan")
}

pub fn read_link_speed(name: &str) -> Option<u64> {
    let speed_path = format!("/sys/class/net/{}/speed", name);
    if let Ok(content) = fs::read_to_string(speed_path)
        && let Ok(speed) = content.trim().parse::<i64>()
        && speed > 0
    {
        return Some(speed as u64);
    }
    None
}

pub fn read_link_duplex(name: &str) -> Option<String> {
    let duplex_path = format!("/sys/class/net/{}/duplex", name);
    fs::read_to_string(duplex_path)
        .ok()
        .map(|s| s.trim().to_string())
}

pub fn read_operstate(name: &str) -> Option<String> {
    let state_path = format!("/sys/class/net/{}/operstate", name);
    fs::read_to_string(state_path)
        .ok()
        .map(|s| s.trim().to_lowercase())
}

pub fn read_carrier(name: &str) -> Option<u32> {
    let carrier_path = format!("/sys/class/net/{}/carrier", name);
    fs::read_to_string(carrier_path).ok()?.trim().parse().ok()
}

pub fn read_wireless_stats(name: &str) -> Option<WirelessStats> {
    let wireless = fs::read_to_string("/proc/net/wireless").ok()?;
    for line in wireless.lines() {
        if let Some((iface_part, stats_part)) = line.split_once(':')
            && iface_part.trim() == name
        {
            let tokens: Vec<&str> = stats_part.split_whitespace().collect();
            // tokens: [status, link_quality, signal_level, noise, ...]
            // Example: ["0000", "68.", "-42.", "-256", ...]
            if tokens.len() >= 3 {
                let quality = tokens[1].trim_end_matches('.').parse::<u32>().ok();
                let level = tokens[2].trim_end_matches('.').parse::<i32>().ok();
                return Some(WirelessStats {
                    link_quality: quality,
                    signal_level_dbm: level,
                });
            }
        }
    }
    None
}

/// Apply TCP_NOTSENT_LOWAT socket option to limit kernel transmit buffer bloat.
pub fn apply_socket_optimizations<S: AsRawFd>(socket: &S, specified_iface: Option<&str>) {
    let fd = socket.as_raw_fd();
    unsafe {
        // Set TCP_NOTSENT_LOWAT = 16 KB (16384 bytes)
        let lowat: libc::c_uint = 16384;
        let _ = libc::setsockopt(
            fd,
            libc::IPPROTO_TCP,
            libc::TCP_NOTSENT_LOWAT,
            &lowat as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_uint>() as libc::socklen_t,
        );

        // Bind to interface if requested and permitted
        if let Some(iface) = specified_iface {
            let iface_bytes = std::ffi::CString::new(iface).ok();
            if let Some(c_iface) = iface_bytes {
                let _ = libc::setsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_BINDTODEVICE,
                    c_iface.as_ptr() as *const libc::c_void,
                    c_iface.as_bytes_with_nul().len() as libc::socklen_t,
                );
            }
        }
    }
}

/// Read live kernel TCP socket metrics via getsockopt(TCP_INFO) directly from Linux kernel
pub fn read_tcp_info(fd: RawFd) -> Option<KernelTcpInfo> {
    if fd < 0 {
        return None;
    }
    unsafe {
        let mut info: libc::tcp_info = std::mem::zeroed();
        let mut len = std::mem::size_of::<libc::tcp_info>() as libc::socklen_t;
        let res = libc::getsockopt(
            fd,
            libc::IPPROTO_TCP,
            libc::TCP_INFO,
            &mut info as *mut _ as *mut libc::c_void,
            &mut len,
        );
        if res == 0 && len as usize >= std::mem::size_of::<u32>() * 8 {
            Some(KernelTcpInfo {
                rtt_us: info.tcpi_rtt,
                min_rtt_us: info.tcpi_min_rtt,
                snd_cwnd: info.tcpi_snd_cwnd,
                retrans: info.tcpi_retrans,
                total_retrans: info.tcpi_total_retrans,
                delivery_rate_bps: if info.tcpi_delivery_rate > 0 {
                    Some(info.tcpi_delivery_rate * 8)
                } else {
                    None
                },
            })
        } else {
            None
        }
    }
}
