use crate::metrics::PlatformInfo;
use std::os::unix::io::AsRawFd;

pub fn detect_platform_info(specified_iface: Option<&str>) -> PlatformInfo {
    let iface = specified_iface.unwrap_or("en0").to_string();
    let is_virtual = super::is_virtual_interface(&iface);
    let is_wireless = is_wireless_interface(&iface);
    let link_type = if is_virtual {
        "Virtual Overlay".to_string()
    } else if is_wireless {
        "Wi-Fi".to_string()
    } else {
        "macOS CoreWLAN/Link".to_string()
    };
    PlatformInfo {
        interface_name: iface,
        link_speed_mbps: None,
        link_type,
        is_virtual,
        bottleneck_warning: None,
    }
}

pub fn is_wireless_interface(name: &str) -> bool {
    let lower = name.to_lowercase();
    name.starts_with("en0")
        || lower.starts_with("wl")
        || lower.contains("wi-fi")
        || lower.contains("wireless")
        || lower.contains("wlan")
}

pub fn apply_socket_optimizations<S: AsRawFd>(socket: &S, _specified_iface: Option<&str>) {
    let fd = socket.as_raw_fd();
    unsafe {
        // macOS also supports TCP_NOTSENT_LOWAT
        let lowat: libc::c_uint = 16384;
        let _ = libc::setsockopt(
            fd,
            libc::IPPROTO_TCP,
            0x201, // TCP_NOTSENT_LOWAT on darwin
            &lowat as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_uint>() as libc::socklen_t,
        );
    }
}
