//! The planner with a real TCP target (real time, waits for events) and push deadlines
//! (paused time: the 15 s pass instantly).

use crate::support::*;
use axum::http::StatusCode;
use serde_json::json;
use std::time::Duration;
use tokio::sync::broadcast;
use uwustatus_server::engine::State;
use uwustatus_server::events::Event;

/// Next state change, skipping other events.
async fn next_change(rx: &mut broadcast::Receiver<Event>) -> (State, State, i64, String) {
    let wait = async {
        loop {
            if let Event::StateChanged { from, to, at, reason, .. } = rx.recv().await.unwrap() {
                return (from, to, at, reason);
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(600), wait).await.expect("no state change")
}

#[tokio::test]
async fn runs_checks_and_applies_edits_at_once() {
    let env = env().await;
    let cookie = env.admin().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            drop(socket);
        }
    });
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port();
    let mut rx = env.state.events.subscribe();
    assert!(uwustatus_server::planner::spawn(&env.state).unwrap());
    assert!(!uwustatus_server::planner::spawn(&env.state).unwrap(), "only once");

    let check = |port: u16| json!({ "name": "Lokal", "kind": "check", "interval_s": 3600, "check": { "type": "tcp", "host": "127.0.0.1", "port": port } });
    let id = env.monitor(&cookie, check(port)).await["id"].as_i64().unwrap();
    assert_eq!(next_change(&mut rx).await.1, State::Up);
    // The next run would be an hour away; the edit runs it now.
    let put = env.send("PUT", &format!("/api/monitors/{id}"), Some(&cookie), Some(check(closed))).await;
    assert_eq!(put.status, StatusCode::OK);
    // A new check starts from scratch (pending), then runs at once.
    assert_eq!(next_change(&mut rx).await.1, State::Pending);
    assert_eq!(next_change(&mut rx).await.1, State::Down);
}

#[tokio::test(start_paused = true)]
async fn push_and_missing_push() {
    let t0 = 1_800_000_000;
    let env = env_at(t0).await;
    let cookie = env.admin().await;
    let mut rx = env.state.events.subscribe();
    uwustatus_server::planner::spawn(&env.state).unwrap();
    let m = env.monitor(&cookie, json!({ "name": "Cron", "kind": "push", "interval_s": 10, "timeout_s": 5 })).await;
    let token = m["push_token"].as_str().unwrap().to_owned();

    assert_eq!(env.send("GET", "/api/push/unknown-token", None, None).await.status, StatusCode::NOT_FOUND);
    let bad = env.send("GET", &format!("/api/push/{token}?status=maybe"), None, None).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    let ok = env.send("GET", &format!("/api/push/{token}?status=up&msg=fertig&ping=12.5"), None, None).await;
    assert_eq!((ok.status, &ok.body), (StatusCode::OK, &json!({ "ok": true })));
    let pushed = env.state.clock.now();
    assert_eq!(next_change(&mut rx).await.1, State::Up);

    let (from, to, at, reason) = next_change(&mut rx).await;
    assert_eq!((from, to, reason.as_str()), (State::Up, State::Down, "Kein Push erhalten"));
    assert!(at >= pushed + 15, "{} s after the push", at - pushed);
    // POST works too and brings it back.
    let post = env.send("POST", &format!("/api/push/{token}?status=up"), None, None).await;
    assert_eq!(post.status, StatusCode::OK, "{:?}", post.body);
    assert_eq!(next_change(&mut rx).await.1, State::Up);
}
