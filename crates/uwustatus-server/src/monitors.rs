//! Monitors: config (one JSON document per row), validation, CRUD API and the push endpoint.

use crate::app::AppState;
use crate::auth::Authed;
use crate::engine::Quorum;
use crate::error::{ApiError, ApiResult, Body};
use crate::settings::Settings;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uwustatus_probe::{Check, CheckKind, Outcome, Status};

pub const LOCAL: &str = "local";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Active: `check` runs at each location.
    Check,
    /// Passive: the service calls `/api/push/{token}`.
    Push,
}

/// What the admin edits. Later phases add fields with `#[serde(default)]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MonitorConfig {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub kind: Kind,
    /// Required for `check`; its `timeout_ms` is set from `timeout_s`.
    #[serde(default)]
    pub check: Option<Check>,
    /// 0 = default from the settings.
    #[serde(default)]
    pub interval_s: u32,
    /// Check: timeout. Push: grace on top of the interval.
    #[serde(default = "default_timeout")]
    pub timeout_s: u32,
    /// Failed results in a row that are tolerated before down.
    #[serde(default)]
    pub retries: u32,
    /// 0 = same as the interval.
    #[serde(default)]
    pub retry_interval_s: u32,
    /// Remind every X minutes while down, 0 = off.
    #[serde(default)]
    pub reminder_min: u32,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub parent_id: Option<i64>,
    /// `local` and/or satellite ids.
    #[serde(default = "default_locations")]
    pub locations: Vec<String>,
    #[serde(default)]
    pub quorum: Quorum,
    /// Certificate expiring within this many days ⇒ degraded, 0 = off.
    #[serde(default = "default_cert_warn")]
    pub cert_warn_days: u32,
}

fn default_timeout() -> u32 {
    10
}
fn default_locations() -> Vec<String> {
    vec![LOCAL.into()]
}
fn default_cert_warn() -> u32 {
    14
}

