//! Shared state and the router (host dispatch: admin app vs. status pages).

use crate::config::Config;
use crate::db::Db;
use crate::oidc::Oidc;
use axum::Router;
use axum::extract::{DefaultBodyLimit, Request};
use axum::http::header;
use axum::routing::{get, post};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
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
}

pub type AppState = Arc<Inner>;

pub fn build(config: Config, db: Db) -> Result<AppState, String> {
    let http = reqwest::Client::builder()
        .user_agent(concat!("UwUStatus/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("HTTP-Client: {e}"))?;
    Ok(Arc::new(Inner {
        oidc: config.oidc.clone().map(|c| Oidc::new(http.clone(), c)),
        http,
        config,
        db,
        callbacks: Limiter::new(20),
        local_failures: Limiter::new(10),
        totp_last_step: Mutex::new(0),
    }))
}

/// The admin app: `/api/*` plus the SPA. Later phases add their routes to `api`.
fn admin_router(state: &AppState) -> Router<AppState> {
    use crate::{auth, local, settings, web};
    let api = Router::new()
        .route("/auth/methods", get(auth::methods))
        .route("/auth/login", get(auth::login))
        .route("/auth/callback", get(auth::callback))
        .route("/auth/local", post(local::login))
        .route("/auth/logout", post(auth::logout))
        .route("/me", get(auth::me))
        .route("/settings", get(settings::get).put(settings::put))
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
