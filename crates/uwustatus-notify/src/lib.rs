//! Sends one alert to one channel. No server state: the caller hands over a self-contained
//! [`Target`] (channel, its secrets, SMTP settings) and a finished [`Message`], so the main
//! instance and (Phase 6) the satellite share this crate. Retries are the caller's business.

use hmac::{Hmac, Mac as _};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lang {
    #[default]
    De,
    En,
}

/// The public part of a channel. Secrets (tokens, keys, the Discord URL) live in [`Secrets`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Channel {
    /// Recipients; server and sender come from [`Smtp`].
    Email { to: Vec<String> },
    /// JSON POST (see [`webhook_body`]); secret `hmac_secret` (optional) signs it.
    Webhook { url: String },
    /// Server root (e.g. `https://ntfy.sh`), topic, priority 1–5; secret `token` (optional).
    Ntfy {
        server: String,
        topic: String,
        #[serde(default = "ntfy_priority")]
        priority: u8,
    },
    /// Gotify base URL, priority 0–10; secret `token` (app token).
    Gotify {
        url: String,
        #[serde(default = "gotify_priority")]
        priority: u8,
    },
    /// Priority −2…1; secrets `user_key` and `token` (app token).
    Pushover {
        #[serde(default)]
        priority: i8,
    },
    /// Secret `webhook_url`.
    Discord {},
    /// Secret `bot_token`.
    Telegram { chat_id: String },
}

fn ntfy_priority() -> u8 {
    4
}
fn gotify_priority() -> u8 {
    8
}

/// Secret name → value.
pub type Secrets = BTreeMap<String, String>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Security {
    #[default]
    Starttls,
    /// Implicit TLS (port 465).
    Tls,
    /// Plain text, only for a relay on a trusted network.
    None,
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Smtp {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub user: String,
    pub password: String,
    pub from: String,
}

impl fmt::Debug for Smtp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Smtp({}:{} {:?}, from {})", self.host, self.port, self.security, self.from)
    }
}

/// Everything needed to send to one channel, serialisable (the tunnel carries it to satellites).
/// No `Debug`: it holds secrets.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub channel: Channel,
    #[serde(default)]
    pub secrets: Secrets,
    #[serde(default)]
    pub lang: Lang,
    /// Required for [`Channel::Email`].
    #[serde(default)]
    pub smtp: Option<Smtp>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Down,
    Degraded,
    Up,
    Info,
}

/// A finished alert in the channel's language.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub title: String,
    pub body: String,
    pub severity: Severity,
    /// Admin UI link.
    pub link: Option<String>,
    /// Machine-readable event for webhooks (`{"type": "state_changed", …}`).
    pub event: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

fn err(message: impl Into<String>) -> Error {
    Error(message.into())
}

/// reqwest errors name the URL, which may carry a token (Telegram, Discord).
fn http_err(e: reqwest::Error) -> Error {
    err(e.without_url().to_string())
}

pub const WEBHOOK_SIGNATURE: &str = "X-UwUStatus-Signature";
/// Bytes read from an answer at most (for the error text); the rest is dropped.
const MAX_RESPONSE: usize = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(15);

fn http_url(value: &str, what: &str) -> Result<Url, String> {
    match Url::parse(value) {
        Ok(url) if matches!(url.scheme(), "http" | "https") && url.host().is_some() => Ok(url),
        _ => Err(format!("{what}: nur http(s)-URLs")),
    }
}

fn need(secrets: &Secrets, name: &str, what: &str) -> Result<(), String> {
    if secrets.get(name).is_some_and(|v| !v.trim().is_empty()) { Ok(()) } else { Err(format!("{what} fehlt")) }
}

fn short(value: &str, max: usize, what: &str) -> Result<(), String> {
    if (1..=max).contains(&value.chars().count()) { Ok(()) } else { Err(format!("{what}: 1 bis {max} Zeichen")) }
}

