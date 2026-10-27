use crate::metrics::PlatformInfo;
use std::fs;
use std::os::unix::io::AsRawFd;

pub fn detect_platform_info(specified_iface: Option<&str>) -> PlatformInfo {
    let iface = match specified_iface {
        Some(name) => name.to_string(),
        None => detect_default_interface().unwrap_or_else(|| "default".to_string()),
    };

    let is_virtual = is_virtual_interface(&iface);
    let is_wireless = is_wireless_interface(&iface);
    let speed_mbps = read_link_speed(&iface);

    let link_type = if is_virtual {
        "Virtual Overlay".to_string()
    } else if is_wireless {
        "Wi-Fi".to_string()
    } else if speed_mbps.is_some() {
        "Ethernet".to_string()
    } else {
        "Link".to_string()
    };

    let bottleneck_warning = if !is_virtual && !is_wireless {
        if let Some(speed) = speed_mbps {
            if speed == 100 {
                Some("Degraded 100 Mbps link detected on physical port (check cable/negotiation)".to_string())
            } else if speed < 100 {
                Some(format!("Low PHY link speed: {} Mbps", speed))
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    PlatformInfo {
        interface_name: iface,
        link_speed_mbps: speed_mbps,
        link_type,
        is_virtual,
        bottleneck_warning,
    }
}

pub fn detect_default_interface() -> Option<String> {
    let routes = fs::read_to_string("/proc/net/route").ok()?;
    for line in routes.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() >= 2 && fields[1] == "00000000" {
            return Some(fields[0].to_string());
        }
    }
    None
}

pub fn is_virtual_interface(name: &str) -> bool {
    let prefixes = ["veth", "docker", "br-", "tun", "wg", "tailscale", "dummy", "virbr"];
    prefixes.iter().any(|prefix| name.starts_with(prefix))
}

pub fn is_wireless_interface(name: &str) -> bool {
    // Check sysfs wireless directory
    let sysfs_path = format!("/sys/class/net/{}/wireless", name);
    if fs::metadata(sysfs_path).is_ok() {
        return true;
    }
    // Check /proc/net/wireless
    if let Ok(wireless) = fs::read_to_string("/proc/net/wireless")
        && wireless.lines().any(|l| l.contains(name))
    {
        return true;
    }
    name.starts_with("wl")
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
