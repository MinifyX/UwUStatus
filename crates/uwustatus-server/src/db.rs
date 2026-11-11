//! SQLite with numbered migrations (`PRAGMA user_version`).

use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::Mutex;

/// Append only: entry `n` takes the schema from version `n` to `n + 1`. Never edit a released one.
const MIGRATIONS: &[&str] = &[
    "
CREATE TABLE sessions (
    id_hash TEXT PRIMARY KEY,
    sub TEXT NOT NULL,
    name TEXT NOT NULL,
    email TEXT,
    method TEXT NOT NULL,
    created INTEGER NOT NULL,
    last_seen INTEGER NOT NULL
);
CREATE TABLE pending_logins (
    cookie_hash TEXT PRIMARY KEY,
    state TEXT NOT NULL,
    nonce TEXT NOT NULL,
    verifier TEXT NOT NULL,
    next TEXT NOT NULL,
    started INTEGER NOT NULL
);
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
",
    "
CREATE TABLE satellites (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    created INTEGER NOT NULL,
    last_seen INTEGER
);
CREATE TABLE monitors (
    id INTEGER PRIMARY KEY,
    config TEXT NOT NULL,
    push_token TEXT UNIQUE,
    created INTEGER NOT NULL,
    updated INTEGER NOT NULL
);
CREATE TABLE monitor_state (
    monitor_id INTEGER PRIMARY KEY REFERENCES monitors (id) ON DELETE CASCADE,
    state TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    since INTEGER NOT NULL,
    via_parent INTEGER NOT NULL DEFAULT 0,
    last_at INTEGER,
    latency_ms INTEGER,
    message TEXT,
    version TEXT,
    cert_days_left INTEGER,
    cert_warned INTEGER NOT NULL DEFAULT 0,
    last_reminder INTEGER
);
CREATE TABLE location_state (
    monitor_id INTEGER NOT NULL REFERENCES monitors (id) ON DELETE CASCADE,
    location TEXT NOT NULL,
    state TEXT NOT NULL,
    failures INTEGER NOT NULL,
    at INTEGER NOT NULL,
    message TEXT NOT NULL,
    PRIMARY KEY (monitor_id, location)
);
CREATE TABLE results (
    id INTEGER PRIMARY KEY,
    monitor_id INTEGER NOT NULL REFERENCES monitors (id) ON DELETE CASCADE,
    location TEXT NOT NULL,
    seq INTEGER,
    at INTEGER NOT NULL,
    status TEXT NOT NULL,
    latency_ms INTEGER NOT NULL,
    message TEXT NOT NULL,
    details TEXT NOT NULL,
    UNIQUE (location, seq)
);
CREATE INDEX results_monitor_at ON results (monitor_id, at);
CREATE INDEX results_at ON results (at);
CREATE TABLE rollups (
    monitor_id INTEGER NOT NULL REFERENCES monitors (id) ON DELETE CASCADE,
    period TEXT NOT NULL,
    start INTEGER NOT NULL,
    up INTEGER NOT NULL,
    degraded INTEGER NOT NULL,
    down INTEGER NOT NULL,
    latency_avg REAL,
    latency_p95 INTEGER,
    uptime REAL,
    PRIMARY KEY (monitor_id, period, start)
);
CREATE TABLE events (
    id INTEGER PRIMARY KEY,
    monitor_id INTEGER NOT NULL REFERENCES monitors (id) ON DELETE CASCADE,
    at INTEGER NOT NULL,
    kind TEXT NOT NULL,
    from_value TEXT NOT NULL,
    to_value TEXT NOT NULL,
    reason TEXT NOT NULL
);
CREATE INDEX events_monitor_at ON events (monitor_id, kind, at);
CREATE INDEX events_at ON events (at);
CREATE TABLE maintenance (
    id INTEGER PRIMARY KEY,
    config TEXT NOT NULL
);
",
    // Migration 3: alerts
    "
CREATE TABLE notification_channels (
    id INTEGER PRIMARY KEY,
    config TEXT NOT NULL,
    secrets TEXT NOT NULL DEFAULT '{}',
    created INTEGER NOT NULL,
    updated INTEGER NOT NULL
);
CREATE TABLE monitor_channels (
    monitor_id INTEGER NOT NULL REFERENCES monitors (id) ON DELETE CASCADE,
    channel_id INTEGER NOT NULL REFERENCES notification_channels (id) ON DELETE CASCADE,
    PRIMARY KEY (monitor_id, channel_id)
);
CREATE INDEX monitor_channels_channel ON monitor_channels (channel_id);
CREATE TABLE notification_log (
    id INTEGER PRIMARY KEY,
    channel_id INTEGER NOT NULL REFERENCES notification_channels (id) ON DELETE CASCADE,
    at INTEGER NOT NULL,
    monitor_id INTEGER,
    event TEXT NOT NULL,
    title TEXT NOT NULL,
    ok INTEGER NOT NULL,
    attempts INTEGER NOT NULL,
    error TEXT NOT NULL
);
CREATE INDEX notification_log_channel ON notification_log (channel_id, id);
",
    // Migration 4: status pages
    "
CREATE TABLE status_pages (
    id INTEGER PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    config TEXT NOT NULL,
    logo BLOB,
    logo_type TEXT,
    created INTEGER NOT NULL,
    updated INTEGER NOT NULL
);
CREATE TABLE monitor_public (
    monitor_id INTEGER PRIMARY KEY REFERENCES monitors (id) ON DELETE CASCADE,
    public_id TEXT NOT NULL UNIQUE,
    badge_public INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE incidents (
    id INTEGER PRIMARY KEY,
    title TEXT NOT NULL,
    auto INTEGER NOT NULL,
    monitor_id INTEGER REFERENCES monitors (id) ON DELETE SET NULL,
    impact TEXT NOT NULL,
    status TEXT NOT NULL,
    started INTEGER NOT NULL,
    resolved INTEGER
);
CREATE INDEX incidents_open ON incidents (monitor_id, resolved);
CREATE TABLE incident_monitors (
    incident_id INTEGER NOT NULL REFERENCES incidents (id) ON DELETE CASCADE,
    monitor_id INTEGER NOT NULL REFERENCES monitors (id) ON DELETE CASCADE,
    PRIMARY KEY (incident_id, monitor_id)
);
CREATE INDEX incident_monitors_monitor ON incident_monitors (monitor_id);
CREATE TABLE incident_updates (
    id INTEGER PRIMARY KEY,
    incident_id INTEGER NOT NULL REFERENCES incidents (id) ON DELETE CASCADE,
    at INTEGER NOT NULL,
    status TEXT NOT NULL,
    body TEXT NOT NULL
);
CREATE INDEX incident_updates_incident ON incident_updates (incident_id, at);
CREATE TABLE api_tokens (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    scopes TEXT NOT NULL,
    created INTEGER NOT NULL,
    last_used INTEGER
);
",
    // Migration 5: satellites (the code holds addresses and fingerprint).
    "
ALTER TABLE satellites ADD COLUMN region TEXT NOT NULL DEFAULT '';
ALTER TABLE satellites ADD COLUMN code TEXT NOT NULL DEFAULT '';
ALTER TABLE satellites ADD COLUMN fingerprint TEXT;
ALTER TABLE satellites ADD COLUMN version TEXT;
",
];

pub struct Db(Mutex<Connection>);

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> rusqlite::Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&mut conn)?;
        Ok(Self(Mutex::new(conn)))
    }

    // shortcut: one connection behind a std mutex, queries are tiny; a pool when it ever gets busy.
    pub fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> rusqlite::Result<T> {
        f(&self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Schema version (number of applied migrations).
    pub fn version(&self) -> rusqlite::Result<i64> {
        self.with(|c| c.pragma_query_value(None, "user_version", |r| r.get(0)))
    }

    /// For `/healthz`.
    pub fn ping(&self) -> rusqlite::Result<()> {
        self.with(|c| c.query_row("SELECT 1", [], |_| Ok(())))
    }

    pub fn setting(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.with(|c| c.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional())
    }

    pub fn set_setting(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
            Ok(())
        })
    }
}

fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if current > latest_version() {
        // A newer UwUStatus wrote this file; running old code on it would corrupt it.
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "database schema {current} is newer than this build ({})",
            latest_version()
        )));
    }
    for (version, sql) in (0i64..).zip(MIGRATIONS).skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", version + 1)?;
        tx.commit()?;
    }
    Ok(())
}

pub fn latest_version() -> i64 {
    MIGRATIONS.len() as i64
}
