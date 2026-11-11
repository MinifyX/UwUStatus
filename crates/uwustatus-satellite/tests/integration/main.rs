//! The satellite against a real tunnel on 127.0.0.1, with a fake webhook and a fake checked service.

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::routing::{get, post};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use uwustatus_satellite::agent::{self, Agent};
use uwustatus_satellite::config::Config;
use uwustatus_satellite::deadman::NotifyNotifier;
use uwustatus_satellite::state::{Saved, Store};
use uwustatus_tunnel::{Assignment, Identity, MainClient, MainEvent, MainSettings, PairingCode, Timing, Token};

const WAIT: Duration = Duration::from_secs(10);

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

async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(WAIT, future).await.expect("timed out")
}

async fn until(mut condition: impl FnMut() -> bool) {
    within(async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
}

async fn serve(app: Router) -> SocketAddr {
    let tcp = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = tcp.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(tcp, app).await.unwrap() });
    address
}

/// A satellite with a waiting pairing code, as after `init`.
async fn satellite(dir: &std::path::Path, page: bool) -> (Agent, PairingCode) {
    Store::open(dir).unwrap().save(&Saved { token: Some(Token::generate().to_text()), ..Saved::default() }).unwrap();
    let mut config =
        Config { listen: (Ipv4Addr::LOCALHOST, 0).into(), state_dir: dir.to_path_buf(), ..Config::default() };
    if page {
        config.emergency.listen = Some((Ipv4Addr::LOCALHOST, 0).into());
    }
    let mut notifier = NotifyNotifier::new().unwrap();
    notifier.retry = [Duration::from_millis(10); 2];
    let agent = agent::start(&config, timing(), Arc::new(notifier)).await.unwrap();
    let code = agent.listener.pairing_code(&[Ipv4Addr::LOCALHOST.into()]).unwrap();
    (agent, PairingCode::parse(&code).unwrap())
}

fn main_client(mut settings: MainSettings) -> (MainClient, mpsc::UnboundedReceiver<MainEvent>) {
    settings.timing = timing();
    MainClient::start(settings)
}

async fn connected(events: &mut mpsc::UnboundedReceiver<MainEvent>) {
    within(async { while !matches!(events.recv().await, Some(MainEvent::Connected { .. })) {} }).await
}

#[tokio::test]
async fn dead_man_alarms_after_the_deadline_and_gives_the_all_clear() {
    let (hooks, mut hooked) = mpsc::unbounded_channel::<Value>();
    let hook = serve(
        Router::new()
            .route(
                "/hook",
                post(|State(tx): State<mpsc::UnboundedSender<Value>>, Json(body): Json<Value>| async move {
                    tx.send(body).unwrap();
                }),
            )
            .with_state(hooks),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let (agent, code) = satellite(dir.path(), false).await;
    let identity = Identity::generate().unwrap();
    let (main, mut events) = main_client(MainSettings::from_code(&code, identity.clone()));
    main.dead_man(json!({
        "name": "Frankfurt",
        "after_s": 1,
        "channels": [{ "channel": { "type": "webhook", "url": format!("http://{hook}/hook") } }],
    }));
    connected(&mut events).await;
    let store = Store::open(dir.path()).unwrap();
    until(|| store.saved().unwrap().dead_man.is_some_and(|d| d.after_s == 1)).await;
    assert!(store.saved().unwrap().main.is_some(), "paired and stored");

    main.stop();
    let alarm = within(hooked.recv()).await.unwrap();
    assert_eq!(alarm["event"]["state"], "alarm");
    assert_eq!(alarm["event"]["satellite"], "Frankfurt");
    assert!(alarm["body"].as_str().unwrap().contains("Frankfurt"));
    assert!(store.saved().unwrap().alarmed);

    let (_main, mut events) = main_client(MainSettings::pinned(code.addresses, code.fingerprint, identity));
    connected(&mut events).await;
    let ok = within(hooked.recv()).await.unwrap();
    assert_eq!(ok["event"]["state"], "ok");
    until(|| !store.saved().unwrap().alarmed).await;
    drop(agent);
}

#[tokio::test]
async fn emergency_page_shows_own_results() {
    let service = serve(Router::new().route("/", get(|| async { "fine" }))).await;
    let dir = tempfile::tempdir().unwrap();
    let (agent, code) = satellite(dir.path(), true).await;
    let page = format!("http://{}/", agent.page.unwrap());
    let client = reqwest::Client::new();
    assert_eq!(client.get(&page).send().await.unwrap().status(), 404, "off until the main instance says so");

    let (main, mut events) = main_client(MainSettings::from_code(&code, Identity::generate().unwrap()));
    main.assign(vec![Assignment {
        id: "7".into(),
        interval_s: 60,
        check: json!({ "type": "http", "url": format!("http://{service}/"), "timeout_ms": 2000 }),
    }]);
    main.dead_man(json!({
        "name": "Frankfurt",
        "page": { "enabled": true, "title": "Beispiel <Status>", "domain": "status.example.com",
                  "monitors": [{ "id": "7", "name": "Website" }, { "id": "8", "name": "Mail" }] },
    }));
    connected(&mut events).await;
    until(|| agent.live.get().latest.contains_key("7")).await;
    main.stop();
    until(|| !agent.live.get().connected).await;

    let wrong_host = client.get(&page).send().await.unwrap();
    assert_eq!(wrong_host.status(), 404, "only for the configured domain");
    let reply = client.get(&page).header("host", "status.example.com:443").send().await.unwrap();
    assert_eq!(reply.status(), 200);
    assert!(reply.headers()["content-security-policy"].to_str().unwrap().starts_with("default-src 'none'"));
    assert!(reply.headers().get("set-cookie").is_none());
    let html = reply.text().await.unwrap();
    assert!(html.contains("Hauptinstanz nicht erreichbar seit"), "{html}");
    assert!(html.contains("Beispiel &lt;Status&gt;"));
    assert!(html.contains("Website</span><span class=\"state ok\">in Betrieb"), "{html}");
    assert!(html.contains("Mail</span><span class=\"state none\">keine Daten"));
    assert!(!html.contains("http://") && !html.contains("https://"), "nothing from elsewhere");
    let english = client.get(&page).header("host", "status.example.com").header("accept-language", "en-GB,en;q=0.8");
    assert!(english.send().await.unwrap().text().await.unwrap().contains("Main instance unreachable since"));
}

#[tokio::test(start_paused = true)]
async fn emergency_page_drops_a_client_that_never_finishes_its_headers() {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let tcp = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = tcp.local_addr().unwrap();
    tokio::spawn(uwustatus_satellite::page::serve(tcp, Router::new().route("/", get(|| async { "hi" }))));
    let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
    client.write_all(b"GET / HTTP/1.1\r\nHost: status.example.com\r\n").await.unwrap();
    let mut rest = Vec::new();
    // Paused time jumps to the header timeout once everything waits; without one this never ends.
    tokio::time::timeout(Duration::from_secs(30), client.read_to_end(&mut rest)).await.expect("still open").unwrap();
}
