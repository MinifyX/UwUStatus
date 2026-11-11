//! Phase 5: status pages (host dispatch, rendering, feeds), incidents, badges, tokens, metrics.

use crate::monitors::{outcome, tcp};
use crate::support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use uwustatus_probe::Status;
use uwustatus_server::engine::State as MonState;
use uwustatus_server::events::Event;
use uwustatus_server::incidents;
use uwustatus_server::ingest::ingest_result;

/// 2026-01-01T00:00:00Z
const T0: i64 = 1_767_225_600;
const PAGE_HOST: &str = "status.example.com";

impl Env {
    /// GET on another host (the public status pages).
    async fn get_on(&self, host: &str, uri: &str, headers: &[(&str, &str)]) -> Reply {
        let mut request = Request::builder().uri(uri).header(header::HOST, host);
        for (k, v) in headers {
            request = request.header(*k, *v);
        }
        self.raw(request.body(Body::empty()).unwrap()).await
    }
}

fn text(reply: &Reply) -> &str {
    reply.body.as_str().unwrap_or_default()
}

/// Two monitors, one page on `PAGE_HOST` showing only the first.
async fn setup(env: &Env) -> (String, i64, i64) {
    let cookie = env.admin().await;
    let web = env.monitor(&cookie, tcp("Web")).await["id"].as_i64().unwrap();
    let db = env.monitor(&cookie, tcp("DB")).await["id"].as_i64().unwrap();
    let page = json!({
        "slug": "main", "title": "Example <Status>", "description": "All our services",
        "domains": ["Status.Example.com."], "accent": "mint",
        "groups": [{ "name": "Core", "monitors": [{ "id": web, "name": "Website", "show_latency": true }] }],
    });
    let reply = env.send("POST", "/api/pages", Some(&cookie), Some(page)).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.body);
    assert_eq!(reply.body["domains"], json!([PAGE_HOST]));
    (cookie, web, db)
}

fn down(id: i64, at: i64, via_parent: bool) -> Event {
    Event::StateChanged {
        monitor_id: id,
        at,
        from: MonState::Up,
        to: MonState::Down,
        reason: String::new(),
        unreachable_via_parent: via_parent,
        alert: true,
    }
}

