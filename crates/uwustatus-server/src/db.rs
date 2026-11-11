//! SQLite with numbered migrations (`PRAGMA user_version`).

use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::Mutex;

/// Append only: entry `n` takes the schema from version `n` to `n + 1`. Never edit a released one.
const MIGRATIONS: &[&str] = &["
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
"];

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