impl Channel {
    /// The secret names this type keeps (and accepts).
    pub fn secret_names(&self) -> &'static [&'static str] {
        match self {
            Channel::Email { .. } => &[],
            Channel::Webhook { .. } => &["hmac_secret"],
            Channel::Ntfy { .. } | Channel::Gotify { .. } => &["token"],
            Channel::Pushover { .. } => &["user_key", "token"],
            Channel::Discord {} => &["webhook_url"],
            Channel::Telegram { .. } => &["bot_token"],
        }
    }

    /// German error text for the admin.
    pub fn validate(&self, secrets: &Secrets) -> Result<(), String> {
        if let Some(name) = secrets.keys().find(|k| !self.secret_names().contains(&k.as_str())) {
            return Err(format!("Unbekanntes Geheimnis {name}"));
        }
        if secrets.values().any(|v| v.chars().count() > 2000 || v.chars().any(char::is_control)) {
            return Err("Geheimnisse: höchstens 2000 Zeichen, keine Steuerzeichen".into());
        }
        match self {
            Channel::Email { to } => {
                if to.is_empty() || to.len() > 20 {
                    return Err("Empfänger: 1 bis 20 Adressen".into());
                }
                if let Some(bad) = to.iter().find(|a| a.parse::<lettre::Address>().is_err()) {
                    return Err(format!("Empfänger {bad}: keine gültige Adresse"));
                }
            }
            Channel::Webhook { url } => {
                http_url(url, "Webhook-URL")?;
            }
            Channel::Ntfy { server, topic, priority } => {
                http_url(server, "ntfy-Server")?;
                short(topic, 64, "Topic")?;
                if !topic.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                    return Err("Topic: nur Buchstaben, Ziffern, - und _".into());
                }
                if !(1..=5).contains(priority) {
                    return Err("ntfy-Priorität: 1 bis 5".into());
                }
            }
            Channel::Gotify { url, priority } => {
                http_url(url, "Gotify-URL")?;
                need(secrets, "token", "App-Token")?;
                if *priority > 10 {
                    return Err("Gotify-Priorität: 0 bis 10".into());
                }
            }
            Channel::Pushover { priority } => {
                need(secrets, "user_key", "User-Key")?;
                need(secrets, "token", "App-Token")?;
                if !(-2..=1).contains(priority) {
                    return Err("Pushover-Priorität: −2 bis 1".into());
                }
            }
            Channel::Discord {} => {
                need(secrets, "webhook_url", "Webhook-URL")?;
                http_url(&secrets["webhook_url"], "Webhook-URL")?;
            }
            Channel::Telegram { chat_id } => {
                need(secrets, "bot_token", "Bot-Token")?;
                short(chat_id, 64, "Chat-ID")?;
                if secrets["bot_token"].contains(['/', '?', '#']) {
                    return Err("Bot-Token: ungültig".into());
                }
            }
        }
        Ok(())
    }
}

impl Smtp {
    pub fn validate(&self) -> Result<(), String> {
        short(&self.host, 253, "SMTP-Server")?;
        if self.port == 0 {
            return Err("SMTP-Port fehlt".into());
        }
        self.from.parse::<lettre::message::Mailbox>().map_err(|_| "Absender: keine gültige Adresse".to_owned())?;
        Ok(())
    }
}

/// The JSON a webhook receives:
/// `{"title", "body", "severity": "down|degraded|up|info", "link", "event": {"type", …}}`.
pub fn webhook_body(message: &Message) -> Value {
    json!({
        "title": message.title,
        "body": message.body,
        "severity": message.severity,
        "link": message.link,
        "event": message.event,
    })
}

