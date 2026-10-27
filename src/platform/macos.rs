use crate::metrics::PlatformInfo;
use std::os::unix::io::AsRawFd;

pub fn detect_platform_info(specified_iface: Option<&str>) -> PlatformInfo {
    let iface = specified_iface.unwrap_or("en0").to_string();
    PlatformInfo {
        interface_name: iface,
        link_speed_mbps: None,
        link_type: "macOS CoreWLAN/Link".to_string(),
        is_virtual: false,
        bottleneck_warning: None,
    }
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
