# Satellit

Ein Satellit prüft Monitore von einem anderen Standort aus und meldet die Ergebnisse an die
Hauptinstanz. Fällt die Hauptinstanz aus, schlägt er Alarm (Totmann) und zeigt eine
Notfall-Statusseite.

## Installation (Debian/Ubuntu, amd64 oder arm64)

1. `uwustatus-satellite-<version>-linux-<arch>.tar.gz` aus dem GitHub-Release laden (enthält
   Binary und `deploy/satellite/`), gegen `SHA256SUMS` prüfen und entpacken.
2. `sudo ./install.sh ./uwustatus-satellite` – legt Benutzer, `/etc/uwustatus-satellite/satellite.toml`
   und den systemd-Dienst an und druckt den Kopplungscode.
3. Vorher bzw. danach in `satellite.toml` `public_addresses` prüfen. Der Code enthält diese
   Adressen; ändern sie sich, neu koppeln.
4. Eingehend UDP 7843 freigeben.

Alternativ als Container: `docker build -f docker/Dockerfile.satellite .` (distroless,
läuft als nonroot, Zustand in `/var/lib/uwustatus-satellite`, Port 7843/udp und 8090).
Code dann mit `docker run --rm -v <volume>:/var/lib/uwustatus-satellite <image> init`.

## Koppeln

In UwUStatus unter **Satelliten** Code, Name und Region eintragen. Der Code gilt einmal;
danach verbindet sich die Hauptinstanz nur noch mit dem gespeicherten Fingerabdruck.
Anschließend in den Monitoren den Satelliten als Standort wählen.

`uwustatus-satellite status` zeigt Kopplung, Zahl der Prüfungen, Totmann und Puffer.
`uwustatus-satellite unpair` löscht die Kopplung (Dienst neu starten, dann `init`).

## Totmann und Notfallseite

Unter **Satelliten → Totmann** einstellen: nach wie vielen Minuten ohne Verbindung, welche
Kanäle, Texte DE/EN. Die Kanäle werden samt Zugangsdaten an die Satelliten übertragen
(verschlüsselter, gepinnter Tunnel) und dort mit 0600 gespeichert.

Die Notfallseite lauscht auf `[emergency] listen` (Vorschlag `127.0.0.1:8090`) und zeigt
die gewählten Monitore mit dem letzten bekannten Stand. Sie antwortet nur, wenn sie in
UwUStatus aktiviert ist und der `Host` zur eingestellten Domain passt. Davor einen
Reverse-Proxy mit TLS setzen, siehe `deploy/satellite/Caddyfile`.

## Sicherheit

- Der Dienst läuft als eigener Benutzer ohne Capabilities, mit `ProtectSystem=strict`.
- Der Satellit nimmt nur Verbindungen der gekoppelten Hauptinstanz an (Fingerabdruck).
- Ergebnisse werden von der Hauptinstanz nur für Monitore angenommen, die den Satelliten
  als Standort führen.
