# UwUStatus – Architektur (Kern, Phase 3)

## Datenfluss

```
Planer (lokal) ─┐
Push-Endpunkt ──┼─> ingest::ingest_result(monitor, location, seq?, at, Outcome)
Satellit (Ph.6)─┘        │ results + location_state (engine::step: Wiederholungen)
                         │ monitor_state (engine::overall: Quorum, Eltern, Wartung, Zert.)
                         ├─> events-Tabelle
                         └─> Event-Bus (tokio broadcast, AppState.events)
Minuten-Tick: ingest::tick = Zustände neu bewerten (Wartung beginnt/endet),
              Erinnerungen, history::maintain (Verdichtung + Aufbewahrung)
```

Alle Zeiten sind Unix-Sekunden aus `AppState.clock` (`Clock::System` oder für Tests
`Clock::Tokio`, das mit `tokio::time::pause/advance` läuft).

## Tabellen

| Tabelle | Inhalt |
|---|---|
| `monitors` | `id`, `config` (JSON `MonitorConfig`), `push_token` (unique), `created`, `updated` |
| `satellites` | `id`, `name`, `status` (`pending` …), `created`, `last_seen` – Standorte neben `local` |
| `monitor_state` | Gesamtzustand je Monitor: `state`, `reason`, `since`, `via_parent`, letzter Lauf (`last_at`, `latency_ms`, `message`), `version`, `cert_days_left`, `cert_warned`, `last_reminder` |
| `location_state` | Zustand je (Monitor, Standort): `state`, `failures` (laufende Wiederholungen), `at`, `message` |
| `results` | Rohergebnisse; `UNIQUE (location, seq)` dedupliziert Satelliten-Nachlieferungen (lokal `seq = NULL`) |
| `rollups` | `period` `hour`/`day`, `start`, Zähler up/degraded/down, `latency_avg`, `latency_p95`, `uptime` |
| `events` | `kind` `state`/`version`/`cert`, `from_value`, `to_value`, `reason` |
| `maintenance` | `config` (JSON: Zeitplan `once`/`weekly` + Monitore/Gruppen) |
| `settings` | `settings` (Aufbewahrung, Standardintervall), `rollup_next` (Fortschritt der Verdichtung) |
| `status_pages` | `slug` (unique), `config` (JSON `PageConfig`: Titel, Domains, Sprache, Akzent, Nyu, Zeitzone, Gruppen), `logo` + `logo_type` |
| `monitor_public` | `public_id` (zufällig, 16 Zeichen) für Badges/öffentliche JSON, `badge_public` |
| `incidents` | `title` (leer = automatischer Titel), `auto`, `monitor_id` (Auslöser), `impact`, `status`, `started`, `resolved` |
| `incident_monitors`, `incident_updates` | betroffene Monitore; Verlauf (`at`, `status`, `body`) |
| `api_tokens` | `name`, `token_hash` (SHA-256), `scopes` (JSON `read`/`metrics`), `last_used` |

## Monitor-Konfiguration

`name`, `description`, `group`, `tags`, `kind` (`check` mit Probe-`Check` | `push`),
`interval_s`, `timeout_s`, `retries`, `retry_interval_s`, `reminder_min`, `muted`, `paused`,
`parent_id` (keine Zyklen), `locations` (`local` + Satelliten-IDs), `quorum`
(`any`|`majority`|`all`), `cert_warn_days`.

## Zustände (`engine`, rein)

`up | degraded | down | maintenance | paused | pending`.

- Standort: Fehlschlag zählt `failures` hoch, erst nach `retries` Wiederholungen `down`.
- Quorum über die Standorte; Teilausfall ohne erreichtes Quorum ⇒ `degraded`.
- Eltern `down` ⇒ Kind `down` mit `unreachable_via_parent`, kein Alarm.
- Wartung ⇒ `maintenance`, pausiert ⇒ `paused`, beides ohne Alarm.
- Zertifikat innerhalb `cert_warn_days` oder healthz `degraded` ⇒ `degraded`.
- Push: kein Push binnen `interval + timeout` ⇒ Fehlschlag „Kein Push erhalten“.

