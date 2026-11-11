//! Maintenance windows: once (start/end) or weekly (weekdays, local time, duration, time zone).
//! A window covers monitors by id or by group; inside it monitors are `maintenance` and silent.

use crate::app::AppState;
use crate::auth::Authed;
use crate::error::{ApiError, ApiResult, Body};
use crate::monitors::MonitorConfig;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Schedule {
    /// Unix seconds, `end` exclusive.
    Once { start: i64, end: i64 },
    Weekly {
        /// 1 = Monday … 7 = Sunday.
        weekdays: Vec<u8>,
        /// Local start time `HH:MM`.
        time: String,
        duration_min: u32,
        /// IANA name, e.g. `Europe/Berlin`.
        timezone: String,
    },
}

const MAX_WEEKLY_MIN: u32 = 7 * 24 * 60;

impl Schedule {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Schedule::Once { start, end } => {
                if end <= start || end - start > 366 * 86400 {
                    return Err("Wartung: Ende nach Beginn, höchstens ein Jahr".into());
                }
            }
            Schedule::Weekly { weekdays, time, duration_min, timezone } => {
                if weekdays.is_empty() || weekdays.iter().any(|d| !(1..=7).contains(d)) {
                    return Err("Wartung: Wochentage 1 (Montag) bis 7 (Sonntag)".into());
                }
                if time.parse::<jiff::civil::Time>().is_err() {
                    return Err("Wartung: Uhrzeit als HH:MM".into());
                }
                if !(1..=MAX_WEEKLY_MIN).contains(duration_min) {
                    return Err("Wartung: Dauer 1 Minute bis 7 Tage".into());
                }
                if jiff::tz::TimeZone::get(timezone).is_err() {
                    return Err("Wartung: unbekannte Zeitzone".into());
                }
            }
        }
        Ok(())
    }

    /// The window (`start`, `end`) that contains `at`, if any.
    pub fn window_at(&self, at: i64) -> Option<(i64, i64)> {
        match self {
            Schedule::Once { start, end } => (*start <= at && at < *end).then_some((*start, *end)),
            Schedule::Weekly { weekdays, time, duration_min, timezone } => {
                let tz = jiff::tz::TimeZone::get(timezone).ok()?;
                let time: jiff::civil::Time = time.parse().ok()?;
                let today = jiff::Timestamp::from_second(at).ok()?.to_zoned(tz.clone()).date();
                let length = i64::from(*duration_min) * 60;
                // A window that started up to `duration` days ago may still be running.
                (0..=i64::from(duration_min.div_ceil(1440))).find_map(|back| {
                    let date = today.checked_sub(jiff::Span::new().days(back)).ok()?;
                    if !weekdays.contains(&(date.weekday().to_monday_one_offset() as u8)) {
                        return None;
                    }
                    // A start inside a DST gap moves forward (jiff's "compatible" rule).
                    let start = date.to_datetime(time).to_zoned(tz.clone()).ok()?.timestamp().as_second();
                    (start <= at && at < start + length).then_some((start, start + length))
                })
            }
        }
    }

    pub fn is_in_maintenance(&self, at: i64) -> bool {
        self.window_at(at).is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowInput {
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub schedule: Schedule,
    #[serde(default)]
    pub monitors: Vec<i64>,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default = "yes")]
    pub active: bool,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize)]
pub struct Window {
    pub id: i64,
    #[serde(flatten)]
    pub input: WindowInput,
}

impl Window {
    pub fn covers(&self, monitor_id: i64, config: &MonitorConfig) -> bool {
        self.input.active
            && (self.input.monitors.contains(&monitor_id)
                || (!config.group.is_empty() && self.input.groups.contains(&config.group)))
    }
}

pub fn all(c: &Connection) -> rusqlite::Result<Vec<Window>> {
    let mut stmt = c.prepare("SELECT id, config FROM maintenance ORDER BY id")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (id, json) = row?;
        match serde_json::from_str(&json) {
            Ok(input) => out.push(Window { id, input }),
            Err(e) => tracing::warn!("maintenance {id}: unreadable: {e}"),
        }
    }
    Ok(out)
}

/// Whether any active window covers this monitor at `at`.
pub fn covers(windows: &[Window], monitor_id: i64, config: &MonitorConfig, at: i64) -> bool {
    windows.iter().any(|w| w.covers(monitor_id, config) && w.input.schedule.is_in_maintenance(at))
}

fn validate(input: &mut WindowInput) -> Result<(), String> {
    input.title = input.title.trim().to_owned();
    if !(1..=100).contains(&input.title.chars().count()) {
        return Err("Titel: 1 bis 100 Zeichen".into());
    }
    if input.description.chars().count() > 2000 {
        return Err("Beschreibung: höchstens 2000 Zeichen".into());
    }
    if input.monitors.len() > 500 || input.groups.len() > 50 || input.groups.iter().any(|g| g.chars().count() > 100) {
        return Err("Wartung: zu viele Monitore oder Gruppen".into());
    }
    if input.monitors.is_empty() && input.groups.is_empty() {
        return Err("Wartung: mindestens ein Monitor oder eine Gruppe".into());
    }
    input.schedule.validate()
}

pub async fn list(State(state): State<AppState>, _: Authed) -> ApiResult<Json<Vec<Window>>> {
    Ok(Json(state.db.with(all)?))
}

