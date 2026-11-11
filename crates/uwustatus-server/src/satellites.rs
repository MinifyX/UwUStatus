//! Satellites: pairing by code, one tunnel client each, their checks (monitors whose locations
//! include the satellite), their results into `ingest_result`, and the dead man's switch config.

use crate::app::{AppState, Inner};
use crate::auth::Authed;
use crate::channels;
use crate::db::Db;
use crate::error::{ApiError, ApiResult, Body};
use crate::monitors::{self, Kind, LOCAL, Monitor};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::mpsc;
use uwustatus_probe::Outcome;
use uwustatus_tunnel::{Assignment, Identity, MainClient, MainEvent, MainSettings, PairingCode, ResultItem, Timing};

/// What the admin sees of a tunnel right now (not stored).
#[derive(Clone, Debug, Default, Serialize)]
pub struct Live {
    pub connected: bool,
    /// Unix seconds of the last connect or loss.
    pub since: Option<i64>,
    /// Last dial or refusal error.
    pub error: Option<String>,
    pub last_result: Option<i64>,
}

struct Link {
    client: Arc<MainClient>,
    live: Arc<Mutex<Live>>,
}

pub struct Registry {
    links: Mutex<HashMap<String, Link>>,
    identity: Mutex<Option<Identity>>,
    /// Tests make it short.
    pub timing: Mutex<Timing>,
}

impl Default for Registry {
    fn default() -> Self {
        Self { links: Mutex::default(), identity: Mutex::default(), timing: Mutex::new(Timing::default()) }
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        for link in self.links().values() {
            link.client.stop();
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Registry {
    fn links(&self) -> std::sync::MutexGuard<'_, HashMap<String, Link>> {
        lock(&self.links)
    }

    /// The main instance's tunnel certificate, made on first use (`data_dir/tunnel-identity.json`, 0600).
    fn identity(&self, dir: &std::path::Path) -> Result<Identity, String> {
        let mut cached = lock(&self.identity);
        if let Some(identity) = cached.as_ref() {
            return Ok(identity.clone());
        }
        let path = dir.join("tunnel-identity.json");
        let identity = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let identity = Identity::generate().map_err(|e| e.to_string())?;
                let json = serde_json::to_vec(&identity).map_err(|e| e.to_string())?;
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&path)
                    .and_then(|mut f| f.write_all(&json).and_then(|()| f.sync_all()))
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                identity
            }
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        *cached = Some(identity.clone());
        Ok(identity)
    }

    fn live(&self, id: &str) -> Live {
        self.links().get(id).map(|l| lock(&l.live).clone()).unwrap_or_default()
    }

    fn remove(&self, id: &str) {
        if let Some(link) = self.links().remove(id) {
            link.client.stop();
        }
    }

    /// Monitors or the dead man changed: every satellite gets its checks and config again.
    pub fn refresh(&self, db: &Db) {
        if self.links().is_empty() {
            return;
        }
        let dead_man = load_dead_man(db);
        let smtp = channels::smtp(db);
        let data = db.with(|c| {
            let monitors = monitors::all(c)?;
            let mut targets = Vec::new();
            for id in &dead_man.channel_ids {
                if let Some(channel) = channels::stored(c, *id)?.filter(|s| s.config.enabled) {
                    targets.push(channel.target(smtp.clone()));
                }
            }
            let mut stmt = c.prepare("SELECT id, name FROM satellites")?;
            let names = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<HashMap<String, String>>>()?;
            Ok((monitors, targets, names))
        });
        let (monitors, targets, names) = match data {
            Ok(data) => data,
            Err(e) => return tracing::error!("satellites: {e}"),
        };
        let page_monitors: Vec<Value> = dead_man
            .page_monitors
            .iter()
            .filter_map(|id| monitors.iter().find(|m| m.id == *id))
            .map(|m| json!({ "id": m.id.to_string(), "name": m.config.name }))
            .collect();
        for (id, link) in self.links().iter() {
            link.client.assign(assignments(&monitors, id));
            link.client.dead_man(json!({
                "name": names.get(id).map_or(id.as_str(), String::as_str),
                "after_s": u64::from(dead_man.after_min) * 60,
                "channels": targets,
                "text_de": dead_man.text_de,
                "text_en": dead_man.text_en,
                "page": {
                    "enabled": dead_man.page_enabled,
                    "title": dead_man.page_title,
                    "domain": dead_man.page_domain,
                    "monitors": page_monitors,
                },
            }));
        }
    }
}

