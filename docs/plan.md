# UwUStatus – Plan

Eigenes Uptime-Monitoring der UwUSuite: prüft Dienste von zu Hause und über
Satelliten von außen, alarmiert über viele Kanäle und zeigt öffentliche
Statusseiten mit Nyu. Ersetzt Uptime Kuma (Monitore werden neu angelegt).

Entscheidungen stehen im Memory (`uwustatus.md`) und hier in Kurzform.

## Aufbau

Cargo-Workspace, ein Docker-Image für die Hauptinstanz, ein eigenes Binary
für Satelliten (Packaging wie `uwumail-gateway`).

| Crate | Inhalt |
|---|---|
| `uwustatus-probe` | Alle Prüfungen (HTTP/Keyword/JSON/Zert, TCP, Ping, DNS, SMTP/IMAP + STARTTLS, Docker über Socket-Proxy, Suite-`/healthz`). Läuft in Hauptinstanz und Satellit. |
| `uwustatus-tunnel` | QUIC-Tunnel Satellit → Hauptinstanz, Ed25519-Fingerprint-Pinning, Pairing-Code `uwusat1…` (Prinzip aus `uwumail-tunnel`, eigener Code, keine Abhängigkeit aufs Mail-Repo). |
| `uwustatus-server` | axum + SQLite, Planer, Zustandsmaschine, Alarme, Statusseiten, API, `/metrics`, eingebettetes `web/dist`. |
| `uwustatus-satellite` | Holt Aufträge über den Tunnel, prüft, meldet Ergebnisse; Totmann: eigener Alarm + Notfall-Statusseite, wenn die Hauptinstanz weg ist. |
| `web/` | React/Vite, `@uwusuite/design`, DE/EN, PWA. |

Richtung der Verbindung wie beim Gateway: die Hauptinstanz steht zu Hause
hinter NAT, darum **wählt sie sich beim Satelliten ein** (QUIC über UDP auf dem
VPS). Der Satellit pinnt die Hauptinstanz per Fingerprint aus dem Pairing.
Verliert der Satellit die Verbindung länger als die eingestellte Frist, greift
der Totmann.

## Fachliche Regeln

- Monitor: Typ, Ziel, Intervall, Timeout, Wiederholungen bis Down,
  Erinnerung alle X Min., stumm, Eltern-Monitor (Kind wird nicht alarmiert,
  wenn Eltern down), Standorte (lokal + Satelliten), Quorum je Monitor
  (einer/Mehrheit/alle).
- Zustände: up, degraded (nur ein Teil der Standorte), down, maintenance, paused.
- Aufbewahrung im UI einstellbar (Rohdaten, Stunden-, Tageswerte).
- Wartungsfenster (einmalig + wiederkehrend) unterdrücken Alarme und Vorfälle.
- Vorfälle entstehen automatisch **öffentlich** auf den Statusseiten der
  betroffenen Monitore, Text ergänzbar, schließen sich bei up.
- Kanäle: SMTP (UwUMail), Webhook (JSON), Push-Dienst im UI wählbar
  (ntfy, Gotify, Pushover), Discord, Telegram, SoChat.
- Statusseiten: mehrere, eigene Domain je Seite (Host-Header), Nyu-Stimmung,
  90-Tage-Balken, Vorfälle, Wartung, RSS/Atom. Keine Cookies, keine Fremd-
  ressourcen.
- Login: UwUAuth-OIDC + lokaler Admin (Passwort + TOTP) als Fallback.
- API: Lese-API mit Token, SVG-Badges, Prometheus `/metrics` (Token).
- Suite-`/healthz`: einheitliches JSON-Format (siehe `docs/healthz.md`),
  wird in allen UwU-Servern eingebaut.

## Phasen

1. [x] Gerüst: Workspace, CI (GitHub-hosted, Cache, concurrency), Dockerfile,
       Web-Gerüst aus UwUDNS (OIDC, Session, i18n), lokaler Admin.
2. [ ] `uwustatus-probe` mit allen Typen + Tests gegen lokale Fakes.
3. [ ] Planer, Zustandsmaschine (Wiederholungen, Quorum, Eltern, Wartung),
       Speicherung + Verdichtung.
4. [ ] Alarm-Kanäle.
5. [ ] Statusseiten, Vorfälle, RSS, Badges, API, `/metrics`.
6. [ ] Tunnel + Satellit + Totmann + Notfall-Statusseite, Packaging wie Gateway.
7. [ ] Web-UI komplett (Dashboard, Monitore, Kanäle, Seiten, Wartung,
       Satelliten, Einstellungen).
8. [ ] `docs/healthz.md` + Einbau in Mail, Auth, Lock, Sync, DNS, Clip,
       je Patch-Release.
9. [ ] Security-Review, Release 0.1.0, Deploy mfxdocker2 (Admin intern über
       caddy-internal, status.minifyx.de öffentlich), Satellit auf dem
       Gateway-VPS, Kuma abschalten, Produktlisten (Website, Profil-README,
       mfxweb).