/// `sha256=<hex HMAC-SHA256 of the body>`.
pub fn signature(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC takes any key");
    mac.update(body);
    let bytes = mac.finalize().into_bytes();
    format!("sha256={}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// Title, body and link as one plain text.
fn plain(message: &Message) -> String {
    match &message.link {
        Some(link) => format!("{}\n\n{link}", message.body),
        None => message.body.clone(),
    }
}

#[derive(Clone)]
pub struct Sender {
    http: reqwest::Client,
    /// Overridable for tests.
    pub pushover_api: String,
    pub telegram_api: String,
}

impl Sender {
    /// Own client: 15 s timeout, no redirects.
    pub fn new() -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("UwUStatus/", env!("CARGO_PKG_VERSION")))
            .timeout(TIMEOUT)
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| format!("HTTP-Client: {e}"))?;
        Ok(Self::with_client(http))
    }

    /// A shared client; it should have a timeout and no redirects.
    pub fn with_client(http: reqwest::Client) -> Self {
        Self { http, pushover_api: "https://api.pushover.net".into(), telegram_api: "https://api.telegram.org".into() }
    }

    pub async fn send(&self, target: &Target, message: &Message) -> Result<(), Error> {
        target.channel.validate(&target.secrets).map_err(err)?;
        let secret = |name: &str| target.secrets.get(name).map(String::as_str).filter(|v| !v.is_empty());
        let alarm = matches!(message.severity, Severity::Down | Severity::Degraded);
        let request = match &target.channel {
            Channel::Email { to } => return self.mail(target.smtp.as_ref(), to, message).await,
            Channel::Webhook { url } => {
                let body = serde_json::to_vec(&webhook_body(message)).map_err(|e| err(e.to_string()))?;
                let mut request =
                    self.http.post(url.as_str()).header(reqwest::header::CONTENT_TYPE, "application/json");
                if let Some(key) = secret("hmac_secret") {
                    request = request.header(WEBHOOK_SIGNATURE, signature(key, &body));
                }
                request.body(body)
            }
            Channel::Ntfy { server, topic, priority } => {
                let tag = match message.severity {
                    Severity::Down => "red_circle",
                    Severity::Degraded => "warning",
                    Severity::Up => "green_circle",
                    Severity::Info => "information_source",
                };
                let mut body = json!({
                    "topic": topic, "title": message.title, "message": message.body,
                    "priority": if alarm { *priority } else { 3 }, "tags": [tag],
                });
                if let Some(link) = &message.link {
                    body["click"] = json!(link);
                }
                let request = self.http.post(server.as_str()).json(&body);
                match secret("token") {
                    Some(token) => request.bearer_auth(token),
                    None => request,
                }
            }
            Channel::Gotify { url, priority } => {
                let mut body = json!({
                    "title": message.title, "message": message.body,
                    "priority": if alarm { *priority } else { 4 },
                });
                if let Some(link) = &message.link {
                    body["extras"] = json!({ "client::notification": { "click": { "url": link } } });
                }
                self.http
                    .post(format!("{}/message", url.trim_end_matches('/')))
                    .header("X-Gotify-Key", secret("token").unwrap_or_default())
                    .json(&body)
            }
            Channel::Pushover { priority } => {
                let priority = if alarm { *priority } else { 0.min(*priority) }.to_string();
                let mut form = vec![
                    ("token", secret("token").unwrap_or_default()),
                    ("user", secret("user_key").unwrap_or_default()),
                    ("title", message.title.as_str()),
                    ("message", message.body.as_str()),
                    ("priority", priority.as_str()),
                ];
                if let Some(link) = &message.link {
                    form.push(("url", link));
                }
                self.http.post(format!("{}/1/messages.json", self.pushover_api.trim_end_matches('/'))).form(&form)
            }
            Channel::Discord {} => {
                let color = match message.severity {
                    Severity::Down => 0xe5484d,
                    Severity::Degraded => 0xf59e0b,
                    Severity::Up => 0x10b981,
                    Severity::Info => 0xe0457b,
                };
                let mut embed = json!({ "title": message.title, "description": message.body, "color": color });
                if let Some(link) = &message.link {
                    embed["url"] = json!(link);
                }
                // No mentions: a monitor name must not ping @everyone.
                let body = json!({ "embeds": [embed], "allowed_mentions": { "parse": [] } });
                self.http.post(secret("webhook_url").unwrap_or_default()).json(&body)
            }
            Channel::Telegram { chat_id } => {
                let body = json!({
                    "chat_id": chat_id,
                    "text": format!("{}\n\n{}", message.title, plain(message)),
                    "link_preview_options": { "is_disabled": true },
                });
                let api = self.telegram_api.trim_end_matches('/');
                self.http.post(format!("{api}/bot{}/sendMessage", secret("bot_token").unwrap_or_default())).json(&body)
            }
        };
        let mut response = request.timeout(TIMEOUT).send().await.map_err(http_err)?;
        let status = response.status();
        let mut answer = Vec::new();
        while answer.len() < MAX_RESPONSE {
            match response.chunk().await {
                Ok(Some(chunk)) => answer.extend_from_slice(&chunk),
                Ok(None) | Err(_) => break,
            }
        }
        if status.is_success() {
            return Ok(());
        }
        let text: String = String::from_utf8_lossy(&answer).chars().filter(|c| !c.is_control()).take(200).collect();
        Err(err(format!("HTTP {}: {}", status.as_u16(), text.trim())))
    }

    async fn mail(&self, smtp: Option<&Smtp>, to: &[String], message: &Message) -> Result<(), Error> {
        use lettre::message::{Mailbox, header::ContentType};
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

        let smtp = smtp.ok_or_else(|| err("SMTP ist nicht eingerichtet"))?;
        smtp.validate().map_err(err)?;
        let builder = match smtp.security {
            Security::Starttls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp.host),
            Security::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&smtp.host),
            Security::None => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&smtp.host)),
        }
        .map_err(|e| err(format!("SMTP: {e}")))?;
        let mut builder = builder.port(smtp.port).timeout(Some(TIMEOUT));
        if !smtp.user.is_empty() {
            builder = builder.credentials(Credentials::new(smtp.user.clone(), smtp.password.clone()));
        }
        let from: Mailbox = smtp.from.parse().map_err(|_| err("Absender: keine gültige Adresse"))?;
        let mut mail = lettre::Message::builder().from(from).subject(&message.title).header(ContentType::TEXT_PLAIN);
        for address in to {
            mail = mail.to(address.parse().map_err(|_| err(format!("Empfänger {address}: ungültig")))?);
        }
        let mail = mail.body(plain(message)).map_err(|e| err(format!("E-Mail: {e}")))?;
        builder.build().send(mail).await.map_err(|e| err(format!("SMTP: {e}")))?;
        Ok(())
    }
}
