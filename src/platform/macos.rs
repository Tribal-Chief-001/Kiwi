use crate::metrics::{KernelTcpInfo, PlatformInfo};
use std::os::unix::io::{AsRawFd, RawFd};

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

#[repr(C)]
#[derive(Default)]
struct DarwinTcpConnectionInfo {
    pub tcpi_state: u8,
    pub tcpi_snd_wscale: u8,
    pub tcpi_rcv_wscale: u8,
    pub __pad1: u8,
    pub tcpi_options: u32,
    pub tcpi_rto: u32,
    pub tcpi_maxseg: u32,
    pub tcpi_snd_ssthresh: u32,
    pub tcpi_snd_cwnd: u32,
    pub tcpi_snd_wnd: u32,
    pub tcpi_sbbytes: u32,
    pub tcpi_rttcur: u32,
    pub tcpi_srtt: u32,
    pub tcpi_rttvar: u32,
    pub tcpi_tfo: u32,
    pub tcpi_txpackets: u64,
    pub tcpi_txbytes: u64,
    pub tcpi_txretransmitbytes: u64,
    pub tcpi_txretransmitpackets: u64,
    pub tcpi_rxpackets: u64,
    pub tcpi_rxbytes: u64,
    pub tcpi_rxoutoforderbytes: u64,
    pub tcpi_txretransmitbytes_total: u64,
}

/// Read live kernel TCP socket metrics via getsockopt(TCP_CONNECTION_INFO) directly from Darwin kernel
pub fn read_tcp_info(fd: RawFd) -> Option<KernelTcpInfo> {
    if fd < 0 {
        return None;
    }
    unsafe {
        let mut info: DarwinTcpConnectionInfo = std::mem::zeroed();
        let mut len = std::mem::size_of::<DarwinTcpConnectionInfo>() as libc::socklen_t;
        // TCP_CONNECTION_INFO on Darwin is 0x106
        let res = libc::getsockopt(
            fd,
            libc::IPPROTO_TCP,
            0x106,
            &mut info as *mut _ as *mut libc::c_void,
            &mut len,
        );
        if res == 0 {
            Some(KernelTcpInfo {
                rtt_us: info.tcpi_srtt * 1000,
                min_rtt_us: info.tcpi_rttcur * 1000,
                snd_cwnd: info.tcpi_snd_cwnd,
                retrans: info.tcpi_txretransmitpackets as u32,
                total_retrans: info.tcpi_txretransmitpackets as u32,
                delivery_rate_bps: None,
            })
        } else {
            None
        }
    }
}

