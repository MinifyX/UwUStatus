//! HTTP(S), Docker-Engine-API und Suite-`/healthz` – alles über reqwest.

use serde::Deserialize;
use serde_json::Value;

use crate::{
    Ctx, Details, DockerCheck, HealthzCheck, HttpCheck, MAX_BODY, Outcome, Status, SubCheck, err_chain, set_cert, short,
};

pub(crate) async fn http(c: &HttpCheck, ctx: &Ctx) -> Result<Outcome, String> {
    let method = reqwest::Method::from_bytes(c.method.trim().to_ascii_uppercase().as_bytes())
        .map_err(|_| format!("invalid method {:?}", c.method))?;
    let ranges = parse_status(&c.accepted_status)?;
    let mut req = ctx.http(c.follow_redirects, c.ignore_tls_errors).request(method, &c.url);
    for (k, v) in &c.headers {
        req = req.header(k, v);
    }
    if let Some(body) = &c.body {
        req = req.body(body.clone());
    }
    let resp = req.send().await.map_err(|e| err_chain(&e))?;
    let code = resp.status().as_u16();
    let mut details = Details { http_status: Some(code), ..Details::default() };
    set_cert(
        &mut details,
        resp.extensions().get::<reqwest::tls::TlsInfo>().and_then(reqwest::tls::TlsInfo::peer_certificate),
    );
    let down = |msg: String, details: Details| Ok(Outcome::new(Status::Down, msg).with(details));
    if !ranges.iter().any(|(lo, hi)| (*lo..=*hi).contains(&code)) {
        return down(format!("HTTP {code}"), details);
    }
    if c.keyword.is_some() || c.json_path.is_some() {
        let body = read_body(resp).await?;
        if let Some(kw) = &c.keyword {
            let found = String::from_utf8_lossy(&body).contains(kw.as_str());
            if found == c.keyword_absent {
                let what = if found { "found" } else { "not found" };
                return down(format!("HTTP {code}, keyword {kw:?} {what}"), details);
            }
        }
        if let Some(m) = &c.json_path {
            let json: Value = serde_json::from_slice(&body).map_err(|e| format!("HTTP {code}, invalid JSON: {e}"))?;
            let Some(v) = json_path(&json, &m.path) else {
                return down(format!("HTTP {code}, JSON path {:?} missing", m.path), details);
            };
            let actual = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            if actual != m.value {
                return down(format!("HTTP {code}, {} = {actual:?}, expected {:?}", m.path, m.value), details);
            }
        }
    }
    Ok(Outcome::new(Status::Up, format!("HTTP {code}")).with(details))
}

/// `"200-299"`, `"301"`; leer = 200–299.
fn parse_status(specs: &[String]) -> Result<Vec<(u16, u16)>, String> {
    if specs.is_empty() {
        return Ok(vec![(200, 299)]);
    }
    specs
        .iter()
        .map(|s| {
            let (lo, hi) = s.split_once('-').unwrap_or((s, s));
            match (lo.trim().parse(), hi.trim().parse()) {
                (Ok(lo), Ok(hi)) if lo <= hi => Ok((lo, hi)),
                _ => Err(format!("invalid status code spec {s:?}")),
            }
        })
        .collect()
}

/// Punkt-Pfad in JSON; Zahlen-Segmente indizieren Arrays. Optionales `$.`-Präfix.
pub fn json_path<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let path = path.strip_prefix('$').unwrap_or(path);
    path.split('.').filter(|s| !s.is_empty()).try_fold(v, |v, seg| match v {
        Value::Object(o) => o.get(seg),
        Value::Array(a) => a.get(seg.parse::<usize>().ok()?),
        _ => None,
    })
}

/// Body lesen, nach `MAX_BODY` abschneiden.
async fn read_body(mut resp: reqwest::Response) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| err_chain(&e))? {
        let room = MAX_BODY - buf.len();
        buf.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if buf.len() >= MAX_BODY {
            break;
        }
    }
    Ok(buf)
}

async fn get_json<T: for<'de> Deserialize<'de>>(ctx: &Ctx, url: &str) -> Result<(u16, Option<T>), String> {
    let resp = ctx.http(true, false).get(url).send().await.map_err(|e| err_chain(&e))?;
    let code = resp.status().as_u16();
    let body = read_body(resp).await?;
    Ok((code, serde_json::from_slice(&body).ok()))
}

