//! Events: what the core tells alerts (Phase 4) and status pages (Phase 5). State changes,
//! version changes and certificate warnings are also stored in `events`; reminders only go
//! over the bus.

use crate::app::AppState;
use crate::auth::Authed;
use crate::engine::State as MonState;
use crate::error::{ApiError, ApiResult};
use axum::Json;
use axum::extract::{Path, Query, State};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    StateChanged {
        monitor_id: i64,
        at: i64,
        from: MonState,
        to: MonState,
        reason: String,
        /// Down because the parent is down.
        unreachable_via_parent: bool,
        /// Send it to the channels: not muted, not via parent, not maintenance/paused/pending,
        /// not a recovery from a state that never alerted.
        alert: bool,
    },
    /// Still down after `reminder_min` (only when the monitor alerts at all).
    Reminder { monitor_id: i64, at: i64, since: i64, reason: String },
    /// The certificate entered the warning window (once until it is renewed).
    CertExpiring { monitor_id: i64, at: i64, days_left: i64 },
    /// `details.version` changed (healthz).
    VersionChanged { monitor_id: i64, at: i64, from: String, to: String },
}

/// Bus capacity; a subscriber that lags behind further loses the oldest events.
pub const BUS: usize = 1024;

pub fn send(state: &AppState, events: Vec<Event>) {
    for event in events {
        // No subscriber yet (Phase 4/5 not running) is fine.
        let _ = state.events.send(event);
    }
}

/// Appends to the `events` table.
pub fn store(
    c: &Connection,
    monitor_id: i64,
    at: i64,
    kind: &str,
    from: &str,
    to: &str,
    reason: &str,
) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO events (monitor_id, at, kind, from_value, to_value, reason) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![monitor_id, at, kind, from, to, reason],
    )?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct Row {
    pub id: i64,
    pub at: i64,
    /// `state`, `version` or `cert`.
    pub kind: String,
    pub from: String,
    pub to: String,
    pub reason: String,
}

#[derive(Deserialize)]
pub struct Page {
    before: Option<i64>,
    limit: Option<u32>,
}

/// `GET /api/monitors/{id}/events?before=<event id>&limit=`, newest first.
pub async fn list(
    State(state): State<AppState>,
    _: Authed,
    Path(id): Path<i64>,
    Query(page): Query<Page>,
) -> ApiResult<Json<Vec<Row>>> {
    let limit = page.limit.unwrap_or(100).clamp(1, 500);
    let rows = state.db.with(|c| {
        if crate::monitors::one(c, id)?.is_none() {
            return Ok(None);
        }
        let mut stmt = c.prepare(
            "SELECT id, at, kind, from_value, to_value, reason FROM events
             WHERE monitor_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![id, page.before.unwrap_or(i64::MAX), limit], |r| {
            Ok(Row { id: r.get(0)?, at: r.get(1)?, kind: r.get(2)?, from: r.get(3)?, to: r.get(4)?, reason: r.get(5)? })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map(Some)
    })?;
    rows.map(Json).ok_or_else(|| ApiError::not_found("Monitor nicht gefunden."))
}
