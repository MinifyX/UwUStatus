//! Rollups (hour and day aggregates), retention, uptime and the read API (overview, history).

use crate::app::AppState;
use crate::auth::Authed;
use crate::engine::{self, State as MonState};
use crate::error::{ApiError, ApiResult};
use crate::settings::Settings;
use axum::Json;
use axum::extract::{Path, Query, State};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uwustatus_probe::Status;

const HOUR: i64 = 3600;
const DAY: i64 = 86400;
/// Hours rolled up per tick, so a long downtime is caught up without holding the lock for long.
const CATCH_UP: u32 = 48;
const MARKER: &str = "rollup_next";

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Agg {
    pub up: u64,
    pub degraded: u64,
    pub down: u64,
    /// Latency only over up/degraded results (a timeout says nothing about speed).
    pub latency_avg: Option<f64>,
    pub latency_p95: Option<u64>,
}

impl Agg {
    /// Percent of results that were up or degraded.
    pub fn uptime(&self) -> Option<f64> {
        let total = self.up + self.degraded + self.down;
        (total > 0).then(|| (self.up + self.degraded) as f64 * 100.0 / total as f64)
    }
}

pub fn summarize(results: &[(Status, u64)]) -> Agg {
    let mut agg = Agg::default();
    let mut latencies: Vec<u64> = Vec::new();
    for &(status, latency) in results {
        match status {
            Status::Up => agg.up += 1,
            Status::Degraded => agg.degraded += 1,
            Status::Down => agg.down += 1,
        }
        if status != Status::Down {
            latencies.push(latency);
        }
    }
    if !latencies.is_empty() {
        latencies.sort_unstable();
        agg.latency_avg = Some(latencies.iter().sum::<u64>() as f64 / latencies.len() as f64);
        // Nearest rank.
        let rank = (latencies.len() * 95).div_ceil(100).max(1);
        agg.latency_p95 = Some(latencies[rank - 1]);
    }
    agg
}

fn roll(c: &Connection, period: &str, start: i64, end: i64) -> rusqlite::Result<()> {
    let mut stmt = c.prepare("SELECT monitor_id, status, latency_ms FROM results WHERE at >= ?1 AND at < ?2")?;
    let mut by_monitor: BTreeMap<i64, Vec<(Status, u64)>> = BTreeMap::new();
    for row in
        stmt.query_map(params![start, end], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?
    {
        let (id, status, latency) = row?;
        if let Some(status) = engine::parse_status(&status) {
            by_monitor.entry(id).or_default().push((status, latency.max(0) as u64));
        }
    }
    for (id, results) in by_monitor {
        let a = summarize(&results);
        c.execute(
            "INSERT OR REPLACE INTO rollups (monitor_id, period, start, up, degraded, down, latency_avg, latency_p95, uptime)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![id, period, start, a.up as i64, a.degraded as i64, a.down as i64, a.latency_avg, a.latency_p95.map(|l| l as i64), a.uptime()],
        )?;
    }
    Ok(())
}

/// Rolls up every finished hour (and, at UTC midnight, the day) since the last run, then deletes
/// what is past retention. Idempotent; the progress lives in the `settings` table.
// shortcut: days are UTC days; per-page time zones for daily bars when status pages need them.
pub fn maintain(c: &Connection, now: i64, settings: &Settings) -> rusqlite::Result<()> {
    let tx = c.unchecked_transaction()?;
    let marker: Option<String> =
        tx.query_row("SELECT value FROM settings WHERE key = ?1", [MARKER], |r| r.get(0)).optional()?;
    let oldest_raw = now - i64::from(settings.retention_raw_days) * DAY;
    let mut next = match marker.and_then(|m| m.parse::<i64>().ok()) {
        Some(m) => m.max(oldest_raw.div_euclid(HOUR) * HOUR),
        None => {
            let first: Option<i64> = tx.query_row("SELECT MIN(at) FROM results", [], |r| r.get(0))?;
            first.unwrap_or(now).div_euclid(HOUR) * HOUR
        }
    };
    for _ in 0..CATCH_UP {
        if next + HOUR > now {
            break;
        }
        roll(&tx, "hour", next, next + HOUR)?;
        next += HOUR;
        if next.rem_euclid(DAY) == 0 {
            roll(&tx, "day", next - DAY, next)?;
        }
    }
    tx.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![MARKER, next.to_string()],
    )?;
    tx.execute("DELETE FROM results WHERE at < ?1", [oldest_raw])?;
    tx.execute(
        "DELETE FROM rollups WHERE period = 'hour' AND start < ?1",
        [now - i64::from(settings.retention_hourly_days) * DAY],
    )?;
    let oldest_daily = now - i64::from(settings.retention_daily_days) * DAY;
    tx.execute("DELETE FROM rollups WHERE period = 'day' AND start < ?1", [oldest_daily])?;
    tx.execute("DELETE FROM events WHERE at < ?1", [oldest_daily])?;
    tx.commit()
}

