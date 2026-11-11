//! Shared state and the router (host dispatch: admin app vs. status pages).

use crate::Clock;
use crate::config::Config;
use crate::db::Db;
use crate::events::Event;
use crate::oidc::Oidc;
use crate::planner;
use axum::Router;
use axum::extract::{DefaultBodyLimit, Request};
use axum::http::header;
use axum::routing::{get, post};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc};
use tower::ServiceExt as _;
use tower_http::timeout::TimeoutLayer;

/// Fixed one-minute window with at most `max` hits.
pub struct Limiter {
    max: u32,
    window: Mutex<(Instant, u32)>,
}

impl Limiter {
    pub fn new(max: u32) -> Self {
        Self { max, window: Mutex::new((Instant::now(), 0)) }
    }

    fn current(&self) -> std::sync::MutexGuard<'_, (Instant, u32)> {
        let mut window = self.window.lock().unwrap_or_else(|e| e.into_inner());
        if window.0.elapsed() > Duration::from_secs(60) {
            *window = (Instant::now(), 0);
        }
        window
    }

    /// Counts one hit; `false` once the window is full.
    pub fn allow(&self) -> bool {
        let mut window = self.current();
        window.1 += 1;
        window.1 <= self.max
    }

    /// Room left, without counting.
    pub fn has_room(&self) -> bool {
        self.current().1 < self.max
    }

    pub fn hit(&self) {
        self.current().1 += 1;
    }
}

pub struct Inner {
    pub config: Config,
    pub db: Db,
    pub oidc: Option<Oidc>,
    pub http: reqwest::Client,
    /// OIDC callbacks, global.
    pub callbacks: Limiter,
    /// Failed local logins, global (one admin, so per-IP buys nothing).
    pub local_failures: Limiter,
    /// Last TOTP step used, against replays.
    // shortcut: in memory, so a restart allows one replay within 90 s; store it when that matters.
    pub totp_last_step: Mutex<i64>,
    pub clock: Clock,
    /// Phase 4/5 hook: `state.events.subscribe()`.
    pub events: broadcast::Sender<Event>,
    pub planner: mpsc::UnboundedSender<planner::Msg>,
    /// Taken by `planner::spawn`.
    pub planner_rx: Mutex<Option<mpsc::UnboundedReceiver<planner::Msg>>>,
    /// Push calls, global.
    // shortcut: one global window (20/s on average); per token when someone pushes that often.
    pub pushes: Limiter,
    pub alerts: crate::alerts::Alerts,
}

impl Inner {
    /// Monitors changed; the planner re-reads them.
    pub fn reload_planner(&self) {
        let _ = self.planner.send(planner::Msg::Reload);
    }

    /// Next local run (or push deadline) of a monitor.
    pub fn plan(&self, id: i64, at: i64) {
        let _ = self.planner.send(planner::Msg::Due { id, at });
    }
}

pub type AppState = Arc<Inner>;

pub fn build(config: Config, db: Db) -> Result<AppState, String> {
    build_with_clock(config, db, Clock::System)
}

pub fn build_with_clock(config: Config, db: Db, clock: Clock) -> Result<AppState, String> {
    let http = reqwest::Client::builder()
        .user_agent(concat!("UwUStatus/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("HTTP-Client: {e}"))?;
    let (planner, planner_rx) = mpsc::unbounded_channel();
    Ok(Arc::new(Inner {
        oidc: config.oidc.clone().map(|c| Oidc::new(http.clone(), c)),
        alerts: crate::alerts::Alerts::new(http.clone()),
        http,
        config,
        db,
        callbacks: Limiter::new(20),
        local_failures: Limiter::new(10),
        totp_last_step: Mutex::new(0),
        clock,
        events: broadcast::channel(crate::events::BUS).0,
        planner,
        planner_rx: Mutex::new(Some(planner_rx)),
        pushes: Limiter::new(1200),
    }))
}

/// The admin app: `/api/*` plus the SPA. Later phases add their routes to `api`.
fn admin_router(state: &AppState) -> Router<AppState> {
    use crate::{auth, events, history, local, maintenance, monitors, settings, web};
    let api = Router::new()
        .route("/auth/methods", get(auth::methods))
        .route("/auth/login", get(auth::login))
        .route("/auth/callback", get(auth::callback))
        .route("/auth/local", post(local::login))
        .route("/auth/logout", post(auth::logout))
        .route("/me", get(auth::me))
        .route("/settings", get(settings::get).put(settings::put))
        .route("/monitors", get(monitors::list).post(monitors::create))
        .route("/monitors/{id}", get(monitors::get).put(monitors::update).delete(monitors::delete))
        .route("/monitors/{id}/history", get(history::history))
        .route("/monitors/{id}/events", get(events::list))
        .route("/overview", get(history::overview))
        .route("/maintenance", get(maintenance::list).post(maintenance::create))
        .route("/maintenance/{id}", get(maintenance::get).put(maintenance::update).delete(maintenance::delete))
        .route("/channels", get(crate::channels::list).post(crate::channels::create))
        .route("/channels/{id}", get(crate::channels::get).put(crate::channels::update).delete(crate::channels::delete))
        .route("/channels/{id}/test", post(crate::channels::test))
        .route("/channels/{id}/log", get(crate::channels::log))
        .route("/monitors/{id}/channels", get(crate::channels::of_monitor_get).put(crate::channels::of_monitor_put))
        .route("/smtp", get(crate::channels::smtp_get).put(crate::channels::smtp_put))
        .fallback(web::api_not_found);
    Router::new()
        .nest("/api", api)
        .fallback(web::assets)
        .layer(axum::middleware::from_fn_with_state(state.clone(), web::csrf))
}

/// `Host` (HTTP/1.1) or the URI authority (HTTP/2), without the port check being optional.
fn authority(request: &Request) -> Option<&str> {
    request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .or_else(|| request.uri().authority().map(|a| a.as_str()))
}

pub fn router(state: AppState) -> Router {
    let admin = admin_router(&state).with_state(state.clone());
    let status = crate::status::router().with_state(state.clone());
    let admin_host = state.config.public_authority();
    Router::new()
        .route("/healthz", get(crate::web::healthz))
        // Services push from anywhere: every host, no session, no CSRF check.
        .route("/api/push/{token}", get(crate::monitors::push).post(crate::monitors::push))
        .fallback(move |request: Request| {
            let target = if authority(&request).is_some_and(|h| h.eq_ignore_ascii_case(&admin_host)) {
                admin.clone()
            } else {
                status.clone()
            };
            async move { target.oneshot(request).await }
        })
        .layer(axum::middleware::from_fn(crate::web::headers))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(TimeoutLayer::with_status_code(axum::http::StatusCode::REQUEST_TIMEOUT, Duration::from_secs(30)))
        .with_state(state)
}
