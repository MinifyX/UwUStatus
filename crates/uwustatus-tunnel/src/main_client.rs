//! The main instance's end: dials a satellite from behind NAT, stays connected, hands it the checks
//! and the dead man's switch, and passes its results on.

use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use quinn::{Connection, Endpoint, VarInt};
use serde_json::Value;
use tokio::sync::{mpsc, watch};
use tokio::time::timeout;

use crate::code::{PairingCode, Token};
use crate::identity::{CERTIFICATE_NAME, Fingerprint, Identity};
use crate::proto::{
    self, Assignment, MAX_HELLO, MAX_MESSAGE, MainMessage, Refusal, ResultItem, SatelliteMessage, VERSION,
};
use crate::quic::{self, Timing};

pub struct MainSettings {
    /// Where the satellite listens, tried in this order.
    pub addresses: Vec<SocketAddr>,
    pub satellite: Fingerprint,
    pub identity: Identity,
    /// The token from the pairing code, until the satellite knows this main instance.
    pub token: Option<Token>,
    pub timing: Timing,
}

impl MainSettings {
    /// For a satellite that is not paired yet.
    pub fn from_code(code: &PairingCode, identity: Identity) -> MainSettings {
        MainSettings {
            addresses: code.addresses.clone(),
            satellite: code.fingerprint,
            identity,
            token: Some(code.token.clone()),
            timing: Timing::default(),
        }
    }

