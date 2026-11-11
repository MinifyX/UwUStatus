//! Incidents: opened and resolved automatically from state changes, edited by the admin.
//! Automatic titles and texts are stored empty and written at display time (page language,
//! display name).

use crate::app::AppState;
use crate::auth::Authed;
use crate::engine::State as MonState;
use crate::error::{ApiError, ApiResult, Body};
use crate::events::Event;
use crate::pages::Lang;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;

pub const STATUSES: [&str; 4] = ["investigating", "identified", "monitoring", "resolved"];
const IMPACTS: [&str; 4] = ["down", "degraded", "maintenance", "none"];

#[derive(Clone, Debug, Serialize)]
pub struct Update {
    pub id: i64,
    pub at: i64,
    pub status: String,
    /// Empty = automatic text.
    pub body: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Incident {
    pub id: i64,
    /// Empty = automatic title.
    pub title: String,
    pub auto: bool,
    /// The monitor whose state change opened it (automatic incidents).
    pub root: Option<i64>,
    pub impact: String,
    pub status: String,
    pub started: i64,
    pub resolved: Option<i64>,
    pub monitors: Vec<i64>,
    /// Oldest first.
    pub updates: Vec<Update>,
}

fn status_label(lang: Lang, status: &str) -> &'static str {
    match (lang, status) {
        (Lang::De, "investigating") => "Wird untersucht",
        (Lang::De, "identified") => "Ursache gefunden",
        (Lang::De, "monitoring") => "Wird beobachtet",
        (Lang::De, _) => "Behoben",
        (Lang::En, "investigating") => "Investigating",
        (Lang::En, "identified") => "Identified",
        (Lang::En, "monitoring") => "Monitoring",
        (Lang::En, _) => "Resolved",
    }
}

pub fn status_text(lang: Lang, status: &str) -> &'static str {
    status_label(lang, status)
}

impl Incident {
    /// `name` turns a monitor id into its display name.
    pub fn display_title(&self, lang: Lang, name: &dyn Fn(i64) -> String) -> String {
        if !self.title.is_empty() {
            return self.title.clone();
        }
        let who = self.root.map(name).unwrap_or_default();
        match (lang, self.impact.as_str()) {
            (Lang::De, "degraded") => format!("Einschränkung: {who}"),
            (Lang::De, _) => format!("Ausfall: {who}"),
            (Lang::En, "degraded") => format!("Degraded: {who}"),
            (Lang::En, _) => format!("Outage: {who}"),
        }
    }

    pub fn display_body(&self, update: &Update, lang: Lang, name: &dyn Fn(i64) -> String) -> String {
        if !update.body.is_empty() || !self.auto {
            return update.body.clone();
        }
        let who = self.root.map(name).unwrap_or_default();
        match (lang, update.status.as_str(), self.impact.as_str()) {
            (Lang::De, "resolved", _) => format!("{who} ist wieder erreichbar."),
            (Lang::En, "resolved", _) => format!("{who} is available again."),
            (Lang::De, _, "degraded") => format!("{who} ist eingeschränkt erreichbar. Wir prüfen die Ursache."),
            (Lang::En, _, "degraded") => format!("{who} is partially unavailable. We are looking into it."),
            (Lang::De, _, _) => format!("{who} ist nicht erreichbar. Wir prüfen die Ursache."),
            (Lang::En, _, _) => format!("{who} is unavailable. We are looking into it."),
        }
    }
}

const SELECT: &str = "SELECT id, title, auto, monitor_id, impact, status, started, resolved FROM incidents";