fn assignments(monitors: &[Monitor], satellite: &str) -> Vec<Assignment> {
    monitors
        .iter()
        .filter(|m| {
            !m.config.paused && m.config.kind == Kind::Check && m.config.locations.iter().any(|l| l == satellite)
        })
        .filter_map(|m| {
            Some(Assignment {
                id: m.id.to_string(),
                interval_s: m.config.interval_s,
                check: serde_json::to_value(m.config.check.as_ref()?).ok()?,
            })
        })
        .collect()
}

/// Starts the tunnel to a satellite (replacing an old one) and follows its events.
fn connect(state: &AppState, id: &str, mut settings: MainSettings) {
    settings.timing = *lock(&state.satellites.timing);
    let (client, events) = MainClient::start(settings);
    let client = Arc::new(client);
    let live = Arc::new(Mutex::new(Live::default()));
    let link = Link { client: client.clone(), live: live.clone() };
    if let Some(old) = state.satellites.links().insert(id.to_owned(), link) {
        old.client.stop();
    }
    state.satellites.refresh(&state.db);
    tokio::spawn(follow(Arc::downgrade(state), id.to_owned(), client, live, events));
}

/// After start: every stored satellite gets its tunnel (pending ones still with their code's token).
pub fn start_all(state: &AppState) {
    let rows = state.db.with(|c| {
        let mut stmt = c.prepare("SELECT id, status, code FROM satellites")?;
        stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()
    });
    let rows = match rows {
        Ok(rows) => rows,
        Err(e) => return tracing::error!("satellites: {e}"),
    };
    let identity = match state.satellites.identity(&state.config.data_dir) {
        Ok(identity) => identity,
        Err(e) => return tracing::error!("satellites: {e}"),
    };
    for (id, status, code) in rows {
        let Ok(code) = PairingCode::parse(&code) else {
            tracing::warn!("satellite {id}: no usable pairing code");
            continue;
        };
        let settings = if status == "paired" {
            MainSettings::pinned(code.addresses, code.fingerprint, identity.clone())
        } else {
            MainSettings::from_code(&code, identity.clone())
        };
        connect(state, &id, settings);
    }
}

async fn follow(
    state: Weak<Inner>,
    id: String,
    client: Arc<MainClient>,
    live: Arc<Mutex<Live>>,
    mut events: mpsc::UnboundedReceiver<MainEvent>,
) {
    while let Some(event) = events.recv().await {
        let Some(state) = state.upgrade() else { break };
        let now = state.clock.now();
        let stored = match event {
            MainEvent::Paired => {
                tracing::info!("satellite {id} paired");
                state.db.with(|c| c.execute("UPDATE satellites SET status = 'paired' WHERE id = ?1", [&id]))
            }
            MainEvent::Connected { satellite_version, .. } => {
                let mut l = lock(&live);
                (l.connected, l.since, l.error) = (true, Some(now), None);
                drop(l);
                state.db.with(|c| {
                    c.execute(
                        "UPDATE satellites SET version = ?2, last_seen = ?3 WHERE id = ?1",
                        params![id, satellite_version, now],
                    )
                })
            }
            MainEvent::Lost { error, .. } => {
                let mut l = lock(&live);
                (l.connected, l.since, l.error) = (false, Some(now), Some(error));
                Ok(0)
            }
            MainEvent::Unreachable { error } | MainEvent::Refused { message: error, .. } => {
                lock(&live).error = Some(error);
                Ok(0)
            }
            MainEvent::Results(items) => {
                if let Some(seq) = store_results(&state, &id, &items) {
                    lock(&live).last_result = Some(now);
                    client.ack(seq);
                }
                state.db.with(|c| c.execute("UPDATE satellites SET last_seen = ?2 WHERE id = ?1", params![id, now]))
            }
        };
        if let Err(e) = stored {
            tracing::error!("satellite {id}: {e}");
        }
    }
}

/// Stores results up to the first database error; returns the `seq` to acknowledge.
fn store_results(state: &AppState, id: &str, items: &[ResultItem]) -> Option<u64> {
    let mut acked = None;
    for item in items {
        if let Err(e) = store_result(state, id, item) {
            tracing::error!("satellite {id}: result not stored: {e}");
            break;
        }
        acked = acked.max(Some(item.seq));
    }
    acked
}

