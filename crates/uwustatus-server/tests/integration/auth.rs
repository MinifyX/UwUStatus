use crate::support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::json;
use uwustatus_server::config::Secret;
use uwustatus_server::local::totp_code;

#[tokio::test]
async fn oidc_login_allow_and_deny() {
    let env = env().await;
    assert_eq!(env.send("GET", "/api/me", None, None).await.status, StatusCode::UNAUTHORIZED);

    let cookie = env.login().await;
    let me = env.send("GET", "/api/me", Some(&cookie), None).await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!((me.body["name"].as_str(), me.body["method"].as_str()), (Some("Ada"), Some("oidc")));

    let by_group = env.flow(json!({ "sub": "sub-other", "groups": ["users", "status-admins"] })).await;
    assert_eq!(by_group.status, StatusCode::FOUND);
    assert!(set_cookie(&by_group, SESSION).is_some());

    let denied = env.flow(json!({ "sub": "sub-nobody", "groups": ["users"] })).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert!(set_cookie(&denied, SESSION).is_none());

    assert_eq!(env.send("POST", "/api/auth/logout", Some(&cookie), None).await.status, StatusCode::NO_CONTENT);
    assert_eq!(env.send("GET", "/api/me", Some(&cookie), None).await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn bad_nonce_and_safe_next() {
    let env = env().await;
    let start = env.send("GET", "/api/auth/login?next=//evil.example/x", None, None).await;
    let login = set_cookie(&start, LOGIN).unwrap();
    let url = url::Url::parse(start.headers[header::LOCATION].to_str().unwrap()).unwrap();
    let state = url.query_pairs().find(|(k, _)| k == "state").unwrap().1.into_owned();
    *env.fake.claims.lock().unwrap() = json!({ "sub": "sub-allowed", "nonce": "not-the-nonce" });
    let cookie = format!("{LOGIN}={login}");
    let reply = env.send("GET", &format!("/api/auth/callback?code=c&state={state}"), Some(&cookie), None).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    // The pending login is single use.
    let again = env.send("GET", &format!("/api/auth/callback?code=c&state={state}"), Some(&cookie), None).await;
    assert_eq!(again.status, StatusCode::BAD_REQUEST);

    use uwustatus_server::auth::safe_next;
    assert_eq!(safe_next(Some("//evil.example/x")), "/");
    assert_eq!(safe_next(Some("https://evil.example/")), "/");
    assert_eq!(safe_next(Some("/\\evil.example")), "/");
    assert_eq!(safe_next(Some("/monitors/1")), "/monitors/1");
}

#[tokio::test]
async fn methods_and_oidc_off() {
    let env = env().await;
    let m = env.send("GET", "/api/auth/methods", None, None).await;
    assert_eq!(m.body, json!({ "oidc": true, "local": true, "local_totp": false }));

    let env = env_with(|c| c.oidc = None).await;
    assert_eq!(env.send("GET", "/api/auth/methods", None, None).await.body["oidc"], false);
    assert_eq!(env.send("GET", "/api/auth/login", None, None).await.status, StatusCode::NOT_FOUND);
}

fn local(password: &str, totp: Option<String>) -> serde_json::Value {
    json!({ "user": "admin", "password": password, "totp": totp })
}

#[tokio::test]
async fn local_login_password_only() {
    let env = env().await;
    let wrong = env.send("POST", "/api/auth/local", None, Some(local("nope", None))).await;
    assert_eq!(wrong.status, StatusCode::FORBIDDEN);
    assert!(set_cookie(&wrong, SESSION).is_none());
    let bad_user =
        env.send("POST", "/api/auth/local", None, Some(json!({ "user": "root", "password": ADMIN_PASSWORD }))).await;
    assert_eq!(bad_user.status, StatusCode::FORBIDDEN);

    let ok = env.send("POST", "/api/auth/local", None, Some(local(ADMIN_PASSWORD, None))).await;
    assert_eq!(ok.status, StatusCode::NO_CONTENT, "{:?}", ok.body);
    let cookie = format!("{SESSION}={}", set_cookie(&ok, SESSION).unwrap());
    let me = env.send("GET", "/api/me", Some(&cookie), None).await;
    assert_eq!((me.body["name"].as_str(), me.body["method"].as_str()), (Some("admin"), Some("local")));
}

#[tokio::test]
async fn local_login_totp_and_replay() {
    let env = env_with(|c| c.admin.as_mut().unwrap().totp = Some(Secret::new(TOTP_SECRET))).await;
    let key = uwustatus_server::local::totp_key(TOTP_SECRET).unwrap();
    let step = uwustatus_server::now() / 30;

    let missing = env.send("POST", "/api/auth/local", None, Some(local(ADMIN_PASSWORD, None))).await;
    assert_eq!(missing.status, StatusCode::FORBIDDEN);
    let stale =
        env.send("POST", "/api/auth/local", None, Some(local(ADMIN_PASSWORD, Some(totp_code(&key, step - 5))))).await;
    assert_eq!(stale.status, StatusCode::FORBIDDEN);

    let code = totp_code(&key, step);
    let ok = env.send("POST", "/api/auth/local", None, Some(local(ADMIN_PASSWORD, Some(code.clone())))).await;
    assert_eq!(ok.status, StatusCode::NO_CONTENT, "{:?}", ok.body);
    let replay = env.send("POST", "/api/auth/local", None, Some(local(ADMIN_PASSWORD, Some(code)))).await;
    assert_eq!(replay.status, StatusCode::FORBIDDEN);
    assert_eq!(*env.state.totp_last_step.lock().unwrap(), step);
}

#[tokio::test]
async fn local_login_rate_limit() {
    let env = env().await;
    for _ in 0..10 {
        assert_eq!(
            env.send("POST", "/api/auth/local", None, Some(local("nope", None))).await.status,
            StatusCode::FORBIDDEN
        );
    }
    // Even the right password waits out the window.
    let locked = env.send("POST", "/api/auth/local", None, Some(local(ADMIN_PASSWORD, None))).await;
    assert_eq!((locked.status, locked.code()), (StatusCode::TOO_MANY_REQUESTS, "rate_limited"));
}

#[tokio::test]
async fn csrf_and_host_dispatch() {
    let env = env().await;
    let cookie = env.login().await;
    let foreign = Request::builder()
        .method("POST")
        .uri("/api/auth/logout")
        .header(header::HOST, HOST)
        .header(header::ORIGIN, "https://evil.example")
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap();
    assert_eq!(env.raw(foreign).await.status, StatusCode::FORBIDDEN);
    let no_origin = Request::builder()
        .method("POST")
        .uri("/api/auth/logout")
        .header(header::HOST, HOST)
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap();
    assert_eq!(env.raw(no_origin).await.status, StatusCode::FORBIDDEN);
    assert_eq!(env.send("GET", "/api/me", Some(&cookie), None).await.status, StatusCode::OK);

    // Any other host is a status page and never sees the admin API.
    let other =
        |uri: &str| Request::builder().uri(uri).header(header::HOST, "status.example.com").body(Body::empty()).unwrap();
    let reply = env.raw(other("/api/me")).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert!(reply.body.as_str().unwrap().contains("Keine Statusseite"));
    assert_eq!(reply.headers[header::X_FRAME_OPTIONS], "DENY");
    assert_eq!(env.raw(other("/healthz")).await.status, StatusCode::OK);
}
