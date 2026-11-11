//! The dead man's switch: the main instance sends its config; when the satellite has not seen the
//! main instance for `after_s`, it alarms through the channels itself, and sends the all-clear once
//! the main instance is back.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc;
use uwustatus_notify::{Lang, Message, Sender, Severity, Target};

/// As sent by the main instance in `DeadMan { config }`. No `Debug`: the targets hold secrets.
#[derive(Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DeadManConfig {
    /// This satellite's name in the main instance.
    pub name: String,
    /// 0 = off.
    pub after_s: u64,
    pub channels: Vec<Target>,
    /// Alarm texts; empty = built-in. `{name}` and `{since}` are filled in.
    pub text_de: String,
    pub text_en: String,
    pub page: PageConfig,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PageConfig {
    pub enabled: bool,
    pub title: String,
    /// Served only for this host when set.
    pub domain: String,
    /// Shown with this satellite's own latest result.
    pub monitors: Vec<PageMonitor>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageMonitor {
    /// The assignment id.
    pub id: String,
    pub name: String,
}

/// One alarm (`alarm`) or all-clear.
#[derive(Clone, PartialEq)]
pub struct Notice {
    pub config: DeadManConfig,
    pub alarm: bool,
    /// Unix seconds since the main instance is away.
    pub since: i64,
}

impl Notice {
    pub fn texts(&self) -> (String, String) {
        let since = crate::utc(self.since);
        let fill = |text: &str| text.replace("{name}", &self.config.name).replace("{since}", &since);
        let name = &self.config.name;
        if !self.alarm {
            return (
                format!("UwUStatus-Satellit „{name}“: Hauptinstanz wieder erreichbar."),
                format!("UwUStatus satellite \"{name}\": main instance reachable again."),
            );
        }
        let de = match self.config.text_de.trim() {
            "" => format!("UwUStatus-Satellit „{name}“ erreicht die Hauptinstanz seit {since} nicht mehr."),
            text => fill(text),
        };
        let en = match self.config.text_en.trim() {
            "" => format!("UwUStatus satellite \"{name}\" has not reached the main instance since {since}."),
            text => fill(text),
        };
        (de, en)
    }
}

pub trait DeadManNotifier: Send + Sync + 'static {
    fn notify(&self, notice: &Notice) -> impl Future<Output = ()> + Send;
}

/// Sends notices one after the other, so an all-clear never overtakes its alarm.
pub fn sender<N: DeadManNotifier>(notifier: Arc<N>) -> mpsc::UnboundedSender<Notice> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Notice>();
    tokio::spawn(async move {
        while let Some(notice) = rx.recv().await {
            notifier.notify(&notice).await;
        }
    });
    tx
}

/// Sends through `uwustatus-notify`, every channel the main instance has set for the dead man.
pub struct NotifyNotifier {
    sender: Sender,
    /// Pauses before the second and third try.
    pub retry: [Duration; 2],
}

impl NotifyNotifier {
    pub fn new() -> Result<NotifyNotifier, String> {
        Ok(NotifyNotifier { sender: Sender::new()?, retry: [Duration::from_secs(5), Duration::from_secs(30)] })
    }
}

impl DeadManNotifier for NotifyNotifier {
    async fn notify(&self, notice: &Notice) {
        let (de, en) = notice.texts();
        let event = json!({
            "type": "dead_man",
            "state": if notice.alarm { "alarm" } else { "ok" },
            "satellite": notice.config.name,
            "since": crate::utc(notice.since),
        });
        for target in &notice.config.channels {
            let (title, body) = match (target.lang, notice.alarm) {
                (Lang::De, true) => ("UwUStatus: Hauptinstanz nicht erreichbar", &de),
                (Lang::De, false) => ("UwUStatus: Hauptinstanz wieder erreichbar", &de),
                (Lang::En, true) => ("UwUStatus: main instance unreachable", &en),
                (Lang::En, false) => ("UwUStatus: main instance reachable again", &en),
            };
            let message = Message {
                title: title.into(),
                body: body.clone(),
                severity: if notice.alarm { Severity::Down } else { Severity::Up },
                link: None,
                event: event.clone(),
            };
            let mut tries = self.retry.iter();
            while let Err(e) = self.sender.send(target, &message).await {
                let Some(pause) = tries.next() else {
                    tracing::error!("dead man notice lost: {e}");
                    break;
                };
                tracing::warn!("dead man notice: {e}, trying again");
                tokio::time::sleep(*pause).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts_fill_in() {
        let config =
            DeadManConfig { name: "Berlin".into(), text_en: "{name} lost it at {since}".into(), ..Default::default() };
        let alarm = Notice { config, alarm: true, since: 0 };
        let (de, en) = alarm.texts();
        assert!(de.contains("„Berlin“") && de.contains("1970-01-01 00:00 UTC"), "{de}");
        assert_eq!(en, "Berlin lost it at 1970-01-01 00:00 UTC");
        let ok = Notice { alarm: false, ..alarm };
        assert!(ok.texts().0.contains("wieder erreichbar"));
    }
}
