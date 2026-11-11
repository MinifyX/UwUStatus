//! Alert dispatcher: listens on the event bus, picks the channels of the monitor, builds a short
//! DE/EN text and sends it with retries. Every delivery ends up in `notification_log`.

use crate::app::AppState;
use crate::channels::{self, Stored};
use crate::engine::State as MonState;
use crate::events::Event;
use std::sync::RwLock;
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use uwustatus_notify::{Lang, Message, Sender, Severity};

/// Rows kept per channel in the delivery log.
pub const LOG_KEEP: i64 = 100;

pub struct Options {
    pub sender: Sender,
    /// Waits before the 2nd, 3rd, … attempt.
    pub backoff: Vec<Duration>,
}

pub struct Alerts(RwLock<Options>);

impl Alerts {
    pub fn new(http: reqwest::Client) -> Self {
        Self(RwLock::new(Options {
            sender: Sender::with_client(http),
            backoff: [10, 60, 300].map(Duration::from_secs).to_vec(),
        }))
    }

    pub fn sender(&self) -> Sender {
        self.0.read().unwrap_or_else(|e| e.into_inner()).sender.clone()
    }

    fn backoff(&self) -> Vec<Duration> {
        self.0.read().unwrap_or_else(|e| e.into_inner()).backoff.clone()
    }

    /// For tests: other API base URLs, no waiting.
    pub fn configure(&self, f: impl FnOnce(&mut Options)) {
        f(&mut self.0.write().unwrap_or_else(|e| e.into_inner()));
    }
}

/// Starts the dispatcher. Call once.
pub fn spawn(state: &AppState) {
    let mut rx = state.events.subscribe();
    let state = state.clone();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => dispatch(&state, &event),
                Err(RecvError::Lagged(n)) => tracing::warn!("alerts: {n} events missed"),
                Err(RecvError::Closed) => break,
            }
        }
    });
}

/// `down`, `degraded`, `up`, `reminder`, `cert`, `version`, or nothing to send.
fn kind(event: &Event) -> Option<&'static str> {
    match event {
        Event::StateChanged { alert: false, .. } => None,
        Event::StateChanged { to: MonState::Down, .. } => Some("down"),
        Event::StateChanged { to: MonState::Degraded, .. } => Some("degraded"),
        Event::StateChanged { to: MonState::Up, .. } => Some("up"),
        Event::StateChanged { .. } => None,
        Event::Reminder { .. } => Some("reminder"),
        Event::CertExpiring { .. } => Some("cert"),
        Event::VersionChanged { .. } => Some("version"),
    }
}

fn monitor_id(event: &Event) -> i64 {
    match event {
        Event::StateChanged { monitor_id, .. }
        | Event::Reminder { monitor_id, .. }
        | Event::CertExpiring { monitor_id, .. }
        | Event::VersionChanged { monitor_id, .. } => *monitor_id,
    }
}

fn wants(events: &channels::Events, kind: &str) -> bool {
    match kind {
        "down" => events.down,
        "degraded" => events.degraded,
        "up" => events.up,
        "reminder" => events.reminder,
        "cert" => events.cert,
        "version" => events.version,
        _ => true,
    }
}

struct Plan {
    name: String,
    /// Outage length for `up`.
    outage: Option<i64>,
    channels: Vec<Stored>,
}

fn plan(state: &AppState, event: &Event, kind: &str) -> rusqlite::Result<Option<Plan>> {
    let id = monitor_id(event);
    let now = state.clock.now();
    state.db.with(|c| {
        let Some(monitor) = crate::monitors::one(c, id)? else { return Ok(None) };
        let current: Option<String> = rusqlite::OptionalExtension::optional(c.query_row(
            "SELECT state FROM monitor_state WHERE monitor_id = ?1",
            [id],
            |r| r.get(0),
        ))?;
        // State changes come gated by the core; certificate and version events do not.
        let quiet = monitor.config.muted
            || monitor.config.paused
            || current.as_deref() == Some(MonState::Maintenance.as_str())
            || crate::maintenance::covers(&crate::maintenance::all(c)?, id, &monitor.config, now);
        if quiet {
            return Ok(None);
        }
        let channels: Vec<Stored> = channels::of_monitor(c, id)?
            .into_iter()
            .filter(|ch| ch.config.enabled && wants(&ch.config.events, kind))
            .collect();
        if channels.is_empty() {
            return Ok(None);
        }
        let outage = match event {
            Event::StateChanged { from, at, .. } if kind == "up" => {
                rusqlite::OptionalExtension::optional(c.query_row(
                    "SELECT at FROM events WHERE monitor_id = ?1 AND kind = 'state' AND to_value = ?2 AND at <= ?3
                 ORDER BY id DESC LIMIT 1",
                    rusqlite::params![id, from.as_str(), at],
                    |r| r.get::<_, i64>(0),
                ))?
                .map(|since| at - since)
            }
            _ => None,
        };
        Ok(Some(Plan { name: monitor.config.name, outage, channels }))
    })
}

fn dispatch(state: &AppState, event: &Event) {
    let Some(kind) = kind(event) else { return };
    let plan = match plan(state, event, kind) {
        Ok(Some(plan)) => plan,
        Ok(None) => return,
        Err(e) => return tracing::warn!("alerts: {e}"),
    };
    let smtp = channels::smtp(&state.db);
    let monitor = monitor_id(event);
    let link = format!("{}/monitors/{monitor}", state.config.public_url);
    for channel in plan.channels {
        let message = text(event, channel.config.lang, &plan.name, plan.outage, &link);
        let target = channel.target(smtp.clone());
        let state = state.clone();
        tokio::spawn(async move {
            let sender = state.alerts.sender();
            let mut waits = state.alerts.backoff().into_iter();
            let mut attempts = 0;
            let result = loop {
                attempts += 1;
                match sender.send(&target, &message).await {
                    Ok(()) => break Ok(()),
                    Err(e) => match waits.next() {
                        Some(wait) => {
                            tracing::info!("channel {}: attempt {attempts} failed: {e}", channel.id);
                            tokio::time::sleep(wait).await;
                        }
                        None => break Err(e),
                    },
                }
            };
            log(&state, channel.id, Some(monitor), kind, &message.title, attempts, result.as_ref().err());
        });
    }
}

