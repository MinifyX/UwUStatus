//! Status page configuration: admin CRUD, logo, and the public ids behind badges.

use crate::app::AppState;
use crate::auth::Authed;
use crate::error::{ApiError, ApiResult, Body};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Lang {
    #[default]
    De,
    En,
}

/// Suite account colours; the CSS has a text-safe ink and a tint per colour and scheme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    #[default]
    Pink,
    Violet,
    Sky,
    Mint,
    Amber,
    Coral,
}

impl Accent {
    pub fn as_str(self) -> &'static str {
        match self {
            Accent::Pink => "pink",
            Accent::Violet => "violet",
            Accent::Sky => "sky",
            Accent::Mint => "mint",
            Accent::Amber => "amber",
            Accent::Coral => "coral",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: i64,
    /// Display name, empty = the monitor's name.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub show_latency: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    #[serde(default)]
    pub monitors: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageConfig {
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// Hosts that serve this page (lowercase, no port).
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default)]
    pub lang: Lang,
    /// Follow `Accept-Language` (de/en) instead of always `lang`.
    #[serde(default)]
    pub auto_lang: bool,
    #[serde(default)]
    pub accent: Accent,
    #[serde(default = "yes")]
    pub nyu: bool,
    /// Days and times on the page are local to this zone.
    #[serde(default = "berlin")]
    pub timezone: String,
    #[serde(default)]
    pub groups: Vec<Group>,
}

fn yes() -> bool {
    true
}
fn berlin() -> String {
    "Europe/Berlin".into()
}

#[derive(Clone, Debug, Deserialize)]
pub struct PageInput {
    pub slug: String,
    #[serde(flatten)]
    pub config: PageConfig,
}

#[derive(Clone, Debug, Serialize)]
pub struct Page {
    pub id: i64,
    pub slug: String,
    #[serde(flatten)]
    pub config: PageConfig,
    pub has_logo: bool,
    pub preview_url: String,
    pub created: i64,
    pub updated: i64,
}

impl Page {
    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.config.groups.iter().flat_map(|g| &g.monitors)
    }

    pub fn shows(&self, monitor_id: i64) -> bool {
        self.entries().any(|e| e.id == monitor_id)
    }
}

const SELECT: &str = "SELECT id, slug, config, logo IS NOT NULL, created, updated FROM status_pages";

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Option<Page>> {
    let id: i64 = r.get(0)?;
    let slug: String = r.get(1)?;
    let json: String = r.get(2)?;
    Ok(match serde_json::from_str(&json) {
        Ok(config) => Some(Page {
            id,
            preview_url: format!("/s/{slug}"),
            slug,
            config,
            has_logo: r.get(3)?,
            created: r.get(4)?,
            updated: r.get(5)?,
        }),
        Err(e) => {
            tracing::warn!("status page {id}: unreadable config: {e}");
            None
        }
    })
}

pub fn all(c: &Connection) -> rusqlite::Result<Vec<Page>> {
    let mut stmt = c.prepare(&format!("{SELECT} ORDER BY slug"))?;
    let rows = stmt.query_map([], from_row)?;
    rows.filter_map(Result::transpose).collect()
}

pub fn one(c: &Connection, id: i64) -> rusqlite::Result<Option<Page>> {
    Ok(c.query_row(&format!("{SELECT} WHERE id = ?1"), [id], from_row).optional()?.flatten())
}

pub fn by_slug(c: &Connection, slug: &str) -> rusqlite::Result<Option<Page>> {
    Ok(c.query_row(&format!("{SELECT} WHERE slug = ?1"), [slug], from_row).optional()?.flatten())
}

/// `Host` header value without port and trailing dot, lowercase.
pub fn normalize_host(host: &str) -> String {
    let host = if host.starts_with('[') { host } else { host.rsplit_once(':').map_or(host, |(h, _)| h) };
    host.trim_end_matches('.').to_ascii_lowercase()
}

// shortcut: scans every page per request; an index table when there are hundreds of pages.
pub fn by_host(c: &Connection, host: &str) -> rusqlite::Result<Option<Page>> {
    let host = normalize_host(host);
    Ok(all(c)?.into_iter().find(|p| p.config.domains.contains(&host)))
}

/// Whether badges for this monitor may be served without login.
pub fn badge_allowed(c: &Connection, monitor_id: i64) -> rusqlite::Result<bool> {
    let flag: Option<bool> = c
        .query_row("SELECT badge_public FROM monitor_public WHERE monitor_id = ?1", [monitor_id], |r| r.get(0))
        .optional()?;
    Ok(flag == Some(true) || all(c)?.iter().any(|p| p.shows(monitor_id)))
}

/// Random, stable id for public URLs (never the row id). Created on first use.
pub fn public_id(c: &Connection, monitor_id: i64) -> rusqlite::Result<String> {
    c.execute(
        "INSERT INTO monitor_public (monitor_id, public_id) VALUES (?1, ?2) ON CONFLICT (monitor_id) DO NOTHING",
        params![monitor_id, &crate::random_token()[..16]],
    )?;
    c.query_row("SELECT public_id FROM monitor_public WHERE monitor_id = ?1", [monitor_id], |r| r.get(0))
}

