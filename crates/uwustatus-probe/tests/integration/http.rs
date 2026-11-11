use std::collections::BTreeMap;

use uwustatus_probe::{CheckKind, DockerCheck, HealthzCheck, HttpCheck, JsonMatch, MAX_BODY, Status};

use crate::{Request, check, http_server, resp, silent_server};

fn router(req: &Request) -> Vec<u8> {
    match req.path.as_str() {
        "/ok" => resp(200, "", "hello uwu"),
        "/fail" => resp(503, "", "nope"),
        "/redirect" => resp(302, "Location: /ok\r\n", ""),
        "/json" => resp(200, "", r#"{"data":{"items":[{"state":"green","n":42}]}}"#),
        "/echo" => {
            let auth = req.headers.iter().any(|(k, v)| k == "x-token" && v == "s3cret");
            match (req.method.as_str(), auth, req.body.as_str()) {
                ("POST", true, "ping") => resp(201, "", "pong"),
                _ => resp(400, "", "bad"),
            }
        }
        "/big" => {
            let mut body = "a".repeat(MAX_BODY + 10);
            body.push_str("needle");
            resp(200, "", &body)
        }
        _ => resp(404, "", ""),
    }
}

fn http(url: String) -> HttpCheck {
    HttpCheck { url, ..HttpCheck::default() }
}

async fn run_http(c: HttpCheck) -> uwustatus_probe::Outcome {
    check(5_000, CheckKind::Http(c)).await
}

#[tokio::test]
async fn status_codes() {
    let base = http_server(false, router).await;
    let o = run_http(http(format!("{base}/ok"))).await;
    assert_eq!((o.status, o.details.http_status, o.message.as_str()), (Status::Up, Some(200), "HTTP 200"));
    assert!(o.details.cert_days_left.is_none());

    let o = run_http(http(format!("{base}/fail"))).await;
    assert_eq!((o.status, o.message.as_str()), (Status::Down, "HTTP 503"));

    let o = run_http(HttpCheck { accepted_status: vec!["500-599".into()], ..http(format!("{base}/fail")) }).await;
    assert_eq!(o.status, Status::Up);

    let o = run_http(HttpCheck { accepted_status: vec!["5xx".into()], ..http(format!("{base}/ok")) }).await;
    assert_eq!(o.status, Status::Down);
    assert!(o.message.contains("invalid status"), "{}", o.message);
}

#[tokio::test]
async fn redirects() {
    let base = http_server(false, router).await;
    let o = run_http(http(format!("{base}/redirect"))).await;
    assert_eq!((o.status, o.details.http_status), (Status::Up, Some(200)));
    let o = run_http(HttpCheck { follow_redirects: false, ..http(format!("{base}/redirect")) }).await;
    assert_eq!((o.status, o.details.http_status), (Status::Down, Some(302)));
}

#[tokio::test]
async fn method_headers_body() {
    let base = http_server(false, router).await;
    let c = HttpCheck {
        method: "post".into(),
        headers: BTreeMap::from([("X-Token".into(), "s3cret".into())]),
        body: Some("ping".into()),
        keyword: Some("pong".into()),
        ..http(format!("{base}/echo"))
    };
    let o = run_http(c).await;
    assert_eq!((o.status, o.details.http_status), (Status::Up, Some(201)), "{}", o.message);
    let o = run_http(HttpCheck { method: "BAD METHOD".into(), ..http(format!("{base}/ok")) }).await;
    assert_eq!(o.status, Status::Down);
}

#[tokio::test]
async fn keyword() {
    let base = http_server(false, router).await;
    let kw =
        |k: &str, absent| HttpCheck { keyword: Some(k.into()), keyword_absent: absent, ..http(format!("{base}/ok")) };
    assert_eq!(run_http(kw("uwu", false)).await.status, Status::Up);
    assert_eq!(run_http(kw("owo", false)).await.status, Status::Down);
    assert_eq!(run_http(kw("owo", true)).await.status, Status::Up);
    let o = run_http(kw("uwu", true)).await;
    assert_eq!(o.status, Status::Down);
    assert!(o.message.contains("found"), "{}", o.message);
    // Hinter dem Limit wird nichts mehr gelesen.
    let o = run_http(HttpCheck { keyword: Some("needle".into()), ..http(format!("{base}/big")) }).await;
    assert_eq!(o.status, Status::Down);
}

#[tokio::test]
async fn json_path() {
    let base = http_server(false, router).await;
    let j = |path: &str, value: &str| HttpCheck {
        json_path: Some(JsonMatch { path: path.into(), value: value.into() }),
        ..http(format!("{base}/json"))
    };
    assert_eq!(run_http(j("data.items.0.state", "green")).await.status, Status::Up);
    assert_eq!(run_http(j("data.items.0.n", "42")).await.status, Status::Up);
    let o = run_http(j("data.items.0.state", "red")).await;
    assert_eq!(o.status, Status::Down);
    assert!(o.message.contains("\"green\""), "{}", o.message);
    let o = run_http(j("data.missing", "x")).await;
    assert!(o.message.contains("missing"), "{}", o.message);
    let o = run_http(HttpCheck { url: format!("{base}/ok"), ..j("a", "b") }).await;
    assert!(o.message.contains("invalid JSON"), "{}", o.message);
}

#[tokio::test]
async fn tls_cert_and_verification() {
    let base = http_server(true, router).await;
    let o = run_http(HttpCheck { ignore_tls_errors: true, ..http(format!("{base}/ok")) }).await;
    assert_eq!(o.status, Status::Up, "{}", o.message);
    assert_eq!(o.details.cert_expires_at.unwrap().to_string(), "2040-05-06T00:00:00Z");
    assert!(o.details.cert_days_left.unwrap() > 365);

    let o = run_http(http(format!("{base}/ok"))).await;
    assert_eq!(o.status, Status::Down);
    assert!(o.message.to_lowercase().contains("certificate"), "{}", o.message);
}

#[tokio::test]
async fn timeout_and_refused() {
    let port = silent_server().await;
    let o = check(200, CheckKind::Http(http(format!("http://127.0.0.1:{port}/")))).await;
    assert_eq!((o.status, o.message.as_str()), (Status::Down, "timed out after 200 ms"));
    assert!(o.latency_ms >= 200);

    let o = run_http(http("http://127.0.0.1:1/".into())).await;
    assert_eq!(o.status, Status::Down);
    let o = run_http(http("not a url".into())).await;
    assert_eq!(o.status, Status::Down);
}

fn docker_router(req: &Request) -> Vec<u8> {
    let state = match req.path.as_str() {
        "/containers/web/json" => r#"{"State":{"Status":"running","Health":{"Status":"healthy"}}}"#,
        "/containers/plain/json" => r#"{"State":{"Status":"running"}}"#,
        "/containers/boot/json" => r#"{"State":{"Status":"running","Health":{"Status":"starting"}}}"#,
        "/containers/sick/json" => r#"{"State":{"Status":"running","Health":{"Status":"unhealthy"}}}"#,
        "/containers/dead/json" => r#"{"State":{"Status":"exited","ExitCode":137}}"#,
        "/containers/junk/json" => "{]",
        _ => return resp(404, "", r#"{"message":"No such container"}"#),
    };
    resp(200, "", state)
}

#[tokio::test]
async fn docker() {
    let base = http_server(false, docker_router).await;
    let d =
        |name: &str| check(5_000, CheckKind::Docker(DockerCheck { url: format!("{base}/"), container: name.into() }));
    let o = d("web").await;
    assert_eq!((o.status, o.details.container_health.as_deref()), (Status::Up, Some("healthy")));
    assert_eq!(d("plain").await.status, Status::Up);
    assert_eq!(d("boot").await.status, Status::Degraded);
    assert_eq!(d("sick").await.status, Status::Down);
    let o = d("dead").await;
    assert_eq!((o.status, o.message.as_str()), (Status::Down, "exited (exit code 137)"));
    assert_eq!(d("missing").await.message, "container missing not found");
    assert_eq!(d("junk").await.status, Status::Down);
    let o = d("../../etc").await;
    assert!(o.message.starts_with("invalid container name"), "{}", o.message);
}

fn healthz_router(req: &Request) -> Vec<u8> {
    match req.path.as_str() {
        "/ok" => resp(200, "", r#"{"status":"ok","version":"1.2.3","checks":{"db":{"status":"ok"}}}"#),
        "/degraded" => resp(
            200,
            "",
            r#"{"status":"degraded","version":"1.2.3","checks":{"db":{"status":"ok"},"smtp":{"status":"degraded","message":"queue slow"}}}"#,
        ),
        "/down" => resp(503, "", r#"{"status":"down","checks":{"db":{"status":"down","message":"no connection"}}}"#),
        "/weird" => resp(200, "", r#"{"status":"sideways"}"#),
        _ => resp(502, "", "<html>bad gateway</html>"),
    }
}

#[tokio::test]
async fn healthz() {
    let base = http_server(false, healthz_router).await;
    let h = |p: &str| check(5_000, CheckKind::Healthz(HealthzCheck { url: format!("{base}{p}") }));
    let o = h("/ok").await;
    assert_eq!((o.status, o.message.as_str(), o.details.version.as_deref()), (Status::Up, "ok", Some("1.2.3")));
    let o = h("/degraded").await;
    assert_eq!((o.status, o.message.as_str()), (Status::Degraded, "degraded: smtp: degraded (queue slow)"));
    assert_eq!(o.details.checks["smtp"].message, "queue slow");
    let o = h("/down").await;
    assert_eq!((o.status, o.message.as_str()), (Status::Down, "down: db: down (no connection)"));
    assert_eq!(h("/weird").await.status, Status::Down);
    assert_eq!(h("/html").await.message, "HTTP 502, no healthz JSON");
}

#[tokio::test]
async fn error_message_leaves_out_the_url() {
    let (closed, port) = crate::listener().await;
    drop(closed);
    let o = run_http(http(format!("http://127.0.0.1:{port}/hook?token=hidden-value"))).await;
    assert_eq!(o.status, Status::Down);
    assert!(!o.message.contains("hidden-value"), "{}", o.message);
}
