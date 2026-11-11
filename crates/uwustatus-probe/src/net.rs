//! TCP-Port und Ping (ICMP Echo).

use std::io::ErrorKind;
use std::sync::atomic::{AtomicU16, Ordering};

use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::{TcpStream, UdpSocket};

use crate::{Outcome, PingCheck, Status, TcpCheck};

pub(crate) async fn tcp(c: &TcpCheck) -> Result<Outcome, String> {
    let stream = TcpStream::connect((c.host.as_str(), c.port)).await.map_err(|e| e.to_string())?;
    let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
    Ok(Outcome::new(Status::Up, format!("port open ({peer})")))
}

static SEQ: AtomicU16 = AtomicU16::new(1);

/// Ein Echo über einen unprivilegierten ICMP-Datagram-Socket (Linux:
/// `net.ipv4.ping_group_range`). Der Kernel setzt Kennung und Prüfsumme und
/// liefert nur Antworten auf diesen Socket, ohne IP-Header.
pub(crate) async fn ping(c: &PingCheck) -> Result<Outcome, String> {
    let addr = tokio::net::lookup_host((c.host.as_str(), 0))
        .await
        .map_err(|e| format!("cannot resolve {}: {e}", c.host))?
        .next()
        .ok_or_else(|| format!("no address for {}", c.host))?;
    let (domain, proto, request, reply) = match addr.is_ipv4() {
        true => (Domain::IPV4, Protocol::ICMPV4, 8, 0),
        false => (Domain::IPV6, Protocol::ICMPV6, 128, 129),
    };
    let sock = Socket::new(domain, Type::DGRAM, Some(proto)).map_err(|e| match e.kind() {
        ErrorKind::PermissionDenied => {
            "ICMP not permitted: unprivileged ping sockets are disabled (sysctl net.ipv4.ping_group_range)".to_owned()
        }
        _ => format!("cannot open ICMP socket: {e}"),
    })?;
    sock.set_nonblocking(true).map_err(|e| e.to_string())?;
    sock.connect(&addr.into()).map_err(|e| e.to_string())?;
    let sock = UdpSocket::from_std(sock.into()).map_err(|e| e.to_string())?;

    let seq = SEQ.fetch_add(1, Ordering::Relaxed).to_be_bytes();
    let packet = [request, 0, 0, 0, 0, 0, seq[0], seq[1], b'u', b'w', b'u', b's', b't', b'a', b't', b's'];
    sock.send(&packet).await.map_err(|e| e.to_string())?;
    let mut buf = [0u8; 1500];
    loop {
        let n = sock.recv(&mut buf).await.map_err(|e| e.to_string())?;
        if n >= 8 && buf[0] == reply && buf[6..8] == seq {
            return Ok(Outcome::new(Status::Up, format!("reply from {}", addr.ip())));
        }
    }
}