## Events (`events::Event`)

```rust
StateChanged { monitor_id, at, from, to, reason, unreachable_via_parent, alert }
Reminder { monitor_id, at, since, reason }        // nur Bus
CertExpiring { monitor_id, at, days_left }        // einmal bis zur Erneuerung
VersionChanged { monitor_id, at, from, to }
```

## Andockstellen für Phase 4–7

- Alarme (4): `state.events.subscribe()`; `alert` bzw. `Reminder` sagt, ob gesendet wird.
- Statusseiten (5): `history::uptime(db, id, from, to)`, `rollups`, `events`, Bus für Live-Updates.
- Satelliten (6): `ingest::ingest_result(state, id, satellite_id, Some(seq), at, &outcome)`;
  `MonitorConfig::runs_locally()` entscheidet, was der lokale Planer übernimmt.
- Web-UI (7): Routen unten; Wartung über `maintenance::covers` / `Schedule::is_in_maintenance`.

## Routen

| Methode | Pfad | |
|---|---|---|
| GET, POST | `/api/monitors` | Liste / Anlegen |
| GET, PUT, DELETE | `/api/monitors/{id}` | |
| GET | `/api/monitors/{id}/history?range=24h\|30d` | `raw`/`hour`/`day` je nach Spanne |
| GET | `/api/monitors/{id}/events?before=&limit=` | neueste zuerst |
| GET | `/api/overview` | Zustand, letzter Lauf, Uptime 24 h / 30 d |
| GET, POST | `/api/maintenance` | |
| GET, PUT, DELETE | `/api/maintenance/{id}` | |
| GET, POST | `/api/push/{token}?status=up\|down&msg=&ping=` | ohne Login, rate-limitiert |
| GET, POST | `/api/pages` · GET, PUT, DELETE `/api/pages/{id}` | Statusseiten |
| PUT, DELETE | `/api/pages/{id}/logo` | Rohdatei, ≤ 60 KB, PNG/JPEG/WebP/SVG |
| GET, PUT | `/api/monitors/{id}/badge` | `public_id`, `badge_public`, Badge-URLs |
| GET, POST | `/api/incidents?status=open&lang=` · GET, PUT, DELETE `/api/incidents/{id}` | Vorfälle |
| POST · PUT | `/api/incidents/{id}/updates` · `/api/incidents/{id}/updates/{uid}` | Meldungen |
| GET, POST · DELETE | `/api/tokens` · `/api/tokens/{id}` | Token nur in der Anlege-Antwort |
| GET | `/s/{slug}/…` | Vorschau einer Seite auf dem Admin-Host (Login) |

Auf jedem Host, ohne Session:

| Methode | Pfad | |
|---|---|---|
| GET | `/api/v1/monitors`, `/api/v1/monitors/{id}` | Bearer-Token mit `read` |
| GET | `/api/v1/pages/{slug}/summary` | öffentlich, `Access-Control-Allow-Origin: *`; nur auf den Domains der Seite (oder dem Admin-Host) |
| GET | `/metrics` | Bearer-Token mit `metrics`, Prometheus-Textformat |
| GET | `/badge/{public_id}/status.svg\|uptime.svg?days=30\|latency.svg` | nur Monitore auf einer Seite oder mit `badge_public` |

## Statusseiten (Phase 5)

- Host-Weiche: der Admin-Host (`public_url`) bekommt die App, jeder andere Host die Seite, deren
  `domains` ihn enthalten (`pages::by_host`), sonst 404. Mit `UWUSTATUS_PUBLIC_BIND` gibt es einen
  zweiten Listener (`app::public_router`) ohne Admin-App: nur Statusseiten und die Routen „auf
  jedem Host“; dorthin zeigt der öffentliche Proxy. Unter der Seite: `/`, `/history`,
  `/feed.atom`, `/feed.rss`, `/logo`, `/_status/*` (CSS, JS, Schrift aus `assets/status/`).
