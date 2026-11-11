use crate::support::*;
use axum::http::{StatusCode, header};
use serde_json::json;
use uwustatus_server::db::Db;

#[tokio::test]
async fn healthz_format() {
    let env = env().await;
    let reply = env.send("GET", "/healthz", None, None).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.body,
        json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION"), "checks": { "database": { "status": "ok" } } })
    );
    assert_eq!(reply.headers[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn settings_roundtrip_and_validation() {
    let env = env().await;
    assert_eq!(env.send("GET", "/api/settings", None, None).await.status, StatusCode::UNAUTHORIZED);
    let cookie = env.login().await;
    let s = env.send("GET", "/api/settings", Some(&cookie), None).await;
    assert_eq!(s.body["retention_raw_days"], 7);

    let put = env.send("PUT", "/api/settings", Some(&cookie), Some(json!({ "retention_raw_days": 14 }))).await;
    assert_eq!((put.status, &put.body["retention_raw_days"]), (StatusCode::OK, &json!(14)));
    assert_eq!(env.send("GET", "/api/settings", Some(&cookie), None).await.body["retention_raw_days"], 14);

    for bad in [json!({ "retention_raw_days": 0 }), json!({ "retention_raw_days": 120 }), json!({ "unknown": 1 })] {
        let reply = env.send("PUT", "/api/settings", Some(&cookie), Some(bad)).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{:?}", reply.body);
    }
    let order = env.send("PUT", "/api/settings", Some(&cookie), Some(json!({ "retention_hourly_days": 10 }))).await;
    assert_eq!(order.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn admin_spa_and_api_fallback() {
    let env = env().await;
    let api = env.send("GET", "/api/nope", None, None).await;
    assert_eq!((api.status, api.code()), (StatusCode::NOT_FOUND, "not_found"));
    let page = env.send("GET", "/monitors", None, None).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.headers[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/html"));
    assert!(page.headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap().contains("frame-ancestors 'none'"));
}

#[test]
fn migrations_reopen_and_newer_schema() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite3");
    let db = Db::open(&path).unwrap();
    assert_eq!(db.version().unwrap(), uwustatus_server::db::latest_version());
    db.set_setting("k", "v").unwrap();
    drop(db);
    let db = Db::open(&path).unwrap();
    assert_eq!(db.setting("k").unwrap().as_deref(), Some("v"));
    db.with(|c| c.pragma_update(None, "user_version", 999)).unwrap();
    drop(db);
    assert!(Db::open(&path).is_err());
}