/// Time-weighted uptime in percent over `[from, to)` from the stored state changes; `None` when
/// the monitor was never up or down in that range.
pub fn uptime_in(c: &Connection, monitor_id: i64, from: i64, to: i64) -> rusqlite::Result<Option<f64>> {
    let initial: Option<String> = c
        .query_row(
            "SELECT to_value FROM events WHERE monitor_id = ?1 AND kind = 'state' AND at <= ?2 ORDER BY at DESC, id DESC LIMIT 1",
            params![monitor_id, from],
            |r| r.get(0),
        )
        .optional()?;
    let mut stmt = c.prepare(
        "SELECT at, to_value FROM events WHERE monitor_id = ?1 AND kind = 'state' AND at > ?2 AND at < ?3 ORDER BY at, id",
    )?;
    let changes = stmt
        .query_map(params![monitor_id, from, to], |r| Ok((r.get(0)?, MonState::parse(&r.get::<_, String>(1)?))))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(engine::uptime(initial.map_or(MonState::Pending, |s| MonState::parse(&s)), &changes, from, to))
}

pub fn uptime(db: &crate::db::Db, monitor_id: i64, from: i64, to: i64) -> rusqlite::Result<Option<f64>> {
    db.with(|c| uptime_in(c, monitor_id, from, to))
}

/// `24h`, `7d`, … (1 hour to 10 years) in seconds.
fn parse_range(s: &str) -> Option<i64> {
    let (n, unit) = match (s.strip_suffix('h'), s.strip_suffix('d')) {
        (Some(n), _) => (n, HOUR),
        (_, Some(n)) => (n, DAY),
        _ => return None,
    };
    let seconds = n.parse::<i64>().ok().filter(|n| *n > 0)?.checked_mul(unit)?;
    (seconds <= 3650 * DAY).then_some(seconds)
}

#[derive(Deserialize)]
pub struct RangeQuery {
    range: Option<String>,
}

#[derive(Serialize)]
pub struct History {
    /// `raw`, `hour` or `day`.
    level: &'static str,
    from: i64,
    to: i64,
    uptime: Option<f64>,
    points: Vec<Value>,
}

