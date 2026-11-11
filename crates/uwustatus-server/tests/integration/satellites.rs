//! A real satellite on 127.0.0.1: pairing through the admin API, a check against a local fake,
//! results in `results`, reconnect over the pinned fingerprint, deletion rules, dead man config.

use crate::support::*;
use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;
use serde_json::{Value, json};
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;
use uwustatus_probe::{Outcome, Status};
use uwustatus_satellite::agent::{self, Agent};
use uwustatus_satellite::config::Config;
use uwustatus_satellite::deadman::NotifyNotifier;
use uwustatus_satellite::state::{Saved, Store};
use uwustatus_server::ingest::ingest_result;
use uwustatus_tunnel::{Timing, Token};

fn timing() -> Timing {
    Timing {
        keep_alive: Duration::from_millis(200),
        idle_timeout: Duration::from_secs(2),
        retry_min: Duration::from_millis(20),
        retry_max: Duration::from_millis(100),
        refused_retry: Duration::from_millis(100),
        handshake_timeout: Duration::from_secs(5),
    }
}

async fn until(mut condition: impl AsyncFnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(15), async {
        while !condition().await {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("timed out");
}

async fn satellite(dir: &std::path::Path) -> (Agent, String) {
    Store::open(dir).unwrap().save(&Saved { token: Some(Token::generate().to_text()), ..Saved::default() }).unwrap();
    let config = Config { listen: (Ipv4Addr::LOCALHOST, 0).into(), state_dir: dir.to_path_buf(), ..Config::default() };
    let agent = agent::start(&config, timing(), Arc::new(NotifyNotifier::new().unwrap())).await.unwrap();
    let code = agent.listener.pairing_code(&[Ipv4Addr::LOCALHOST.into()]).unwrap();
    (agent, code)
}

fn results(env: &Env, id: i64) -> Vec<(String, Option<i64>)> {
    env.state
        .db
        .with(|c| {
            let mut stmt = c.prepare("SELECT location, seq FROM results WHERE monitor_id = ?1 ORDER BY seq")?;
            stmt.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
        })
        .unwrap()
}

async fn list(env: &Env, cookie: &str) -> Value {
    env.send("GET", "/api/satellites", Some(cookie), None).await.body
}

#[tokio::test]
async fn pairs_checks_reconnects_and_stores_each_result_once() {
    let data = tempfile::tempdir().unwrap();
    let sat_dir = tempfile::tempdir().unwrap();
    let env = env_with(|c| c.data_dir = data.path().into()).await;
    *env.state.satellites.timing.lock().unwrap() = timing();
    let cookie = env.admin().await;

    let tcp = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let web = tcp.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(tcp, Router::new().route("/", get(|| async { "ok" }))).await.unwrap() });

    let (_agent, code) = satellite(sat_dir.path()).await;
    let bad =
        env.send("POST", "/api/satellites", Some(&cookie), Some(json!({ "code": "uwusat1nope", "name": "X" }))).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    let body = json!({ "code": code, "name": "Frankfurt 1", "region": "EU" });
    let created = env.send("POST", "/api/satellites", Some(&cookie), Some(body.clone())).await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
    assert_eq!(created.body["id"], "frankfurt-1");
    // Same satellite again (same fingerprint) is refused.
    let again = json!({ "code": code, "name": "Other" });
    assert_eq!(env.send("POST", "/api/satellites", Some(&cookie), Some(again)).await.status, StatusCode::BAD_REQUEST);
    assert!(data.path().join("tunnel-identity.json").exists());

    until(async || list(&env, &cookie).await[0]["status"] == "paired").await;

    let monitor = json!({
        "name": "Web", "kind": "check", "locations": ["frankfurt-1"],
        "check": { "type": "http", "url": format!("http://{web}/") },
    });
    let id = env.monitor(&cookie, monitor).await["id"].as_i64().unwrap();
    until(async || !results(&env, id).is_empty()).await;
    let first = results(&env, id)[0].clone();
    assert_eq!(first.0, "frankfurt-1");
    let seq = first.1.expect("satellite results carry their seq");
    let item = &list(&env, &cookie).await[0];
    assert!(item["live"]["connected"].as_bool().unwrap() && item["live"]["last_result"].is_i64(), "{item:?}");
    assert!(item["version"].is_string());

    // In use: not deletable.
    assert_eq!(
        env.send("DELETE", "/api/satellites/frankfurt-1", Some(&cookie), None).await.status,
        StatusCode::BAD_REQUEST
    );

    // Restart path: new tunnel over the pinned fingerprint, then the same seq again is a duplicate.
    let since = item["live"]["since"].as_i64().unwrap();
    uwustatus_server::satellites::start_all(&env.state);
    until(async || list(&env, &cookie).await[0]["live"]["connected"] == true).await;
    let outcome = Outcome { status: Status::Up, latency_ms: 1, message: "ok".into(), details: Default::default() };
    assert!(ingest_result(&env.state, id, "frankfurt-1", Some(seq), since, &outcome).unwrap().duplicate);
    let rows = results(&env, id);
    let mut seqs: Vec<_> = rows.iter().map(|r| r.1).collect();
    seqs.dedup();
    assert_eq!(seqs.len(), rows.len());

    env.send("DELETE", &format!("/api/monitors/{id}"), Some(&cookie), None).await;
    assert_eq!(
        env.send("DELETE", "/api/satellites/frankfurt-1", Some(&cookie), None).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(list(&env, &cookie).await, json!([]));
}

#[tokio::test]
async fn dead_man_settings_are_checked() {
    let env = env().await;
    let cookie = env.admin().await;
    let reply = env.send("GET", "/api/satellites/dead-man", Some(&cookie), None).await;
    assert_eq!(reply.body["after_min"], 0);
    let put = |body| env.send("PUT", "/api/satellites/dead-man", Some(&cookie), Some(body));
    assert_eq!(put(json!({ "channel_ids": [99] })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(put(json!({ "page_domain": "bad domain" })).await.status, StatusCode::BAD_REQUEST);
    assert!(put(json!({ "unknown": 1 })).await.status.is_client_error());
    let ok = put(json!({ "after_min": 15, "page_enabled": true, "page_domain": "Notfall.Example.com" })).await;
    assert_eq!(ok.status, StatusCode::OK, "{:?}", ok.body);
    assert_eq!(ok.body["page_domain"], "notfall.example.com");
    let reply = env.send("GET", "/api/satellites/dead-man", Some(&cookie), None).await;
    assert_eq!(reply.body["after_min"], 15);
}
