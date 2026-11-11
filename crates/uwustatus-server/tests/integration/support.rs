use crate::fakes::{self, FakeState};
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use std::sync::OnceLock;
use tower::ServiceExt as _;
use uwustatus_server::Clock;
use uwustatus_server::app::{self, AppState};
use uwustatus_server::config::{AdminConfig, Config, OidcConfig, Secret};
use uwustatus_server::db::Db;

pub const HOST: &str = "status-admin.test";
pub const ORIGIN: &str = "https://status-admin.test";
pub const ADMIN_PASSWORD: &str = "local-test-password-0123";
/// RFC 4648 test value, base32 of `12345678901234567890`.
pub const TOTP_SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
pub const SESSION: &str = "__Host-uwustatus_session";
pub const LOGIN: &str = "__Host-uwustatus_login";

/// One cheap hash for all tests.
fn admin_hash() -> String {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        uwustatus_server::local::hash_password(ADMIN_PASSWORD, argon2::Params::new(8, 1, 1, None).unwrap()).unwrap()
    })
    .clone()
}

pub struct Env {
    pub fake: FakeState,
    pub state: AppState,
    pub router: Router,
}

pub fn config(fake: &FakeState) -> Config {
    Config {
        public_url: ORIGIN.into(),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_bind: None,
        data_dir: std::env::temp_dir(),
        oidc: Some(OidcConfig {
            issuer: format!("{}/oidc", fake.base),
            client_id: fakes::CLIENT_ID.into(),
            client_secret: Secret::new(fakes::CLIENT_SECRET),
            backchannel: None,
        }),
        allowed_group: Some("status-admins".into()),
        allowed_subjects: vec!["sub-allowed".into()],
        admin: Some(AdminConfig { user: "admin".into(), password_hash: Secret::new(admin_hash()), totp: None }),
    }
}

pub async fn env() -> Env {
    env_with(|_| {}).await
}

pub async fn env_with(tweak: impl FnOnce(&mut Config)) -> Env {
    env_full(tweak, Clock::System).await
}

/// Time from tokio's clock starting at `unix` (pair with `#[tokio::test(start_paused = true)]`).
pub async fn env_at(unix: i64) -> Env {
    env_full(|_| {}, Clock::tokio(unix)).await
}

async fn env_full(tweak: impl FnOnce(&mut Config), clock: Clock) -> Env {
    let fake = fakes::start().await;
    let mut config = config(&fake);
    tweak(&mut config);
    let state = app::build_with_clock(config, Db::open_in_memory().unwrap(), clock).unwrap();
    Env { router: app::router(state.clone()), fake, state }
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: axum::http::HeaderMap,
    pub body: Value,
}

impl Reply {
    pub fn code(&self) -> &str {
        self.body["error"].as_str().unwrap_or_default()
    }
}

impl Env {
    pub async fn send(&self, method: &str, uri: &str, cookie: Option<&str>, body: Option<Value>) -> Reply {
        let mut request = Request::builder().method(method).uri(uri).header(header::HOST, HOST);
        if method != "GET" {
            request = request.header(header::ORIGIN, ORIGIN);
        }
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie);
        }
        let request = match body {
            Some(b) => request.header(header::CONTENT_TYPE, "application/json").body(Body::from(b.to_string())),
            None => request.body(Body::empty()),
        };
        self.raw(request.unwrap()).await
    }

    pub async fn raw(&self, request: Request<Body>) -> Reply {
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
        let body =
            serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
        Reply { status, headers, body }
    }

    /// Runs the OIDC login with `claims`; returns the callback reply.
    pub async fn flow(&self, claims: Value) -> Reply {
        let start = self.send("GET", "/api/auth/login?next=/monitors", None, None).await;
        assert_eq!(start.status, StatusCode::FOUND, "{:?}", start.body);
        let url = url::Url::parse(start.headers[header::LOCATION].to_str().unwrap()).unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        let login = set_cookie(&start, LOGIN).unwrap();
        let mut claims = claims;
        claims["nonce"] = json!(query["nonce"]);
        *self.fake.claims.lock().unwrap() = claims;
        let uri = format!("/api/auth/callback?code=c0de&state={}", query["state"]);
        self.send("GET", &uri, Some(&format!("{LOGIN}={login}")), None).await
    }

    /// Local admin login (no network, so it also works with paused time).
    pub async fn admin(&self) -> String {
        let body = json!({ "user": "admin", "password": ADMIN_PASSWORD });
        let reply = self.send("POST", "/api/auth/local", None, Some(body)).await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT, "{:?}", reply.body);
        format!("{SESSION}={}", set_cookie(&reply, SESSION).unwrap())
    }

    /// `POST /api/monitors`, expecting 201; returns the monitor.
    pub async fn monitor(&self, cookie: &str, body: Value) -> Value {
        let reply = self.send("POST", "/api/monitors", Some(cookie), Some(body)).await;
        assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.body);
        reply.body
    }

    /// OIDC login as an allowed subject; returns the session cookie.
    pub async fn login(&self) -> String {
        let reply = self.flow(json!({ "sub": "sub-allowed", "name": "Ada" })).await;
        assert_eq!(reply.status, StatusCode::FOUND, "{:?}", reply.body);
        assert_eq!(reply.headers[header::LOCATION], "/monitors");
        format!("{SESSION}={}", set_cookie(&reply, SESSION).unwrap())
    }
}

pub fn set_cookie(reply: &Reply, name: &str) -> Option<String> {
    reply.headers.get_all(header::SET_COOKIE).iter().filter_map(|v| v.to_str().ok()).find_map(|v| {
        let value = v.strip_prefix(&format!("{name}="))?.split(';').next()?;
        (!value.is_empty()).then(|| value.to_owned())
    })
}
