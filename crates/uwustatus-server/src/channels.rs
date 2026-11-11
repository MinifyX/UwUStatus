//! Notification channels: storage, CRUD API, monitor ↔ channel assignment, global SMTP settings
//! and the delivery log. Secrets sit in their own column and never leave the server: answers
//! only say which ones are set.

use crate::app::AppState;
use crate::auth::Authed;
use crate::error::{ApiError, ApiResult, Body};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uwustatus_notify::{Channel as Kind, Lang, Secrets, Smtp, Target};

/// Which events a channel gets.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Events {
    pub down: bool,
    pub degraded: bool,
    pub up: bool,
    pub reminder: bool,
    pub cert: bool,
    pub version: bool,
}

impl Default for Events {
    fn default() -> Self {
        Self { down: true, degraded: true, up: true, reminder: true, cert: true, version: false }
    }
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelConfig {
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub lang: Lang,
    #[serde(default)]
    pub events: Events,
    /// New monitors get this channel.
    #[serde(default)]
    pub is_default: bool,
    pub channel: Kind,
}

#[derive(Debug, Serialize)]
pub struct Channel {
    pub id: i64,
    #[serde(flatten)]
    pub config: ChannelConfig,
    /// Names of the secrets that are set (values are never sent).
    pub secrets: Vec<String>,
    pub created: i64,
    pub updated: i64,
}

/// A channel with its secrets, for sending.
pub struct Stored {
    pub id: i64,
    pub config: ChannelConfig,
    pub secrets: Secrets,
}

impl Stored {
    /// Self-contained: what the notify crate (and later a satellite) needs to send.
    pub fn target(&self, smtp: Option<Smtp>) -> Target {
        Target {
            channel: self.config.channel.clone(),
            secrets: self.secrets.clone(),
            lang: self.config.lang,
            smtp: matches!(self.config.channel, Kind::Email { .. }).then_some(smtp).flatten(),
        }
    }
}

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Option<(Stored, i64, i64)>> {
    let id: i64 = r.get(0)?;
    let (config, secrets): (String, String) = (r.get(1)?, r.get(2)?);
    match (serde_json::from_str(&config), serde_json::from_str(&secrets)) {
        (Ok(config), Ok(secrets)) => Ok(Some((Stored { id, config, secrets }, r.get(3)?, r.get(4)?))),
        _ => {
            tracing::warn!("channel {id}: unreadable config");
            Ok(None)
        }
    }
}

const SELECT: &str = "SELECT id, config, secrets, created, updated FROM notification_channels";

fn public((s, created, updated): (Stored, i64, i64)) -> Channel {
    Channel { id: s.id, config: s.config, secrets: s.secrets.into_keys().collect(), created, updated }
}

pub fn stored(c: &Connection, id: i64) -> rusqlite::Result<Option<Stored>> {
    Ok(c.query_row(&format!("{SELECT} WHERE id = ?1"), [id], from_row).optional()?.flatten().map(|r| r.0))
}

/// The enabled channels of a monitor.
pub fn of_monitor(c: &Connection, monitor_id: i64) -> rusqlite::Result<Vec<Stored>> {
    let mut stmt = c.prepare(&format!(
        "{SELECT} WHERE id IN (SELECT channel_id FROM monitor_channels WHERE monitor_id = ?1) ORDER BY id"
    ))?;
    let rows = stmt.query_map([monitor_id], from_row)?;
    Ok(rows.filter_map(Result::transpose).collect::<rusqlite::Result<Vec<_>>>()?.into_iter().map(|r| r.0).collect())
}

/// Called when a monitor is created: it gets the default channels.
pub fn assign_defaults(c: &Connection, monitor_id: i64) -> rusqlite::Result<()> {
    let mut stmt = c.prepare(&format!("{SELECT} ORDER BY id"))?;
    let defaults: Vec<i64> = stmt
        .query_map([], from_row)?
        .filter_map(Result::transpose)
        .filter_map(|r| r.ok().filter(|r| r.0.config.is_default).map(|r| r.0.id))
        .collect();
    for id in defaults {
        c.execute("INSERT OR IGNORE INTO monitor_channels (monitor_id, channel_id) VALUES (?1, ?2)", [monitor_id, id])?;
    }
    Ok(())
}

pub fn smtp(db: &crate::db::Db) -> Option<Smtp> {
    db.setting("smtp").ok().flatten().and_then(|s| serde_json::from_str(&s).ok())
}

#[derive(Deserialize)]
pub struct Input {
    #[serde(flatten)]
    config: ChannelConfig,
    /// Name → new value; `""` deletes, a missing name keeps the stored value.
    #[serde(default)]
    secrets: BTreeMap<String, String>,
}

fn validate(
    config: &mut ChannelConfig,
    secrets: &mut Secrets,
    changes: BTreeMap<String, String>,
) -> Result<(), String> {
    config.name = config.name.trim().to_owned();
    if !(1..=100).contains(&config.name.chars().count()) {
        return Err("Name: 1 bis 100 Zeichen".into());
    }
    if let Kind::Email { to } = &mut config.channel {
        *to = to.iter().map(|a| a.trim().to_owned()).filter(|a| !a.is_empty()).collect();
    }
    let names = config.channel.secret_names();
    if let Some(name) = changes.keys().find(|k| !names.contains(&k.as_str())) {
        return Err(format!("Unbekanntes Geheimnis {name}"));
    }
    // Another type: secrets of the old one go.
    secrets.retain(|k, _| names.contains(&k.as_str()));
    for (name, value) in changes {
        match value.trim() {
            "" => secrets.remove(&name),
            v => secrets.insert(name, v.to_owned()),
        };
    }
    config.channel.validate(secrets)
}

fn not_found() -> ApiError {
    ApiError::not_found("Kanal nicht gefunden.")
}

pub async fn list(State(state): State<AppState>, _: Authed) -> ApiResult<Json<Vec<Channel>>> {
    let rows = state.db.with(|c| {
        let mut stmt = c.prepare(&format!("{SELECT} ORDER BY id"))?;
        let rows = stmt.query_map([], from_row)?;
        rows.filter_map(Result::transpose).collect::<rusqlite::Result<Vec<_>>>()
    })?;
    Ok(Json(rows.into_iter().map(public).collect()))
}

fn one(c: &Connection, id: i64) -> rusqlite::Result<Option<Channel>> {
    Ok(c.query_row(&format!("{SELECT} WHERE id = ?1"), [id], from_row).optional()?.flatten().map(public))
}

pub async fn get(State(state): State<AppState>, _: Authed, Path(id): Path<i64>) -> ApiResult<Json<Channel>> {
    state.db.with(|c| one(c, id))?.map(Json).ok_or_else(not_found)
}

pub async fn create(
    State(state): State<AppState>,
    user: Authed,
    Body(input): Body<Input>,
) -> ApiResult<(StatusCode, Json<Channel>)> {
    let (mut config, mut secrets) = (input.config, Secrets::new());
    validate(&mut config, &mut secrets, input.secrets).map_err(ApiError::invalid)?;
    let now = state.clock.now();
    let channel = state.db.with(|c| {
        c.execute(
            "INSERT INTO notification_channels (config, secrets, created, updated) VALUES (?1, ?2, ?3, ?3)",
            params![json(&config), json(&secrets), now],
        )?;
        one(c, c.last_insert_rowid())
    })?;
    let channel = channel.ok_or_else(not_found)?;
    tracing::info!("channel {} created by {}", channel.id, user.actor());
    Ok((StatusCode::CREATED, Json(channel)))
}

fn json(value: &impl Serialize) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

pub async fn update(
    State(state): State<AppState>,
    user: Authed,
    Path(id): Path<i64>,
    Body(input): Body<Input>,
) -> ApiResult<Json<Channel>> {
    let now = state.clock.now();
    let result = state.db.with(|c| {
        let Some(old) = stored(c, id)? else { return Ok(Err(not_found())) };
        let (mut config, mut secrets) = (input.config, old.secrets);
        if let Err(e) = validate(&mut config, &mut secrets, input.secrets) {
            return Ok(Err(ApiError::invalid(e)));
        }
        c.execute(
            "UPDATE notification_channels SET config = ?2, secrets = ?3, updated = ?4 WHERE id = ?1",
            params![id, json(&config), json(&secrets), now],
        )?;
        Ok(one(c, id)?.ok_or_else(not_found))
    })?;
    let channel = result?;
    tracing::info!("channel {id} changed by {}", user.actor());
    state.satellites.refresh(&state.db);
    Ok(Json(channel))
}

pub async fn delete(State(state): State<AppState>, user: Authed, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    if state.db.with(|c| c.execute("DELETE FROM notification_channels WHERE id = ?1", [id]))? == 0 {
        return Err(not_found());
    }
    tracing::info!("channel {id} deleted by {}", user.actor());
    state.satellites.refresh(&state.db);
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/channels/{id}/test`: one attempt, logged; a failure comes back as 502 with the reason.
pub async fn test(State(state): State<AppState>, user: Authed, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let channel = state.db.with(|c| stored(c, id))?.ok_or_else(not_found)?;
    let message = crate::alerts::test_message(&state, channel.config.lang, &user.name);
    let target = channel.target(smtp(&state.db));
    let result = state.alerts.sender().send(&target, &message).await;
    crate::alerts::log(&state, id, None, "test", &message.title, 1, result.as_ref().err());
    result.map(|()| StatusCode::NO_CONTENT).map_err(|e| ApiError::upstream(format!("Nicht gesendet: {e}")))
}

#[derive(Debug, Serialize)]
pub struct LogRow {
    pub id: i64,
    pub at: i64,
    pub monitor_id: Option<i64>,
    pub event: String,
    pub title: String,
    pub ok: bool,
    pub attempts: i64,
    pub error: String,
}

/// `GET /api/channels/{id}/log`: the last deliveries, newest first.
pub async fn log(State(state): State<AppState>, _: Authed, Path(id): Path<i64>) -> ApiResult<Json<Vec<LogRow>>> {
    let rows = state.db.with(|c| {
        if one(c, id)?.is_none() {
            return Ok(None);
        }
        let mut stmt = c.prepare(
            "SELECT id, at, monitor_id, event, title, ok, attempts, error FROM notification_log
             WHERE channel_id = ?1 ORDER BY id DESC",
        )?;
        let rows = stmt.query_map([id], |r| {
            Ok(LogRow {
                id: r.get(0)?,
                at: r.get(1)?,
                monitor_id: r.get(2)?,
                event: r.get(3)?,
                title: r.get(4)?,
                ok: r.get(5)?,
                attempts: r.get(6)?,
                error: r.get(7)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map(Some)
    })?;
    rows.map(Json).ok_or_else(not_found)
}

/// `GET /api/monitors/{id}/channels`: ids of the assigned channels.
pub async fn of_monitor_get(
    State(state): State<AppState>,
    _: Authed,
    Path(id): Path<i64>,
) -> ApiResult<Json<Vec<i64>>> {
    let ids = state.db.with(|c| {
        if crate::monitors::one(c, id)?.is_none() {
            return Ok(None);
        }
        let mut stmt =
            c.prepare("SELECT channel_id FROM monitor_channels WHERE monitor_id = ?1 ORDER BY channel_id")?;
        stmt.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<i64>>>().map(Some)
    })?;
    ids.map(Json).ok_or_else(|| ApiError::not_found("Monitor nicht gefunden."))
}

/// `PUT /api/monitors/{id}/channels` with `[channel ids]` replaces the assignment.
pub async fn of_monitor_put(
    State(state): State<AppState>,
    user: Authed,
    Path(id): Path<i64>,
    Body(mut ids): Body<Vec<i64>>,
) -> ApiResult<Json<Vec<i64>>> {
    ids.sort_unstable();
    ids.dedup();
    let result = state.db.with(|c| {
        if crate::monitors::one(c, id)?.is_none() {
            return Ok(Err(ApiError::not_found("Monitor nicht gefunden.")));
        }
        for channel in &ids {
            if stored(c, *channel)?.is_none() {
                return Ok(Err(ApiError::invalid(format!("Kanal {channel} unbekannt"))));
            }
        }
        let tx = c.unchecked_transaction()?;
        tx.execute("DELETE FROM monitor_channels WHERE monitor_id = ?1", [id])?;
        for channel in &ids {
            tx.execute("INSERT INTO monitor_channels (monitor_id, channel_id) VALUES (?1, ?2)", [id, *channel])?;
        }
        tx.commit()?;
        Ok(Ok(()))
    })?;
    result?;
    tracing::info!("monitor {id}: channels changed by {}", user.actor());
    Ok(Json(ids))
}

/// SMTP settings as the API shows them: the password only as `password_set`.
#[derive(Serialize)]
pub struct SmtpView {
    host: String,
    port: u16,
    security: uwustatus_notify::Security,
    user: String,
    from: String,
    password_set: bool,
    configured: bool,
}

fn view(smtp: Option<Smtp>) -> SmtpView {
    let configured = smtp.is_some();
    let s = smtp.unwrap_or(Smtp { port: 587, ..Default::default() });
    SmtpView {
        password_set: !s.password.is_empty(),
        host: s.host,
        port: s.port,
        security: s.security,
        user: s.user,
        from: s.from,
        configured,
    }
}

pub async fn smtp_get(State(state): State<AppState>, _: Authed) -> Json<SmtpView> {
    Json(view(smtp(&state.db)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SmtpInput {
    host: String,
    port: u16,
    #[serde(default)]
    security: uwustatus_notify::Security,
    #[serde(default)]
    user: String,
    /// Missing: keep; `""`: delete.
    password: Option<String>,
    from: String,
}

/// `PUT /api/smtp`; an empty host removes the settings.
pub async fn smtp_put(
    State(state): State<AppState>,
    user: Authed,
    Body(input): Body<SmtpInput>,
) -> ApiResult<Json<SmtpView>> {
    if input.host.trim().is_empty() {
        state.db.with(|c| c.execute("DELETE FROM settings WHERE key = 'smtp'", []))?;
        return Ok(Json(view(None)));
    }
    let password = input.password.unwrap_or_else(|| smtp(&state.db).map(|s| s.password).unwrap_or_default());
    let s = Smtp {
        host: input.host.trim().to_owned(),
        port: input.port,
        security: input.security,
        user: input.user.trim().to_owned(),
        password,
        from: input.from.trim().to_owned(),
    };
    s.validate().map_err(ApiError::invalid)?;
    state.db.set_setting("smtp", &json(&s))?;
    tracing::info!("smtp settings changed by {}", user.actor());
    Ok(Json(view(Some(s))))
}