fn load(c: &Connection, sql: &str, args: impl rusqlite::Params) -> rusqlite::Result<Vec<Incident>> {
    let mut stmt = c.prepare(sql)?;
    let mut out = stmt
        .query_map(args, |r| {
            Ok(Incident {
                id: r.get(0)?,
                title: r.get(1)?,
                auto: r.get(2)?,
                root: r.get(3)?,
                impact: r.get(4)?,
                status: r.get(5)?,
                started: r.get(6)?,
                resolved: r.get(7)?,
                monitors: Vec::new(),
                updates: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut monitors =
        c.prepare("SELECT monitor_id FROM incident_monitors WHERE incident_id = ?1 ORDER BY monitor_id")?;
    let mut updates =
        c.prepare("SELECT id, at, status, body FROM incident_updates WHERE incident_id = ?1 ORDER BY at, id")?;
    for incident in &mut out {
        incident.monitors = monitors.query_map([incident.id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        incident.updates = updates
            .query_map([incident.id], |r| {
                Ok(Update { id: r.get(0)?, at: r.get(1)?, status: r.get(2)?, body: r.get(3)? })
            })?
            .collect::<rusqlite::Result<_>>()?;
    }
    Ok(out)
}

pub fn one(c: &Connection, id: i64) -> rusqlite::Result<Option<Incident>> {
    Ok(load(c, &format!("{SELECT} WHERE id = ?1"), [id])?.pop())
}

/// Newest first; `open` only unresolved ones.
pub fn all(c: &Connection, open: bool, limit: u32) -> rusqlite::Result<Vec<Incident>> {
    let filter = if open { "WHERE resolved IS NULL" } else { "" };
    load(c, &format!("{SELECT} {filter} ORDER BY started DESC, id DESC LIMIT ?1"), [limit])
}

/// Incidents touching any of `monitors`, newest first.
pub fn for_monitors(c: &Connection, monitors: &[i64], open: bool, limit: u32) -> rusqlite::Result<Vec<Incident>> {
    if monitors.is_empty() {
        return Ok(Vec::new());
    }
    // Integers only, so formatting them into the query is safe.
    let ids = monitors.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
    let filter = if open { "AND resolved IS NULL" } else { "" };
    load(
        c,
        &format!(
            "{SELECT} WHERE id IN (SELECT incident_id FROM incident_monitors WHERE monitor_id IN ({ids})) {filter} \
             ORDER BY started DESC, id DESC LIMIT ?1"
        ),
        [limit],
    )
}

fn open_auto(c: &Connection, root: i64) -> rusqlite::Result<Option<i64>> {
    c.query_row(
        "SELECT id FROM incidents WHERE auto = 1 AND monitor_id = ?1 AND resolved IS NULL ORDER BY id DESC LIMIT 1",
        [root],
        |r| r.get(0),
    )
    .optional()
}

fn parent_of(c: &Connection, id: i64) -> rusqlite::Result<Option<i64>> {
    Ok(crate::monitors::one(c, id)?.and_then(|m| m.config.parent_id))
}

fn add_update(c: &Connection, incident: i64, at: i64, status: &str, body: &str) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO incident_updates (incident_id, at, status, body) VALUES (?1, ?2, ?3, ?4)",
        params![incident, at, status, body],
    )?;
    let resolved = (status == "resolved").then_some(at);
    c.execute("UPDATE incidents SET status = ?1, resolved = ?2 WHERE id = ?3", params![status, resolved, incident])?;
    Ok(())
}

/// The automation: down/degraded opens (or attaches to the parent's incident), up resolves.
pub fn handle(c: &Connection, event: &Event) -> rusqlite::Result<()> {
    let Event::StateChanged { monitor_id: id, at, to, unreachable_via_parent, .. } = event else {
        return Ok(());
    };
    let (id, at) = (*id, *at);
    match to {
        MonState::Down | MonState::Degraded if *unreachable_via_parent => {
            let mut next = parent_of(c, id)?;
            // Bounded: parent chains are validated acyclic, this just guards a bad row.
            for _ in 0..20 {
                let Some(parent) = next else { break };
                if let Some(incident) = open_auto(c, parent)? {
                    c.execute(
                        "INSERT OR IGNORE INTO incident_monitors (incident_id, monitor_id) VALUES (?1, ?2)",
                        params![incident, id],
                    )?;
                    break;
                }
                next = parent_of(c, parent)?;
            }
        }
        MonState::Down | MonState::Degraded => {
            if let Some(incident) = open_auto(c, id)? {
                c.execute("UPDATE incidents SET impact = ?1 WHERE id = ?2", params![to.as_str(), incident])?;
            } else {
                c.execute(
                    "INSERT INTO incidents (title, auto, monitor_id, impact, status, started) \
                     VALUES ('', 1, ?1, ?2, 'investigating', ?3)",
                    params![id, to.as_str(), at],
                )?;
                let incident = c.last_insert_rowid();
                c.execute(
                    "INSERT INTO incident_monitors (incident_id, monitor_id) VALUES (?1, ?2)",
                    params![incident, id],
                )?;
                add_update(c, incident, at, "investigating", "")?;
            }
        }
        MonState::Up | MonState::Paused => {
            if let Some(incident) = open_auto(c, id)? {
                add_update(c, incident, at, "resolved", "")?;
            }
        }
        // Maintenance or pending says nothing about the outage: the incident stays as it is.
        MonState::Maintenance | MonState::Pending => {}
    }
    Ok(())
}

/// Listens on the event bus. Subscribe before the planner starts so no change is missed.
pub fn spawn(state: &AppState) {
    let mut rx = state.events.subscribe();
    let state = state.clone();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Err(e) = state.db.with(|c| handle(c, &event)) {
                        tracing::warn!("incidents: {e}");
                    }
                }
                // shortcut: lost events are not replayed, so an incident can stay open; reconcile from
                // monitor_state when that ever happens in practice.
                Err(RecvError::Lagged(n)) => tracing::warn!("incidents: missed {n} events"),
                Err(RecvError::Closed) => break,
            }
        }
    });
}