/// `GET /api/monitors/{id}/history?range=24h`: raw results for short ranges, hour or day
/// aggregates for longer ones (whatever retention still has).
pub async fn history(
    State(state): State<AppState>,
    _: Authed,
    Path(id): Path<i64>,
    Query(q): Query<RangeQuery>,
) -> ApiResult<Json<History>> {
    let span = parse_range(q.range.as_deref().unwrap_or("24h"))
        .ok_or_else(|| ApiError::invalid("range: z. B. 24h, 7d, 90d"))?;
    let settings = crate::settings::load(&state.db);
    let to = state.clock.now();
    let from = to - span;
    let level = if span <= 2 * DAY && span <= i64::from(settings.retention_raw_days) * DAY {
        "raw"
    } else if span <= 31 * DAY && span <= i64::from(settings.retention_hourly_days) * DAY {
        "hour"
    } else {
        "day"
    };
    let found = state.db.with(|c| {
        if crate::monitors::one(c, id)?.is_none() {
            return Ok(None);
        }
        let points = if level == "raw" {
            let mut stmt = c.prepare(
                "SELECT at, location, status, latency_ms, message FROM results WHERE monitor_id = ?1 AND at >= ?2 ORDER BY at, id",
            )?;
            stmt.query_map(params![id, from], |r| {
                Ok(json!({
                    "at": r.get::<_, i64>(0)?, "location": r.get::<_, String>(1)?, "status": r.get::<_, String>(2)?,
                    "latency_ms": r.get::<_, i64>(3)?, "message": r.get::<_, String>(4)?,
                }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let mut stmt = c.prepare(
                "SELECT start, up, degraded, down, latency_avg, latency_p95, uptime FROM rollups
                 WHERE monitor_id = ?1 AND period = ?2 AND start >= ?3 ORDER BY start",
            )?;
            stmt.query_map(params![id, level, from], |r| {
                Ok(json!({
                    "start": r.get::<_, i64>(0)?, "up": r.get::<_, i64>(1)?, "degraded": r.get::<_, i64>(2)?,
                    "down": r.get::<_, i64>(3)?, "latency_avg": r.get::<_, Option<f64>>(4)?,
                    "latency_p95": r.get::<_, Option<i64>>(5)?, "uptime": r.get::<_, Option<f64>>(6)?,
                }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok(Some((points, uptime_in(c, id, from, to)?)))
    })?;
    let (points, uptime) = found.ok_or_else(|| ApiError::not_found("Monitor nicht gefunden."))?;
    Ok(Json(History { level, from, to, uptime, points }))
}

#[derive(Serialize)]
pub struct OverviewItem {
    id: i64,
    name: String,
    group: String,
    tags: Vec<String>,
    kind: crate::monitors::Kind,
    paused: bool,
    muted: bool,
    parent_id: Option<i64>,
    state: MonState,
    reason: String,
    since: Option<i64>,
    unreachable_via_parent: bool,
    last_check: Option<i64>,
    latency_ms: Option<i64>,
    message: Option<String>,
    uptime_24h: Option<f64>,
    uptime_30d: Option<f64>,
    version: Option<String>,
    cert_days_left: Option<i64>,
    /// Last results, oldest first, for the heartbeat bar.
    /// shortcut: mixes all locations, split per location if the bar gets confusing.
    recent: Vec<Beat>,
    locations: Vec<LocationItem>,
}

#[derive(Serialize)]
pub struct Beat {
    at: i64,
    status: String,
}

#[derive(Serialize)]
pub struct LocationItem {
    location: String,
    state: String,
    failures: i64,
    at: i64,
    message: String,
}

const RECENT: i64 = 30;

/// `GET /api/overview`: every monitor with its current state and uptime.
pub async fn overview(State(state): State<AppState>, _: Authed) -> ApiResult<Json<Vec<OverviewItem>>> {
    let now = state.clock.now();
    let items = state.db.with(|c| {
        let mut out = Vec::new();
        for m in crate::monitors::all(c)? {
            type Row = (String, String, i64, bool, Option<i64>, Option<i64>, Option<String>, Option<String>, Option<i64>);
            let row: Option<Row> = c
                .query_row(
                    "SELECT state, reason, since, via_parent, last_at, latency_ms, message, version, cert_days_left
                     FROM monitor_state WHERE monitor_id = ?1",
                    [m.id],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get(6)?,
                            r.get(7)?,
                            r.get(8)?,
                        ))
                    },
                )
                .optional()?;
            let (s, reason, since, via, last_check, latency_ms, message, version, cert_days_left) = match row {
                Some((s, reason, since, via, last, latency, message, version, cert)) => {
                    (MonState::parse(&s), reason, Some(since), via, last, latency, message, version, cert)
                }
                None => (MonState::Pending, String::new(), None, false, None, None, None, None, None),
            };
            let mut recent = c
                .prepare_cached("SELECT at, status FROM results WHERE monitor_id = ?1 ORDER BY at DESC, id DESC LIMIT ?2")?
                .query_map([m.id, RECENT], |r| Ok(Beat { at: r.get(0)?, status: r.get(1)? }))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            recent.reverse();
            let locations = c
                .prepare_cached(
                    "SELECT location, state, failures, at, message FROM location_state WHERE monitor_id = ?1 ORDER BY location",
                )?
                .query_map([m.id], |r| {
                    Ok(LocationItem {
                        location: r.get(0)?,
                        state: r.get(1)?,
                        failures: r.get(2)?,
                        at: r.get(3)?,
                        message: r.get(4)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            out.push(OverviewItem {
                id: m.id,
                uptime_24h: uptime_in(c, m.id, now - DAY, now)?,
                uptime_30d: uptime_in(c, m.id, now - 30 * DAY, now)?,
                name: m.config.name,
                group: m.config.group,
                tags: m.config.tags,
                kind: m.config.kind,
                paused: m.config.paused,
                muted: m.config.muted,
                parent_id: m.config.parent_id,
                state: s,
                reason,
                since,
                unreachable_via_parent: via,
                last_check,
                latency_ms,
                message,
                version,
                cert_days_left,
                recent,
                locations,
            });
        }
        Ok(out)
    })?;
    Ok(Json(items))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_counts_and_latency() {
        let a = summarize(&[(Status::Up, 10), (Status::Up, 30), (Status::Degraded, 20), (Status::Down, 9000)]);
        assert_eq!((a.up, a.degraded, a.down), (2, 1, 1));
        assert_eq!(a.latency_avg, Some(20.0));
        assert_eq!(a.latency_p95, Some(30));
        assert_eq!(a.uptime(), Some(75.0));
        let p = summarize(&(1..=100).map(|l| (Status::Up, l)).collect::<Vec<_>>());
        assert_eq!(p.latency_p95, Some(95));
        assert_eq!(summarize(&[]), Agg::default());
        assert_eq!(summarize(&[(Status::Down, 5)]).latency_avg, None);
    }

    #[test]
    fn ranges() {
        assert_eq!(parse_range("24h"), Some(DAY));
        assert_eq!(parse_range("7d"), Some(7 * DAY));
        for bad in ["", "h", "0d", "-1d", "5w", "99999d", "9999999999999999999h", "1ä", "ä"] {
            assert_eq!(parse_range(bad), None, "{bad}");
        }
    }
}
