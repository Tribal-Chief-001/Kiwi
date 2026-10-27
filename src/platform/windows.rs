use crate::metrics::PlatformInfo;

pub fn detect_platform_info(specified_iface: Option<&str>) -> PlatformInfo {
    let iface = specified_iface.unwrap_or("Wi-Fi / Ethernet").to_string();
    let is_virtual = super::is_virtual_interface(&iface);
    let is_wireless = is_wireless_interface(&iface);
    let link_type = if is_virtual {
        "Virtual Overlay".to_string()
    } else if is_wireless {
        "Wi-Fi".to_string()
    } else {
        "Windows Native Link".to_string()
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
    lower.contains("wi-fi") || lower.contains("wireless") || lower.contains("wlan")
}

pub fn apply_socket_optimizations<S>(_socket: &S, _specified_iface: Option<&str>) {
    // Windows socket options if needed
}
