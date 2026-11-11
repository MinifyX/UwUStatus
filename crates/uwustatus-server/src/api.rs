//! Read API (`/api/v1`), API tokens, Prometheus `/metrics` and SVG badges.

use crate::app::AppState;
use crate::auth::Authed;
use crate::engine::State as MonState;
use crate::error::{ApiError, ApiResult, Body};
use crate::history::uptime_in;
use crate::pages;
use crate::render;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

const DAY: i64 = 86_400;
pub const SCOPES: [&str; 2] = ["read", "metrics"];

/// Routes for every host (tokens or public ids, no session).
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/monitors", get(monitors))
        .route("/api/v1/monitors/{id}", get(monitor))
        .route("/api/v1/pages/{slug}/summary", get(summary))
        .route("/metrics", get(metrics))
        .route("/badge/{public_id}/{file}", get(badge))
}

// ---- Tokens (admin) ----

#[derive(Serialize)]
pub struct Token {
    id: i64,
    name: String,
    scopes: Vec<String>,
    created: i64,
    last_used: Option<i64>,
    /// Only in the create response.
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewToken {
    name: String,
    scopes: Vec<String>,
}

pub async fn list_tokens(State(state): State<AppState>, _: Authed) -> ApiResult<Json<Vec<Token>>> {
    let tokens = state.db.with(|c| {
        let mut stmt = c.prepare("SELECT id, name, scopes, created, last_used FROM api_tokens ORDER BY id")?;
        stmt.query_map([], |r| {
            Ok(Token {
                id: r.get(0)?,
                name: r.get(1)?,
                scopes: serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or_default(),
                created: r.get(3)?,
                last_used: r.get(4)?,
                token: None,
            })
        })?
        .collect()
    })?;
    Ok(Json(tokens))
}

pub async fn create_token(
    State(state): State<AppState>,
    user: Authed,
    Body(input): Body<NewToken>,
) -> ApiResult<(StatusCode, Json<Token>)> {
    let name = input.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::invalid("Name fehlt oder ist zu lang."));
    }
    let mut scopes = input.scopes;
    scopes.sort();
    scopes.dedup();
    if scopes.is_empty() || scopes.iter().any(|s| !SCOPES.contains(&s.as_str())) {
        return Err(ApiError::invalid("Bereiche: read und/oder metrics."));
    }
    let token = format!("uwus_{}", crate::random_token());
    let now = state.clock.now();
    let id = state.db.with(|c| {
        c.execute(
            "INSERT INTO api_tokens (name, token_hash, scopes, created) VALUES (?1, ?2, ?3, ?4)",
            params![name, crate::sha256_hex(&token), serde_json::to_string(&scopes).unwrap_or_default(), now],
        )?;
        Ok(c.last_insert_rowid())
    })?;
    tracing::info!(actor = user.actor(), id, "api token created");
    Ok((StatusCode::CREATED, Json(Token { id, name, scopes, created: now, last_used: None, token: Some(token) })))
}

pub async fn delete_token(State(state): State<AppState>, user: Authed, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let n = state.db.with(|c| c.execute("DELETE FROM api_tokens WHERE id = ?1", [id]))?;
    if n == 0 {
        return Err(ApiError::not_found("Token nicht gefunden."));
    }
    tracing::info!(actor = user.actor(), id, "api token deleted");
    Ok(StatusCode::NO_CONTENT)
}

/// `Authorization: Bearer uwus_…` with `scope`: 401 without a known token, 403 without the scope.
fn bearer(state: &AppState, headers: &HeaderMap, scope: &str) -> ApiResult<()> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(ApiError::unauthenticated)?;
    let now = state.clock.now();
    let found: Option<(i64, String)> = state.db.with(|c| {
        c.query_row("SELECT id, scopes FROM api_tokens WHERE token_hash = ?1", [crate::sha256_hex(token.trim())], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()
    })?;
    let (id, scopes) = found.ok_or_else(ApiError::unauthenticated)?;
    let scopes: Vec<String> = serde_json::from_str(&scopes).unwrap_or_default();
    if !scopes.iter().any(|s| s == scope) {
        return Err(ApiError::forbidden(format!("Token ohne Bereich „{scope}“.")));
    }
    // shortcut: one write per request; batch it if a scraper hits this many times a second.
    state.db.with(|c| c.execute("UPDATE api_tokens SET last_used = ?1 WHERE id = ?2", params![now, id]))?;
    Ok(())
}

// ---- Read API ----

