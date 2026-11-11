//! Results in, state out: [`ingest_result`] is shared by the local planner, push and (Phase 6)
//! satellites. It stores the result, steps the location, re-evaluates the monitor and emits events.

use crate::app::AppState;
use crate::engine::{self, Inputs, Loc, LocInput, State as MonState};
use crate::events::{self, Event};
use crate::maintenance::{self, Window};
use crate::monitors::{self, Monitor};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;
use uwustatus_probe::Outcome;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ingested {
    /// `(location, seq)` was seen before; nothing changed (ack it again).
    pub duplicate: bool,
    /// The location is confirming a failure: check again after the retry interval.
    pub retrying: bool,
}

/// `location` is `local` or a satellite id; `seq` is the satellite's sequence number (dedup).
/// Results older than the location's latest are stored but do not move its state.
pub fn ingest_result(
    state: &AppState,
    monitor_id: i64,
    location: &str,
    seq: Option<i64>,
    at: i64,
    outcome: &Outcome,
) -> rusqlite::Result<Ingested> {
    let mut out = Vec::new();
    let result = state.db.with(|c| {
        let tx = c.unchecked_transaction()?;
        let ingested = ingest_in(&tx, monitor_id, location, seq, at, outcome, &mut out)?;
        tx.commit()?;
        Ok(ingested)
    })?;
    events::send(state, out);
    Ok(result)
}

