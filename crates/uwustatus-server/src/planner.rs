//! The local scheduler: one tokio task that runs each monitor on its own grid
//! (`phase + k * interval`, so monitors spread out and never drift), limits parallel checks and
//! watches push deadlines. Edits reach it through [`Msg::Reload`] right away.

use crate::app::AppState;
use crate::engine;
use crate::ingest;
use crate::monitors::{self, Kind, LOCAL, Monitor};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Semaphore, mpsc};
use uwustatus_probe::{Ctx, Outcome, Status};

pub enum Msg {
    /// Monitors changed: re-read them.
    Reload,
    /// Next run of `id` (a finished check, or a push that moved the deadline).
    Due { id: i64, at: i64 },
}

pub const MAX_PARALLEL: usize = 32;
/// States, reminders, rollups and retention run this often (seconds).
pub const TICK: i64 = 60;
/// A run is in flight; its task sends the next `Due`.
const BUSY: i64 = i64::MAX;

struct Entry {
    monitor: Monitor,
    due: i64,
}

/// Starts the planner (once; later calls return `Ok(false)`).
pub fn spawn(state: &AppState) -> Result<bool, String> {
    let Some(rx) = state.planner_rx.lock().unwrap_or_else(|e| e.into_inner()).take() else { return Ok(false) };
    let ctx = Arc::new(Ctx::new().map_err(|e| format!("Prüf-Kontext: {e}"))?);
    tokio::spawn(run(state.clone(), rx, ctx));
    Ok(true)
}

fn interval(m: &Monitor) -> i64 {
    i64::from(m.config.interval_s.max(1))
}

fn push_deadline(m: &Monitor, now: i64) -> i64 {
    now + interval(m) + i64::from(m.config.timeout_s)
}

/// Brings `entries` in line with the stored monitors. Unchanged ones keep their slot, new or
/// edited checks run now (at startup: on their grid unless overdue), push monitors get a fresh deadline.
fn reload(state: &AppState, entries: &mut HashMap<i64, Entry>, now: i64, startup: bool) {
    let loaded = state.db.with(|c| {
        let mut stmt = c.prepare("SELECT monitor_id, last_at FROM monitor_state WHERE last_at IS NOT NULL")?;
        let last: HashMap<i64, i64> =
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        Ok((monitors::all(c)?, last))
    });
    let (monitors, last) = match loaded {
        Ok(m) => m,
        Err(e) => return tracing::error!("planner: {e}"),
    };
    let mut next = HashMap::new();
    for monitor in monitors.into_iter().filter(|m| m.config.runs_locally()) {
        let due = match entries.remove(&monitor.id) {
            Some(old) if old.monitor.config == monitor.config || old.due == BUSY => old.due,
            _ => match monitor.config.kind {
                Kind::Push => push_deadline(&monitor, now),
                Kind::Check if startup && last.get(&monitor.id).is_some_and(|at| at + interval(&monitor) > now) => {
                    engine::next_slot(now, interval(&monitor), engine::phase(monitor.id, interval(&monitor)))
                }
                Kind::Check => now,
            },
        };
        next.insert(monitor.id, Entry { monitor, due });
    }
    *entries = next;
}

async fn run(state: AppState, mut rx: mpsc::UnboundedReceiver<Msg>, ctx: Arc<Ctx>) {
    let permits = Arc::new(Semaphore::new(MAX_PARALLEL));
    let mut entries = HashMap::new();
    reload(&state, &mut entries, state.clock.now(), true);
    let mut next_tick = state.clock.now();
    loop {
        let now = state.clock.now();
        if now >= next_tick {
            ingest::tick(&state);
            next_tick = now + TICK;
        }
        for entry in entries.values_mut().filter(|e| e.due <= now) {
            fire(&state, &entry.monitor, now, &permits, &ctx);
            entry.due = BUSY;
        }
        let wake = entries.values().map(|e| e.due).min().unwrap_or(BUSY).min(next_tick);
        // Whole seconds, at least one: up to a second late, never early (slots are on the grid).
        let wait = Duration::from_secs(wake.saturating_sub(now).max(1) as u64);
        tokio::select! {
            msg = rx.recv() => match msg {
                None => return,
                Some(Msg::Reload) => reload(&state, &mut entries, state.clock.now(), false),
                Some(Msg::Due { id, at }) => {
                    if let Some(entry) = entries.get_mut(&id) {
                        entry.due = at;
                    }
                }
            },
            () = tokio::time::sleep(wait) => {}
        }
    }
}

fn fire(state: &AppState, monitor: &Monitor, now: i64, permits: &Arc<Semaphore>, ctx: &Arc<Ctx>) {
    let (state, monitor, permits, ctx) = (state.clone(), monitor.clone(), permits.clone(), ctx.clone());
    let id = monitor.id;
    let retry = i64::from(monitor.config.retry_interval_s.max(1));
    match (monitor.config.kind, monitor.config.check) {
        (Kind::Push, _) => {
            // The deadline passed without a push.
            let outcome = Outcome {
                status: Status::Down,
                latency_ms: 0,
                message: "Kein Push erhalten".into(),
                details: Default::default(),
            };
            let retrying = ingest::ingest_result(&state, id, LOCAL, None, now, &outcome).is_ok_and(|i| i.retrying);
            let interval = i64::from(monitor.config.interval_s.max(1));
            state.plan(id, now + if retrying { retry } else { interval });
        }
        (Kind::Check, Some(check)) => {
            let interval = i64::from(monitor.config.interval_s.max(1));
            tokio::spawn(async move {
                let Ok(_permit) = permits.acquire_owned().await else { return };
                let started = state.clock.now();
                let outcome = uwustatus_probe::run(&check, &ctx).await;
                let next = match ingest::ingest_result(&state, id, LOCAL, None, started, &outcome) {
                    Ok(i) if i.retrying => started + retry,
                    Ok(_) => engine::next_slot(started, interval, engine::phase(id, interval)),
                    Err(e) => {
                        tracing::error!("monitor {id}: {e}");
                        engine::next_slot(started, interval, engine::phase(id, interval))
                    }
                };
                state.plan(id, next);
            });
        }
        // validate() never lets this through.
        (Kind::Check, None) => state.plan(id, now + i64::from(monitor.config.interval_s.max(1))),
    }
}
