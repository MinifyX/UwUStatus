#!/bin/sh
# Installiert den UwUStatus-Satelliten als systemd-Dienst.
# Aufruf als root: ./install.sh [pfad/zur/uwustatus-satellite]
set -eu
here=$(dirname "$0")
bin=${1:-$here/uwustatus-satellite}
[ "$(id -u)" = 0 ] || { echo "Bitte als root ausführen." >&2; exit 1; }
[ -x "$bin" ] || { echo "Binary nicht gefunden: $bin" >&2; exit 1; }

install -m 0755 "$bin" /usr/local/bin/uwustatus-satellite
id uwustatus-satellite >/dev/null 2>&1 ||
	useradd --system --home-dir /var/lib/uwustatus-satellite --shell /usr/sbin/nologin uwustatus-satellite
install -d -m 0700 -o uwustatus-satellite -g uwustatus-satellite /var/lib/uwustatus-satellite
install -d -m 0755 /etc/uwustatus-satellite
[ -e /etc/uwustatus-satellite/satellite.toml ] ||
	install -m 0644 "$here/satellite.toml" /etc/uwustatus-satellite/satellite.toml
install -m 0644 "$here/uwustatus-satellite.service" /etc/systemd/system/uwustatus-satellite.service
systemctl daemon-reload

echo "Konfiguration prüfen: /etc/uwustatus-satellite/satellite.toml (public_addresses!)"
if runuser -u uwustatus-satellite -- /usr/local/bin/uwustatus-satellite status | grep -q '^Nicht gekoppelt (uwustatus-satellite init)'; then
	runuser -u uwustatus-satellite -- /usr/local/bin/uwustatus-satellite init
fi
systemctl enable uwustatus-satellite
systemctl restart uwustatus-satellite
echo "UDP-Port 7843 in der Firewall freigeben."
