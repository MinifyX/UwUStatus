//! Monitor CRUD and validation, maintenance API, overview, history and events API.

use crate::support::*;
use axum::http::StatusCode;
use serde_json::{Value, json};
use uwustatus_probe::{Outcome, Status};
use uwustatus_server::ingest::ingest_result;

pub fn tcp(name: &str) -> Value {
    json!({ "name": name, "kind": "check", "check": { "type": "tcp", "host": "192.0.2.10", "port": 443 } })
}

pub fn outcome(status: Status) -> Outcome {
    Outcome { status, latency_ms: 20, message: format!("{status:?}"), details: Default::default() }
}

#[tokio::test]
async fn crud_and_validation() {
    let env = env().await;
    assert_eq!(env.send("GET", "/api/monitors", None, None).await.status, StatusCode::UNAUTHORIZED);
    let cookie = env.admin().await;
    let m = env.monitor(&cookie, tcp(" Web ")).await;
    assert_eq!(
        (m["name"].as_str(), m["interval_s"].as_u64(), m["retry_interval_s"].as_u64()),
        (Some("Web"), Some(60), Some(60))
    );
    assert_eq!(m["check"]["timeout_ms"], 10_000);
    assert_eq!((m["locations"].clone(), m["quorum"].as_str()), (json!(["local"]), Some("any")));
    assert!(m["push_token"].is_null());
    let id = m["id"].as_i64().unwrap();

    let push = env.monitor(&cookie, json!({ "name": "Cron", "kind": "push", "interval_s": 300 })).await;
    assert_eq!(push["push_token"].as_str().map(str::len), Some(43));

    for bad in [
        json!({ "name": "", "kind": "push" }),
        json!({ "name": "x", "kind": "check" }),
        json!({ "name": "x", "kind": "push", "check": { "type": "tcp", "host": "192.0.2.10", "port": 1 } }),
        json!({ "name": "x", "kind": "push", "unknown": 1 }),
        json!({ "name": "x", "kind": "push", "interval_s": 5 }),
        json!({ "name": "x", "kind": "push", "locations": ["sat-unknown"] }),
        json!({ "name": "x", "kind": "check", "check": { "type": "http", "url": "ftp://example.com/" } }),
        json!({ "name": "x", "kind": "check", "interval_s": 30, "timeout_s": 60, "check": { "type": "tcp", "host": "192.0.2.10", "port": 1 } }),
        json!({ "name": "x", "kind": "push", "parent_id": 999 }),
    ] {
        let reply = env.send("POST", "/api/monitors", Some(&cookie), Some(bad.clone())).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{bad}: {:?}", reply.body);
    }

    let mut edit = tcp("Web 2");
    edit["paused"] = json!(true);
    let put = env.send("PUT", &format!("/api/monitors/{id}"), Some(&cookie), Some(edit)).await;
    assert_eq!((put.status, put.body["name"].as_str()), (StatusCode::OK, Some("Web 2")));
    let list = env.send("GET", "/api/monitors", Some(&cookie), None).await;
    assert_eq!(list.body.as_array().unwrap().len(), 2);
    let overview = env.send("GET", "/api/overview", Some(&cookie), None).await;
    assert_eq!(overview.body[0]["state"], "paused");

    assert_eq!(
        env.send("DELETE", &format!("/api/monitors/{id}"), Some(&cookie), None).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        env.send("GET", &format!("/api/monitors/{id}"), Some(&cookie), None).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        env.send("DELETE", &format!("/api/monitors/{id}"), Some(&cookie), None).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn parents_and_satellites() {
    let env = env().await;
    let cookie = env.admin().await;
    let a = env.monitor(&cookie, tcp("A")).await["id"].as_i64().unwrap();
    let mut b = tcp("B");
    b["parent_id"] = json!(a);
    let b = env.monitor(&cookie, b).await["id"].as_i64().unwrap();
    let mut cycle = tcp("A");
    cycle["parent_id"] = json!(b);
    let reply = env.send("PUT", &format!("/api/monitors/{a}"), Some(&cookie), Some(cycle)).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{:?}", reply.body);
    let mut own = tcp("A");
    own["parent_id"] = json!(a);
    assert_eq!(
        env.send("PUT", &format!("/api/monitors/{a}"), Some(&cookie), Some(own)).await.status,
        StatusCode::BAD_REQUEST
    );

    env.state
        .db
        .with(|c| c.execute("INSERT INTO satellites (id, name, created) VALUES ('sat-a', 'A', 0)", []))
        .unwrap();
    let mut sat = tcp("Sat");
    sat["locations"] = json!(["local", "sat-a"]);
    sat["quorum"] = json!("all");
    let m = env.monitor(&cookie, sat).await;
    assert_eq!(m["locations"], json!(["local", "sat-a"]));

    // Deleting the parent detaches the child.
    assert_eq!(
        env.send("DELETE", &format!("/api/monitors/{a}"), Some(&cookie), None).await.status,
        StatusCode::NO_CONTENT
    );
    assert!(env.send("GET", &format!("/api/monitors/{b}"), Some(&cookie), None).await.body["parent_id"].is_null());
}

#[tokio::test]
async fn maintenance_api() {
    let env = env().await;
    let cookie = env.admin().await;
    let mut m = tcp("Web");
    m["group"] = json!("Mail");
    let id = env.monitor(&cookie, m).await["id"].as_i64().unwrap();
    let now = uwustatus_server::now();
    let window = json!({ "title": "Update", "schedule": { "type": "once", "start": now - 60, "end": now + 3600 }, "groups": ["Mail"] });
    let created = env.send("POST", "/api/maintenance", Some(&cookie), Some(window)).await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
    let wid = created.body["id"].as_i64().unwrap();
    let overview = env.send("GET", "/api/overview", Some(&cookie), None).await;
    assert_eq!((overview.body[0]["id"].as_i64(), overview.body[0]["state"].as_str()), (Some(id), Some("maintenance")));

    for bad in [
        json!({ "title": "x", "schedule": { "type": "once", "start": 10, "end": 5 }, "monitors": [id] }),
        json!({ "title": "x", "schedule": { "type": "weekly", "weekdays": [8], "time": "03:00", "duration_min": 30, "timezone": "Europe/Berlin" }, "monitors": [id] }),
        json!({ "title": "x", "schedule": { "type": "once", "start": 1, "end": 5 } }),
        json!({ "title": "", "schedule": { "type": "once", "start": 1, "end": 5 }, "monitors": [id] }),
    ] {
        let reply = env.send("POST", "/api/maintenance", Some(&cookie), Some(bad.clone())).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{bad}: {:?}", reply.body);
    }
    let weekly = json!({ "title": "Nachts", "schedule": { "type": "weekly", "weekdays": [1, 7], "time": "03:00", "duration_min": 30, "timezone": "Europe/Berlin" }, "monitors": [id] });
    let put = env.send("PUT", &format!("/api/maintenance/{wid}"), Some(&cookie), Some(weekly)).await;
    assert_eq!(put.status, StatusCode::OK, "{:?}", put.body);
    assert_eq!(env.send("GET", "/api/maintenance", Some(&cookie), None).await.body[0]["schedule"]["type"], "weekly");
    assert_eq!(
        env.send("DELETE", &format!("/api/maintenance/{wid}"), Some(&cookie), None).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        env.send("GET", &format!("/api/maintenance/{wid}"), Some(&cookie), None).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn history_and_events_api() {
    let env = env().await;
    let cookie = env.admin().await;
    let id = env.monitor(&cookie, tcp("Web")).await["id"].as_i64().unwrap();
    let now = uwustatus_server::now();
    ingest_result(&env.state, id, "local", None, now - 600, &outcome(Status::Up)).unwrap();
    ingest_result(&env.state, id, "local", None, now - 300, &outcome(Status::Down)).unwrap();

    let h = env.send("GET", &format!("/api/monitors/{id}/history?range=24h"), Some(&cookie), None).await;
    assert_eq!((h.status, h.body["level"].as_str()), (StatusCode::OK, Some("raw")));
    assert_eq!(h.body["points"].as_array().unwrap().len(), 2);
    assert!((h.body["uptime"].as_f64().unwrap() - 50.0).abs() < 1.0, "{:?}", h.body);
    let long = env.send("GET", &format!("/api/monitors/{id}/history?range=30d"), Some(&cookie), None).await;
    assert_eq!(long.body["level"], "hour");
    assert_eq!(
        env.send("GET", &format!("/api/monitors/{id}/history?range=365d"), Some(&cookie), None).await.body["level"],
        "day"
    );
    let bad = env.send("GET", &format!("/api/monitors/{id}/history?range=soon"), Some(&cookie), None).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);

    let events = env.send("GET", &format!("/api/monitors/{id}/events"), Some(&cookie), None).await;
    let kinds: Vec<(&str, &str)> = events
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["from"].as_str().unwrap(), e["to"].as_str().unwrap()))
        .collect();
    assert_eq!(kinds, [("up", "down"), ("pending", "up")]);
    let overview = env.send("GET", "/api/overview", Some(&cookie), None).await;
    assert_eq!(
        (overview.body[0]["state"].as_str(), overview.body[0]["message"].as_str()),
        (Some("down"), Some("Down"))
    );
    let recent: Vec<&str> =
        overview.body[0]["recent"].as_array().unwrap().iter().map(|b| b["status"].as_str().unwrap()).collect();
    assert_eq!(recent, ["up", "down"], "heartbeat oldest first");
    let loc = &overview.body[0]["locations"][0];
    assert_eq!((loc["location"].as_str(), loc["state"].as_str()), (Some("local"), Some("down")), "{loc:?}");
    assert!(overview.body[0].get("cert_days_left").is_some());
    assert_eq!(env.send("GET", "/api/monitors/999/events", Some(&cookie), None).await.status, StatusCode::NOT_FOUND);
}