fn ingest_in(
    c: &Connection,
    monitor_id: i64,
    location: &str,
    seq: Option<i64>,
    at: i64,
    outcome: &Outcome,
    out: &mut Vec<Event>,
) -> rusqlite::Result<Ingested> {
    let not_moved = Ingested { duplicate: false, retrying: false };
    // A satellite may still report a monitor that was just deleted.
    let Some(monitor) = monitors::one(c, monitor_id)? else { return Ok(not_moved) };
    let details = serde_json::to_string(&outcome.details).unwrap_or_else(|_| "{}".into());
    let inserted = c.execute(
        "INSERT INTO results (monitor_id, location, seq, at, status, latency_ms, message, details)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ON CONFLICT (location, seq) DO NOTHING",
        params![
            monitor_id,
            location,
            seq,
            at,
            engine::status_str(outcome.status),
            i64::try_from(outcome.latency_ms).unwrap_or(i64::MAX),
            outcome.message,
            details
        ],
    )?;
    if inserted == 0 {
        return Ok(Ingested { duplicate: true, retrying: false });
    }
    let prev: Option<(String, u32, i64)> = c
        .query_row(
            "SELECT state, failures, at FROM location_state WHERE monitor_id = ?1 AND location = ?2",
            params![monitor_id, location],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let prev_loc = prev.as_ref().map_or(Loc::default(), |(s, f, _)| Loc { state: MonState::parse(s), failures: *f });
    if prev.as_ref().is_some_and(|(_, _, last)| at < *last) {
        return Ok(Ingested { duplicate: false, retrying: engine::retrying(prev_loc) });
    }
    let config = &monitor.config;
    let loc = engine::step(prev_loc, outcome.status, config.retries);
    c.execute(
        "INSERT INTO location_state (monitor_id, location, state, failures, at, message) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (monitor_id, location) DO UPDATE SET state = excluded.state, failures = excluded.failures,
         at = excluded.at, message = excluded.message",
        params![monitor_id, location, loc.state.as_str(), loc.failures, at, outcome.message],
    )?;

    ensure_state(c, monitor_id, at)?;
    let (old_version, cert_warned): (Option<String>, bool) =
        c.query_row("SELECT version, cert_warned FROM monitor_state WHERE monitor_id = ?1", [monitor_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    if let (Some(old), Some(new)) = (&old_version, &outcome.details.version)
        && old != new
    {
        events::store(c, monitor_id, at, "version", old, new, "")?;
        out.push(Event::VersionChanged { monitor_id, at, from: old.clone(), to: new.clone() });
    }
    let days = outcome.details.cert_days_left;
    let warn = i64::from(config.cert_warn_days);
    let warning = days.is_some_and(|d| warn > 0 && d <= warn);
    if let Some(d) = days.filter(|_| warning && !cert_warned) {
        events::store(c, monitor_id, at, "cert", "", &d.to_string(), "")?;
        out.push(Event::CertExpiring { monitor_id, at, days_left: d });
    }
    c.execute(
        "UPDATE monitor_state SET last_at = ?2, latency_ms = ?3, message = ?4, version = COALESCE(?5, version),
         cert_days_left = ?6, cert_warned = ?7 WHERE monitor_id = ?1",
        params![
            monitor_id,
            at,
            i64::try_from(outcome.latency_ms).unwrap_or(i64::MAX),
            outcome.message,
            outcome.details.version,
            days,
            warning
        ],
    )?;
    let windows = maintenance::all(c)?;
    reevaluate_in(c, &monitor, at, &windows, out, 0)?;
    Ok(Ingested { duplicate: false, retrying: engine::retrying(loc) })
}

fn ensure_state(c: &Connection, monitor_id: i64, at: i64) -> rusqlite::Result<()> {
    c.execute(
        "INSERT OR IGNORE INTO monitor_state (monitor_id, state, since) VALUES (?1, 'pending', ?2)",
        params![monitor_id, at],
    )?;
    Ok(())
}

/// Recomputes the overall state from the stored location states; a change is stored, emitted and
/// passed on to the children (parent down ⇒ child `unreachable_via_parent`).
fn reevaluate_in(
    c: &Connection,
    monitor: &Monitor,
    at: i64,
    windows: &[Window],
    out: &mut Vec<Event>,
    depth: u32,
) -> rusqlite::Result<()> {
    let id = monitor.id;
    let config = &monitor.config;
    ensure_state(c, id, at)?;
    let mut stmt =
        c.prepare("SELECT location, state, failures, message, at FROM location_state WHERE monitor_id = ?1")?;
    let rows: HashMap<String, (Loc, String, i64)> = stmt
        .query_map([id], |r| {
            let loc = Loc { state: MonState::parse(&r.get::<_, String>(1)?), failures: r.get(2)? };
            Ok((r.get(0)?, (loc, r.get(3)?, r.get(4)?)))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let locations: Vec<LocInput> = config
        .locations
        .iter()
        // Satellite locations gone quiet are `unknown`, not part of the quorum.
        .filter(|name| {
            *name == monitors::LOCAL
                || rows.get(*name).is_none_or(|(_, _, last)| !engine::stale(*last, at, config.interval_s))
        })
        .map(|name| {
            let (loc, message) = rows.get(name).map_or((Loc::default(), ""), |(l, m, _)| (*l, m.as_str()));
            LocInput { name, loc, message }
        })
        .collect();
    let parent_down = match config.parent_id {
        Some(parent) => c
            .query_row("SELECT state FROM monitor_state WHERE monitor_id = ?1", [parent], |r| r.get::<_, String>(0))
            .optional()?
            .is_some_and(|s| s == "down"),
        None => false,
    };
    let (old_state, old_reason, old_via, cert_days_left): (String, String, bool, Option<i64>) = c.query_row(
        "SELECT state, reason, via_parent, cert_days_left FROM monitor_state WHERE monitor_id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    let old_state = MonState::parse(&old_state);
    let next = engine::overall(&Inputs {
        locations: &locations,
        quorum: config.quorum,
        paused: config.paused,
        in_maintenance: maintenance::covers(windows, id, config, at),
        parent_down,
        cert_days_left,
        cert_warn_days: config.cert_warn_days,
    });
    // The parent came back but the child is still down: that is news now.
    let unmasked = next.state == MonState::Down && old_via && !next.via_parent;
    if next.state != old_state || unmasked {
        let alert = engine::alerts(old_state, old_via, &next, config.muted);
        c.execute(
            "UPDATE monitor_state SET state = ?2, reason = ?3, since = ?4, via_parent = ?5, last_reminder = NULL
             WHERE monitor_id = ?1",
            params![id, next.state.as_str(), next.reason, at, next.via_parent],
        )?;
        events::store(c, id, at, "state", old_state.as_str(), next.state.as_str(), &next.reason)?;
        out.push(Event::StateChanged {
            monitor_id: id,
            at,
            from: old_state,
            to: next.state,
            reason: next.reason.clone(),
            unreachable_via_parent: next.via_parent,
            alert,
        });
        if depth < 20 {
            for child in monitors::all(c)?.iter().filter(|m| m.config.parent_id == Some(id)) {
                reevaluate_in(c, child, at, windows, out, depth + 1)?;
            }
        }
    } else if next.reason != old_reason || next.via_parent != old_via {
        c.execute(
            "UPDATE monitor_state SET reason = ?2, via_parent = ?3 WHERE monitor_id = ?1",
            params![id, next.reason, next.via_parent],
        )?;
    }
    Ok(())
}

/// Re-evaluates one monitor now (after an edit: paused, maintenance, parent …).
pub fn reevaluate(state: &AppState, monitor_id: i64) -> rusqlite::Result<()> {
    let now = state.clock.now();
    let mut out = Vec::new();
    state.db.with(|c| {
        let tx = c.unchecked_transaction()?;
        if let Some(monitor) = monitors::one(&tx, monitor_id)? {
            reevaluate_in(&tx, &monitor, now, &maintenance::all(&tx)?, &mut out, 0)?;
        }
        tx.commit()
    })?;
    events::send(state, out);
    Ok(())
}

/// Every monitor re-evaluated (maintenance windows begin and end without a result) and
/// reminders for monitors that stay down.
pub fn tick_states(state: &AppState) {
    let now = state.clock.now();
    let mut out = Vec::new();
    let result = state.db.with(|c| {
        let tx = c.unchecked_transaction()?;
        let windows = maintenance::all(&tx)?;
        let all = monitors::all(&tx)?;
        for monitor in &all {
            reevaluate_in(&tx, monitor, now, &windows, &mut out, 0)?;
        }
        for monitor in all.iter().filter(|m| m.config.reminder_min > 0 && !m.config.muted) {
            let (s, since, via, reason, last): (String, i64, bool, String, Option<i64>) = tx.query_row(
                "SELECT state, since, via_parent, reason, last_reminder FROM monitor_state WHERE monitor_id = ?1",
                [monitor.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )?;
            if s == "down" && !via && now - last.unwrap_or(since) >= i64::from(monitor.config.reminder_min) * 60 {
                tx.execute(
                    "UPDATE monitor_state SET last_reminder = ?2 WHERE monitor_id = ?1",
                    params![monitor.id, now],
                )?;
                out.push(Event::Reminder { monitor_id: monitor.id, at: now, since, reason });
            }
        }
        tx.commit()
    });
    if let Err(e) = result {
        tracing::error!("state tick: {e}");
    }
    events::send(state, out);
}

/// The planner's minute job: states, reminders, rollups and retention.
pub fn tick(state: &AppState) {
    tick_states(state);
    let settings = crate::settings::load(&state.db);
    if let Err(e) = state.db.with(|c| crate::history::maintain(c, state.clock.now(), &settings)) {
        tracing::error!("rollup: {e}");
    }
}