// ---- Admin API ----

#[derive(Serialize)]
pub struct NamedMonitor {
    pub id: i64,
    pub name: String,
}

#[derive(Serialize)]
pub struct UpdateJson {
    pub id: i64,
    pub at: i64,
    pub status: String,
    pub body: String,
    pub display_body: String,
}

#[derive(Serialize)]
pub struct IncidentJson {
    pub id: i64,
    pub title: String,
    pub display_title: String,
    pub auto: bool,
    pub impact: String,
    pub status: String,
    pub started: i64,
    pub resolved: Option<i64>,
    pub monitors: Vec<NamedMonitor>,
    pub updates: Vec<UpdateJson>,
}

fn to_json(c: &Connection, incident: Incident, lang: Lang) -> rusqlite::Result<IncidentJson> {
    let monitors = crate::monitors::all(c)?;
    let name = |id: i64| monitors.iter().find(|m| m.id == id).map(|m| m.config.name.clone()).unwrap_or_default();
    Ok(IncidentJson {
        display_title: incident.display_title(lang, &name),
        updates: incident
            .updates
            .iter()
            .map(|u| UpdateJson {
                id: u.id,
                at: u.at,
                status: u.status.clone(),
                body: u.body.clone(),
                display_body: incident.display_body(u, lang, &name),
            })
            .collect(),
        monitors: incident.monitors.iter().map(|&id| NamedMonitor { id, name: name(id) }).collect(),
        id: incident.id,
        title: incident.title,
        auto: incident.auto,
        impact: incident.impact,
        status: incident.status,
        started: incident.started,
        resolved: incident.resolved,
    })
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub lang: Lang,
}

#[derive(Deserialize)]
pub struct LangQuery {
    #[serde(default)]
    pub lang: Lang,
}