fn store_result(state: &AppState, id: &str, item: &ResultItem) -> rusqlite::Result<()> {
    // The satellite is paired but still remote input: odd items are dropped (and acknowledged).
    let (Ok(monitor_id), Ok(seq)) = (item.assignment_id.parse::<i64>(), i64::try_from(item.seq)) else {
        return Ok(());
    };
    let Ok(mut outcome) = serde_json::from_value::<Outcome>(item.outcome.clone()) else {
        tracing::warn!("satellite {id}: unreadable result for monitor {monitor_id}");
        return Ok(());
    };
    let uses =
        state.db.with(|c| monitors::one(c, monitor_id))?.is_some_and(|m| m.config.locations.iter().any(|l| l == id));
    if !uses {
        return Ok(());
    }
    if let Some((cut, _)) = outcome.message.char_indices().nth(1000) {
        outcome.message.truncate(cut);
    }
    let at = (item.at_unix_ms / 1000).min(state.clock.now());
    crate::ingest::ingest_result(state, monitor_id, id, Some(seq), at, &outcome)?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct Satellite {
    pub id: String,
    pub name: String,
    pub region: String,
    /// `pending` until the satellite took the code's token, then `paired`.
    pub status: String,
    pub created: i64,
    pub last_seen: Option<i64>,
    pub version: Option<String>,
    pub fingerprint: Option<String>,
    pub live: Live,
}

fn all(state: &AppState) -> rusqlite::Result<Vec<Satellite>> {
    let rows = state.db.with(|c| {
        let mut stmt = c.prepare(
            "SELECT id, name, region, status, created, last_seen, version, fingerprint FROM satellites ORDER BY name",
        )?;
        stmt.query_map([], |r| {
            Ok(Satellite {
                id: r.get(0)?,
                name: r.get(1)?,
                region: r.get(2)?,
                status: r.get(3)?,
                created: r.get(4)?,
                last_seen: r.get(5)?,
                version: r.get(6)?,
                fingerprint: r.get(7)?,
                live: Live::default(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
    })?;
    Ok(rows.into_iter().map(|s| Satellite { live: state.satellites.live(&s.id), ..s }).collect())
}

fn not_found() -> ApiError {
    ApiError::not_found("Satellit nicht gefunden.")
}

fn one(state: &AppState, id: &str) -> ApiResult<Satellite> {
    all(state)?.into_iter().find(|s| s.id == id).ok_or_else(not_found)
}

pub async fn list(State(state): State<AppState>, _: Authed) -> ApiResult<Json<Vec<Satellite>>> {
    Ok(Json(all(&state)?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    code: String,
    name: String,
    #[serde(default)]
    region: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    name: String,
    #[serde(default)]
    region: String,
}

fn names(name: &str, region: &str) -> ApiResult<(String, String)> {
    let (name, region) = (name.trim().to_owned(), region.trim().to_owned());
    if !(1..=50).contains(&name.chars().count()) || region.chars().count() > 50 {
        return Err(ApiError::invalid("Name 1 bis 50 Zeichen, Region höchstens 50 Zeichen"));
    }
    Ok((name, region))
}

/// `Frankfurt 2` → `frankfurt-2`: the location id in monitors and results.
pub fn slug(name: &str) -> String {
    let mapped: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    mapped.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-").chars().take(40).collect()
}

/// `POST /api/satellites`: redeem a pairing code from `uwustatus-satellite init`.
pub async fn create(
    State(state): State<AppState>,
    user: Authed,
    Body(input): Body<Input>,
) -> ApiResult<(StatusCode, Json<Satellite>)> {
    let (name, region) = names(&input.name, &input.region)?;
    let code_text = input.code.trim();
    let code = PairingCode::parse(code_text).map_err(|_| ApiError::invalid("Kopplungscode ungültig"))?;
    let id = slug(&name);
    if id.is_empty() || id == LOCAL {
        return Err(ApiError::invalid("Name: braucht Buchstaben oder Ziffern und darf nicht „local“ sein"));
    }
    let identity = state.satellites.identity(&state.config.data_dir).map_err(ApiError::internal)?;
    let fingerprint = code.fingerprint.to_string();
    let now = state.clock.now();
    let inserted = state.db.with(|c| {
        let taken = c
            .query_row("SELECT 1 FROM satellites WHERE id = ?1 OR fingerprint = ?2", params![id, fingerprint], |_| Ok(()))
            .optional()?;
        if taken.is_some() {
            return Ok(false);
        }
        c.execute(
            "INSERT INTO satellites (id, name, region, status, created, code, fingerprint) VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6)",
            params![id, name, region, now, code_text, fingerprint],
        )?;
        Ok(true)
    })?;
    if !inserted {
        return Err(ApiError::invalid("Diesen Namen oder diesen Satelliten gibt es schon"));
    }
    tracing::info!("satellite {id} added by {}", user.actor());
    connect(&state, &id, MainSettings::from_code(&code, identity));
    Ok((StatusCode::CREATED, Json(one(&state, &id)?)))
}

pub async fn update(
    State(state): State<AppState>,
    user: Authed,
    Path(id): Path<String>,
    Body(edit): Body<Edit>,
) -> ApiResult<Json<Satellite>> {
    let (name, region) = names(&edit.name, &edit.region)?;
    let n = state
        .db
        .with(|c| c.execute("UPDATE satellites SET name = ?2, region = ?3 WHERE id = ?1", params![id, name, region]))?;
    if n == 0 {
        return Err(not_found());
    }
    tracing::info!("satellite {id} changed by {}", user.actor());
    state.satellites.refresh(&state.db);
    Ok(Json(one(&state, &id)?))
}

pub async fn delete(State(state): State<AppState>, user: Authed, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let users: Vec<String> = state
        .db
        .with(monitors::all)?
        .into_iter()
        .filter(|m| m.config.locations.contains(&id))
        .map(|m| m.config.name)
        .collect();
    if !users.is_empty() {
        return Err(ApiError::invalid(format!("Noch in Monitoren eingetragen: {}", users.join(", "))));
    }
    let n = state.db.with(|c| {
        c.execute("DELETE FROM location_state WHERE location = ?1", [&id])?;
        c.execute("DELETE FROM satellites WHERE id = ?1", [&id])
    })?;
    if n == 0 {
        return Err(not_found());
    }
    state.satellites.remove(&id);
    tracing::info!("satellite {id} deleted by {}", user.actor());
    Ok(StatusCode::NO_CONTENT)
}

/// The dead man's switch, the same for every satellite: alarm through these channels when a
/// satellite has not seen the main instance for `after_min`, and its emergency page.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DeadMan {
    /// 0 = off.
    pub after_min: u32,
    /// From `notification_channels`; sent to the satellites with their secrets.
    pub channel_ids: Vec<i64>,
    /// Empty = built-in text. `{name}` and `{since}` are filled in.
    pub text_de: String,
    pub text_en: String,
    pub page_enabled: bool,
    pub page_title: String,
    /// Empty = any host.
    pub page_domain: String,
    pub page_monitors: Vec<i64>,
}

fn load_dead_man(db: &Db) -> DeadMan {
    db.setting("dead_man").ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub async fn dead_man_get(State(state): State<AppState>, _: Authed) -> Json<DeadMan> {
    Json(load_dead_man(&state.db))
}

pub async fn dead_man_put(
    State(state): State<AppState>,
    user: Authed,
    Body(mut d): Body<DeadMan>,
) -> ApiResult<Json<DeadMan>> {
    d.page_domain = d.page_domain.trim().to_ascii_lowercase();
    d.page_title = d.page_title.trim().to_owned();
    d.channel_ids.sort_unstable();
    d.channel_ids.dedup();
    d.page_monitors.dedup();
    let len = |s: &str| s.chars().count();
    if d.after_min > 10080 {
        return Err(ApiError::invalid("Totmann: höchstens 7 Tage"));
    }
    if len(&d.text_de) > 1000 || len(&d.text_en) > 1000 || len(&d.page_title) > 100 {
        return Err(ApiError::invalid("Texte höchstens 1000, Seitentitel höchstens 100 Zeichen"));
    }
    if len(&d.page_domain) > 253 || !d.page_domain.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
        return Err(ApiError::invalid("Domain: nur Buchstaben, Ziffern, Punkt und Bindestrich"));
    }
    if d.channel_ids.len() > 20 || d.page_monitors.len() > 100 {
        return Err(ApiError::invalid("Höchstens 20 Kanäle und 100 Monitore"));
    }
    let missing = state.db.with(|c| {
        for id in &d.channel_ids {
            if channels::stored(c, *id)?.is_none() {
                return Ok(Some(format!("Kanal {id} unbekannt")));
            }
        }
        for id in &d.page_monitors {
            if monitors::one(c, *id)?.is_none() {
                return Ok(Some(format!("Monitor {id} unbekannt")));
            }
        }
        Ok(None)
    })?;
    if let Some(message) = missing {
        return Err(ApiError::invalid(message));
    }
    state.db.set_setting("dead_man", &serde_json::to_string(&d).map_err(ApiError::internal)?)?;
    tracing::info!("dead man changed by {}", user.actor());
    state.satellites.refresh(&state.db);
    Ok(Json(d))
}

#[cfg(test)]
mod tests {
    #[test]
    fn slug() {
        assert_eq!(super::slug(" Frankfurt (2) "), "frankfurt-2");
        assert_eq!(super::slug("Zürich"), "z-rich");
        assert_eq!(super::slug("---"), "");
    }
}