pub fn monitor_by_public_id(c: &Connection, public_id: &str) -> rusqlite::Result<Option<i64>> {
    c.query_row("SELECT monitor_id FROM monitor_public WHERE public_id = ?1", [public_id], |r| r.get(0)).optional()
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

fn validate(c: &Connection, state: &AppState, id: Option<i64>, input: &mut PageInput) -> Result<(), String> {
    let p = &mut input.config;
    input.slug = input.slug.trim().to_owned();
    if !(1..=50).contains(&input.slug.len())
        || !input.slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err("Kurzname: 1 bis 50 Zeichen a–z, 0–9, -".into());
    }
    p.title = p.title.trim().to_owned();
    if !(1..=100).contains(&p.title.chars().count()) {
        return Err("Titel: 1 bis 100 Zeichen".into());
    }
    if p.description.chars().count() > 2000 {
        return Err("Beschreibung: höchstens 2000 Zeichen".into());
    }
    if jiff::tz::TimeZone::get(&p.timezone).is_err() {
        return Err("Unbekannte Zeitzone".into());
    }
    let admin = normalize_host(&state.config.public_authority());
    for domain in &mut p.domains {
        *domain = normalize_host(domain.trim());
        if !valid_host(domain) {
            return Err(format!("Domain „{domain}“ ist ungültig"));
        }
        if *domain == admin {
            return Err("Die Admin-Domain kann keine Statusseite sein".into());
        }
    }
    p.domains.sort();
    p.domains.dedup();
    if p.domains.len() > 20 {
        return Err("Höchstens 20 Domains".into());
    }
    let others = all(c).map_err(|e| e.to_string())?;
    if let Some(taken) =
        others.iter().filter(|o| Some(o.id) != id).find_map(|o| p.domains.iter().find(|d| o.config.domains.contains(d)))
    {
        return Err(format!("Domain „{taken}“ gehört schon zu einer anderen Statusseite"));
    }
    if p.groups.len() > 50 || p.groups.iter().map(|g| g.monitors.len()).sum::<usize>() > 500 {
        return Err("Höchstens 50 Gruppen und 500 Monitore".into());
    }
    let known = crate::monitors::all(c).map_err(|e| e.to_string())?;
    for group in &mut p.groups {
        group.name = group.name.trim().to_owned();
        if group.name.chars().count() > 100 {
            return Err("Gruppenname: höchstens 100 Zeichen".into());
        }
        for entry in &mut group.monitors {
            entry.name = entry.name.trim().to_owned();
            if entry.name.chars().count() > 100 {
                return Err("Anzeigename: höchstens 100 Zeichen".into());
            }
            if !known.iter().any(|m| m.id == entry.id) {
                return Err(format!("Monitor {} gibt es nicht", entry.id));
            }
        }
    }
    Ok(())
}

fn conflict(e: rusqlite::Error) -> ApiError {
    match e {
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
            ApiError::invalid("Kurzname ist schon vergeben")
        }
        e => e.into(),
    }
}

pub async fn list(State(state): State<AppState>, _: Authed) -> ApiResult<Json<Vec<Page>>> {
    Ok(Json(state.db.with(all)?))
}

pub async fn get(State(state): State<AppState>, _: Authed, Path(id): Path<i64>) -> ApiResult<Json<Page>> {
    state.db.with(|c| one(c, id))?.map(Json).ok_or_else(|| ApiError::not_found("Statusseite nicht gefunden."))
}

fn save(c: &Connection, state: &AppState, id: Option<i64>, mut input: PageInput) -> ApiResult<Page> {
    validate(c, state, id, &mut input).map_err(ApiError::invalid)?;
    let json = serde_json::to_string(&input.config).map_err(ApiError::internal)?;
    let now = state.clock.now();
    let id = match id {
        None => {
            c.execute(
                "INSERT INTO status_pages (slug, config, created, updated) VALUES (?1, ?2, ?3, ?3)",
                params![input.slug, json, now],
            )
            .map_err(conflict)?;
            c.last_insert_rowid()
        }
        Some(id) => {
            let n = c
                .execute(
                    "UPDATE status_pages SET slug = ?1, config = ?2, updated = ?3 WHERE id = ?4",
                    params![input.slug, json, now, id],
                )
                .map_err(conflict)?;
            if n == 0 {
                return Err(ApiError::not_found("Statusseite nicht gefunden."));
            }
            id
        }
    };
    one(c, id)?.ok_or_else(|| ApiError::internal("page vanished"))
}

pub async fn create(
    State(state): State<AppState>,
    user: Authed,
    Body(input): Body<PageInput>,
) -> ApiResult<(StatusCode, Json<Page>)> {
    let page = state.db.with(|c| Ok(save(c, &state, None, input)))??;
    tracing::info!("status page {} created by {}", page.slug, user.actor());
    Ok((StatusCode::CREATED, Json(page)))
}