    /// For a satellite paired before, from what was stored after [`MainEvent::Paired`].
    pub fn pinned(addresses: Vec<SocketAddr>, satellite: Fingerprint, identity: Identity) -> MainSettings {
        MainSettings { addresses, satellite, identity, token: None, timing: Timing::default() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MainEvent {
    /// The satellite took the token and knows this main instance now; store it as pinned.
    Paired,
    Connected {
        at: SystemTime,
        remote: SocketAddr,
        satellite_version: String,
    },
    /// A connection that was up is gone; dialling again.
    Lost {
        at: SystemTime,
        error: String,
    },
    /// Dialling failed; trying again with growing pauses.
    Unreachable {
        error: String,
    },
    /// The satellite does not accept this main instance. Asked again after `Timing::refused_retry`.
    Refused {
        reason: Refusal,
        message: String,
    },
    /// Store them, then [`MainClient::ack`] the last `seq`. Unacknowledged ones come again after a
    /// reconnect, so store once per `(satellite, seq)`.
    Results(Vec<ResultItem>),
}

/// Dials in the background until dropped or [`MainClient::stop`].
pub struct MainClient {
    shared: Arc<Shared>,
    stop: watch::Sender<bool>,
}

struct Shared {
    latest: Mutex<Latest>,
    connected: AtomicBool,
}

/// What every new connection gets first, and where messages for the current one go.
#[derive(Default)]
struct Latest {
    assign: Option<Vec<Assignment>>,
    dead_man: Option<Value>,
    outbox: Option<mpsc::UnboundedSender<MainMessage>>,
}

enum Ended {
    Lost(String),
    Stopped,
}

enum Failure {
    Refused(Refusal, String),
    Failed(String),
}

impl MainClient {
    pub fn start(settings: MainSettings) -> (MainClient, mpsc::UnboundedReceiver<MainEvent>) {
        let (events, receiver) = mpsc::unbounded_channel();
        let (stop, stopped) = watch::channel(false);
        let shared = Arc::new(Shared { latest: Mutex::default(), connected: AtomicBool::new(false) });
        tokio::spawn(run(shared.clone(), settings, events, stopped));
        (MainClient { shared, stop }, receiver)
    }

    /// Replaces the satellite's checks, now if connected and again on every reconnect.
    pub fn assign(&self, checks: Vec<Assignment>) {
        let mut latest = self.latest();
        latest.assign = Some(checks.clone());
        send(&latest, MainMessage::Assign { checks });
    }

    /// Replaces the dead man's switch, now if connected and again on every reconnect.
    pub fn dead_man(&self, config: Value) {
        let mut latest = self.latest();
        latest.dead_man = Some(config.clone());
        send(&latest, MainMessage::DeadMan { config });
    }

    /// Tells the satellite results up to `up_to_seq` are stored. Says whether it went out; when it
    /// did not, the satellite sends them again after the reconnect.
    pub fn ack(&self, up_to_seq: u64) -> bool {
        send(&self.latest(), MainMessage::Ack { up_to_seq })
    }

    pub fn is_connected(&self) -> bool {
        self.shared.connected.load(Ordering::Relaxed)
    }

    pub fn stop(&self) {
        let _ = self.stop.send(true);
    }

    fn latest(&self) -> std::sync::MutexGuard<'_, Latest> {
        self.shared.latest.lock().expect("main client state poisoned")
    }
}

impl Drop for MainClient {
    fn drop(&mut self) {
        self.stop();
    }
}

fn send(latest: &Latest, message: MainMessage) -> bool {
    latest.outbox.as_ref().is_some_and(|outbox| outbox.send(message).is_ok())
}

async fn run(
    shared: Arc<Shared>,
    mut settings: MainSettings,
    events: mpsc::UnboundedSender<MainEvent>,
    mut stop: watch::Receiver<bool>,
) {
    let timing = settings.timing;
    let mut backoff = timing.retry_min;
    loop {
        let outcome = session(&shared, &mut settings, &events, &mut stop).await;
        shared.connected.store(false, Ordering::Relaxed);
        shared.latest.lock().expect("main client state poisoned").outbox = None;
        let wait = match outcome {
            Ok(Ended::Stopped) => break,
            Ok(Ended::Lost(error)) => {
                tracing::warn!(%error, "lost the connection to the satellite, reconnecting");
                let _ = events.send(MainEvent::Lost { at: SystemTime::now(), error });
                backoff = timing.retry_min;
                backoff
            }
            Err(Failure::Refused(reason, message)) => {
                tracing::error!(?reason, %message, "the satellite refused this main instance");
                let _ = events.send(MainEvent::Refused { reason, message });
                timing.refused_retry
            }
            Err(Failure::Failed(error)) => {
                tracing::warn!(%error, "could not reach the satellite");
                let _ = events.send(MainEvent::Unreachable { error });
                let wait = backoff;
                backoff = (backoff * 2).min(timing.retry_max);
                wait
            }
        };
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = stop.changed() => break,
        }
    }
}

async fn session(
    shared: &Shared,
    settings: &mut MainSettings,
    events: &mpsc::UnboundedSender<MainEvent>,
    stop: &mut watch::Receiver<bool>,
) -> Result<Ended, Failure> {
    let (endpoint, connection) = dial(settings).await.map_err(Failure::Failed)?;
    let answer = timeout(settings.timing.handshake_timeout, async {
        let (mut send, mut recv) = connection.open_bi().await.map_err(io::Error::other)?;
        let hello = MainMessage::Hello { version: VERSION, token: settings.token.as_ref().map(Token::to_text) };
        proto::write_message(&mut send, &hello).await?;
        let answer: SatelliteMessage = proto::read_message(&mut recv, MAX_HELLO).await?;
        Ok::<_, io::Error>((send, recv, answer))
    })
    .await
    .map_err(|_| Failure::Failed("the satellite did not answer".into()))?
    .map_err(|err| Failure::Failed(format!("talking to the satellite failed: {err}")))?;
    let (mut send, mut recv, satellite_version) = match answer {
        (send, recv, SatelliteMessage::Welcome { satellite_version, .. }) => (send, recv, satellite_version),
        (_, _, SatelliteMessage::Refused { reason, message }) => {
            connection.close(VarInt::from_u32(0), b"refused");
            return Err(Failure::Refused(reason, message));
        }
        (_, _, other) => {
            connection.close(VarInt::from_u32(0), b"no welcome");
            return Err(Failure::Failed(format!("the satellite answered {other:?} instead of a welcome")));
        }
    };
    if settings.token.take().is_some() {
        tracing::info!("paired with the satellite");
        let _ = events.send(MainEvent::Paired);
    }

    let (outbox, mut queued) = mpsc::unbounded_channel();
    {
        let mut latest = shared.latest.lock().expect("main client state poisoned");
        if let Some(checks) = latest.assign.clone() {
            let _ = outbox.send(MainMessage::Assign { checks });
        }
        if let Some(config) = latest.dead_man.clone() {
            let _ = outbox.send(MainMessage::DeadMan { config });
        }
        latest.outbox = Some(outbox);
    }
    shared.connected.store(true, Ordering::Relaxed);
    let remote = connection.remote_address();
    tracing::info!(%remote, %satellite_version, "connected to the satellite");
    let _ = events.send(MainEvent::Connected { at: SystemTime::now(), remote, satellite_version });

    let writing = async {
        while let Some(message) = queued.recv().await {
            proto::write_message(&mut send, &message).await?;
        }
        Ok::<_, io::Error>(())
    };
    let reading = async {
        loop {
            match proto::read_known(&mut recv, MAX_MESSAGE).await {
                Ok(Some(SatelliteMessage::Results { items })) => {
                    let _ = events.send(MainEvent::Results(items));
                }
                Ok(_) => {}
                Err(err) => break err,
            }
        }
    };
    let ended = tokio::select! {
        error = connection.closed() => Ended::Lost(error.to_string()),
        result = writing => Ended::Lost(format!("sending to the satellite failed: {}", describe(result))),
        error = reading => Ended::Lost(format!("reading from the satellite failed: {error}")),
        _ = stop.changed() => Ended::Stopped,
    };
    close(&endpoint, &connection, matches!(ended, Ended::Stopped)).await;
    Ok(ended)
}

fn describe(result: io::Result<()>) -> String {
    result.err().map(|err| err.to_string()).unwrap_or_else(|| "ended".into())
}

async fn close(endpoint: &Endpoint, connection: &Connection, stopped: bool) {
    connection.close(VarInt::from_u32(0), if stopped { b"stopped" } else { b"lost" });
    if stopped {
        let _ = timeout(Duration::from_secs(2), endpoint.wait_idle()).await;
    }
}

async fn dial(settings: &MainSettings) -> Result<(Endpoint, Connection), String> {
    let config =
        quic::client_config(&settings.identity, settings.satellite, &settings.timing).map_err(|err| err.to_string())?;
    let mut errors = Vec::new();
    for address in &settings.addresses {
        // Wildcard sockets of one family do not reliably reach the other on every system.
        let bind = match address {
            SocketAddr::V4(_) => SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
            SocketAddr::V6(_) => SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)),
        };
        let attempt = async {
            let endpoint = Endpoint::client(bind).map_err(|err| err.to_string())?;
            let connecting =
                endpoint.connect_with(config.clone(), *address, CERTIFICATE_NAME).map_err(|err| err.to_string())?;
            match timeout(settings.timing.handshake_timeout, connecting).await {
                Ok(Ok(connection)) => Ok((endpoint, connection)),
                Ok(Err(err)) => Err(err.to_string()),
                Err(_) => Err("no answer".to_owned()),
            }
        };
        match attempt.await {
            Ok(connected) => return Ok(connected),
            Err(err) => errors.push(format!("{address}: {err}")),
        }
    }
    if errors.is_empty() {
        errors.push("no address for the satellite".into());
    }
    Err(errors.join("; "))
}