pub(crate) async fn docker(c: &DockerCheck, ctx: &Ctx) -> Result<Outcome, String> {
    #[derive(Deserialize)]
    struct Inspect {
        #[serde(rename = "State")]
        state: State,
    }
    #[derive(Deserialize)]
    struct State {
        #[serde(rename = "Status", default)]
        status: String,
        #[serde(rename = "ExitCode", default)]
        exit_code: i64,
        #[serde(rename = "Health")]
        health: Option<Health>,
    }
    #[derive(Deserialize)]
    struct Health {
        #[serde(rename = "Status", default)]
        status: String,
    }

    // Docker-Namen/IDs: [a-zA-Z0-9][a-zA-Z0-9_.-]*, so landet nichts anderes im Pfad.
    let name = c.container.trim_start_matches('/');
    let valid = name.starts_with(|ch: char| ch.is_ascii_alphanumeric())
        && name.chars().all(|ch| ch.is_ascii_alphanumeric() || "_.-".contains(ch));
    if !valid {
        return Err(format!("invalid container name {:?}", c.container));
    }
    let url = format!("{}/containers/{name}/json", c.url.trim_end_matches('/'));
    let (code, inspect) = get_json::<Inspect>(ctx, &url).await?;
    match (code, inspect) {
        (404, _) => Err(format!("container {name} not found")),
        (200, Some(Inspect { state })) => {
            let health = state.health.map(|h| h.status).filter(|s| !s.is_empty() && s != "none");
            let details = Details {
                container_status: Some(state.status.clone()),
                container_health: health.clone(),
                ..Details::default()
            };
            let (status, msg) = match (state.status.as_str(), health.as_deref()) {
                ("running", None | Some("healthy")) => (Status::Up, health.unwrap_or_else(|| "running".into())),
                ("running", Some("starting")) => (Status::Degraded, "running, health check starting".into()),
                ("running", Some(h)) => (Status::Down, format!("running but {h}")),
                ("exited" | "dead", _) => (Status::Down, format!("{} (exit code {})", state.status, state.exit_code)),
                (s, _) => (Status::Down, s.to_owned()),
            };
            Ok(Outcome::new(status, msg).with(details))
        }
        (code, _) => Err(format!("Docker API: HTTP {code}")),
    }
}

pub(crate) async fn healthz(c: &HealthzCheck, ctx: &Ctx) -> Result<Outcome, String> {
    #[derive(Deserialize)]
    struct Healthz {
        status: String,
        #[serde(default)]
        version: Option<String>,
        #[serde(default)]
        checks: std::collections::BTreeMap<String, SubCheck>,
    }

    let (code, body) = get_json::<Healthz>(ctx, &c.url).await?;
    let Some(h) = body else {
        return Err(format!("HTTP {code}, no healthz JSON"));
    };
    let status = match h.status.as_str() {
        "ok" => Status::Up,
        "degraded" => Status::Degraded,
        "down" => Status::Down,
        other => return Err(format!("unknown healthz status {:?}", short(other))),
    };
    let failing: Vec<String> = h
        .checks
        .iter()
        .filter(|(_, s)| s.status != "ok")
        .map(|(name, s)| match s.message.is_empty() {
            true => format!("{name}: {}", s.status),
            false => format!("{name}: {} ({})", s.status, s.message),
        })
        .collect();
    let msg = if failing.is_empty() { h.status.clone() } else { format!("{}: {}", h.status, failing.join(", ")) };
    let checks = h
        .checks
        .into_iter()
        .take(100)
        .map(|(k, s)| (short(&k), SubCheck { status: short(&s.status), message: short(&s.message) }))
        .collect();
    let details = Details { version: h.version.map(|v| short(&v)), checks, ..Details::default() };
    Ok(Outcome::new(status, msg).with(details))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path() {
        let v: Value = serde_json::from_str(r#"{"a":{"b":[1,{"c":"x"}]},"t":true}"#).unwrap();
        assert_eq!(json_path(&v, "a.b.1.c"), Some(&Value::from("x")));
        assert_eq!(json_path(&v, "$.t"), Some(&Value::from(true)));
        assert_eq!(json_path(&v, ""), Some(&v));
        assert_eq!(json_path(&v, "a.b.9"), None);
        assert_eq!(json_path(&v, "a.b.-1"), None);
        assert_eq!(json_path(&v, "t.x"), None);
    }

    #[test]
    fn path_fuzz() {
        let v: Value = serde_json::from_str(r#"{"a":[{"b":{"c":[0,1,2]}}],"ä":{"..":1}}"#).unwrap();
        let alphabet = ["a", "b", "c", "0", "1", "99999999999999999999", ".", "..", "$", "ä", "", " "];
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..20_000 {
            let mut p = String::new();
            for _ in 0..(x % 9) {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                p.push_str(alphabet[(x % alphabet.len() as u64) as usize]);
            }
            let _ = json_path(&v, &p);
        }
    }

    #[test]
    fn status_specs() {
        assert_eq!(parse_status(&[]).unwrap(), vec![(200, 299)]);
        assert_eq!(parse_status(&["301".into(), " 400 - 404 ".into()]).unwrap(), vec![(301, 301), (400, 404)]);
        assert!(parse_status(&["5xx".into()]).is_err());
        assert!(parse_status(&["404-400".into()]).is_err());
        assert!(parse_status(&["-".into()]).is_err());
    }
}
