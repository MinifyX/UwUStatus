//! The satellite's end: listens on a public UDP port, lets in the paired main instance only, takes
//! its assignments and keeps results until the main instance acknowledges them.

use std::collections::VecDeque;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use quinn::{Connection, Endpoint, Incoming, RecvStream, SendStream, VarInt};
use serde_json::Value;
use tokio::sync::{Notify, mpsc};
use tokio::time::timeout;

use crate::TunnelError;
use crate::code::{PairingCode, Token};
use crate::identity::{Fingerprint, Identity};
use crate::limits::{HandshakeSlot, Handshakes};
use crate::proto::{
    self, Assignment, MAX_HELLO, MAX_MESSAGE, MainMessage, Refusal, ResultItem, SatelliteMessage, VERSION,
};
use crate::quic::{self, Timing};

const MAX_UNPAIRED_HANDSHAKES: usize = 16;
const MAX_UNPAIRED_PER_NETWORK: usize = 2;
const UNPAIRED_PER_MINUTE: u32 = 20;
/// Room a batch of results leaves inside [`MAX_MESSAGE`] for the envelope around them.
const BATCH_BYTES: usize = MAX_MESSAGE - 4 * 1024;

const CLOSE_REFUSED: VarInt = VarInt::from_u32(1);
const CLOSE_REPLACED: VarInt = VarInt::from_u32(2);
const CLOSE_LOST: VarInt = VarInt::from_u32(3);
const CLOSE_SHUTDOWN: VarInt = VarInt::from_u32(4);

pub struct SatelliteSettings {
    /// Where to listen for the main instance (UDP).
    pub listen: SocketAddr,
    pub identity: Identity,
    /// The paired main instance, as stored after [`SatelliteEvent::Paired`].
    pub main: Option<Fingerprint>,
    /// The one-time token for the pairing code while not paired.
    pub token: Option<Token>,
    /// Told to the main instance in the welcome.
    pub satellite_version: String,
    /// Results kept until acknowledged; past this the oldest are dropped.
    pub buffer_limit: usize,
    pub timing: Timing,
}