#[derive(Serialize)]
pub struct MonitorOut {
    id: i64,
    public_id: String,
    name: String,
    group: String,
    kind: crate::monitors::Kind,
    state: MonState,
    reason: String,
    since: Option<i64>,
    last_at: Option<i64>,
    latency_ms: Option<i64>,
    cert_days_left: Option<i64>,
    uptime_24h: Option<f64>,
    uptime_7d: Option<f64>,
    uptime_30d: Option<f64>,
}

fn monitor_out(c: &Connection, m: crate::monitors::Monitor, now: i64) -> rusqlite::Result<MonitorOut> {
    type Row = (String, String, i64, Option<i64>, Option<i64>, Option<i64>);
    let row: Option<Row> = c
        .query_row(
            "SELECT state, reason, since, last_at, latency_ms, cert_days_left FROM monitor_state WHERE monitor_id = ?1",
            [m.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .optional()?;
    let (s, reason, since, last_at, latency_ms, cert_days_left) = match row {
        Some((s, reason, since, last, latency, cert)) => {
            (MonState::parse(&s), reason, Some(since), last, latency, cert)
        }
        None => (MonState::Pending, String::new(), None, None, None, None),
    };
    Ok(MonitorOut {
        public_id: pages::public_id(c, m.id)?,
        uptime_24h: uptime_in(c, m.id, now - DAY, now)?,
        uptime_7d: uptime_in(c, m.id, now - 7 * DAY, now)?,
        uptime_30d: uptime_in(c, m.id, now - 30 * DAY, now)?,
        id: m.id,
        name: m.config.name,
        group: m.config.group,
        kind: m.config.kind,
        state: s,
        reason,
        since,
        last_at,
        latency_ms,
        cert_days_left,
    })
}

async fn monitors(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Vec<MonitorOut>>> {
    bearer(&state, &headers, "read")?;
    let now = state.clock.now();
    let out = state.db.with(|c| crate::monitors::all(c)?.into_iter().map(|m| monitor_out(c, m, now)).collect())?;
    Ok(Json(out))
}

async fn monitor(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<Json<MonitorOut>> {
    bearer(&state, &headers, "read")?;
    let now = state.clock.now();
    let out = state.db.with(|c| crate::monitors::one(c, id)?.map(|m| monitor_out(c, m, now)).transpose())?;
    out.map(Json).ok_or_else(|| ApiError::not_found("Monitor nicht gefunden."))
}

/// Public: what the page shows, as JSON (CORS open, so other sites can embed it).
async fn summary(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    let result = state.db.with(|c| {
        let Some(page) = pages::by_slug(c, &slug)? else { return Ok(None) };
        let lang = page.config.lang;
        let v = crate::status::view(c, &state, &page, "", lang)?;
        let monitors: Vec<_> = v
            .groups
            .iter()
            .flat_map(|g| {
                g.monitors.iter().map(|m| {
                    serde_json::json!({
                        "public_id": m.public_id, "name": m.name, "group": g.name, "state": m.state,
                        "latency_ms": m.latency_ms, "uptime_90d": m.uptime,
                    })
                })
            })
            .collect();
        let name = |id: i64| v.name(id);
        let incidents: Vec<_> = v
            .open
            .iter()
            .map(|i| {
                serde_json::json!({
                    "id": i.id, "title": i.display_title(lang, &name), "status": i.status,
                    "impact": i.impact, "started": crate::iso(i.started),
                })
            })
            .collect();
        let maintenance: Vec<_> = v
            .maintenance
            .iter()
            .map(|m| serde_json::json!({ "title": m.title, "start": crate::iso(m.start), "end": crate::iso(m.end), "running": m.running }))
            .collect();
        Ok(Some(serde_json::json!({
            "slug": page.slug, "title": page.config.title, "overall": v.overall, "updated": crate::iso(v.now),
            "monitors": monitors, "incidents": incidents, "maintenance": maintenance,
        })))
    });
    match result {
        Ok(Some(body)) => ([(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")], Json(body)).into_response(),
        Ok(None) => ApiError::not_found("Statusseite nicht gefunden.").into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

// ---- Prometheus ----

fn label(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

const STATES: [MonState; 6] =
    [MonState::Pending, MonState::Up, MonState::Degraded, MonState::Down, MonState::Maintenance, MonState::Paused];

async fn metrics(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    bearer(&state, &headers, "metrics")?;
    let now = state.clock.now();
    let body = state.db.with(|c| {
        let mut up = String::from(
            "# HELP uwustatus_monitor_up 1 when the location sees the monitor up or degraded.\n# TYPE uwustatus_monitor_up gauge\n",
        );
        let mut st =
            String::from("# HELP uwustatus_monitor_state Current state (one series per state).\n# TYPE uwustatus_monitor_state gauge\n");
        let mut lat = String::from("# HELP uwustatus_monitor_latency_ms Last response time.\n# TYPE uwustatus_monitor_latency_ms gauge\n");
        let mut cert =
            String::from("# HELP uwustatus_monitor_cert_days_left Days until the TLS certificate expires.\n# TYPE uwustatus_monitor_cert_days_left gauge\n");
        let mut upt = String::from("# HELP uwustatus_monitor_uptime_ratio Uptime as 0..1.\n# TYPE uwustatus_monitor_uptime_ratio gauge\n");
        let mut locations = c.prepare("SELECT location, state FROM location_state WHERE monitor_id = ?1 ORDER BY location")?;
        for m in crate::monitors::all(c)? {
            let l = format!("id=\"{}\",monitor=\"{}\"", m.id, label(&m.config.name));
            for row in locations.query_map([m.id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                let (location, s) = row?;
                let v = match MonState::parse(&s) {
                    MonState::Up | MonState::Degraded => 1,
                    MonState::Down => 0,
                    _ => continue,
                };
                let _ = writeln!(up, "uwustatus_monitor_up{{{l},location=\"{}\"}} {v}", label(&location));
            }
            let row: Option<(String, Option<i64>, Option<i64>)> = c
                .query_row("SELECT state, latency_ms, cert_days_left FROM monitor_state WHERE monitor_id = ?1", [m.id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .optional()?;
            let (current, latency, days) = row.map_or((MonState::Pending, None, None), |(s, l, d)| (MonState::parse(&s), l, d));
            for s in STATES {
                let _ = writeln!(st, "uwustatus_monitor_state{{{l},state=\"{}\"}} {}", s.as_str(), u8::from(s == current));
            }
            if let Some(v) = latency {
                let _ = writeln!(lat, "uwustatus_monitor_latency_ms{{{l}}} {v}");
            }
            if let Some(v) = days {
                let _ = writeln!(cert, "uwustatus_monitor_cert_days_left{{{l}}} {v}");
            }
            for (window, seconds) in [("24h", DAY), ("7d", 7 * DAY), ("30d", 30 * DAY)] {
                if let Some(v) = uptime_in(c, m.id, now - seconds, now)? {
                    let _ = writeln!(upt, "uwustatus_monitor_uptime_ratio{{{l},window=\"{window}\"}} {}", v / 100.0);
                }
            }
        }
        Ok([up, st, lat, cert, upt].concat())
    })?;
    Ok(([(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")], body).into_response())
}

// ---- Badges ----

#[derive(Deserialize)]
struct BadgeQuery {
    days: Option<i64>,
}

async fn badge(
    State(state): State<AppState>,
    Path((public_id, file)): Path<(String, String)>,
    Query(q): Query<BadgeQuery>,
) -> ApiResult<Response> {
    let days = q.days.unwrap_or(30);
    if !(1..=365).contains(&days) {
        return Err(ApiError::invalid("days: 1 bis 365."));
    }
    let now = state.clock.now();
    let svg = state.db.with(|c| {
        let Some(id) = pages::monitor_by_public_id(c, &public_id)? else { return Ok(None) };
        if !pages::badge_allowed(c, id)? {
            return Ok(None);
        }
        let row: Option<(String, Option<i64>)> = c
            .query_row("SELECT state, latency_ms FROM monitor_state WHERE monitor_id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let (s, latency) = row.map_or((MonState::Pending, None), |(s, l)| (MonState::parse(&s), l));
        Ok(Some(match file.as_str() {
            "status.svg" => render::badge("status", s.as_str(), render::state_color(s)),
            "uptime.svg" => {
                let uptime = uptime_in(c, id, now - days * DAY, now)?;
                let color = match uptime {
                    None => "#716672",
                    Some(u) if u >= 99.0 => "#17796a",
                    Some(u) if u >= 95.0 => "#8e5510",
                    Some(_) => "#b4233a",
                };
                let text = uptime.map_or("–".into(), |u| format!("{:.2}%", (u * 100.0).floor() / 100.0));
                render::badge(&format!("uptime {days}d"), &text, color)
            }
            "latency.svg" => match latency.filter(|_| matches!(s, MonState::Up | MonState::Degraded)) {
                Some(ms) => render::badge("latency", &format!("{ms} ms"), "#17796a"),
                None => render::badge("latency", "–", "#716672"),
            },
            _ => return Ok(None),
        }))
    })?;
    let svg = svg.ok_or_else(|| ApiError::not_found("Badge nicht gefunden."))?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/svg+xml; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=60"),
            (header::CONTENT_SECURITY_POLICY, "default-src 'none'; style-src 'unsafe-inline'"),
        ],
        svg,
    )
        .into_response())
}
