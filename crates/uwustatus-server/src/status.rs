//! Public status pages: every `Host` that is not the admin app lands here; the admin app
//! previews any page under `/s/{slug}`.

use crate::app::AppState;
use crate::auth::Authed;
use crate::engine::State as MonState;
use crate::incidents::{self, Incident};
use crate::maintenance::{self, Schedule};
use crate::pages::{self, Lang, Page};
use crate::render;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, Uri, header};
use axum::response::{IntoResponse, Response};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

pub const DAYS: i64 = 90;
const MAINTENANCE_AHEAD: i64 = 14 * 86400;

pub fn router() -> Router<AppState> {
    Router::new().fallback(public)
}

async fn public(State(state): State<AppState>, headers: HeaderMap, uri: Uri) -> Response {
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or_default();
    match state.db.with(|c| pages::by_host(c, host)) {
        Ok(Some(page)) => {
            serve(&state, page, "", uri.path(), &headers, format!("https://{}", pages::normalize_host(host)))
        }
        Ok(None) => render::not_found(Lang::De),
        Err(e) => crate::error::ApiError::internal(e).into_response(),
    }
}

/// `/s/{slug}/…` on the admin host, logged in only.
pub async fn preview(
    State(state): State<AppState>,
    _: Authed,
    Path(rest): Path<String>,
    headers: HeaderMap,
) -> Response {
    let (slug, sub) = rest.split_once('/').map_or((rest.as_str(), ""), |(s, r)| (s, r));
    let sub = format!("/{sub}");
    match state.db.with(|c| pages::by_slug(c, slug)) {
        Ok(Some(page)) => {
            let base = format!("/s/{slug}");
            let origin = state.config.public_url.clone();
            let mut response = serve(&state, page, &base, &sub, &headers, origin);
            response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
            response
        }
        Ok(None) => render::not_found(Lang::De),
        Err(e) => crate::error::ApiError::internal(e).into_response(),
    }
}

fn serve(state: &AppState, page: Page, base: &str, sub: &str, headers: &HeaderMap, origin: String) -> Response {
    let lang = language(&page, headers);
    if let Some(file) = sub.strip_prefix("/_status/") {
        return render::asset(file);
    }
    let result = state.db.with(|c| {
        Ok(match sub {
            "/" | "" => html(render::index(&view(c, state, &page, base, lang)?)),
            "/history" => {
                let v = view(c, state, &page, base, lang)?;
                let all = incidents::for_monitors(c, &v.monitor_ids, false, 200)?;
                html(render::history(&v, &all))
            }
            "/feed.atom" | "/feed.rss" => {
                let v = view(c, state, &page, base, lang)?;
                let all = incidents::for_monitors(c, &v.monitor_ids, false, 50)?;
                let feed = render::Feed { view: &v, incidents: &all, origin: &origin };
                if sub == "/feed.atom" {
                    xml("application/atom+xml; charset=utf-8", feed.atom())
                } else {
                    xml("application/rss+xml; charset=utf-8", feed.rss())
                }
            }
            "/logo" => pages::logo_response(pages::logo(c, page.id)?),
            _ => render::not_found(lang),
        })
    });
    let mut response = result.unwrap_or_else(|e| crate::error::ApiError::internal(e).into_response());
    if page.config.auto_lang {
        response.headers_mut().insert(header::VARY, HeaderValue::from_static("Accept-Language"));
    }
    response
}

pub const PAGE_CSP: &str = "default-src 'none'; style-src 'self'; img-src 'self' data:; font-src 'self'; \
                            script-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

fn html(body: String) -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=30"),
            (header::CONTENT_SECURITY_POLICY, PAGE_CSP),
        ],
        body,
    )
        .into_response()
}

fn xml(kind: &'static str, body: String) -> Response {
    ([(header::CONTENT_TYPE, kind), (header::CACHE_CONTROL, "public, max-age=60")], body).into_response()
}

