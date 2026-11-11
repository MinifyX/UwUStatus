//! `run`: the tunnel listener, the scheduler for the assigned checks, the dead man's switch and
//! the optional emergency page, driven by one event loop that also keeps `state.json` current.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use uwustatus_probe::{Check, Ctx};
use uwustatus_tunnel::{Assignment, SatelliteEvent, SatelliteListener, SatelliteSettings, Timing, Token};

use crate::config::Config;
use crate::deadman::{self, DeadManConfig, DeadManNotifier, Notice};
use crate::state::{Saved, Status, Store};
use crate::{Latest, Live, unix_now};

pub struct Agent {
    pub listener: Arc<SatelliteListener>,
    pub live: Arc<Live>,
    /// Where the emergency page listens, when configured.
    pub page: Option<SocketAddr>,
    tasks: Vec<JoinHandle<()>>,
}

impl Agent {
    pub fn stop(&self) {
        self.listener.close();
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        self.stop();
    }
}

fn unix(at: SystemTime) -> i64 {
    at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

pub async fn start<N: DeadManNotifier>(config: &Config, timing: Timing, notifier: Arc<N>) -> Result<Agent, String> {
    let store = Store::open(&config.state_dir).map_err(|e| format!("{}: {e}", config.state_dir.display()))?;
    let identity = store.identity().map_err(|e| format!("Identität: {e}"))?;
    let saved = store.saved().map_err(|e| format!("Zustand: {e}"))?;
    let mut settings = SatelliteSettings::new(config.listen, identity);
    settings.main = saved.main;
    settings.token = saved.token.as_deref().and_then(Token::from_text);
    settings.buffer_limit = config.buffer_limit;
    settings.timing = timing;
    let (listener, events) = SatelliteListener::bind(settings).map_err(|e| format!("{}: {e}", config.listen))?;
    let listener = Arc::new(listener);
    let live = Arc::new(Live::default());
    {
        let mut l = live.get();
        l.config = saved.dead_man.clone();
        l.since = saved.lost_since;
    }
    let mut tasks = Vec::new();
    let mut page = None;
    if let Some(address) = config.emergency.listen {
        let image = match &config.emergency.image {
            Some(path) => Some(std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?),
            None => None,
        };
        let tcp = tokio::net::TcpListener::bind(address).await.map_err(|e| format!("{address}: {e}"))?;
        page = Some(tcp.local_addr().map_err(|e| e.to_string())?);
        let app = crate::page::router(live.clone(), image);
        tasks.push(tokio::spawn(async move {
            if let Err(e) = axum::serve(tcp, app).await {
                tracing::error!("emergency page: {e}");
            }
        }));
    }
    let ctx = Arc::new(Ctx::new().map_err(|e| format!("Prüf-Kontext: {e}"))?);
    let notices = deadman::sender(notifier);
    let lp =
        Loop { store, saved, listener: listener.clone(), live: live.clone(), ctx, notices, checks: HashMap::new() };
    tasks.push(tokio::spawn(lp.run(events)));
    Ok(Agent { listener, live, page, tasks })
}

struct Loop {
    store: Store,
    saved: Saved,
    listener: Arc<SatelliteListener>,
    live: Arc<Live>,
    ctx: Arc<Ctx>,
    notices: mpsc::UnboundedSender<Notice>,
    checks: HashMap<String, (Assignment, JoinHandle<()>)>,
}

impl Loop {
    async fn run(mut self, mut events: mpsc::UnboundedReceiver<SatelliteEvent>) {
        // Paired but not connected yet: the main instance is away from now (or from before a restart).
        if self.saved.main.is_some() && self.saved.lost_since.is_none() {
            self.saved.lost_since = Some(unix_now());
            self.live.get().since = self.saved.lost_since;
        }
        let mut lost_at = self.saved.lost_since.map(since_instant);
        self.schedule(self.saved.assignments.clone());
        self.save();
        let mut status = tokio::time::interval(Duration::from_secs(30));
        loop {
            let deadline = match (&self.saved.dead_man, lost_at, self.saved.alarmed) {
                (Some(c), Some(at), false) if c.after_s > 0 => Some(at + Duration::from_secs(c.after_s)),
                _ => None,
            };
            tokio::select! {
                event = events.recv() => match event {
                    None => break,
                    Some(event) => {
                        if let Some(at) = self.on(event) {
                            lost_at = at;
                        }
                        self.save();
                    }
                },
                () = sleep_until(deadline) => {
                    tracing::warn!("main instance away past the deadline, alarming");
                    self.saved.alarmed = true;
                    self.notify(true);
                    self.save();
                }
                _ = status.tick() => self.save_status(),
            }
        }
        for (_, (_, task)) in self.checks.drain() {
            task.abort();
        }
    }

    /// Handles one event; `Some` sets when the main instance went away (`None` inside: it is back).
    fn on(&mut self, event: SatelliteEvent) -> Option<Option<Instant>> {
        match event {
            SatelliteEvent::Paired { main } => {
                tracing::info!("paired with main instance {main}");
                self.saved.main = Some(main);
                self.saved.token = None;
            }
            SatelliteEvent::Connected { at, remote } => {
                tracing::info!("main instance connected from {remote}");
                if self.saved.alarmed {
                    self.notify(false);
                }
                self.saved.alarmed = false;
                self.saved.lost_since = None;
                let mut live = self.live.get();
                live.connected = true;
                live.since = Some(unix(at));
                return Some(None);
            }
            SatelliteEvent::Lost { at, error } => {
                tracing::warn!("main instance lost: {error}");
                let mut live = self.live.get();
                live.connected = false;
                live.since = Some(unix(at));
                drop(live);
                if self.saved.lost_since.is_none() {
                    self.saved.lost_since = Some(unix(at));
                    return Some(Some(since_instant(unix(at))));
                }
            }
            SatelliteEvent::Assign(assignments) => {
                tracing::info!("{} checks assigned", assignments.len());
                self.schedule(assignments.clone());
                self.saved.assignments = assignments;
            }
            SatelliteEvent::DeadMan(value) => match serde_json::from_value::<DeadManConfig>(value) {
                Ok(config) => {
                    self.live.get().config = Some(config.clone());
                    self.saved.dead_man = Some(config);
                }
                Err(e) => tracing::warn!("dead man config ignored: {e}"),
            },
        }
        None
    }

    fn notify(&self, alarm: bool) {
        if let (Some(config), Some(since)) = (&self.saved.dead_man, self.saved.lost_since) {
            let _ = self.notices.send(Notice { config: config.clone(), alarm, since });
        }
    }

    /// Unchanged checks keep running on their grid; the rest restart.
    fn schedule(&mut self, assignments: Vec<Assignment>) {
        let mut old = std::mem::take(&mut self.checks);
        for assignment in assignments {
            let entry = match old.remove(&assignment.id) {
                Some(entry) if entry.0 == assignment => entry,
                stale => {
                    if let Some((_, task)) = stale {
                        task.abort();
                    }
                    let task = tokio::spawn(check_loop(
                        assignment.clone(),
                        self.listener.clone(),
                        self.ctx.clone(),
                        self.live.clone(),
                    ));
                    (assignment, task)
                }
            };
            self.checks.insert(entry.0.id.clone(), entry);
        }
        for (id, (_, task)) in old {
            task.abort();
            self.live.get().latest.remove(&id);
        }
    }

    fn save(&self) {
        if let Err(e) = self.store.save(&self.saved) {
            tracing::error!("state.json: {e}");
        }
        self.save_status();
    }

    fn save_status(&self) {
        let live = self.live.get();
        let status = Status {
            connected: live.connected,
            since: live.since,
            buffered: self.listener.buffered(),
            assignments: self.checks.len(),
        };
        drop(live);
        if let Err(e) = self.store.save_status(&status) {
            tracing::error!("status.json: {e}");
        }
    }
}

/// The tokio instant that corresponds to unix second `since` (now or in the past).
fn since_instant(since: i64) -> Instant {
    let ago = Duration::from_secs(unix_now().saturating_sub(since).max(0) as u64);
    Instant::now().checked_sub(ago).unwrap_or_else(Instant::now)
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

async fn check_loop(assignment: Assignment, listener: Arc<SatelliteListener>, ctx: Arc<Ctx>, live: Arc<Live>) {
    let check: Check = match serde_json::from_value(assignment.check.clone()) {
        Ok(check) => check,
        Err(e) => return tracing::warn!(id = %assignment.id, "check not understood: {e}"),
    };
    // shortcut: no retry interval on satellites (the main instance's retries still count); send one along when needed.
    let mut ticks = tokio::time::interval(Duration::from_secs(u64::from(assignment.interval_s.max(5))));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        let at_ms = jiff::Timestamp::now().as_millisecond();
        let outcome = uwustatus_probe::run(&check, &ctx).await;
        live.get().latest.insert(
            assignment.id.clone(),
            Latest { at: at_ms / 1000, status: outcome.status, latency_ms: outcome.latency_ms },
        );
        match serde_json::to_value(&outcome) {
            Ok(value) => {
                listener.push_result(assignment.id.clone(), at_ms, value);
            }
            Err(e) => tracing::error!("result: {e}"),
        }
    }
}