impl SatelliteSettings {
    pub fn new(listen: SocketAddr, identity: Identity) -> SatelliteSettings {
        SatelliteSettings {
            listen,
            identity,
            main: None,
            token: None,
            satellite_version: env!("CARGO_PKG_VERSION").to_owned(),
            buffer_limit: 10_000,
            timing: Timing::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SatelliteEvent {
    /// A main instance used the token. Store its fingerprint; the token is used up.
    Paired {
        main: Fingerprint,
    },
    /// The paired main instance is connected. The dead man's switch rests.
    Connected {
        at: SystemTime,
        remote: SocketAddr,
    },
    /// The connection to the main instance is gone. The dead man's switch starts counting.
    Lost {
        at: SystemTime,
        error: String,
    },
    /// The complete list of checks, replacing the one before.
    Assign(Vec<Assignment>),
    DeadMan(Value),
}

pub struct SatelliteListener {
    shared: Arc<Shared>,
    endpoint: Endpoint,
}

struct Shared {
    timing: Timing,
    fingerprint: Fingerprint,
    satellite_version: String,
    buffer_limit: usize,
    state: Mutex<State>,
    /// Woken when results arrive.
    results: Notify,
    events: mpsc::UnboundedSender<SatelliteEvent>,
    handshakes: Arc<Handshakes>,
}

struct State {
    main: Option<Fingerprint>,
    token: Option<Token>,
    /// Where the main instance last came from: it never waits behind strangers' handshakes.
    main_ip: Option<IpAddr>,
    /// Ordered by `seq`, with each item's encoded size.
    buffer: VecDeque<(ResultItem, usize)>,
    next_seq: u64,
    active: Option<Connection>,
}

impl SatelliteListener {
    /// Binds the UDP port and accepts in the background until dropped or [`SatelliteListener::close`].
    pub fn bind(
        settings: SatelliteSettings,
    ) -> Result<(SatelliteListener, mpsc::UnboundedReceiver<SatelliteEvent>), TunnelError> {
        let endpoint = quic::server_endpoint(settings.listen, &settings.identity, &settings.timing)?;
        let (events, receiver) = mpsc::unbounded_channel();
        // Starting at the boot time keeps `seq` growing across restarts, though the buffer is gone.
        let boot_ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or_default();
        let shared = Arc::new(Shared {
            timing: settings.timing,
            fingerprint: settings.identity.fingerprint(),
            satellite_version: settings.satellite_version,
            buffer_limit: settings.buffer_limit.max(1),
            state: Mutex::new(State {
                main: settings.main,
                token: settings.token,
                main_ip: None,
                buffer: VecDeque::new(),
                next_seq: boot_ms * 1000 + 1,
                active: None,
            }),
            results: Notify::new(),
            events,
            handshakes: Handshakes::new(MAX_UNPAIRED_HANDSHAKES, MAX_UNPAIRED_PER_NETWORK, UNPAIRED_PER_MINUTE),
        });
        tokio::spawn(accept(shared.clone(), endpoint.clone()));
        Ok((SatelliteListener { shared, endpoint }, receiver))
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.endpoint.local_addr()
    }

    pub fn fingerprint(&self) -> Fingerprint {
        self.shared.fingerprint
    }

    /// The pairing code for `public` addresses on this listener's port, while a token is waiting.
    pub fn pairing_code(&self, public: &[IpAddr]) -> Option<String> {
        let token = self.state().token.clone()?;
        let port = self.local_addr().ok()?.port();
        let addresses = public.iter().map(|ip| SocketAddr::new(*ip, port)).collect();
        Some(PairingCode { addresses, fingerprint: self.shared.fingerprint, token }.encode())
    }

    pub fn paired_main(&self) -> Option<Fingerprint> {
        self.state().main
    }

    pub fn is_connected(&self) -> bool {
        self.state().active.is_some()
    }

    /// Results waiting for the main instance's acknowledgement.
    pub fn buffered(&self) -> usize {
        self.state().buffer.len()
    }

    /// Keeps a result until the main instance acknowledges it, and sends it now if connected.
    /// Returns its `seq`, or `None` for an outcome too big to ever send.
    pub fn push_result(&self, assignment_id: impl Into<String>, at_unix_ms: i64, outcome: Value) -> Option<u64> {
        let mut state = self.state();
        let item = ResultItem { seq: state.next_seq, assignment_id: assignment_id.into(), at_unix_ms, outcome };
        let size = serde_json::to_vec(&item).map(|body| body.len()).unwrap_or(usize::MAX);
        if size > BATCH_BYTES {
            tracing::warn!(assignment = %item.assignment_id, size, "dropped a result too big for the tunnel");
            return None;
        }
        state.next_seq += 1;
        let seq = item.seq;
        state.buffer.push_back((item, size));
        while state.buffer.len() > self.shared.buffer_limit {
            state.buffer.pop_front();
        }
        drop(state);
        self.shared.results.notify_waiters();
        Some(seq)
    }

    /// Ends the current connection; the main instance dials again.
    pub fn disconnect(&self) {
        if let Some(connection) = self.state().active.as_ref() {
            connection.close(CLOSE_LOST, b"disconnected by the satellite");
        }
    }

    pub fn close(&self) {
        self.endpoint.close(CLOSE_SHUTDOWN, b"the satellite is shutting down");
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.shared.state()
    }
}

impl Drop for SatelliteListener {
    fn drop(&mut self) {
        self.close();
    }
}

impl Shared {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("satellite state poisoned")
    }

    fn authorize(
        &self,
        fingerprint: Fingerprint,
        hello: &MainMessage,
        ip: IpAddr,
    ) -> Result<(), (Refusal, &'static str)> {
        let MainMessage::Hello { version, token } = hello else {
            return Err((Refusal::Other, "the first message must be a hello"));
        };
        if *version != VERSION {
            return Err((Refusal::Version, "this satellite speaks another tunnel version, please update"));
        }
        let mut state = self.state();
        match state.main {
            Some(main) if main == fingerprint => {}
            Some(_) => return Err((Refusal::OtherMain, "this satellite is paired with another main instance")),
            None => {
                let Some(given) = token else {
                    return Err((Refusal::NotPaired, "this satellite is not paired yet"));
                };
                match (&state.token, Token::from_text(given)) {
                    (Some(expected), Some(given)) if expected.matches(&given) => {
                        state.main = Some(fingerprint);
                        state.token = None;
                        tracing::info!(%fingerprint, "paired with a UwUStatus main instance");
                        let _ = self.events.send(SatelliteEvent::Paired { main: fingerprint });
                    }
                    _ => return Err((Refusal::WrongToken, "the pairing code is not valid (any more)")),
                }
            }
        }
        state.main_ip = Some(ip);
        Ok(())
    }

    /// Up to a batch of results after `after`, in order.
    fn next_batch(&self, after: u64) -> Vec<ResultItem> {
        let state = self.state();
        let start = state.buffer.partition_point(|(item, _)| item.seq <= after);
        let mut size = 0;
        let mut batch = Vec::new();
        for (item, len) in state.buffer.range(start..) {
            if size + len > BATCH_BYTES {
                break;
            }
            size += len;
            batch.push(item.clone());
        }
        batch
    }

    fn ack(&self, up_to_seq: u64) {
        let mut state = self.state();
        while state.buffer.front().is_some_and(|(item, _)| item.seq <= up_to_seq) {
            state.buffer.pop_front();
        }
    }
}

async fn accept(shared: Arc<Shared>, endpoint: Endpoint) {
    while let Some(incoming) = endpoint.accept().await {
        if !incoming.remote_address_validated() {
            // A stateless retry proves the source address before it can use up or skip any limit.
            let _ = incoming.retry();
            continue;
        }
        let ip = incoming.remote_address().ip().to_canonical();
        let own = shared.state().main_ip == Some(ip);
        let slot = if own {
            None
        } else {
            match shared.handshakes.begin(ip) {
                Some(slot) => Some(slot),
                None => {
                    tracing::debug!(client = %ip, "turned a tunnel handshake away: too many at once");
                    incoming.refuse();
                    continue;
                }
            }
        };
        tokio::spawn(handle(shared.clone(), incoming, slot));
    }
}

async fn handle(shared: Arc<Shared>, incoming: Incoming, slot: Option<HandshakeSlot>) {
    let remote = incoming.remote_address();
    let limit = shared.timing.handshake_timeout;
    let connection = match timeout(limit, incoming).await {
        Ok(Ok(connection)) => connection,
        Ok(Err(err)) => {
            tracing::debug!(%remote, %err, "a tunnel handshake failed");
            return;
        }
        Err(_) => return,
    };
    let Some(fingerprint) = quic::peer_fingerprint(&connection) else {
        connection.close(CLOSE_REFUSED, b"no certificate");
        return;
    };
    let hello = timeout(limit, async {
        let (send, mut recv) = connection.accept_bi().await.map_err(io::Error::other)?;
        let hello: MainMessage = proto::read_message(&mut recv, MAX_HELLO).await?;
        Ok::<_, io::Error>((send, recv, hello))
    })
    .await;
    let Ok(Ok((mut send, recv, hello))) = hello else {
        tracing::debug!(%remote, "no hello on the tunnel");
        connection.close(CLOSE_REFUSED, b"no hello");
        return;
    };

    match shared.authorize(fingerprint, &hello, remote.ip().to_canonical()) {
        Ok(()) => {
            // The paired main instance: its place among the strangers is free again.
            drop(slot);
            quic::open_up(&connection);
            let welcome =
                SatelliteMessage::Welcome { version: VERSION, satellite_version: shared.satellite_version.clone() };
            if proto::write_message(&mut send, &welcome).await.is_ok() {
                serve(shared, connection, send, recv, remote).await;
            }
        }
        Err((reason, message)) => {
            tracing::warn!(%remote, ?reason, %fingerprint, "refused a tunnel");
            let refused = SatelliteMessage::Refused { reason, message: message.to_owned() };
            let _ = proto::write_message(&mut send, &refused).await;
            let _ = send.finish();
            // The main instance hangs up after reading the answer; closing first could swallow it.
            let _ = timeout(limit, connection.closed()).await;
            connection.close(CLOSE_REFUSED, b"refused");
        }
    }
}

async fn serve(
    shared: Arc<Shared>,
    connection: Connection,
    send: SendStream,
    mut recv: RecvStream,
    remote: SocketAddr,
) {
    if let Some(previous) = shared.state().active.replace(connection.clone()) {
        previous.close(CLOSE_REPLACED, b"replaced by a new connection");
    }
    tracing::info!(%remote, "the main instance is connected");
    let _ = shared.events.send(SatelliteEvent::Connected { at: SystemTime::now(), remote });

    let reading = async {
        loop {
            match proto::read_known::<_, MainMessage>(&mut recv, MAX_MESSAGE).await {
                Ok(Some(MainMessage::Assign { checks })) => {
                    let _ = shared.events.send(SatelliteEvent::Assign(checks));
                }
                Ok(Some(MainMessage::DeadMan { config })) => {
                    let _ = shared.events.send(SatelliteEvent::DeadMan(config));
                }
                Ok(Some(MainMessage::Ack { up_to_seq })) => shared.ack(up_to_seq),
                Ok(Some(MainMessage::Hello { .. }) | None) => {}
                Err(err) => return err,
            }
        }
    };
    let error = tokio::select! {
        error = connection.closed() => error.to_string(),
        error = reading => format!("reading from the main instance failed: {error}"),
        error = send_results(&shared, send) => format!("sending results failed: {error}"),
    };
    connection.close(CLOSE_LOST, b"connection lost");
    let still_active = {
        let mut state = shared.state();
        let current = state.active.as_ref().is_some_and(|active| active.stable_id() == connection.stable_id());
        if current {
            state.active = None;
        }
        current
    };
    // A connection replaced by a newer one is no loss.
    if still_active {
        tracing::warn!(%remote, %error, "lost the main instance");
        let _ = shared.events.send(SatelliteEvent::Lost { at: SystemTime::now(), error });
    }
}

/// Sends everything not acknowledged yet, from the start for every connection, then each new
/// result as it comes.
async fn send_results(shared: &Shared, mut send: SendStream) -> io::Error {
    let mut sent = 0;
    loop {
        let notified = shared.results.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let batch = shared.next_batch(sent);
        let Some(last) = batch.last() else {
            notified.await;
            continue;
        };
        sent = last.seq;
        if let Err(err) = proto::write_message(&mut send, &SatelliteMessage::Results { items: batch }).await {
            return err;
        }
    }
}
