//! Channels API, secret masking, default channels, the dispatcher against a local webhook fake.

use crate::monitors::{outcome, tcp};
use crate::support::{Env, env};
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use uwustatus_probe::Status;
use uwustatus_server::events::Event;
use uwustatus_server::ingest::ingest_result;

struct Hook {
    url: String,
    hits: mpsc::UnboundedReceiver<Value>,
    status: Arc<Mutex<StatusCode>>,
}

async fn hook() -> Hook {
    type S = (mpsc::UnboundedSender<Value>, Arc<Mutex<StatusCode>>);
    async fn record(State((tx, status)): State<S>, body: Bytes) -> StatusCode {
        let _ = tx.send(serde_json::from_slice(&body).unwrap_or_default());
        *status.lock().unwrap()
    }
    let (tx, hits) = mpsc::unbounded_channel();
    let status = Arc::new(Mutex::new(StatusCode::OK));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    let app = Router::new().fallback(record).with_state((tx, status.clone()));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Hook { url, hits, status }
}

fn webhook(name: &str, url: &str) -> Value {
    json!({ "name": name, "channel": { "type": "webhook", "url": url } })
}

async fn channel(env: &Env, cookie: &str, body: Value) -> i64 {
    let reply = env.send("POST", "/api/channels", Some(cookie), Some(body)).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.body);
    reply.body["id"].as_i64().unwrap()
}