/// Page language: fixed, or the visitor's best of de/en.
pub fn language(page: &Page, headers: &HeaderMap) -> Lang {
    if !page.config.auto_lang {
        return page.config.lang;
    }
    let accept = headers.get(header::ACCEPT_LANGUAGE).and_then(|h| h.to_str().ok()).unwrap_or_default();
    let mut best: Option<(f32, Lang)> = None;
    for item in accept.split(',') {
        let mut parts = item.split(';');
        let tag = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
        let q = parts.find_map(|p| p.trim().strip_prefix("q=")?.parse::<f32>().ok()).unwrap_or(1.0);
        let lang = match tag.split('-').next() {
            Some("de") => Lang::De,
            Some("en") => Lang::En,
            _ => continue,
        };
        if q > 0.0 && best.is_none_or(|(b, _)| q > b) {
            best = Some((q, lang));
        }
    }
    best.map_or(page.config.lang, |(_, l)| l)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Overall {
    Ok,
    Partial,
    Outage,
    Maintenance,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Day {
    /// Local date `YYYY-MM-DD`.
    pub date: String,
    pub uptime: Option<f64>,
    pub down_s: i64,
    pub degraded_s: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct MonitorView {
    #[serde(skip)]
    pub id: i64,
    pub public_id: String,
    pub name: String,
    pub state: MonState,
    pub latency_ms: Option<i64>,
    pub uptime: Option<f64>,
    pub days: Vec<Day>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GroupView {
    pub name: String,
    pub monitors: Vec<MonitorView>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MaintenanceView {
    #[serde(skip)]
    pub id: i64,
    pub title: String,
    pub description: String,
    pub start: i64,
    pub end: i64,
    pub running: bool,
}

pub struct View {
    pub page: Page,
    pub lang: Lang,
    pub base: String,
    pub now: i64,
    pub tz: jiff::tz::TimeZone,
    pub overall: Overall,
    pub groups: Vec<GroupView>,
    pub monitor_ids: Vec<i64>,
    pub open: Vec<Incident>,
    pub maintenance: Vec<MaintenanceView>,
    /// Display name per monitor id (page name, else monitor name).
    pub names: Vec<(i64, String)>,
}

impl View {
    pub fn name(&self, id: i64) -> String {
        self.names.iter().find(|(i, _)| *i == id).map(|(_, n)| n.clone()).unwrap_or_default()
    }
}

/// Local day bounds `[start, end)` for the last `DAYS` days, the last one ending now.
fn day_bounds(now: i64, tz: &jiff::tz::TimeZone) -> Vec<(String, i64, i64)> {
    let Ok(today) = jiff::Timestamp::from_second(now).map(|t| t.to_zoned(tz.clone()).date()) else {
        return Vec::new();
    };
    let start_of = |d: jiff::civil::Date| d.to_zoned(tz.clone()).map(|z| z.timestamp().as_second()).ok();
    (0..DAYS)
        .rev()
        .filter_map(|back| {
            let date = today.checked_sub(jiff::Span::new().days(back)).ok()?;
            let start = start_of(date)?;
            let end = start_of(date.tomorrow().ok()?)?.min(now);
            Some((date.to_string(), start, end))
        })
        .collect()
}

/// Per day: time up/degraded/down from the state changes (pending, maintenance, paused do not count).
pub fn days(initial: MonState, changes: &[(i64, MonState)], bounds: &[(String, i64, i64)]) -> Vec<Day> {
    let mut i = 0;
    let mut current = initial;
    bounds
        .iter()
        .map(|(date, start, end)| {
            while i < changes.len() && changes[i].0 <= *start {
                current = changes[i].1;
                i += 1;
            }
            let (mut good, mut degraded, mut down) = (0, 0, 0);
            let (mut t, mut state, mut j) = (*start, current, i);
            loop {
                let next = changes.get(j).map_or(*end, |c| c.0.min(*end));
                let span = next - t;
                match state {
                    MonState::Up => good += span,
                    MonState::Degraded => degraded += span,
                    MonState::Down => down += span,
                    _ => {}
                }
                if next >= *end {
                    break;
                }
                (t, state) = (next, changes[j].1);
                j += 1;
            }
            let counted = good + degraded + down;
            Day {
                date: date.clone(),
                uptime: (counted > 0).then(|| (good + degraded) as f64 * 100.0 / counted as f64),
                down_s: down,
                degraded_s: degraded,
            }
        })
        .collect()
}

fn history_of(c: &Connection, id: i64, from: i64, to: i64) -> rusqlite::Result<(MonState, Vec<(i64, MonState)>)> {
    let initial: Option<String> = c
        .query_row(
            "SELECT to_value FROM events WHERE monitor_id = ?1 AND kind = 'state' AND at <= ?2 ORDER BY at DESC, id DESC LIMIT 1",
            params![id, from],
            |r| r.get(0),
        )
        .optional()?;
    let mut stmt = c.prepare(
        "SELECT at, to_value FROM events WHERE monitor_id = ?1 AND kind = 'state' AND at > ?2 AND at < ?3 ORDER BY at, id",
    )?;
    let changes = stmt
        .query_map(params![id, from, to], |r| Ok((r.get(0)?, MonState::parse(&r.get::<_, String>(1)?))))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok((initial.map_or(MonState::Pending, |s| MonState::parse(&s)), changes))
}

/// Next start of a window after `now` (or the running one) within `ahead` seconds.
fn next_window(schedule: &Schedule, now: i64, ahead: i64) -> Option<(i64, i64)> {
    if let Some(running) = schedule.window_at(now) {
        return Some(running);
    }
    match schedule {
        Schedule::Once { start, end } => (*start > now && *start < now + ahead).then_some((*start, *end)),
        Schedule::Weekly { weekdays, time, duration_min, timezone } => {
            let tz = jiff::tz::TimeZone::get(timezone).ok()?;
            let time: jiff::civil::Time = time.parse().ok()?;
            let today = jiff::Timestamp::from_second(now).ok()?.to_zoned(tz.clone()).date();
            (0..=ahead / 86400 + 1).find_map(|k| {
                let date = today.checked_add(jiff::Span::new().days(k)).ok()?;
                if !weekdays.contains(&(date.weekday().to_monday_one_offset() as u8)) {
                    return None;
                }
                let start = date.to_datetime(time).to_zoned(tz.clone()).ok()?.timestamp().as_second();
                (start > now && start < now + ahead).then_some((start, start + i64::from(*duration_min) * 60))
            })
        }
    }
}

pub fn view(c: &Connection, state: &AppState, page: &Page, base: &str, lang: Lang) -> rusqlite::Result<View> {
    let now = state.clock.now();
    let tz = jiff::tz::TimeZone::get(&page.config.timezone).unwrap_or(jiff::tz::TimeZone::UTC);
    let bounds = day_bounds(now, &tz);
    let from = bounds.first().map_or(now, |b| b.1);
    let monitors = crate::monitors::all(c)?;
    let mut names = Vec::new();
    let mut groups = Vec::new();
    let mut counted = Vec::new();
    for group in &page.config.groups {
        let mut list = Vec::new();
        for entry in &group.monitors {
            let Some(monitor) = monitors.iter().find(|m| m.id == entry.id) else { continue };
            let name = if entry.name.is_empty() { monitor.config.name.clone() } else { entry.name.clone() };
            let (current, latency): (String, Option<i64>) = c
                .query_row("SELECT state, latency_ms FROM monitor_state WHERE monitor_id = ?1", [entry.id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()?
                .unwrap_or_else(|| ("pending".into(), None));
            let current = MonState::parse(&current);
            let (initial, changes) = history_of(c, entry.id, from, now)?;
            names.push((entry.id, name.clone()));
            counted.push(current);
            list.push(MonitorView {
                id: entry.id,
                public_id: pages::public_id(c, entry.id)?,
                name,
                state: current,
                latency_ms: latency
                    .filter(|_| entry.show_latency && matches!(current, MonState::Up | MonState::Degraded)),
                uptime: crate::engine::uptime(initial, &changes, from, now),
                days: days(initial, &changes, &bounds),
            });
        }
        groups.push(GroupView { name: group.name.clone(), monitors: list });
    }
    let relevant: Vec<MonState> =
        counted.into_iter().filter(|s| !matches!(s, MonState::Pending | MonState::Paused)).collect();
    let bad = relevant.iter().filter(|s| matches!(s, MonState::Down | MonState::Degraded)).count();
    let overall = if !relevant.is_empty() && relevant.iter().all(|s| *s == MonState::Down) {
        Overall::Outage
    } else if bad > 0 {
        Overall::Partial
    } else if relevant.contains(&MonState::Maintenance) {
        Overall::Maintenance
    } else {
        Overall::Ok
    };
    let monitor_ids: Vec<i64> = names.iter().map(|(id, _)| *id).collect();
    let mut planned: Vec<MaintenanceView> = maintenance::all(c)?
        .into_iter()
        .filter(|w| monitors.iter().any(|m| monitor_ids.contains(&m.id) && w.covers(m.id, &m.config)))
        .filter_map(|w| {
            let (start, end) = next_window(&w.input.schedule, now, MAINTENANCE_AHEAD)?;
            Some(MaintenanceView {
                id: w.id,
                title: w.input.title,
                description: w.input.description,
                start,
                end,
                running: start <= now,
            })
        })
        .collect();
    planned.sort_by_key(|m| m.start);
    Ok(View {
        open: incidents::for_monitors(c, &monitor_ids, true, 50)?,
        page: page.clone(),
        lang,
        base: base.to_owned(),
        now,
        tz,
        overall,
        groups,
        monitor_ids,
        maintenance: planned,
        names,
    })
}