pub async fn create(
    State(state): State<AppState>,
    user: Authed,
    Body(mut input): Body<WindowInput>,
) -> ApiResult<(StatusCode, Json<Window>)> {
    validate(&mut input).map_err(ApiError::invalid)?;
    let json = serde_json::to_string(&input).map_err(ApiError::internal)?;
    let id = state.db.with(|c| {
        c.execute("INSERT INTO maintenance (config) VALUES (?1)", [json])?;
        Ok(c.last_insert_rowid())
    })?;
    tracing::info!("maintenance {id} created by {}", user.actor());
    crate::ingest::tick_states(&state);
    Ok((StatusCode::CREATED, Json(Window { id, input })))
}

pub async fn update(
    State(state): State<AppState>,
    user: Authed,
    Path(id): Path<i64>,
    Body(mut input): Body<WindowInput>,
) -> ApiResult<Json<Window>> {
    validate(&mut input).map_err(ApiError::invalid)?;
    let json = serde_json::to_string(&input).map_err(ApiError::internal)?;
    let n = state.db.with(|c| c.execute("UPDATE maintenance SET config = ?2 WHERE id = ?1", params![id, json]))?;
    if n == 0 {
        return Err(ApiError::not_found("Wartung nicht gefunden."));
    }
    tracing::info!("maintenance {id} changed by {}", user.actor());
    crate::ingest::tick_states(&state);
    Ok(Json(Window { id, input }))
}

pub async fn delete(State(state): State<AppState>, user: Authed, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let n = state.db.with(|c| c.execute("DELETE FROM maintenance WHERE id = ?1", [id]))?;
    if n == 0 {
        return Err(ApiError::not_found("Wartung nicht gefunden."));
    }
    tracing::info!("maintenance {id} deleted by {}", user.actor());
    crate::ingest::tick_states(&state);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get(State(state): State<AppState>, _: Authed, Path(id): Path<i64>) -> ApiResult<Json<Window>> {
    let json: Option<String> = state
        .db
        .with(|c| c.query_row("SELECT config FROM maintenance WHERE id = ?1", [id], |r| r.get(0)).optional())?;
    let input = json
        .and_then(|j| serde_json::from_str(&j).ok())
        .ok_or_else(|| ApiError::not_found("Wartung nicht gefunden."))?;
    Ok(Json(Window { id, input }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> i64 {
        s.parse::<jiff::Timestamp>().unwrap().as_second()
    }

    fn weekly(days: &[u8], time: &str, minutes: u32) -> Schedule {
        Schedule::Weekly {
            weekdays: days.to_vec(),
            time: time.into(),
            duration_min: minutes,
            timezone: "Europe/Berlin".into(),
        }
    }

    #[test]
    fn once() {
        let s = Schedule::Once { start: 100, end: 200 };
        assert!(s.validate().is_ok());
        assert!(!s.is_in_maintenance(99));
        assert!(s.is_in_maintenance(100));
        assert!(!s.is_in_maintenance(200));
        assert!(Schedule::Once { start: 200, end: 200 }.validate().is_err());
    }

    #[test]
    fn weekly_follows_local_time_across_dst() {
        // Monday 03:00 Berlin = 02:00 UTC in winter, 01:00 UTC in summer.
        let s = weekly(&[1], "03:00", 60);
        assert!(s.is_in_maintenance(ts("2026-01-05T02:30:00Z")));
        assert!(!s.is_in_maintenance(ts("2026-01-05T01:30:00Z")));
        assert!(s.is_in_maintenance(ts("2026-07-06T01:30:00Z")));
        assert!(!s.is_in_maintenance(ts("2026-07-06T02:30:00Z")));
        // Tuesday is not listed.
        assert!(!s.is_in_maintenance(ts("2026-01-06T02:30:00Z")));
        assert_eq!(
            s.window_at(ts("2026-01-05T02:59:59Z")),
            Some((ts("2026-01-05T02:00:00Z"), ts("2026-01-05T03:00:00Z")))
        );
    }

    #[test]
    fn weekly_in_dst_gap_and_fold() {
        // 29.03.2026 (Sunday): 02:30 does not exist, the window starts at 03:30 CEST = 01:30 UTC.
        let gap = weekly(&[7], "02:30", 60);
        assert_eq!(
            gap.window_at(ts("2026-03-29T01:45:00Z")),
            Some((ts("2026-03-29T01:30:00Z"), ts("2026-03-29T02:30:00Z")))
        );
        assert!(!gap.is_in_maintenance(ts("2026-03-29T01:15:00Z")));
        // 25.10.2026: 02:30 happens twice, the first one (CEST, 00:30 UTC) counts; length stays 60 min.
        let fold = weekly(&[7], "02:30", 60);
        assert_eq!(
            fold.window_at(ts("2026-10-25T01:00:00Z")),
            Some((ts("2026-10-25T00:30:00Z"), ts("2026-10-25T01:30:00Z")))
        );
        assert!(!fold.is_in_maintenance(ts("2026-10-25T01:45:00Z")));
    }

    #[test]
    fn weekly_over_midnight_and_validation() {
        // Sunday 23:00 for 3 h reaches into Monday.
        let s = weekly(&[7], "23:00", 180);
        assert!(s.is_in_maintenance(ts("2026-01-05T00:30:00Z")));
        assert!(!s.is_in_maintenance(ts("2026-01-05T01:30:00Z")));
        assert!(s.validate().is_ok());
        assert!(weekly(&[0], "23:00", 10).validate().is_err());
        assert!(weekly(&[1], "25:00", 10).validate().is_err());
        assert!(weekly(&[1], "23:00", 0).validate().is_err());
        let bad_tz =
            Schedule::Weekly { weekdays: vec![1], time: "01:00".into(), duration_min: 5, timezone: "Mars/Base".into() };
        assert!(bad_tz.validate().is_err());
    }
}