#[tokio::test]
async fn page_crud_validation_and_host_dispatch() {
    let env = env().await;
    let (cookie, web, _) = setup(&env).await;
    for bad in [
        json!({ "slug": "Bad Slug", "title": "x" }),
        json!({ "slug": "x", "title": "" }),
        json!({ "slug": "x", "title": "x", "domains": [HOST] }),
        json!({ "slug": "x", "title": "x", "domains": [PAGE_HOST] }),
        json!({ "slug": "x", "title": "x", "domains": ["not a host"] }),
        json!({ "slug": "x", "title": "x", "timezone": "Mars/Base" }),
        json!({ "slug": "x", "title": "x", "groups": [{ "name": "g", "monitors": [{ "id": 999 }] }] }),
        json!({ "slug": "main", "title": "x" }),
    ] {
        let reply = env.send("POST", "/api/pages", Some(&cookie), Some(bad.clone())).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{bad}: {:?}", reply.body);
    }

    // The page host gets the page, never the admin app or its API.
    let page = env.get_on("status.example.com:443", "/", &[]).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(text(&page).contains("Example &lt;Status&gt;"));
    assert_eq!(env.get_on(PAGE_HOST, "/api/me", &[]).await.status, StatusCode::NOT_FOUND);
    assert_eq!(env.get_on("other.example.net", "/", &[]).await.status, StatusCode::NOT_FOUND);
    // Admin host: SPA, and the preview only with a login.
    assert_eq!(env.send("GET", "/s/main/", None, None).await.status, StatusCode::UNAUTHORIZED);
    let preview = env.send("GET", "/s/main", Some(&cookie), None).await;
    assert_eq!(preview.status, StatusCode::OK);
    assert!(text(&preview).contains("href=\"/s/main/_status/status.css\""));
    assert_eq!(preview.headers[header::CACHE_CONTROL], "private, no-store");
    // Preview is not reachable from a page host.
    assert_ne!(env.get_on(PAGE_HOST, "/s/main", &[]).await.status, StatusCode::OK);

    let list = env.send("GET", "/api/pages", Some(&cookie), None).await;
    let id = list.body[0]["id"].as_i64().unwrap();
    let mut edit = list.body[0].clone();
    for key in ["id", "has_logo", "preview_url", "created", "updated"] {
        edit.as_object_mut().unwrap().remove(key);
    }
    edit["title"] = json!("Renamed");
    edit["groups"][0]["monitors"][0]["id"] = json!(web);
    let put = env.send("PUT", &format!("/api/pages/{id}"), Some(&cookie), Some(edit)).await;
    assert_eq!((put.status, put.body["title"].as_str()), (StatusCode::OK, Some("Renamed")), "{:?}", put.body);
    assert_eq!(
        env.send("DELETE", &format!("/api/pages/{id}"), Some(&cookie), None).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(env.get_on(PAGE_HOST, "/", &[]).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test(start_paused = true)]
async fn render_is_self_contained_and_accessible() {
    let env = env_at(T0 + 90 * 86_400).await;
    let (_, web, _) = setup(&env).await;
    let now = T0 + 90 * 86_400;
    ingest_result(&env.state, web, "local", None, now - 2 * 86_400, &outcome(Status::Up)).unwrap();
    ingest_result(&env.state, web, "local", None, now - 86_400, &outcome(Status::Down)).unwrap();
    ingest_result(&env.state, web, "local", None, now - 86_400 + 600, &outcome(Status::Up)).unwrap();

    let page = env.get_on(PAGE_HOST, "/", &[]).await;
    let csp = page.headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
    assert!(csp.contains("default-src 'none'") && csp.contains("script-src 'self'"), "{csp}");
    assert!(page.headers.get(header::SET_COOKIE).is_none());
    let html = text(&page);
    for part in [
        "<html lang=\"de\">",
        "class=\"accent-mint\"",
        "href=\"#main\"",
        "Alle Systeme laufen",
        "Website",
        "Antwortzeit: 20 ms",
        "class=\"pill s-up\">Betriebsbereit",
        "aria-label=\"Verfügbarkeit der letzten 90 Tage: Website\"",
        "class=\"d-down\"",
        "Ausfall: 10 min",
        "/feed.atom",
    ] {
        assert!(html.contains(part), "missing {part}");
    }
    assert_eq!(html.matches("<li class=\"d-").count(), 90);
    assert_eq!(html.matches("tabindex=\"0\"").count(), 1);
    assert!(!html.contains(">DB<"), "monitor not on the page leaked");
    // No outside resources: no absolute URLs at all (SVG namespaces would be the only excuse).
    assert!(!html.contains("http"), "external URL in page");

    // Accept-Language only with auto_lang.
    let en = env.get_on(PAGE_HOST, "/", &[("accept-language", "en-US,en;q=0.9")]).await;
    assert!(text(&en).contains("<html lang=\"de\">"));

    for (file, kind) in [("status.css", "text/css"), ("status.js", "text/javascript"), ("uwu-sans.woff2", "font/woff2")]
    {
        let asset = env.get_on(PAGE_HOST, &format!("/_status/{file}"), &[]).await;
        assert_eq!(asset.status, StatusCode::OK);
        assert!(asset.headers[header::CONTENT_TYPE].to_str().unwrap().starts_with(kind));
    }
    let css = env.get_on(PAGE_HOST, "/_status/status.css", &[]).await;
    assert!(!text(&css).contains("http"));
}

#[tokio::test]
async fn auto_lang_and_nyu_off() {
    let env = env().await;
    let cookie = env.admin().await;
    let page = json!({ "slug": "en", "title": "T", "domains": [PAGE_HOST], "auto_lang": true, "nyu": false });
    assert_eq!(env.send("POST", "/api/pages", Some(&cookie), Some(page)).await.status, StatusCode::CREATED);
    let reply = env.get_on(PAGE_HOST, "/", &[("accept-language", "fr;q=1, en;q=0.8, de;q=0.5")]).await;
    assert_eq!(reply.headers[header::VARY], "Accept-Language");
    let html = text(&reply);
    assert!(html.contains("<html lang=\"en\">") && html.contains("All systems operational"));
    assert!(html.contains("class=\"dot\"") && !html.contains("<svg"));
}

#[tokio::test(start_paused = true)]
async fn incident_automation() {
    let env = env_at(T0).await;
    let cookie = env.admin().await;
    let router = env.monitor(&cookie, tcp("Router")).await["id"].as_i64().unwrap();
    let mut child = tcp("NAS");
    child["parent_id"] = json!(router);
    let nas = env.monitor(&cookie, child).await["id"].as_i64().unwrap();
    let db = &env.state.db;

    db.with(|c| incidents::handle(c, &down(router, T0, false))).unwrap();
    // Down again (or degraded) does not open a second one.
    db.with(|c| incidents::handle(c, &down(router, T0 + 5, false))).unwrap();
    db.with(|c| incidents::handle(c, &down(nas, T0 + 10, true))).unwrap();
    let open = db.with(|c| incidents::all(c, true, 10)).unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!((open[0].monitors.clone(), open[0].status.as_str()), (vec![router, nas], "investigating"));

    let list = env.send("GET", "/api/incidents?status=open", Some(&cookie), None).await;
    assert_eq!(list.body[0]["display_title"], "Ausfall: Router");

    let up = Event::StateChanged {
        monitor_id: router,
        at: T0 + 600,
        from: MonState::Down,
        to: MonState::Up,
        reason: String::new(),
        unreachable_via_parent: false,
        alert: true,
    };
    db.with(|c| incidents::handle(c, &up)).unwrap();
    let all = db.with(|c| incidents::all(c, false, 10)).unwrap();
    assert_eq!((all[0].status.as_str(), all[0].resolved), ("resolved", Some(T0 + 600)));
    assert_eq!(all[0].updates.len(), 2);

    // The listener on the bus does the same for real state changes.
    incidents::spawn(&env.state);
    for status in [Status::Down, Status::Down, Status::Down] {
        ingest_result(&env.state, router, "local", None, T0 + 700, &outcome(status)).unwrap();
    }
    let mut opened = false;
    for _ in 0..100 {
        tokio::task::yield_now().await;
        if db.with(|c| incidents::all(c, true, 10)).unwrap().len() == 1 {
            opened = true;
            break;
        }
    }
    assert!(opened, "listener did not open an incident");
}

#[tokio::test(start_paused = true)]
async fn manual_incident_on_page_and_feeds() {
    let env = env_at(T0).await;
    let (cookie, web, db) = setup(&env).await;
    let new = json!({ "title": "Slow & <odd>", "body": "Looking into it", "impact": "degraded", "monitors": [web] });
    let created = env.send("POST", "/api/incidents", Some(&cookie), Some(new)).await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
    let id = created.body["id"].as_i64().unwrap();
    let hidden = json!({ "title": "Internal", "monitors": [db] });
    assert_eq!(env.send("POST", "/api/incidents", Some(&cookie), Some(hidden)).await.status, StatusCode::CREATED);
    let bad = json!({ "title": "x", "status": "weird", "monitors": [] });
    assert_eq!(env.send("POST", "/api/incidents", Some(&cookie), Some(bad)).await.status, StatusCode::BAD_REQUEST);

    let html = env.get_on(PAGE_HOST, "/", &[]).await;
    let html = text(&html);
    assert!(html.contains("Slow &amp; &lt;odd&gt;") && html.contains("Looking into it"));
    assert!(html.contains("Aktuelle Vorfälle") && !html.contains("Internal"));

    let update = json!({ "status": "resolved", "body": "Fixed" });
    let reply = env.send("POST", &format!("/api/incidents/{id}/updates"), Some(&cookie), Some(update)).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    assert_eq!(reply.body["status"], "resolved");
    let history = env.get_on(PAGE_HOST, "/history", &[]).await;
    assert!(text(&history).contains(&format!("id=\"incident-{id}\"")));
    assert!(!text(&env.get_on(PAGE_HOST, "/", &[]).await).contains("Aktuelle Vorfälle"));

    let atom = env.get_on(PAGE_HOST, "/feed.atom", &[]).await;
    assert!(atom.headers[header::CONTENT_TYPE].to_str().unwrap().starts_with("application/atom+xml"));
    let doc = roxmltree::Document::parse(text(&atom)).expect("atom parses");
    let root = doc.root_element();
    assert_eq!((root.tag_name().name(), root.tag_name().namespace()), ("feed", Some("http://www.w3.org/2005/Atom")));
    let entries: Vec<_> = root.children().filter(|n| n.has_tag_name("entry")).collect();
    assert_eq!(entries.len(), 1);
    let entry_id = entries[0].children().find(|n| n.has_tag_name("id")).unwrap().text().unwrap();
    assert_eq!(entry_id, format!("https://{PAGE_HOST}/history#incident-{id}"));
    assert!(root.children().any(|n| n.has_tag_name("updated")));

    let rss = env.get_on(PAGE_HOST, "/feed.rss", &[]).await;
    let doc = roxmltree::Document::parse(text(&rss)).expect("rss parses");
    let items: Vec<_> = doc.descendants().filter(|n| n.has_tag_name("item")).collect();
    assert_eq!(items.len(), 1);
    let date = items[0].children().find(|n| n.has_tag_name("pubDate")).unwrap().text().unwrap();
    assert!(date.starts_with("Thu, 01 Jan 2026 00:00:00 +0000"), "{date}");
}

#[tokio::test]
async fn badges_only_for_public_monitors() {
    let env = env().await;
    let (cookie, web, db) = setup(&env).await;
    let on_page = env.send("GET", &format!("/api/monitors/{web}/badge"), Some(&cookie), None).await;
    assert_eq!(on_page.body["on_page"], true);
    let pid = on_page.body["public_id"].as_str().unwrap().to_owned();
    assert!(pid.len() >= 16 && pid != web.to_string());

    let status = env.get_on("anything.example.org", &format!("/badge/{pid}/status.svg"), &[]).await;
    assert_eq!(status.status, StatusCode::OK);
    assert!(status.headers[header::CONTENT_TYPE].to_str().unwrap().starts_with("image/svg+xml"));
    assert!(text(&status).contains("aria-label=\"status: pending\""));
    let uptime = env.get_on(PAGE_HOST, &format!("/badge/{pid}/uptime.svg?days=7"), &[]).await;
    assert!(text(&uptime).contains("uptime 7d"));
    assert_eq!(
        env.get_on(PAGE_HOST, &format!("/badge/{pid}/uptime.svg?days=0"), &[]).await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(env.get_on(PAGE_HOST, &format!("/badge/{pid}/latency.svg"), &[]).await.status, StatusCode::OK);
    assert_eq!(env.get_on(PAGE_HOST, &format!("/badge/{pid}/other.svg"), &[]).await.status, StatusCode::NOT_FOUND);

    // Not on a page: 404 until badge_public, by public id only.
    let hidden = env.send("GET", &format!("/api/monitors/{db}/badge"), Some(&cookie), None).await;
    let hidden_pid = hidden.body["public_id"].as_str().unwrap().to_owned();
    let url = format!("/badge/{hidden_pid}/status.svg");
    assert_eq!(env.get_on(PAGE_HOST, &url, &[]).await.status, StatusCode::NOT_FOUND);
    assert_eq!(env.get_on(PAGE_HOST, &format!("/badge/{db}/status.svg"), &[]).await.status, StatusCode::NOT_FOUND);
    let put = env
        .send("PUT", &format!("/api/monitors/{db}/badge"), Some(&cookie), Some(json!({ "badge_public": true })))
        .await;
    assert_eq!(put.status, StatusCode::OK, "{:?}", put.body);
    assert_eq!(env.get_on(PAGE_HOST, &url, &[]).await.status, StatusCode::OK);
}

async fn token(env: &Env, cookie: &str, scopes: Value) -> String {
    let reply = env.send("POST", "/api/tokens", Some(cookie), Some(json!({ "name": "ci", "scopes": scopes }))).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.body);
    reply.body["token"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn tokens_scopes_and_read_api() {
    let env = env().await;
    let (cookie, web, _) = setup(&env).await;
    assert_eq!(env.send("GET", "/api/tokens", None, None).await.status, StatusCode::UNAUTHORIZED);
    let bad = json!({ "name": "x", "scopes": ["admin"] });
    assert_eq!(env.send("POST", "/api/tokens", Some(&cookie), Some(bad)).await.status, StatusCode::BAD_REQUEST);
    let read = token(&env, &cookie, json!(["read"])).await;
    let metrics = token(&env, &cookie, json!(["metrics"])).await;
    let list = env.send("GET", "/api/tokens", Some(&cookie), None).await;
    assert_eq!(list.body.as_array().unwrap().len(), 2);
    assert!(list.body[0].get("token").is_none() && !list.body.to_string().contains(&read));

    let bearer = |t: &str| format!("Bearer {t}");
    let get = |uri: &'static str, auth: Option<String>| {
        let env = &env;
        async move {
            let mut request = Request::builder().uri(uri).header(header::HOST, "api.example.net");
            if let Some(auth) = auth {
                request = request.header(header::AUTHORIZATION, auth);
            }
            env.raw(request.body(Body::empty()).unwrap()).await
        }
    };
    assert_eq!(get("/api/v1/monitors", None).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(get("/api/v1/monitors", Some(bearer("uwus_wrong"))).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(get("/api/v1/monitors", Some(bearer(&metrics))).await.status, StatusCode::FORBIDDEN);
    assert_eq!(get("/metrics", Some(bearer(&read))).await.status, StatusCode::FORBIDDEN);
    let monitors = get("/api/v1/monitors", Some(bearer(&read))).await;
    assert_eq!(monitors.status, StatusCode::OK);
    assert_eq!(monitors.body.as_array().unwrap().len(), 2);
    assert_eq!((monitors.body[0]["id"].as_i64(), monitors.body[0]["state"].as_str()), (Some(web), Some("pending")));
    let list = env.send("GET", "/api/tokens", Some(&cookie), None).await;
    assert!(list.body[0]["last_used"].is_i64());

    let summary = get("/api/v1/pages/main/summary", None).await;
    assert_eq!(summary.status, StatusCode::OK);
    assert_eq!(summary.headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    assert_eq!(
        (summary.body["overall"].as_str(), summary.body["monitors"][0]["name"].as_str()),
        (Some("ok"), Some("Website"))
    );
    assert_eq!(get("/api/v1/pages/nope/summary", None).await.status, StatusCode::NOT_FOUND);

    let id = list.body[0]["id"].as_i64().unwrap();
    assert_eq!(
        env.send("DELETE", &format!("/api/tokens/{id}"), Some(&cookie), None).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(get("/api/v1/monitors", Some(bearer(&read))).await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test(start_paused = true)]
async fn metrics_format() {
    let env = env_at(T0 + 3600).await;
    let cookie = env.admin().await;
    let web = env.monitor(&cookie, tcp("Web \"main\"")).await["id"].as_i64().unwrap();
    ingest_result(&env.state, web, "local", None, T0, &outcome(Status::Up)).unwrap();
    let t = token(&env, &cookie, json!(["metrics"])).await;
    let request = Request::builder()
        .uri("/metrics")
        .header(header::HOST, HOST)
        .header(header::AUTHORIZATION, format!("Bearer {t}"))
        .body(Body::empty())
        .unwrap();
    let reply = env.raw(request).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.headers[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/plain; version=0.0.4"));
    let body = text(&reply);
    let l = format!("id=\"{web}\",monitor=\"Web \\\"main\\\"\"");
    for line in [
        format!("uwustatus_monitor_up{{{l},location=\"local\"}} 1"),
        format!("uwustatus_monitor_state{{{l},state=\"up\"}} 1"),
        format!("uwustatus_monitor_state{{{l},state=\"down\"}} 0"),
        format!("uwustatus_monitor_latency_ms{{{l}}} 20"),
        format!("uwustatus_monitor_uptime_ratio{{{l},window=\"24h\"}} 1"),
    ] {
        assert!(body.lines().any(|x| x == line), "missing {line} in\n{body}");
    }
    // Every sample line: name{labels} number, and each metric has HELP and TYPE.
    for line in body.lines().filter(|l| !l.starts_with('#')) {
        let (series, value) = line.rsplit_once(' ').unwrap();
        assert!(value.parse::<f64>().is_ok(), "{line}");
        assert!(series.starts_with("uwustatus_monitor_") && series.ends_with('}'), "{line}");
    }
    assert_eq!(body.matches("# TYPE ").count(), 5);
}
