//! Every channel type against local fakes: one HTTP server that records requests, one SMTP server.

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use uwustatus_notify::{Channel, Lang, Message, Secrets, Security, Sender, Severity, Smtp, Target, signature};

#[derive(Debug)]
struct Hit {
    path: String,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Hit {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

struct Fake {
    base: String,
    hits: mpsc::UnboundedReceiver<Hit>,
    status: Arc<Mutex<StatusCode>>,
}

async fn fake() -> Fake {
    type S = (mpsc::UnboundedSender<Hit>, Arc<Mutex<StatusCode>>);
    async fn record(State((tx, status)): State<S>, uri: Uri, headers: HeaderMap, body: Bytes) -> (StatusCode, String) {
        tx.send(Hit { path: uri.to_string(), headers, body: body.to_vec() }).unwrap();
        let status = *status.lock().unwrap();
        // A big answer must not be read whole.
        (status, if status.is_success() { "{}".into() } else { "nope ".repeat(100_000) })
    }
    let (tx, hits) = mpsc::unbounded_channel();
    let status = Arc::new(Mutex::new(StatusCode::OK));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().fallback(record).with_state((tx, status.clone()));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Fake { base, hits, status }
}

fn message(severity: Severity) -> Message {
    Message {
        title: "Web: ausgefallen".into(),
        body: "Grund: Timeout".into(),
        severity,
        link: Some("https://status.example.com/monitors/1".into()),
        event: json!({ "type": "state_changed", "monitor_id": 1 }),
    }
}

fn secrets(pairs: &[(&str, &str)]) -> Secrets {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn target(channel: Channel, pairs: &[(&str, &str)]) -> Target {
    Target { channel, secrets: secrets(pairs), lang: Lang::De, smtp: None }
}

fn sender(fake: &Fake) -> Sender {
    let mut sender = Sender::new().unwrap();
    sender.pushover_api = fake.base.clone();
    sender.telegram_api = format!("{}/tg/", fake.base);
    sender
}

#[tokio::test]
async fn webhook_signed_and_errors() {
    let mut fake = fake().await;
    let s = sender(&fake);
    let t = target(Channel::Webhook { url: format!("{}/hook", fake.base) }, &[("hmac_secret", "k")]);
    s.send(&t, &message(Severity::Down)).await.unwrap();
    let hit = fake.hits.recv().await.unwrap();
    assert_eq!(hit.path, "/hook");
    assert_eq!(hit.header("x-uwustatus-signature"), Some(signature("k", &hit.body).as_str()));
    assert_eq!(hit.json()["severity"], "down");
    assert_eq!(hit.json()["event"]["type"], "state_changed");

    *fake.status.lock().unwrap() = StatusCode::INTERNAL_SERVER_ERROR;
    let e = s.send(&t, &message(Severity::Down)).await.unwrap_err();
    assert!(e.0.starts_with("HTTP 500: nope") && e.0.len() < 260, "{e}");
    fake.hits.recv().await.unwrap();

    let unsigned = target(Channel::Webhook { url: format!("{}/hook", fake.base) }, &[]);
    *fake.status.lock().unwrap() = StatusCode::OK;
    s.send(&unsigned, &message(Severity::Up)).await.unwrap();
    assert!(fake.hits.recv().await.unwrap().header("x-uwustatus-signature").is_none());
}

#[tokio::test]
async fn push_services() {
    let mut fake = fake().await;
    let s = sender(&fake);
    let down = message(Severity::Down);

    let ntfy = Channel::Ntfy { server: fake.base.clone(), topic: "alerts".into(), priority: 5 };
    s.send(&target(ntfy, &[("token", "tk")]), &down).await.unwrap();
    let hit = fake.hits.recv().await.unwrap();
    assert_eq!((hit.path.as_str(), hit.header("authorization")), ("/", Some("Bearer tk")));
    let body = hit.json();
    assert_eq!((body["topic"].clone(), body["priority"].clone()), (json!("alerts"), json!(5)));
    assert_eq!(body["click"], "https://status.example.com/monitors/1");

    let gotify = Channel::Gotify { url: format!("{}/g/", fake.base), priority: 9 };
    s.send(&target(gotify, &[("token", "app")]), &message(Severity::Up)).await.unwrap();
    let hit = fake.hits.recv().await.unwrap();
    assert_eq!((hit.path.as_str(), hit.header("x-gotify-key")), ("/g/message", Some("app")));
    assert_eq!(hit.json()["priority"], 4, "recovery is not urgent");

    s.send(&target(Channel::Pushover { priority: 1 }, &[("user_key", "u"), ("token", "t")]), &down).await.unwrap();
    let hit = fake.hits.recv().await.unwrap();
    assert_eq!(hit.path, "/1/messages.json");
    let form = String::from_utf8(hit.body).unwrap();
    assert!(form.contains("token=t") && form.contains("user=u") && form.contains("priority=1"), "{form}");

    let discord = target(Channel::Discord {}, &[("webhook_url", &format!("{}/api/webhooks/1/x", fake.base))]);
    s.send(&discord, &down).await.unwrap();
    let hit = fake.hits.recv().await.unwrap();
    assert_eq!(hit.path, "/api/webhooks/1/x");
    assert_eq!(hit.json()["embeds"][0]["color"], 0xe5484d);
    assert_eq!(hit.json()["allowed_mentions"]["parse"], json!([]));

    let telegram = target(Channel::Telegram { chat_id: "-100".into() }, &[("bot_token", "123:abc")]);
    s.send(&telegram, &down).await.unwrap();
    let hit = fake.hits.recv().await.unwrap();
    assert_eq!(hit.path, "/tg/bot123:abc/sendMessage");
    assert_eq!(hit.json()["chat_id"], "-100");
    assert!(hit.json()["text"].as_str().unwrap().starts_with("Web: ausgefallen\n\nGrund: Timeout"));

    // A failing Telegram call must not leak the token into the error text.
    *fake.status.lock().unwrap() = StatusCode::NOT_FOUND;
    let e = s.send(&telegram, &down).await.unwrap_err();
    assert!(!e.0.contains("123:abc"), "{e}");
}

#[tokio::test]
async fn validation() {
    let bad = [
        target(Channel::Webhook { url: "file:///etc/passwd".into() }, &[]),
        target(Channel::Webhook { url: "http://192.0.2.1/".into() }, &[("token", "x")]),
        target(Channel::Ntfy { server: "https://ntfy.example.com".into(), topic: "a/b".into(), priority: 3 }, &[]),
        target(Channel::Gotify { url: "https://gotify.example.com".into(), priority: 5 }, &[]),
        target(Channel::Pushover { priority: 2 }, &[("user_key", "u"), ("token", "t")]),
        target(Channel::Discord {}, &[("webhook_url", "ftp://discord.example.com")]),
        target(Channel::Telegram { chat_id: "1".into() }, &[("bot_token", "a/../b")]),
        target(Channel::Email { to: vec!["nobody".into()] }, &[]),
    ];
    for t in bad {
        assert!(t.channel.validate(&t.secrets).is_err(), "{:?}", t.channel);
    }
    // Unreachable host: an error, not a panic.
    let t = target(Channel::Webhook { url: "http://127.0.0.1:1/".into() }, &[]);
    assert!(Sender::new().unwrap().send(&t, &message(Severity::Info)).await.is_err());
    // Email without SMTP settings.
    let t = target(Channel::Email { to: vec!["ops@example.com".into()] }, &[]);
    assert_eq!(
        Sender::new().unwrap().send(&t, &message(Severity::Info)).await.unwrap_err().0,
        "SMTP ist nicht eingerichtet"
    );
}

/// Minimal SMTP server: accepts one mail and hands over its DATA.
async fn fake_smtp() -> (u16, mpsc::UnboundedReceiver<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let (read, mut write) = socket.into_split();
        let mut lines = BufReader::new(read).lines();
        write.write_all(b"220 fake ESMTP\r\n").await.unwrap();
        let mut data: Option<String> = None;
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(d) = data.as_mut() {
                if line == "." {
                    tx.send(data.take().unwrap()).unwrap();
                    write.write_all(b"250 queued\r\n").await.unwrap();
                } else {
                    d.push_str(&line);
                    d.push('\n');
                }
                continue;
            }
            let reply: &[u8] = match line.get(..4).unwrap_or_default().to_ascii_uppercase().as_str() {
                "EHLO" => b"250-fake\r\n250 AUTH PLAIN\r\n",
                "AUTH" => b"235 ok\r\n",
                "DATA" => {
                    data = Some(String::new());
                    b"354 go\r\n"
                }
                "QUIT" => {
                    write.write_all(b"221 bye\r\n").await.unwrap();
                    break;
                }
                _ => b"250 ok\r\n",
            };
            write.write_all(reply).await.unwrap();
        }
    });
    (port, rx)
}

#[tokio::test]
async fn email() {
    let (port, mut mails) = fake_smtp().await;
    let smtp = Smtp {
        host: "127.0.0.1".into(),
        port,
        security: Security::None,
        user: "status".into(),
        password: "pw".into(),
        from: "UwUStatus <status@example.com>".into(),
    };
    let t = Target {
        channel: Channel::Email { to: vec!["ops@example.com".into()] },
        secrets: Secrets::new(),
        lang: Lang::En,
        smtp: Some(smtp),
    };
    // The target survives a round trip (it travels to satellites).
    let t: Target = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
    Sender::new().unwrap().send(&t, &message(Severity::Down)).await.unwrap();
    let mail = mails.recv().await.unwrap();
    assert!(mail.contains("To: ops@example.com"), "{mail}");
    assert!(mail.contains("Subject: Web: ausgefallen"), "{mail}");
    assert!(mail.contains("https://status.example.com/monitors/1"), "{mail}");
}
