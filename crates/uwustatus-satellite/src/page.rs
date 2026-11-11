//! The emergency status page: plain HTTP behind a TLS proxy, no cookies, no scripts, nothing
//! loaded from elsewhere. Shown only when the main instance switched it on.

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use uwustatus_probe::Status;

use crate::{Live, utc};

#[derive(Clone)]
struct Page {
    live: Arc<Live>,
    image: Option<Arc<Vec<u8>>>,
}

pub fn router(live: Arc<Live>, image: Option<Vec<u8>>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/image.png", get(picture))
        .with_state(Page { live, image: image.map(Arc::new) })
}

/// Open connections at most; one stranger must not use up the file descriptors the checks need.
const MAX_CONNECTIONS: usize = 256;
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(60);

/// Like `axum::serve`, but with a header read timeout, a cap on connection lifetime and a cap on
/// open connections (`axum::serve` sets no timer, so hyper's header timeout never runs).
pub async fn serve(tcp: TcpListener, app: Router) {
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let Ok(slot) = slots.clone().acquire_owned().await else { return };
        let stream = match tcp.accept().await {
            Ok((stream, _)) => stream,
            Err(e) => {
                tracing::debug!("emergency page accept: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let service = TowerToHyperService::new(app.clone());
        tokio::spawn(async move {
            let connection = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(HEADER_TIMEOUT)
                .serve_connection(TokioIo::new(stream), service);
            let _ = tokio::time::timeout(CONNECTION_TIMEOUT, connection).await;
            drop(slot);
        });
    }
}

const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

fn secure(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// The page is on and this is its host (when one is set).
fn shown(page: &Page, headers: &HeaderMap) -> bool {
    let live = page.live.get();
    let Some(config) = live.config.as_ref().filter(|c| c.page.enabled) else { return false };
    let domain = config.page.domain.trim();
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    let host =
        host.rsplit_once(':').filter(|(_, port)| port.bytes().all(|b| b.is_ascii_digit())).map_or(host, |(h, _)| h);
    domain.is_empty() || host.eq_ignore_ascii_case(domain)
}

async fn picture(State(page): State<Page>, headers: HeaderMap) -> Response {
    match page.image.as_ref().filter(|_| shown(&page, &headers)) {
        Some(image) => secure(([(header::CONTENT_TYPE, "image/png")], image.as_ref().clone()).into_response()),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

async fn index(State(page): State<Page>, headers: HeaderMap) -> Response {
    if !shown(&page, &headers) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let english = headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|l| l.trim_start().to_ascii_lowercase().starts_with("en"));
    let t = |de: &'static str, en: &'static str| if english { en } else { de };
    let live = page.live.get().clone();
    let Some(config) = live.config else { return StatusCode::NOT_FOUND.into_response() };
    let title = match config.page.title.trim() {
        "" => t("Status", "Status"),
        title => title,
    };
    let since = live.since.map(utc).unwrap_or_default();
    let (class, headline) = if live.connected {
        ("ok", t("Hauptinstanz erreichbar", "Main instance reachable").to_owned())
    } else {
        ("down", format!("{} {since}", t("Hauptinstanz nicht erreichbar seit", "Main instance unreachable since")))
    };
    let mut rows = String::new();
    for monitor in &config.page.monitors {
        let (class, label, detail) = match live.latest.get(&monitor.id) {
            Some(l) => {
                let (class, label) = match l.status {
                    Status::Up => ("ok", t("in Betrieb", "operational")),
                    Status::Degraded => ("warn", t("eingeschränkt", "degraded")),
                    Status::Down => ("down", t("gestört", "down")),
                };
                (class, label, format!("{} · {} ms", utc(l.at), l.latency_ms))
            }
            None => ("none", t("keine Daten", "no data"), String::new()),
        };
        let _ = write!(
            rows,
            "<li><span class=\"dot {class}\"></span><span class=\"name\">{}</span><span class=\"state {class}\">{label}</span><span class=\"at\">{detail}</span></li>",
            escape(&monitor.name)
        );
    }
    let image = if page.image.is_some() { "<img src=\"/image.png\" alt=\"\" width=\"96\" height=\"96\">" } else { "" };
    let footer =
        format!("{} „{}“ · UwUStatus", t("Gemeldet vom Satelliten", "Reported by the satellite"), escape(&config.name));
    let html = format!(
        r#"<!doctype html>
<html lang="{lang}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta http-equiv="refresh" content="60">
<meta name="robots" content="noindex">
<title>{title}</title>
<style>
:root {{ color-scheme: light dark; --canvas: #f8f4f6; --surface: #ffffff; --ink: #1c1420; --muted: #716672; --border: #e9dde4; --pink: #ff4d8d; --ok: #12a06d; --ok-ink: #17796a; --warn: #b86e00; --warn-ink: #8e5510; --down: #d92d44; --down-ink: #b4233a; --none: #b3a8b3; }}
@media (prefers-color-scheme: dark) {{ :root {{ --canvas: #17111a; --surface: #211a24; --ink: #f6eef3; --muted: #b9adb8; --border: #3a2f3d; --ok-ink: #4fd1a1; --warn-ink: #f0b357; --down-ink: #ff8a99; }} }}
* {{ box-sizing: border-box; }}
body {{ margin: 0; background: var(--canvas); color: var(--ink); font: 16px/1.5 system-ui, sans-serif; }}
main {{ max-width: 40rem; margin: 0 auto; padding: 2rem 1rem; }}
header {{ display: flex; gap: 1rem; align-items: center; }}
h1 {{ font-size: 1.6rem; margin: 0; }}
.banner {{ margin: 1.5rem 0; padding: 1rem 1.25rem; border-radius: 1rem; background: var(--surface); border: 1px solid var(--border); border-left: 6px solid var(--{class}); font-weight: 600; }}
ul {{ list-style: none; margin: 0; padding: 0; background: var(--surface); border: 1px solid var(--border); border-radius: 1rem; }}
li {{ display: grid; grid-template-columns: auto 1fr auto; gap: 0 .75rem; align-items: center; padding: .75rem 1.25rem; border-top: 1px solid var(--border); }}
li:first-child {{ border-top: 0; }}
.dot {{ width: .75rem; height: .75rem; border-radius: 50%; background: var(--none); }}
.dot.ok {{ background: var(--ok); }} .dot.warn {{ background: var(--warn); }} .dot.down {{ background: var(--down); }}
.state.ok {{ color: var(--ok-ink); }} .state.warn {{ color: var(--warn-ink); }} .state.down {{ color: var(--down-ink); }} .state.none {{ color: var(--muted); }}
.at {{ grid-column: 2 / 4; color: var(--muted); font-size: .85rem; }}
footer {{ margin-top: 1.5rem; color: var(--muted); font-size: .85rem; }}
</style>
</head>
<body>
<main>
<header>{image}<h1>{title}</h1></header>
<p class="banner" role="status">{headline}</p>
<ul>{rows}</ul>
<footer>{footer}</footer>
</main>
</body>
</html>
"#,
        lang = if english { "en" } else { "de" },
        title = escape(title),
    );
    secure(([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response())
}
