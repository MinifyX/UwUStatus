//! Public status pages: every `Host` that is not the admin app lands here.

use crate::app::AppState;
use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;

// shortcut: no status pages yet, every host gets a 404; Phase 5 looks the host up and renders its page.
pub fn router() -> Router<AppState> {
    Router::new().fallback(|| async {
        (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            "<!doctype html><meta charset=utf-8><title>UwUStatus</title><p>Keine Statusseite für diesen Host.</p>",
        )
            .into_response()
    })
}
