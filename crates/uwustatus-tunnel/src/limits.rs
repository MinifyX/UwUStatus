//! How many tunnel handshakes strangers may start, in total and per network.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Tunnel handshakes of peers that have not shown yet to be the paired main instance. Anyone can
/// start one, and each costs a TLS handshake and a task waiting for its hello: a few at once in
/// all, two at once and twenty a minute per network. The main instance's last address never waits
/// for them.
pub(crate) struct Handshakes {
    state: Mutex<HandshakeState>,
    max_total: usize,
    max_per_network: usize,
    per_minute: u32,
}

#[derive(Default)]
struct HandshakeState {
    total: usize,
    pending: HashMap<IpAddr, usize>,
    started: HashMap<IpAddr, (u32, Instant)>,
}

/// A handshake under way. Gives its place back when dropped.
pub(crate) struct HandshakeSlot {
    handshakes: Arc<Handshakes>,
    network: IpAddr,
}

const HANDSHAKE_WINDOW: Duration = Duration::from_secs(60);

impl Handshakes {
    pub fn new(max_total: usize, max_per_network: usize, per_minute: u32) -> Arc<Handshakes> {
        Arc::new(Handshakes {
            state: Mutex::new(HandshakeState::default()),
            max_total: max_total.max(1),
            max_per_network: max_per_network.max(1),
            per_minute: per_minute.max(1),
        })
    }

    pub fn begin(self: &Arc<Self>, ip: IpAddr) -> Option<HandshakeSlot> {
        let network = network_of(ip);
        let mut state = self.state.lock().expect("handshakes poisoned");
        if state.total >= self.max_total || state.pending.get(&network).copied().unwrap_or(0) >= self.max_per_network {
            return None;
        }
        if state.started.len() > 10_000 {
            state.started.retain(|_, (_, since)| since.elapsed() < HANDSHAKE_WINDOW);
        }
        let started = state.started.entry(network).or_insert((0, Instant::now()));
        if started.1.elapsed() >= HANDSHAKE_WINDOW {
            *started = (0, Instant::now());
        }
        if started.0 >= self.per_minute {
            return None;
        }
        started.0 += 1;
        state.total += 1;
        *state.pending.entry(network).or_default() += 1;
        Some(HandshakeSlot { handshakes: self.clone(), network })
    }
}

impl Drop for HandshakeSlot {
    fn drop(&mut self) {
        let mut state = self.handshakes.state.lock().expect("handshakes poisoned");
        state.total -= 1;
        if let Some(count) = state.pending.get_mut(&self.network) {
            *count -= 1;
            if *count == 0 {
                state.pending.remove(&self.network);
            }
        }
    }
}

/// One IPv4 address, or the /64 an IPv6 address belongs to (what one household or server gets).
fn network_of(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => IpAddr::V6(Ipv6Addr::from(u128::from(v6) & (u128::MAX << 64))),
        v4 => v4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strangers_get_a_few_handshakes() {
        let handshakes = Handshakes::new(3, 2, 4);
        let a: IpAddr = "192.0.2.1".parse().unwrap();
        let first = handshakes.begin(a).unwrap();
        let second = handshakes.begin(a).unwrap();
        assert!(handshakes.begin(a).is_none(), "two at once per network");
        let other = handshakes.begin("2001:db8::1".parse().unwrap()).unwrap();
        assert!(handshakes.begin("2001:db8:1::1".parse().unwrap()).is_none(), "three at once in all");
        drop((first, second, other));
        let third = handshakes.begin(a).unwrap();
        let fourth = handshakes.begin(a).unwrap();
        drop((third, fourth));
        assert!(handshakes.begin(a).is_none(), "four a minute per network");
        assert!(handshakes.begin("192.0.2.2".parse().unwrap()).is_some(), "others still get theirs");
        assert_eq!(network_of("2001:db8::1:2".parse().unwrap()), "2001:db8::".parse::<IpAddr>().unwrap());
    }
}