- `render.rs` baut HTML ohne Templates; jeder Text aus der DB läuft durch `esc`. Keine Cookies,
  keine fremden Quellen, CSP `default-src 'none'` + `'self'` für CSS/JS/Schrift. Das JS ist
  optional (Pfeiltasten über die Tagesbalken, Escape schließt Tooltips).
- Gesamtzustand → Nyu: alle down → `outage`/sad, irgendwas down/degraded → `partial`/puzzled
  (es gibt kein „worried“), Wartung → sleepy, sonst happy. `pending`/`paused` zählen nicht.
- 90 Tagesbalken in der Seiten-Zeitzone aus den Zustands-Events (`status::days`).
- Vorfälle: `incidents::spawn` hört auf den Bus. down/degraded öffnet (oder ändert die
  Auswirkung des offenen), down mit `unreachable_via_parent` hängt den Monitor an den offenen
  Vorfall eines Vorfahren, up/paused schließt mit „behoben“. Wartung/pending ändern nichts.
  Wartungsfenster der nächsten 14 Tage erscheinen als geplante Wartung.
- Assets: `web/scripts/status-assets.mjs` erzeugt die Nyu-SVGs und kopiert Schrift + `nyu.css`
  aus `@uwusuite/design`; nach einem Design-Update neu ausführen.

## Verdichtung und Aufbewahrung

Fertige Stunden (und um Mitternacht UTC der Tag) werden zu `rollups` verdichtet, höchstens
48 Stunden je Tick nachgeholt. Danach werden Rohdaten nach `retention_raw_days`, Stunden nach
`retention_hourly_days`, Tage und Events nach `retention_daily_days` gelöscht. Die Uptime ist
zeitgewichtet aus den Zustands-Events; `pending`/`paused`/`maintenance` zählen nicht,
`degraded` zählt als erreichbar.

## Alarme (Phase 4)

```
Event-Bus ─> alerts::spawn (ein Task) ─> Monitor stumm/pausiert/Wartung? ─> Kanäle des Monitors
          ─> je Kanal: Text DE/EN ─> uwustatus_notify::Sender::send, Wiederholung nach 10 s/60 s/5 min
          ─> notification_log (letzte 100 je Kanal)
```

- Gesendet wird `StateChanged` mit `alert = true` (`down`, `degraded`, `up`), `Reminder`
  (`reminder`), `CertExpiring` (`cert`) und `VersionChanged` (`version`, standardmäßig aus).
  Welche davon ein Kanal bekommt, steht in seinen `events`.
- Zertifikats- und Versions-Events prüft der Dispatcher selbst gegen `muted`, `paused`,
  `maintenance` und aktive Wartungsfenster (der Kern filtert nur Zustandswechsel).
- Text: `{Monitor}: ausgefallen | eingeschränkt | wieder erreichbar | weiterhin ausgefallen |
  Zertifikat läuft in n Tagen ab | neue Version x`, dazu Grund, Zeit (UTC), Ausfalldauer und
  Link `{public_url}/monitors/{id}`.
- Neue Monitore bekommen alle Kanäle mit `is_default` (`channels::assign_defaults`).

**Crate `uwustatus-notify`** (ohne Server-Zustand, auch für den Satelliten): `Target { channel,
secrets, lang, smtp }` ist serialisierbar und vollständig; `Sender::send(&Target, &Message)`.
`Channel` = `email { to }` | `webhook { url }` | `ntfy { server, topic, priority }` |
`gotify { url, priority }` | `pushover { priority }` | `discord {}` | `telegram { chat_id }`.
Geheimnisse (`Channel::secret_names`): Webhook `hmac_secret`, ntfy/Gotify `token`, Pushover
`user_key` + `token`, Discord `webhook_url`, Telegram `bot_token`. Nur http/https, 15 s
Timeout, keine Redirects, Antworten höchstens 64 KiB gelesen, URLs nie in Fehlertexten.

**Webhook-Body:** `{ "title", "body", "severity": "down|degraded|up|info", "link", "event" }`
(`event` = das Bus-Event als JSON, beim Test `{ "type": "test" }`). Mit `hmac_secret` trägt
`X-UwUStatus-Signature` den Wert `sha256=<hex HMAC-SHA256 über den Body>`.