/// Waits until the channel's log has `n` rows (the log is written right after the send returns).
async fn log(env: &Env, cookie: &str, id: i64, n: usize) -> Vec<Value> {
    for _ in 0..1000 {
        let reply = env.send("GET", &format!("/api/channels/{id}/log"), Some(cookie), None).await;
        let rows = reply.body.as_array().unwrap().clone();
        if rows.len() >= n {
            return rows;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("log of channel {id} never reached {n} rows");
}

#[tokio::test]
async fn crud_and_secrets() {
    let env = env().await;
    assert_eq!(env.send("GET", "/api/channels", None, None).await.status, StatusCode::UNAUTHORIZED);
    let cookie = env.admin().await;
    let c = Some(cookie.as_str());

    let mut body = webhook(" Ops ", "https://hooks.example.com/x");
    body["secrets"] = json!({ "hmac_secret": "s3cret-value" });
    let id = channel(&env, &cookie, body).await;
    let reply = env.send("GET", &format!("/api/channels/{id}"), c, None).await;
    assert_eq!(reply.body["name"], "Ops");
    assert_eq!(reply.body["secrets"], json!(["hmac_secret"]));
    assert_eq!(reply.body["events"]["version"], false);
    let list = env.send("GET", "/api/channels", c, None).await;
    assert!(!list.body.to_string().contains("s3cret"), "secrets never leave the server");

    // No `secrets` key keeps the stored value, "" deletes it.
    let path = format!("/api/channels/{id}");
    let reply = env.send("PUT", &path, c, Some(webhook("Ops", "https://hooks.example.com/y"))).await;
    assert_eq!((reply.status, reply.body["secrets"].clone()), (StatusCode::OK, json!(["hmac_secret"])));
    let mut body = webhook("Ops", "https://hooks.example.com/y");
    body["secrets"] = json!({ "hmac_secret": "" });
    assert_eq!(env.send("PUT", &path, c, Some(body)).await.body["secrets"], json!([]));

    // A type change drops secrets of the old type.
    let mut body = webhook("Ops", "https://hooks.example.com/y");
    body["secrets"] = json!({ "hmac_secret": "k" });
    env.send("PUT", &path, c, Some(body)).await;
    let body =
        json!({ "name": "Ops", "channel": { "type": "ntfy", "server": "https://ntfy.example.com", "topic": "ops" } });
    assert_eq!(env.send("PUT", &path, c, Some(body)).await.body["secrets"], json!([]));

    for bad in [
        webhook("Ops", "file:///etc/passwd"),
        webhook("", "https://hooks.example.com"),
        json!({ "name": "x", "channel": { "type": "webhook", "url": "https://hooks.example.com" }, "secrets": { "token": "x" } }),
        json!({ "name": "x", "channel": { "type": "sochat" } }),
        json!({ "name": "x", "channel": { "type": "telegram", "chat_id": "1" } }),
    ] {
        let reply = env.send("POST", "/api/channels", c, Some(bad.clone())).await;
        assert!(reply.status.is_client_error(), "{bad} → {:?}", reply.body);
    }

    // SMTP: the password is write-only.
    let smtp = json!({ "host": "smtp.example.com", "port": 587, "user": "u", "password": "pw-1", "from": "status@example.com" });
    let reply = env.send("PUT", "/api/smtp", c, Some(smtp)).await;
    assert_eq!((reply.status, reply.body["password_set"].clone()), (StatusCode::OK, json!(true)));
    let keep = json!({ "host": "smtp.example.com", "port": 465, "security": "tls", "from": "status@example.com" });
    let reply = env.send("PUT", "/api/smtp", c, Some(keep)).await;
    assert_eq!(reply.body["password_set"], true);
    assert!(!env.send("GET", "/api/smtp", c, None).await.body.to_string().contains("pw-1"));

    assert_eq!(env.send("DELETE", &path, c, None).await.status, StatusCode::NO_CONTENT);
    assert_eq!(env.send("GET", &path, c, None).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn dispatch_defaults_and_filters() {
    let env = env().await;
    let cookie = env.admin().await;
    let c = Some(cookie.as_str());
    let mut hook = hook().await;
    let mut body = webhook("Ops", &hook.url);
    body["is_default"] = json!(true);
    let ch = channel(&env, &cookie, body).await;
    uwustatus_server::alerts::spawn(&env.state);

    let mut m = tcp("Web");
    m["retries"] = json!(0);
    let id = env.monitor(&cookie, m).await["id"].as_i64().unwrap();
    let channels = env.send("GET", &format!("/api/monitors/{id}/channels"), c, None).await;
    assert_eq!(channels.body, json!([ch]), "new monitors get the default channels");

    let ingest = |at, status| ingest_result(&env.state, id, "local", None, at, &outcome(status)).unwrap();
    ingest(100, Status::Up);
    ingest(160, Status::Down);
    let hit = hook.hits.recv().await.unwrap();
    assert_eq!((hit["title"].as_str(), hit["severity"].as_str()), (Some("Web: ausgefallen"), Some("down")));
    assert_eq!(hit["link"], format!("https://status-admin.test/monitors/{id}"));
    ingest(400, Status::Up);
    let hit = hook.hits.recv().await.unwrap();
    assert_eq!(hit["title"], "Web: wieder erreichbar");
    assert!(hit["body"].as_str().unwrap().contains("Ausfall dauerte 4 min"), "{hit}");

    // Version events are off by default; a monitor in maintenance stays quiet.
    let quiet = env.monitor(&cookie, tcp("Quiet")).await["id"].as_i64().unwrap();
    env.state
        .db
        .with(|c| c.execute("UPDATE monitor_state SET state = 'maintenance' WHERE monitor_id = ?1", [quiet]))
        .unwrap();
    let send = |e| env.state.events.send(e).unwrap();
    send(Event::VersionChanged { monitor_id: id, at: 500, from: "1".into(), to: "2".into() });
    send(Event::CertExpiring { monitor_id: quiet, at: 500, days_left: 3 });
    send(Event::CertExpiring { monitor_id: id, at: 500, days_left: 7 });
    let hit = hook.hits.recv().await.unwrap();
    assert_eq!(hit["title"], "Web: Zertifikat läuft in 7 Tagen ab");

    // English channel; unknown channel ids are refused.
    let mut en = webhook("En", &hook.url);
    en["lang"] = json!("en");
    let en = channel(&env, &cookie, en).await;
    let path = format!("/api/monitors/{id}/channels");
    assert_eq!(env.send("PUT", &path, c, Some(json!([en, 999]))).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(env.send("PUT", &path, c, Some(json!([en, en]))).await.body, json!([en]));
    send(Event::Reminder { monitor_id: id, at: 4000, since: 400, reason: "timeout".into() });
    let hit = hook.hits.recv().await.unwrap();
    assert_eq!(hit["title"], "Web: still down");
    assert!(hit["body"].as_str().unwrap().starts_with("Down for 1 h 0 min"), "{hit}");
    assert_eq!(log(&env, &cookie, en, 1).await[0]["event"], "reminder");
}

#[tokio::test]
async fn retries_test_button_and_log() {
    let env = env().await;
    let cookie = env.admin().await;
    let c = Some(cookie.as_str());
    let mut hook = hook().await;
    let ch = channel(&env, &cookie, webhook("Ops", &hook.url)).await;
    env.state.alerts.configure(|o| o.backoff = vec![Duration::ZERO; 2]);
    uwustatus_server::alerts::spawn(&env.state);

    let test = format!("/api/channels/{ch}/test");
    assert_eq!(env.send("POST", &test, c, None).await.status, StatusCode::NO_CONTENT);
    assert_eq!(hook.hits.recv().await.unwrap()["title"], "UwUStatus: Testnachricht");
    *hook.status.lock().unwrap() = StatusCode::INTERNAL_SERVER_ERROR;
    let reply = env.send("POST", &test, c, None).await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY, "{:?}", reply.body);
    hook.hits.recv().await.unwrap();

    let mut m = tcp("Web");
    m["retries"] = json!(0);
    let id = env.monitor(&cookie, m).await["id"].as_i64().unwrap();
    env.send("PUT", &format!("/api/monitors/{id}/channels"), c, Some(json!([ch]))).await;
    ingest_result(&env.state, id, "local", None, 100, &outcome(Status::Up)).unwrap();
    ingest_result(&env.state, id, "local", None, 160, &outcome(Status::Down)).unwrap();
    for _ in 0..3 {
        hook.hits.recv().await.unwrap();
    }
    let rows = log(&env, &cookie, ch, 3).await;
    assert_eq!((rows[0]["event"].as_str(), rows[0]["attempts"].as_i64()), (Some("down"), Some(3)));
    assert_eq!((rows[0]["ok"].as_bool(), rows[0]["monitor_id"].as_i64()), (Some(false), Some(id)));
    assert!(rows[0]["error"].as_str().unwrap().starts_with("HTTP 500"));
    assert_eq!((rows[1]["ok"].as_bool(), rows[2]["ok"].as_bool()), (Some(false), Some(true)));

    // The log keeps the last rows only.
    for _ in 0..uwustatus_server::alerts::LOG_KEEP {
        uwustatus_server::alerts::log(&env.state, ch, None, "test", "x", 1, None);
    }
    let rows = log(&env, &cookie, ch, 1).await;
    assert_eq!(rows.len() as i64, uwustatus_server::alerts::LOG_KEEP);
    assert_eq!(env.send("GET", "/api/channels/999/log", c, None).await.status, StatusCode::NOT_FOUND);
}