pub async fn update(
    State(state): State<AppState>,
    user: Authed,
    Path(id): Path<i64>,
    Body(input): Body<PageInput>,
) -> ApiResult<Json<Page>> {
    let page = state.db.with(|c| Ok(save(c, &state, Some(id), input)))??;
    tracing::info!("status page {} updated by {}", page.slug, user.actor());
    Ok(Json(page))
}

pub async fn delete(State(state): State<AppState>, user: Authed, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    if state.db.with(|c| c.execute("DELETE FROM status_pages WHERE id = ?1", [id]))? == 0 {
        return Err(ApiError::not_found("Statusseite nicht gefunden."));
    }
    tracing::info!("status page {id} deleted by {}", user.actor());
    Ok(StatusCode::NO_CONTENT)
}

pub const MAX_LOGO: usize = 60 * 1024;

/// Content type by magic bytes, whatever the client claims.
fn logo_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        let text = std::str::from_utf8(bytes).ok()?.trim_start_matches('\u{feff}').trim_start();
        (text.starts_with("<svg") || (text.starts_with("<?xml") && text.contains("<svg"))).then_some("image/svg+xml")
    }
}

pub async fn put_logo(
    State(state): State<AppState>,
    user: Authed,
    Path(id): Path<i64>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    if body.is_empty() || body.len() > MAX_LOGO {
        return Err(ApiError::invalid("Logo: höchstens 60 KB"));
    }
    let kind = logo_type(&body).ok_or_else(|| ApiError::invalid("Logo: PNG, JPEG, WebP oder SVG"))?;
    let n = state.db.with(|c| {
        c.execute("UPDATE status_pages SET logo = ?1, logo_type = ?2 WHERE id = ?3", params![body.as_ref(), kind, id])
    })?;
    if n == 0 {
        return Err(ApiError::not_found("Statusseite nicht gefunden."));
    }
    tracing::info!("status page {id}: logo set by {}", user.actor());
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_logo(State(state): State<AppState>, _: Authed, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    state.db.with(|c| c.execute("UPDATE status_pages SET logo = NULL, logo_type = NULL WHERE id = ?1", [id]))?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn logo(c: &Connection, page_id: i64) -> rusqlite::Result<Option<(Vec<u8>, String)>> {
    Ok(c.query_row("SELECT logo, logo_type FROM status_pages WHERE id = ?1", [page_id], |r| {
        Ok(r.get::<_, Option<Vec<u8>>>(0)?.zip(r.get::<_, Option<String>>(1)?))
    })
    .optional()?
    .flatten())
}

#[derive(Serialize)]
pub struct Badge {
    pub public_id: String,
    pub badge_public: bool,
    pub on_page: bool,
    pub status_url: String,
    pub uptime_url: String,
    pub latency_url: String,
}

fn badge_of(c: &Connection, monitor_id: i64) -> ApiResult<Badge> {
    if crate::monitors::one(c, monitor_id)?.is_none() {
        return Err(ApiError::not_found("Monitor nicht gefunden."));
    }
    let pid = public_id(c, monitor_id)?;
    let flag: bool =
        c.query_row("SELECT badge_public FROM monitor_public WHERE monitor_id = ?1", [monitor_id], |r| r.get(0))?;
    Ok(Badge {
        status_url: format!("/badge/{pid}/status.svg"),
        uptime_url: format!("/badge/{pid}/uptime.svg?days=30"),
        latency_url: format!("/badge/{pid}/latency.svg"),
        public_id: pid,
        badge_public: flag,
        on_page: all(c)?.iter().any(|p| p.shows(monitor_id)),
    })
}

pub async fn get_badge(State(state): State<AppState>, _: Authed, Path(id): Path<i64>) -> ApiResult<Json<Badge>> {
    state.db.with(|c| Ok(badge_of(c, id)))?.map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BadgeInput {
    pub badge_public: bool,
}

pub async fn put_badge(
    State(state): State<AppState>,
    _: Authed,
    Path(id): Path<i64>,
    Body(input): Body<BadgeInput>,
) -> ApiResult<Json<Badge>> {
    state
        .db
        .with(|c| {
            Ok(badge_of(c, id).and_then(|_| {
                c.execute(
                    "UPDATE monitor_public SET badge_public = ?1 WHERE monitor_id = ?2",
                    params![input.badge_public, id],
                )?;
                badge_of(c, id)
            }))
        })?
        .map(Json)
}

/// Logo response for the public page; an SVG logo cannot run anything even when opened directly.
pub fn logo_response(logo: Option<(Vec<u8>, String)>) -> axum::response::Response {
    use axum::response::IntoResponse;
    match logo {
        None => StatusCode::NOT_FOUND.into_response(),
        Some((bytes, kind)) => {
            let mut headers = HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, kind.parse().expect("stored type"));
            headers.insert(header::CACHE_CONTROL, "public, max-age=300".parse().expect("static"));
            headers.insert(
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; style-src 'unsafe-inline'; sandbox".parse().expect("static"),
            );
            (headers, bytes).into_response()
        }
    }
}
