//! Which addresses belong to the public internet.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};

/// Whether `ip` is reachable on the public internet. Loopback, private, link-local, shared
/// (carrier-grade NAT), documentation, benchmarking, multicast and reserved addresses are not.
pub fn is_global(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_global_v4(ip),
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_global_v4(mapped);
            }
            let s = ip.segments();
            // NAT64 (64:ff9b::/96) carries an IPv4 address, which decides.
            if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0] {
                return is_global_v4(Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8));
            }
            !(ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local
                || (s[0] & 0xffc0) == 0xfe80 // link-local
                || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
                || (s[0] == 0x2001 && s[1] < 0x0200) // IETF protocol assignments, Teredo
                || s[0] == 0x2002 // 6to4
                || (s[0] == 0x0100 && s[1..4] == [0, 0, 0])) // discard-only
        }
    }
}

fn is_global_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_documentation()
        || ip.is_multicast()
        || a >= 240 // reserved and broadcast
        || (a == 100 && (b & 0xc0) == 64) // shared address space (carrier-grade NAT)
        || (a == 192 && b == 0 && c == 0) // IETF protocol assignments
        || (a == 198 && (b & 0xfe) == 18)) // benchmarking
}

/// The public addresses this machine sends from, IPv4 first. Asks the routing table by
/// "connecting" UDP sockets, which sends no packet at all.
pub fn public_addresses() -> Vec<IpAddr> {
    let mut found = Vec::new();
    for (bind, target) in [("0.0.0.0:0", "1.1.1.1:53"), ("[::]:0", "[2606:4700:4700::1111]:53")] {
        let Ok(socket) = UdpSocket::bind(bind) else { continue };
        let Ok(target) = target.parse::<SocketAddr>() else { continue };
        if socket.connect(target).is_ok()
            && let Ok(local) = socket.local_addr()
            && is_global(local.ip())
        {
            found.push(local.ip());
        }
    }
    found
}
