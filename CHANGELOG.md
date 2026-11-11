# Changelog

## 0.1.0

First release.

- Server (`uwustatus`): SQLite with migrations, scheduler, state machine with retries, quorum per monitor (one/majority/all locations), parent monitors, reminders, mute, maintenance windows (one-off and recurring), retention and rollups set in the UI.
- Checks: HTTP(S) with keyword, JSON path and certificate expiry, TCP, ping, DNS, SMTP/IMAP with STARTTLS certificate, push/heartbeat, Docker via socket proxy, UwUSuite `/healthz`.
- Alerts: SMTP, webhook, ntfy, Gotify, Pushover, Discord, Telegram; send log per channel.
- Status pages: several per instance with their own domains, automatic public incidents, updates, maintenance, 90-day bars, RSS/Atom, Nyu mood; no cookies, no third-party resources.
- API: read API with tokens, SVG badges, Prometheus `/metrics`.
- Satellites (`uwustatus-satellite`): external checks over a QUIC tunnel with fingerprint pinning and `uwusat1…` pairing codes; dead-man alert and emergency status page when the main instance is gone. systemd unit and install script under `deploy/satellite/`.
- Sign-in via UwUAuth (OIDC) and a local admin (Argon2 password, optional TOTP).
- Web app (German/English, PWA) with dashboard, monitors, channels, status pages, incidents, maintenance, satellites, API tokens and settings.
- Optional separate public listener (`UWUSTATUS_PUBLIC_BIND`) that never serves the admin app.
- Container image for amd64 and arm64 on ghcr.io; satellite binaries for amd64 and arm64 attached to the release.
