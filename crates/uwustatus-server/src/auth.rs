//! Sessions, UwUAuth (OIDC) login, logout, `me`. The local admin login is in [`crate::local`].

use crate::app::AppState;
use crate::error::{ApiError, ApiResult};
use crate::{now, random_token, sha256_hex};
use axum::Json;
use axum::extract::{FromRequestParts, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};

// `__Host-`: no sibling subdomain can set or overwrite these (needs Secure, Path=/, no Domain).
pub const SESSION_COOKIE: &str = "__Host-uwustatus_session";
pub const LOGIN_COOKIE: &str = "__Host-uwustatus_login";
const IDLE: i64 = 12 * 3600;
// Group membership is only checked at login, so sessions stay short.
const ABSOLUTE: i64 = 86400;
const LOGIN_MAX_AGE: i64 = 600;
/// Open logins at most; more means someone is flooding `/api/auth/login`.
const MAX_PENDING: i64 = 1000;

pub fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

/// A logged-in user (session cookie checked against SQLite). Take it as a handler argument to
/// require a login.
pub struct Authed {
    pub id_hash: String,
    /// OIDC `sub`, or `local:<user>` for the local admin.
    pub sub: String,
    pub name: String,
    pub email: Option<String>,
    /// `oidc` or `local`.
    pub method: String,
}

impl Authed {
    /// Name for logs and, later, the audit trail.
    pub fn actor(&self) -> String {
        self.email.clone().unwrap_or_else(|| self.name.clone())
    }
}

fn session(state: &AppState, id_hash: &str) -> rusqlite::Result<Option<Authed>> {
    let t = now();
    state.db.with(|c| {
        c.execute(
            "DELETE FROM sessions WHERE id_hash = ?1 AND (last_seen < ?2 OR created < ?3)",
            params![id_hash, t - IDLE, t - ABSOLUTE],
        )?;
        let found = c
            .query_row("SELECT sub, name, email, method FROM sessions WHERE id_hash = ?1", [id_hash], |r| {
                Ok(Authed {
                    id_hash: id_hash.to_owned(),
                    sub: r.get(0)?,
                    name: r.get(1)?,
                    email: r.get(2)?,
                    method: r.get(3)?,
                })
            })
            .optional()?;
        if found.is_some() {
            c.execute("UPDATE sessions SET last_seen = ?2 WHERE id_hash = ?1", params![id_hash, t])?;
        }
        Ok(found)
    })
}

impl FromRequestParts<AppState> for Authed {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let id = cookie(&parts.headers, SESSION_COOKIE).ok_or_else(ApiError::unauthenticated)?;
        session(state, &sha256_hex(id))?.ok_or_else(ApiError::unauthenticated)
    }
}

/// Stores a new session and puts its cookie on `response`.
pub fn start_session(
    state: &AppState,
    response: &mut Response,
    sub: &str,
    name: &str,
    email: Option<&str>,
    method: &str,
) -> rusqlite::Result<()> {
    let id = random_token();
    let t = now();
    state.db.with(|c| {
        c.execute("DELETE FROM sessions WHERE last_seen < ?1 OR created < ?2", params![t - IDLE, t - ABSOLUTE])?;
        c.execute(
            "INSERT INTO sessions (id_hash, sub, name, email, method, created, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![sha256_hex(&id), sub, name, email, method, t],
        )
    })?;
    set_cookie(
        response,
        format!("{SESSION_COOKIE}={id}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={ABSOLUTE}"),
    );
    Ok(())
}

/// `next` only as a relative path on this site.
pub fn safe_next(next: Option<&str>) -> String {
    match next {
        Some(n)
            if n.starts_with('/')
                && !n.starts_with("//")
                && !n.starts_with("/\\")
                && n.len() <= 512
                && !n.chars().any(|c| c.is_control()) =>
        {
            n.to_owned()
        }
        _ => "/".to_owned(),
    }
}

pub fn set_cookie(response: &mut Response, value: String) {
    if let Ok(value) = HeaderValue::from_str(&value) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
}

fn redirect(location: &str) -> Response {
    (StatusCode::FOUND, [(header::LOCATION, location.to_owned())]).into_response()
}

fn oidc(state: &AppState) -> ApiResult<&crate::oidc::Oidc> {
    state.oidc.as_ref().ok_or_else(|| ApiError::not_found("Anmeldung über UwUAuth ist nicht eingerichtet."))
}

#[derive(Deserialize)]
pub struct NextQuery {
    next: Option<String>,
}