impl MonitorConfig {
    pub fn runs_locally(&self) -> bool {
        !self.paused && self.locations.iter().any(|l| l == LOCAL)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Monitor {
    pub id: i64,
    #[serde(flatten)]
    pub config: MonitorConfig,
    /// Push monitors only.
    pub push_token: Option<String>,
    pub created: i64,
    pub updated: i64,
}

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Option<Monitor>> {
    let id: i64 = r.get(0)?;
    let json: String = r.get(1)?;
    Ok(match serde_json::from_str(&json) {
        Ok(config) => Some(Monitor { id, config, push_token: r.get(2)?, created: r.get(3)?, updated: r.get(4)? }),
        Err(e) => {
            tracing::warn!("monitor {id}: unreadable config: {e}");
            None
        }
    })
}

const SELECT: &str = "SELECT id, config, push_token, created, updated FROM monitors";

pub fn all(c: &Connection) -> rusqlite::Result<Vec<Monitor>> {
    let mut stmt = c.prepare(&format!("{SELECT} ORDER BY id"))?;
    let rows = stmt.query_map([], from_row)?;
    rows.filter_map(Result::transpose).collect()
}

pub fn one(c: &Connection, id: i64) -> rusqlite::Result<Option<Monitor>> {
    Ok(c.query_row(&format!("{SELECT} WHERE id = ?1"), [id], from_row).optional()?.flatten())
}

fn by_token(c: &Connection, token: &str) -> rusqlite::Result<Option<Monitor>> {
    Ok(c.query_row(&format!("{SELECT} WHERE push_token = ?1"), [token], from_row).optional()?.flatten())
}

fn chars(s: &str) -> usize {
    s.chars().count()
}

fn host_ok(host: &str) -> bool {
    (1..=253).contains(&host.len()) && !host.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn url_ok(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some())
}

fn validate_check(check: &Check) -> Result<(), String> {
    let ok = match &check.kind {
        CheckKind::Http(c) => url_ok(&c.url),
        CheckKind::Healthz(c) => url_ok(&c.url),
        CheckKind::Docker(c) => url_ok(&c.url) && !c.container.is_empty() && chars(&c.container) <= 200,
        CheckKind::Tcp(c) => host_ok(&c.host) && c.port != 0,
        CheckKind::Ping(c) => host_ok(&c.host),
        CheckKind::Dns(c) => host_ok(&c.name),
        CheckKind::Smtp(c) | CheckKind::Imap(c) => host_ok(&c.host) && c.port != 0,
    };
    if ok { Ok(()) } else { Err("Prüfung: Ziel (URL, Host oder Port) ungültig".into()) }
}

/// Normalises and checks a config; `id` is the monitor being edited (for the parent cycle check).
pub fn validate(c: &Connection, id: Option<i64>, m: &mut MonitorConfig, settings: &Settings) -> Result<(), String> {
    m.name = m.name.trim().to_owned();
    m.group = m.group.trim().to_owned();
    if !(1..=100).contains(&chars(&m.name)) {
        return Err("Name: 1 bis 100 Zeichen".into());
    }
    if chars(&m.description) > 2000 || chars(&m.group) > 100 {
        return Err("Beschreibung höchstens 2000, Gruppe höchstens 100 Zeichen".into());
    }
    m.tags = m.tags.iter().map(|t| t.trim().to_owned()).collect();
    m.tags.sort();
    m.tags.dedup();
    if m.tags.len() > 20 || m.tags.iter().any(|t| !(1..=50).contains(&chars(t))) {
        return Err("Tags: höchstens 20, je 1 bis 50 Zeichen".into());
    }
    if m.interval_s == 0 {
        m.interval_s = settings.default_interval_s;
    }
    if m.retry_interval_s == 0 {
        m.retry_interval_s = m.interval_s;
    }
    if !(10..=86400).contains(&m.interval_s) || !(10..=86400).contains(&m.retry_interval_s) {
        return Err("Intervall und Wiederholungs-Intervall: 10 s bis 1 Tag".into());
    }
    if m.retries > 10 || m.reminder_min > 10080 || m.cert_warn_days > 365 {
        return Err("Wiederholungen höchstens 10, Erinnerung höchstens 7 Tage, Zert-Warnung höchstens 365 Tage".into());
    }
    match (m.kind, m.check.as_mut()) {
        (Kind::Check, Some(check)) => {
            if !(1..=300).contains(&m.timeout_s) || m.timeout_s > m.interval_s {
                return Err("Timeout: 1 bis 300 s und nicht länger als das Intervall".into());
            }
            check.timeout_ms = u64::from(m.timeout_s) * 1000;
            validate_check(check)?;
        }
        (Kind::Check, None) => return Err("Prüfung fehlt".into()),
        (Kind::Push, Some(_)) => return Err("Push-Monitore haben keine Prüfung".into()),
        (Kind::Push, None) => {
            if m.timeout_s > 86400 {
                return Err("Kulanz: höchstens 1 Tag".into());
            }
            if m.locations != [LOCAL] {
                return Err("Push-Monitore laufen nur am Standort local".into());
            }
        }
    }
    let mut seen = m.locations.clone();
    seen.sort();
    seen.dedup();
    if m.locations.is_empty() || m.locations.len() > 20 || seen.len() != m.locations.len() {
        return Err("Standorte: 1 bis 20, ohne Doppelte".into());
    }
    for location in m.locations.iter().filter(|l| *l != LOCAL) {
        let known = c
            .query_row("SELECT 1 FROM satellites WHERE id = ?1", [location], |_| Ok(()))
            .optional()
            .map_err(|e| e.to_string())?;
        if known.is_none() {
            return Err(format!("Standort {location} unbekannt"));
        }
    }
    // Walk up from the new parent; reaching this monitor again would be a cycle.
    let mut next = m.parent_id;
    for _ in 0..=100 {
        let Some(parent) = next else { return Ok(()) };
        if Some(parent) == id {
            return Err("Eltern-Monitor: Zyklus".into());
        }
        next = match one(c, parent).map_err(|e| e.to_string())? {
            Some(p) => p.config.parent_id,
            None if Some(parent) == m.parent_id => return Err("Eltern-Monitor unbekannt".into()),
            None => None,
        };
    }
    Err("Eltern-Monitor: Kette zu lang".into())
}

pub async fn list(State(state): State<AppState>, _: Authed) -> ApiResult<Json<Vec<Monitor>>> {
    Ok(Json(state.db.with(all)?))
}

pub async fn get(State(state): State<AppState>, _: Authed, Path(id): Path<i64>) -> ApiResult<Json<Monitor>> {
    state.db.with(|c| one(c, id))?.map(Json).ok_or_else(not_found)
}

fn not_found() -> ApiError {
    ApiError::not_found("Monitor nicht gefunden.")
}

pub async fn create(
    State(state): State<AppState>,
    user: Authed,
    Body(mut config): Body<MonitorConfig>,
) -> ApiResult<(StatusCode, Json<Monitor>)> {
    let settings = crate::settings::load(&state.db);
    let now = state.clock.now();
    let id = state.db.with(|c| {
        if let Err(message) = validate(c, None, &mut config, &settings) {
            return Ok(Err(message));
        }
        let token = (config.kind == Kind::Push).then(crate::random_token);
        c.execute(
            "INSERT INTO monitors (config, push_token, created, updated) VALUES (?1, ?2, ?3, ?3)",
            params![serde_json::to_string(&config).unwrap_or_default(), token, now],
        )?;
        let id = c.last_insert_rowid();
        c.execute("INSERT INTO monitor_state (monitor_id, state, since) VALUES (?1, 'pending', ?2)", params![id, now])?;
        crate::channels::assign_defaults(c, id)?;
        Ok(Ok(id))
    })?;
    let id = id.map_err(ApiError::invalid)?;
    tracing::info!("monitor {id} created by {}", user.actor());
    changed(&state, id);
    Ok((StatusCode::CREATED, Json(state.db.with(|c| one(c, id))?.ok_or_else(not_found)?)))
}

pub async fn update(
    State(state): State<AppState>,
    user: Authed,
    Path(id): Path<i64>,
    Body(mut config): Body<MonitorConfig>,
) -> ApiResult<Json<Monitor>> {
    let settings = crate::settings::load(&state.db);
    let now = state.clock.now();
    let result = state.db.with(|c| {
        let Some(old) = one(c, id)? else { return Ok(Err(not_found())) };
        if let Err(message) = validate(c, Some(id), &mut config, &settings) {
            return Ok(Err(ApiError::invalid(message)));
        }
        let token = match config.kind {
            Kind::Push => Some(old.push_token.unwrap_or_else(crate::random_token)),
            Kind::Check => None,
        };
        c.execute(
            "UPDATE monitors SET config = ?2, push_token = ?3, updated = ?4 WHERE id = ?1",
            params![id, serde_json::to_string(&config).unwrap_or_default(), token, now],
        )?;
        // Resuming (or another kind of check) starts from scratch, not from stale results.
        if (old.config.paused && !config.paused) || old.config.kind != config.kind || old.config.check != config.check {
            c.execute("DELETE FROM location_state WHERE monitor_id = ?1", [id])?;
        }
        Ok(Ok(()))
    })?;
    result?;
    tracing::info!("monitor {id} changed by {}", user.actor());
    changed(&state, id);
    Ok(Json(state.db.with(|c| one(c, id))?.ok_or_else(not_found)?))
}

pub async fn delete(State(state): State<AppState>, user: Authed, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let n = state.db.with(|c| {
        // Children lose their parent (the config is JSON, so no foreign key does this).
        for mut child in all(c)?.into_iter().filter(|m| m.config.parent_id == Some(id)) {
            child.config.parent_id = None;
            c.execute(
                "UPDATE monitors SET config = ?2 WHERE id = ?1",
                params![child.id, serde_json::to_string(&child.config).unwrap_or_default()],
            )?;
        }
        c.execute("DELETE FROM monitors WHERE id = ?1", [id])
    })?;
    if n == 0 {
        return Err(not_found());
    }
    tracing::info!("monitor {id} deleted by {}", user.actor());
    state.reload_planner();
    Ok(StatusCode::NO_CONTENT)
}

/// After a create/update: state right away (paused, maintenance …) and the planner picks it up.
fn changed(state: &AppState, id: i64) {
    if let Err(e) = crate::ingest::reevaluate(state, id) {
        tracing::error!("monitor {id}: reevaluate: {e}");
    }
    state.reload_planner();
}

#[derive(Deserialize)]
pub struct PushQuery {
    status: Option<String>,
    msg: Option<String>,
    ping: Option<String>,
}

/// `GET|POST /api/push/{token}?status=up|degraded|down&msg=…&ping=<ms>`, no login, on every host.
pub async fn push(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Query(q): Query<PushQuery>,
) -> ApiResult<Json<Value>> {
    let status = match q.status.as_deref() {
        None | Some("") => Status::Up,
        Some(s) => crate::engine::parse_status(s).ok_or_else(|| ApiError::invalid("status: up, degraded oder down"))?,
    };
    let latency_ms = match q.ping.as_deref() {
        None | Some("") => 0,
        Some(p) => p
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map(|v| v.min(1e9) as u64)
            .ok_or_else(|| ApiError::invalid("ping: Millisekunden"))?,
    };
    let message: String =
        q.msg.unwrap_or_default().chars().map(|c| if c.is_control() { ' ' } else { c }).take(300).collect();
    let monitor = state.db.with(|c| by_token(c, &token))?.ok_or_else(|| ApiError::not_found("Unbekanntes Token."))?;
    // Only known tokens count, so a flood of made-up ones cannot block real pushes.
    if !state.pushes.allow() {
        return Err(ApiError::rate_limited("Zu viele Push-Meldungen, bitte warten."));
    }
    if monitor.config.paused {
        return Ok(Json(json!({ "ok": false, "message": "Monitor pausiert." })));
    }
    let now = state.clock.now();
    let outcome = Outcome { status, latency_ms, message: message.trim().to_owned(), details: Default::default() };
    crate::ingest::ingest_result(&state, monitor.id, LOCAL, None, now, &outcome)?;
    let deadline = now + i64::from(monitor.config.interval_s) + i64::from(monitor.config.timeout_s);
    state.plan(monitor.id, deadline);
    Ok(Json(json!({ "ok": true })))
}
