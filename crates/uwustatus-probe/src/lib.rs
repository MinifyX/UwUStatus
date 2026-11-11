//! Alle aktiven Prüfungen von UwUStatus. Läuft in Hauptinstanz und Satellit.
//!
//! Der Satellit prüft, was ihm die Hauptinstanz schickt – darum gibt es hier
//! bewusst keine Ziel-Filter. Jede Prüfung läuft unter einem Gesamt-Timeout,
//! Antworten werden größenbegrenzt gelesen.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

mod cert;
mod dns;
mod http;
mod mail;
mod net;

pub use cert::not_after;
pub use http::json_path;

/// Höchstens so viele Bytes Antwort-Body werden gelesen.
pub const MAX_BODY: usize = 1024 * 1024;
const DEFAULT_TIMEOUT_MS: u64 = 10_000;
const MAX_TIMEOUT_MS: u64 = 300_000;

/// Eine Prüfung samt Timeout. JSON: `{"type":"http","timeout_ms":5000,"url":"…"}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Check {
    /// Gesamt-Timeout der Prüfung (0 = Standard 10 s, höchstens 300 s).
    #[serde(default)]
    pub timeout_ms: u64,
    #[serde(flatten)]
    pub kind: CheckKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CheckKind {
    Http(HttpCheck),
    Tcp(TcpCheck),
    Ping(PingCheck),
    Dns(DnsCheck),
    Smtp(MailCheck),
    Imap(MailCheck),
    Docker(DockerCheck),
    Healthz(HealthzCheck),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HttpCheck {
    pub url: String,
    pub method: String,
    pub headers: BTreeMap<String, String>,
    pub body: Option<String>,
    /// Erlaubte Statuscodes als `"200-299"` oder `"301"`; leer = 200–299.
    pub accepted_status: Vec<String>,
    pub follow_redirects: bool,
    pub ignore_tls_errors: bool,
    /// Muss im Body vorkommen (bzw. fehlen, wenn `keyword_absent`).
    pub keyword: Option<String>,
    pub keyword_absent: bool,
    pub json_path: Option<JsonMatch>,
}

impl Default for HttpCheck {
    fn default() -> Self {
        Self {
            url: String::new(),
            method: "GET".into(),
            headers: BTreeMap::new(),
            body: None,
            accepted_status: Vec::new(),
            follow_redirects: true,
            ignore_tls_errors: false,
            keyword: None,
            keyword_absent: false,
            json_path: None,
        }
    }
}

/// Einfacher Punkt-Pfad (`data.items.0.state`); Zahlen indizieren Arrays.
/// Vergleich: Strings direkt, alles andere als JSON-Text (`42`, `true`, `null`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JsonMatch {
    pub path: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TcpCheck {
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PingCheck {
    pub host: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DnsCheck {
    pub name: String,
    /// `A`, `AAAA`, `CNAME`, `MX`, `TXT`, `NS`, `SOA`, `SRV`, `CAA`, `PTR` …
    #[serde(default = "default_record_type")]
    pub record_type: String,
    /// `IP` oder `IP:Port`; ohne = System-Resolver.
    #[serde(default)]
    pub resolver: Option<String>,
    /// Alle müssen in der Antwort vorkommen; leer = irgendeine Antwort genügt.
    #[serde(default)]
    pub expected: Vec<String>,
}

fn default_record_type() -> String {
    "A".into()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MailCheck {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub tls: MailTls,
    #[serde(default)]
    pub ignore_tls_errors: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailTls {
    /// Nur Banner lesen.
    #[default]
    None,
    StartTls,
    /// TLS ab dem ersten Byte (465/993).
    Tls,
}

/// Container über die Docker-Engine-API hinter einem Socket-Proxy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DockerCheck {
    /// z. B. `http://docker-proxy:2375`
    pub url: String,
    /// Name oder ID.
    pub container: String,
}

/// URL, die das UwU-`/healthz`-JSON liefert.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HealthzCheck {
    pub url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Up,
    /// Erreichbar, meldet aber Einschränkungen (healthz `degraded`, Docker `starting`).
    Degraded,
    Down,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub status: Status,
    pub latency_ms: u64,
    /// Kurz und menschenlesbar.
    pub message: String,
    #[serde(default)]
    pub details: Details,
}

/// Messwerte je nach Typ; Warnschwellen (z. B. Zertifikat) setzt der Server.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Details {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cert_expires_at: Option<jiff::Timestamp>,
    /// Ganze Tage bis Ablauf, negativ = abgelaufen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cert_days_left: Option<i64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dns_answers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub banner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_health: Option<String>,
    /// Version aus healthz.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Teilprüfungen aus healthz.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub checks: BTreeMap<String, SubCheck>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubCheck {
    pub status: String,
    #[serde(default)]
    pub message: String,
}

/// Geteilter Zustand: vorgebaute HTTP-Clients und TLS-Konfigurationen.
pub struct Ctx {
    /// Index: `follow_redirects as usize * 2 + insecure as usize`
    http: [reqwest::Client; 4],
    tls_verified: Arc<rustls::ClientConfig>,
    tls_insecure: Arc<rustls::ClientConfig>,
}

pub type Error = Box<dyn std::error::Error + Send + Sync>;

impl Ctx {
    pub fn new() -> Result<Self, Error> {
        let client = |follow: bool, insecure: bool| {
            reqwest::Client::builder()
                .user_agent(concat!("UwUStatus/", env!("CARGO_PKG_VERSION")))
                .redirect(if follow {
                    reqwest::redirect::Policy::limited(10)
                } else {
                    reqwest::redirect::Policy::none()
                })
                .tls_danger_accept_invalid_certs(insecure)
                .tls_info(true)
                // Jede Prüfung baut eine frische Verbindung auf (echte Latenz, aktuelles Zertifikat).
                .pool_max_idle_per_host(0)
                .build()
        };
        let (tls_verified, tls_insecure) = mail::tls_configs()?;
        Ok(Self {
            http: [client(false, false)?, client(false, true)?, client(true, false)?, client(true, true)?],
            tls_verified,
            tls_insecure,
        })
    }

    fn http(&self, follow: bool, insecure: bool) -> &reqwest::Client {
        &self.http[follow as usize * 2 + insecure as usize]
    }
}

/// Führt eine Prüfung aus. Fehler landen als `Down` im Ergebnis, nie als Panic.
pub async fn run(check: &Check, ctx: &Ctx) -> Outcome {
    let ms = if check.timeout_ms == 0 { DEFAULT_TIMEOUT_MS } else { check.timeout_ms.min(MAX_TIMEOUT_MS) };
    let timeout = Duration::from_millis(ms);
    let start = Instant::now();
    let fut = async {
        match &check.kind {
            CheckKind::Http(c) => http::http(c, ctx).await,
            CheckKind::Docker(c) => http::docker(c, ctx).await,
            CheckKind::Healthz(c) => http::healthz(c, ctx).await,
            CheckKind::Tcp(c) => net::tcp(c).await,
            CheckKind::Ping(c) => net::ping(c).await,
            CheckKind::Dns(c) => dns::dns(c, timeout).await,
            CheckKind::Smtp(c) => mail::mail(c, ctx, mail::Proto::Smtp).await,
            CheckKind::Imap(c) => mail::mail(c, ctx, mail::Proto::Imap).await,
        }
    };
    let mut outcome = match tokio::time::timeout(timeout, fut).await {
        Ok(Ok(o)) => o,
        Ok(Err(msg)) => Outcome::new(Status::Down, msg),
        Err(_) => Outcome::new(Status::Down, format!("timed out after {ms} ms")),
    };
    outcome.latency_ms = start.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    outcome
}

impl Outcome {
    fn new(status: Status, message: impl AsRef<str>) -> Self {
        Self { status, latency_ms: 0, message: short(message.as_ref()), details: Details::default() }
    }

    fn with(mut self, details: Details) -> Self {
        self.details = details;
        self
    }
}

/// Fehlerkette als eine Zeile (reqwest/hickory verstecken die Ursache sonst).
fn err_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut src = e.source();
    while let Some(e) = src {
        let m = e.to_string();
        if !s.contains(&m) {
            s.push_str(": ");
            s.push_str(&m);
        }
        src = e.source();
    }
    s
}

/// Steuerzeichen raus, auf 300 Zeichen kürzen – Texte stammen oft vom Ziel.
fn short(s: &str) -> String {
    let mut out: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).take(300).collect();
    if s.chars().nth(300).is_some() {
        out.push('…');
    }
    out.trim().to_owned()
}

/// Ablauf und Resttage aus einem DER-Zertifikat in `details` eintragen.
fn set_cert(details: &mut Details, der: Option<&[u8]>) {
    if let Some(at) = der.and_then(cert::not_after) {
        details.cert_expires_at = Some(at);
        details.cert_days_left = Some(at.duration_since(jiff::Timestamp::now()).as_secs().div_euclid(86_400));
    }
}