pub async fn login(State(state): State<AppState>, Query(q): Query<NextQuery>) -> ApiResult<Response> {
    let (login, state_param, nonce, verifier) = (random_token(), random_token(), random_token(), random_token());
    let url = oidc(&state)?
        .authorize_url(&state.config.redirect_uri(), &state_param, &nonce, &verifier, false)
        .await
        .map_err(ApiError::upstream)?;
    let t = now();
    let inserted = state.db.with(|c| {
        c.execute("DELETE FROM pending_logins WHERE started < ?1", [t - LOGIN_MAX_AGE])?;
        let open: i64 = c.query_row("SELECT count(*) FROM pending_logins", [], |r| r.get(0))?;
        if open >= MAX_PENDING {
            return Ok(0);
        }
        c.execute(
            "INSERT INTO pending_logins (cookie_hash, state, nonce, verifier, next, started)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![sha256_hex(&login), state_param, nonce, verifier, safe_next(q.next.as_deref()), t],
        )
    })?;
    if inserted == 0 {
        return Err(ApiError::rate_limited("Zu viele offene Anmeldungen."));
    }
    let mut response = redirect(&url);
    // Lax: the cookie must come back on the IdP's top-level redirect.
    set_cookie(
        &mut response,
        format!("{LOGIN_COOKIE}={login}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={LOGIN_MAX_AGE}"),
    );
    Ok(response)
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

struct Pending {
    state: String,
    nonce: String,
    verifier: String,
    next: String,
    started: i64,
}

pub fn allowed(state: &AppState, sub: &str, groups: &[String]) -> bool {
    state.config.allowed_subjects.iter().any(|s| s == sub)
        || state.config.allowed_group.as_ref().is_some_and(|g| groups.iter().any(|x| x == g))
}

pub async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<CallbackQuery>,
) -> ApiResult<Response> {
    let oidc = oidc(&state)?;
    let login = cookie(&headers, LOGIN_COOKIE).ok_or_else(|| ApiError::invalid("Anmeldung abgelaufen."))?;
    let pending = state.db.with(|c| {
        let hash = sha256_hex(login);
        let row = c
            .query_row(
                "SELECT state, nonce, verifier, next, started FROM pending_logins WHERE cookie_hash = ?1",
                [&hash],
                |r| {
                    Ok(Pending {
                        state: r.get(0)?,
                        nonce: r.get(1)?,
                        verifier: r.get(2)?,
                        next: r.get(3)?,
                        started: r.get(4)?,
                    })
                },
            )
            .optional()?;
        c.execute("DELETE FROM pending_logins WHERE cookie_hash = ?1", [&hash])?;
        Ok(row)
    })?;
    let pending = pending
        .filter(|p| now() - p.started <= LOGIN_MAX_AGE)
        .ok_or_else(|| ApiError::invalid("Anmeldung abgelaufen."))?;
    // Counted only for a real pending login, so random requests cannot lock everyone out.
    if !state.callbacks.allow() {
        return Err(ApiError::rate_limited("Zu viele Anmeldeversuche."));
    }
    if let Some(error) = q.error {
        return Err(ApiError::forbidden(format!(
            "Anmeldung abgebrochen: {}",
            error.chars().take(100).collect::<String>()
        )));
    }
    if !q.state.as_deref().is_some_and(|s| crate::same(s, &pending.state)) {
        return Err(ApiError::invalid("Anmeldung ungültig (state)."));
    }
    let code = q.code.ok_or_else(|| ApiError::invalid("Anmeldung ohne Code."))?;
    let identity =
        oidc.exchange(&code, &pending.verifier, &state.config.redirect_uri(), &pending.nonce).await.map_err(|e| {
            tracing::warn!("login failed: {e}");
            ApiError::forbidden("Anmeldung fehlgeschlagen.")
        })?;
    if !allowed(&state, &identity.sub, &identity.groups) {
        tracing::warn!("login denied for sub {} (groups: {:?})", identity.sub, identity.groups);
        return Err(ApiError::forbidden("Kein Zugriff auf UwUStatus."));
    }
    let mut response = redirect(&pending.next);
    set_cookie(&mut response, format!("{LOGIN_COOKIE}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0"));
    start_session(&state, &mut response, &identity.sub, &identity.name, identity.email.as_deref(), "oidc")?;
    Ok(response)
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(id) = cookie(&headers, SESSION_COOKIE) {
        state.db.with(|c| c.execute("DELETE FROM sessions WHERE id_hash = ?1", [sha256_hex(id)]))?;
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    set_cookie(&mut response, format!("{SESSION_COOKIE}=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0"));
    Ok(response)
}

/// Which sign-in methods the login page offers (no login needed).
pub async fn methods(State(state): State<AppState>) -> Json<Value> {
    let admin = state.config.admin.as_ref();
    Json(json!({
        "oidc": state.oidc.is_some(),
        "local": admin.is_some(),
        "local_totp": admin.is_some_and(|a| a.totp.is_some()),
    }))
}

pub async fn me(user: Authed) -> Json<Value> {
    Json(json!({ "name": user.name, "email": user.email, "method": user.method }))
}
