//! Security headers, CSRF guard for the admin app, `/healthz` and the embedded SPA.

use crate::app::AppState;
use crate::error::ApiError;
use axum::Json;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, Uri, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(rust_embed::RustEmbed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
struct Assets;

const CSP: &str = "default-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; \
                   form-action 'self'; object-src 'none'";

/// On every response, admin and status pages alike.
pub async fn headers(request: Request, next: Next) -> Response {
    let api = request.uri().path().starts_with("/api/");
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    for (name, value) in [
        (header::CONTENT_SECURITY_POLICY, CSP),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (header::REFERRER_POLICY, "no-referrer"),
    ] {
        headers.entry(name).or_insert(HeaderValue::from_static(value));
    }
    if api {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response
}

/// Admin app only: writes must come from the admin origin (CSRF).
pub async fn csrf(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let write = !matches!(*request.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    let origin = request.headers().get(header::ORIGIN).and_then(|h| h.to_str().ok());
    if write && origin != Some(state.config.public_url.as_str()) {
        return ApiError::forbidden("Anfrage von fremder Seite.").into_response();
    }
    next.run(request).await
}

/// Suite format, see docs/healthz.md.
pub async fn healthz(State(state): State<AppState>) -> Response {
    let database = match state.db.ping() {
        Ok(()) => "ok",
        Err(e) => {
            tracing::error!("healthz: database: {e}");
            "down"
        }
    };
    let code = if database == "down" { StatusCode::SERVICE_UNAVAILABLE } else { StatusCode::OK };
    let body = json!({
        "status": database,
        "version": env!("CARGO_PKG_VERSION"),
        "checks": { "database": { "status": database } },
    });
    (code, [(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

pub async fn api_not_found() -> ApiError {
    ApiError::not_found("Unbekannter API-Pfad.")
}

const PLACEHOLDER: &str = "<!doctype html><meta charset=utf-8><title>UwUStatus</title>\
                           <p>UwUStatus läuft, aber die Oberfläche wurde nicht mitgebaut (web/dist fehlt).</p>";

/// Files from `web/dist`; every other path gets `index.html` (client-side routing).
pub async fn assets(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if let Some(file) = Assets::get(path).filter(|_| !path.is_empty()) {
        let cache = if path.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
        return (
            [(header::CONTENT_TYPE, file.metadata.mimetype().to_owned()), (header::CACHE_CONTROL, cache.to_owned())],
            Body::from(file.data.into_owned()),
        )
            .into_response();
    }
    match Assets::get("index.html") {
        Some(index) => (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")],
            Body::from(index.data.into_owned()),
        )
            .into_response(),
        None => ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], PLACEHOLDER).into_response(),
    }
}