| Tabelle | Inhalt |
|---|---|
| `notification_channels` | `config` (JSON: `name`, `enabled`, `lang`, `events`, `is_default`, `channel`), `secrets` (JSON, nie in Antworten) |
| `monitor_channels` | `(monitor_id, channel_id)`, beide `ON DELETE CASCADE` |
| `notification_log` | `channel_id`, `at`, `monitor_id`, `event`, `title`, `ok`, `attempts`, `error` |
| `settings` | `smtp` (globale SMTP-Einstellungen) |

| Methode | Pfad | |
|---|---|---|
| GET, POST | `/api/channels` | Antworten zeigen nur `secrets: ["name", …]` (gesetzte Namen) |
| GET, PUT, DELETE | `/api/channels/{id}` | PUT: fehlendes Geheimnis bleibt, `""` löscht |
| POST | `/api/channels/{id}/test` | ein Versuch, protokolliert; Fehler ⇒ 502 mit Grund |
| GET | `/api/channels/{id}/log` | neueste zuerst |
| GET, PUT | `/api/monitors/{id}/channels` | Liste von Kanal-IDs |
| GET, PUT | `/api/smtp` | Passwort nur schreibbar (`password_set`); leerer `host` entfernt |

## Satelliten (Phase 6)

```
Satellit (QUIC-Server, UDP 7843)  <── MainClient je Satellit (Hauptinstanz wählt sich ein)
  Prüfungen nach Zuweisung ──> Ergebnisse mit seq ──> satellites::follow ──> ingest_result ──> ack
  Totmann: keine Verbindung > after_s ──> uwustatus_notify::Sender an die übergebenen Targets
  Notfall-Statusseite (127.0.0.1:8090) mit letztem bekannten Stand der übergebenen Monitore
```

- Kopplung: `uwustatus-satellite init` druckt einen einmaligen Code (Adressen, Fingerabdruck,
  Token). `POST /api/satellites` legt den Satelliten `pending` an und verbindet; nach `Paired`
  gilt er als `paired` und wird ab dann mit festem Fingerabdruck (`MainSettings::pinned`)
  verbunden. Die Identität der Hauptinstanz liegt in `data_dir/tunnel-identity.json` (0600).
- `satellites::start_all` beim Start; `Registry::refresh` schickt Zuweisungen (Monitore mit dem
  Satelliten in `locations`, nicht pausiert) und Totmann-Konfig bei jeder Änderung an Monitoren,
  Kanälen oder Totmann-Einstellungen neu. Der Tunnel cached beides und schickt es nach
  Reconnect erneut.
- Ergebnisse: Monitor muss den Standort führen, Meldung ≤ 1000 Zeichen, Zeit ≤ jetzt.
  `UNIQUE(location, seq)` macht erneutes Senden nach Reconnect idempotent; `ack` bis zur
  höchsten gespeicherten seq.
- Veraltet: ein Standort ohne Ergebnis seit mehr als 3 × Intervall zählt als `unknown` und
  fällt aus dem Quorum (`engine::stale`).
- Totmann-Konfig (`settings.dead_man`): Kanal-IDs werden zu vollständigen `Target`s (mit
  Geheimnissen und SMTP) und gehen über den gepinnten Tunnel an jeden Satelliten.

| Tabelle | Inhalt |
|---|---|
| `satellites` (Migration 5) | `id` (Slug), `name`, `region`, `status` (`pending`/`paired`), `code`, `fingerprint`, `version`, `last_seen` |

| Methode | Pfad | |
|---|---|---|
| GET, POST | `/api/satellites` | POST `{code, name, region}`; Liste mit `live` (verbunden, seit, Fehler, letztes Ergebnis) |
| PUT, DELETE | `/api/satellites/{id}` | DELETE nur, wenn kein Monitor ihn mehr nutzt |
| GET, PUT | `/api/satellites/dead-man` | `after_min` (0 = aus), `channel_ids`, Texte, Notfallseite |