pub async fn list(
    State(state): State<AppState>,
    _: Authed,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<IncidentJson>>> {
    let open = q.status.as_deref() == Some("open");
    Ok(Json(state.db.with(|c| all(c, open, 500)?.into_iter().map(|i| to_json(c, i, q.lang)).collect())?))
}

fn reply(c: &Connection, id: i64, lang: Lang) -> ApiResult<IncidentJson> {
    let incident = one(c, id)?.ok_or_else(|| ApiError::not_found("Vorfall nicht gefunden."))?;
    Ok(to_json(c, incident, lang)?)
}

pub async fn get(
    State(state): State<AppState>,
    _: Authed,
    Path(id): Path<i64>,
    Query(q): Query<LangQuery>,
) -> ApiResult<Json<IncidentJson>> {
    state.db.with(|c| Ok(reply(c, id, q.lang)))?.map(Json)
}

fn check_text(text: &str, max: usize, what: &str) -> ApiResult<()> {
    if text.chars().count() > max {
        return Err(ApiError::invalid(format!("{what}: höchstens {max} Zeichen")));
    }
    Ok(())
}

fn check_monitors(c: &Connection, ids: &[i64]) -> ApiResult<()> {
    if ids.is_empty() || ids.len() > 500 {
        return Err(ApiError::invalid("Mindestens ein Monitor"));
    }
    let known = crate::monitors::all(c)?;
    if let Some(id) = ids.iter().find(|id| !known.iter().any(|m| m.id == **id)) {
        return Err(ApiError::invalid(format!("Monitor {id} gibt es nicht")));
    }
    Ok(())
}

fn set_monitors(c: &Connection, incident: i64, ids: &[i64]) -> rusqlite::Result<()> {
    c.execute("DELETE FROM incident_monitors WHERE incident_id = ?1", [incident])?;
    for id in ids {
        c.execute(
            "INSERT OR IGNORE INTO incident_monitors (incident_id, monitor_id) VALUES (?1, ?2)",
            params![incident, id],
        )?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewIncident {
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default = "investigating")]
    pub status: String,
    #[serde(default = "down")]
    pub impact: String,
    pub monitors: Vec<i64>,
}

fn investigating() -> String {
    "investigating".into()
}
fn down() -> String {
    "down".into()
}

fn check_choice(value: &str, allowed: &[&str], what: &str) -> ApiResult<()> {
    if allowed.contains(&value) { Ok(()) } else { Err(ApiError::invalid(format!("{what}: {}", allowed.join(", ")))) }
}

pub async fn create(
    State(state): State<AppState>,
    user: Authed,
    Query(q): Query<LangQuery>,
    Body(input): Body<NewIncident>,
) -> ApiResult<(StatusCode, Json<IncidentJson>)> {
    let title = input.title.trim();
    if title.is_empty() {
        return Err(ApiError::invalid("Titel fehlt"));
    }
    check_text(title, 200, "Titel")?;
    check_text(&input.body, 5000, "Text")?;
    check_choice(&input.status, &STATUSES, "Status")?;
    check_choice(&input.impact, &IMPACTS, "Auswirkung")?;
    let at = state.clock.now();
    let incident = state.db.with(|c| {
        Ok((|| {
            check_monitors(c, &input.monitors)?;
            c.execute(
                "INSERT INTO incidents (title, auto, impact, status, started) VALUES (?1, 0, ?2, ?3, ?4)",
                params![title, input.impact, input.status, at],
            )?;
            let id = c.last_insert_rowid();
            set_monitors(c, id, &input.monitors)?;
            add_update(c, id, at, &input.status, input.body.trim())?;
            reply(c, id, q.lang)
        })())
    })??;
    tracing::info!("incident {} created by {}", incident.id, user.actor());
    Ok((StatusCode::CREATED, Json(incident)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditIncident {
    pub title: Option<String>,
    pub impact: Option<String>,
    pub monitors: Option<Vec<i64>>,
}

pub async fn update(
    State(state): State<AppState>,
    user: Authed,
    Path(id): Path<i64>,
    Query(q): Query<LangQuery>,
    Body(input): Body<EditIncident>,
) -> ApiResult<Json<IncidentJson>> {
    let incident = state.db.with(|c| {
        Ok((|| {
            let current = one(c, id)?.ok_or_else(|| ApiError::not_found("Vorfall nicht gefunden."))?;
            if let Some(title) = &input.title {
                let title = title.trim();
                if title.is_empty() && !current.auto {
                    return Err(ApiError::invalid("Titel fehlt"));
                }
                check_text(title, 200, "Titel")?;
                c.execute("UPDATE incidents SET title = ?1 WHERE id = ?2", params![title, id])?;
            }
            if let Some(impact) = &input.impact {
                check_choice(impact, &IMPACTS, "Auswirkung")?;
                c.execute("UPDATE incidents SET impact = ?1 WHERE id = ?2", params![impact, id])?;
            }
            if let Some(monitors) = &input.monitors {
                check_monitors(c, monitors)?;
                set_monitors(c, id, monitors)?;
            }
            reply(c, id, q.lang)
        })())
    })??;
    tracing::info!("incident {id} edited by {}", user.actor());
    Ok(Json(incident))
}

pub async fn delete(State(state): State<AppState>, user: Authed, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    if state.db.with(|c| c.execute("DELETE FROM incidents WHERE id = ?1", [id]))? == 0 {
        return Err(ApiError::not_found("Vorfall nicht gefunden."));
    }
    tracing::info!("incident {id} deleted by {}", user.actor());
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewUpdate {
    pub status: String,
    pub body: String,
}

pub async fn add(
    State(state): State<AppState>,
    user: Authed,
    Path(id): Path<i64>,
    Query(q): Query<LangQuery>,
    Body(input): Body<NewUpdate>,
) -> ApiResult<Json<IncidentJson>> {
    check_choice(&input.status, &STATUSES, "Status")?;
    let body = input.body.trim();
    if body.is_empty() {
        return Err(ApiError::invalid("Text fehlt"));
    }
    check_text(body, 5000, "Text")?;
    let at = state.clock.now();
    let incident = state.db.with(|c| {
        Ok((|| {
            one(c, id)?.ok_or_else(|| ApiError::not_found("Vorfall nicht gefunden."))?;
            add_update(c, id, at, &input.status, body)?;
            reply(c, id, q.lang)
        })())
    })??;
    tracing::info!("incident {id}: update by {}", user.actor());
    Ok(Json(incident))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditUpdate {
    pub body: String,
}

pub async fn edit_update(
    State(state): State<AppState>,
    user: Authed,
    Path((id, uid)): Path<(i64, i64)>,
    Query(q): Query<LangQuery>,
    Body(input): Body<EditUpdate>,
) -> ApiResult<Json<IncidentJson>> {
    let body = input.body.trim();
    check_text(body, 5000, "Text")?;
    let incident = state.db.with(|c| {
        Ok((|| {
            let n = c.execute(
                "UPDATE incident_updates SET body = ?1 WHERE id = ?2 AND incident_id = ?3",
                params![body, uid, id],
            )?;
            if n == 0 {
                return Err(ApiError::not_found("Eintrag nicht gefunden."));
            }
            reply(c, id, q.lang)
        })())
    })??;
    tracing::info!("incident {id}: update {uid} edited by {}", user.actor());
    Ok(Json(incident))
}
