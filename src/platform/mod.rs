#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "windows")]
pub use windows::*;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub mod generic {
    use crate::metrics::PlatformInfo;
    pub fn detect_platform_info(specified_iface: Option<&str>) -> PlatformInfo {
        PlatformInfo {
            interface_name: specified_iface.unwrap_or("eth0").to_string(),
            link_speed_mbps: None,
            link_type: "Generic Link".to_string(),
            is_virtual: false,
            bottleneck_warning: None,
        }
    }
    pub fn is_wireless_interface(name: &str) -> bool {
        name.starts_with("wl")
    }
    pub fn apply_socket_optimizations<S>(_socket: &S, _specified_iface: Option<&str>) {}
}
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub use generic::*;

/// Determine if an interface is virtual across platforms (bridges, containers, tunnels, VPNs)
pub fn is_virtual_interface(name: &str) -> bool {
    let prefixes = [
        "veth",
        "docker",
        "br-",
        "tun",
        "tap",
        "wg",
        "tailscale",
        "dummy",
        "virbr",
        "cni",
        "flannel",
        "kube",
        "podman",
        "utun",
        "bridge",
        "p2p",
        "awdl",
        "llw",
    ];
    let lower = name.to_lowercase();
    prefixes.iter().any(|prefix| lower.starts_with(prefix))
        || lower.contains("virtual")
        || lower.contains("loopback")
        || lower.contains("vethernet")
}
