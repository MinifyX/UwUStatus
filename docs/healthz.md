# Suite-`/healthz`

Ein gemeinsames Format, damit UwUStatus bei jedem UwU-Server Version und
Teilzustände erkennt (Monitor-Typ „UwUSuite-Server“).

## Anfrage

`GET /healthz`, ohne Anmeldung, ohne Cookies. Antwortet schnell (Ziel unter
1 s) und schreibt nichts.

## Antwort

```json
{
  "status": "ok",
  "version": "0.28.1",
  "checks": {
    "database": { "status": "ok" },
    "queue": { "status": "degraded", "message": "12 messages waiting longer than 1 h" }
  }
}
```

| Feld | Pflicht | Bedeutung |
|---|---|---|
| `status` | ja | `ok`, `degraded` oder `down`. Das schlechteste Ergebnis der `checks`. |
| `version` | ja | Version des Servers (`CARGO_PKG_VERSION`). |
| `checks` | ja, darf leer sein | Teilprüfungen, Name → `{status, message?}`. |
| `checks.*.message` | nein | Kurzer Text für Menschen, höchstens 200 Zeichen. |

HTTP-Status: **200** bei `ok` und `degraded`, **503** bei `down`. So bleiben
Docker-Healthchecks, `install.sh` und `update.sh` gültig, die nur auf den
Statuscode schauen.

## Regeln

- Abwärtskompatibel: Felder, die ein Server heute schon liefert (`ok`,
  `database`, `uptimeSeconds` …), bleiben erhalten. Die neuen Felder kommen
  dazu.
- Die Endpunkte sind öffentlich: `message` enthält keine Hostnamen, Pfade,
  Adressen, Kontonamen oder Fehlertexte von Bibliotheken. Details gehören ins
  Log.
- Teilprüfungen sind billig: z. B. ein `SELECT 1` oder ein Blick auf einen
  Zähler im Speicher. Keine Anfragen an externe Dienste bei jedem Aufruf. Was
  teuer ist, prüft ein Hintergrundjob, und `/healthz` liest dessen letztes
  Ergebnis.
- Namen der Teilprüfungen: kleingeschrieben, `snake_case`, stabil
  (UwUStatus merkt sie sich je Monitor).

## Bewertung in UwUStatus

- `ok` → up, `degraded` → degraded (zählt nicht als Ausfall, wird aber
  angezeigt und kann alarmieren), `down`, 5xx, Timeout oder kein gültiges
  JSON → down.
- Ein Versionswechsel wird im Verlauf des Monitors vermerkt.