/// Appends to the delivery log and keeps the last [`LOG_KEEP`] rows of the channel.
pub fn log(
    state: &AppState,
    channel_id: i64,
    monitor_id: Option<i64>,
    event: &str,
    title: &str,
    attempts: i64,
    error: Option<&uwustatus_notify::Error>,
) {
    let error: String = error.map(|e| e.0.chars().take(500).collect()).unwrap_or_default();
    let at = state.clock.now();
    let result = state.db.with(|c| {
        c.execute(
            "INSERT INTO notification_log (channel_id, at, monitor_id, event, title, ok, attempts, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![channel_id, at, monitor_id, event, title, error.is_empty(), attempts, error],
        )?;
        c.execute(
            "DELETE FROM notification_log WHERE channel_id = ?1 AND id <=
             (SELECT id FROM notification_log WHERE channel_id = ?1 ORDER BY id DESC LIMIT 1 OFFSET ?2)",
            rusqlite::params![channel_id, LOG_KEEP],
        )
    });
    match result {
        Err(e) => tracing::warn!("alerts: log for channel {channel_id}: {e}"),
        Ok(_) if !error.is_empty() => tracing::warn!("channel {channel_id}: not sent: {error}"),
        Ok(_) => {}
    }
}

/// "3 h 12 min", "45 s", "2 d 4 h".
pub fn duration(seconds: i64, lang: Lang) -> String {
    let s = seconds.max(0);
    let (d, h, m) = (s / 86_400, s % 86_400 / 3600, s % 3600 / 60);
    let day = if lang == Lang::De { "T" } else { "d" };
    match () {
        _ if d > 0 => format!("{d} {day} {h} h"),
        _ if h > 0 => format!("{h} h {m} min"),
        _ if m > 0 => format!("{m} min"),
        _ => format!("{s} s"),
    }
}

fn utc(at: i64) -> String {
    let iso = crate::iso(at);
    format!("{} UTC", iso.trim_end_matches('Z').replacen('T', " ", 1))
}

pub fn text(event: &Event, lang: Lang, name: &str, outage: Option<i64>, link: &str) -> Message {
    let de = lang == Lang::De;
    let pick = |a: &str, b: &str| if de { a.to_owned() } else { b.to_owned() };
    let reason = |r: &str| if r.is_empty() { pick("unbekannt", "unknown") } else { r.to_owned() };
    let (title, body, severity) = match event {
        Event::StateChanged { to: MonState::Up, at, .. } => {
            let mut body = format!("{} {}", pick("Wieder erreichbar seit", "Back up since"), utc(*at));
            if let Some(o) = outage {
                body.push_str(&format!("\n{} {}", pick("Ausfall dauerte", "Outage lasted"), duration(o, lang)));
            }
            (pick("wieder erreichbar", "back up"), body, Severity::Up)
        }
        Event::StateChanged { to, at, reason: r, .. } => {
            let (word, severity) = if *to == MonState::Degraded {
                (pick("eingeschränkt", "degraded"), Severity::Degraded)
            } else {
                (pick("ausgefallen", "down"), Severity::Down)
            };
            let body = format!("{}: {}\n{}: {}", pick("Grund", "Reason"), reason(r), pick("Seit", "Since"), utc(*at));
            (word, body, severity)
        }
        Event::Reminder { at, since, reason: r, .. } => {
            let body = format!(
                "{} {} ({} {})\n{}: {}",
                pick("Ausgefallen seit", "Down for"),
                duration(at - since, lang),
                pick("seit", "since"),
                utc(*since),
                pick("Grund", "Reason"),
                reason(r)
            );
            (pick("weiterhin ausgefallen", "still down"), body, Severity::Down)
        }
        Event::CertExpiring { days_left, .. } => (
            if de {
                format!("Zertifikat läuft in {days_left} Tagen ab")
            } else {
                format!("certificate expires in {days_left} days")
            },
            pick("Bitte das Zertifikat erneuern.", "Please renew the certificate."),
            Severity::Info,
        ),
        Event::VersionChanged { from, to, .. } => (
            format!("{} {to}", pick("neue Version", "new version")),
            format!("{}: {}", pick("Vorher", "Before"), if from.is_empty() { "–" } else { from }),
            Severity::Info,
        ),
    };
    Message {
        title: format!("{name}: {title}"),
        body,
        severity,
        link: Some(link.to_owned()),
        event: serde_json::to_value(event).unwrap_or_default(),
    }
}

/// For `POST /api/channels/{id}/test`.
pub fn test_message(state: &AppState, lang: Lang, user: &str) -> Message {
    let de = lang == Lang::De;
    Message {
        title: if de { "UwUStatus: Testnachricht" } else { "UwUStatus: test message" }.into(),
        body: if de {
            format!("Dieser Kanal funktioniert. Ausgelöst von {user}.")
        } else {
            format!("This channel works. Sent by {user}.")
        },
        severity: Severity::Info,
        link: Some(state.config.public_url.clone()),
        event: serde_json::json!({ "type": "test" }),
    }
}
