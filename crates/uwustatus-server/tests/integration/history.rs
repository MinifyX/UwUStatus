//! Rollups, retention and uptime against a real (in-memory) database.

use crate::monitors::{outcome, tcp};
use crate::support::*;
use uwustatus_probe::Status;
use uwustatus_server::history::{maintain, uptime};
use uwustatus_server::ingest::ingest_result;
use uwustatus_server::settings::Settings;

/// 2026-01-01T00:00:00Z
const T0: i64 = 1_767_225_600;

#[tokio::test]
async fn rollups_and_retention() {
    let env = env().await;
    let cookie = env.admin().await;
    let id = env.monitor(&cookie, tcp("Web")).await["id"].as_i64().unwrap();
    for (i, status) in [Status::Up, Status::Up, Status::Degraded, Status::Down].into_iter().enumerate() {
        let mut o = outcome(status);
        o.latency_ms = 10 * (i as u64 + 1);
        ingest_result(&env.state, id, "local", None, T0 + 60 * i as i64, &o).unwrap();
    }
    ingest_result(&env.state, id, "local", None, T0 + 3600 + 5, &outcome(Status::Up)).unwrap();
    let settings = Settings { retention_raw_days: 1, ..Settings::default() };
    let db = &env.state.db;
    type Roll = (i64, i64, i64, i64, Option<f64>, Option<i64>, Option<f64>);
    let rollups = |period: &str| -> Vec<Roll> {
        db.with(|c| {
            let mut stmt = c.prepare(
                "SELECT start, up, degraded, down, latency_avg, latency_p95, uptime FROM rollups WHERE period = ?1 ORDER BY start",
            )?;
            stmt.query_map([period], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))?
                .collect()
        })
        .unwrap()
    };
    let raw = || db.with(|c| c.query_row("SELECT COUNT(*) FROM results", [], |r| r.get::<_, i64>(0))).unwrap();

    // Only finished hours; running it twice changes nothing.
    db.with(|c| maintain(c, T0 + 3600 + 10, &settings)).unwrap();
    db.with(|c| maintain(c, T0 + 3600 + 20, &settings)).unwrap();
    assert_eq!(rollups("hour"), [(T0, 2, 1, 1, Some(20.0), Some(30), Some(75.0))]);
    db.with(|c| maintain(c, T0 + 86400 + 10, &settings)).unwrap();
    assert_eq!(rollups("hour").len(), 2);
    assert_eq!(rollups("day"), [(T0, 3, 1, 1, Some(20.0), Some(30), Some(80.0))]);
    // Raw data past one day goes, the aggregates stay.
    assert_eq!(raw(), 4);
    db.with(|c| maintain(c, T0 + 2 * 86400, &settings)).unwrap();
    assert_eq!((raw(), rollups("hour").len(), rollups("day").len()), (0, 2, 1));
    // Past hourly retention the hour rows go as well.
    db.with(|c| maintain(c, T0 + 91 * 86400, &settings)).unwrap();
    assert_eq!((rollups("hour").len(), rollups("day").len()), (0, 1));
}

#[tokio::test]
async fn uptime_is_time_weighted() {
    let env = env().await;
    let cookie = env.admin().await;
    let id = env.monitor(&cookie, tcp("Web")).await["id"].as_i64().unwrap();
    for (at, status) in [(T0, Status::Up), (T0 + 600, Status::Down), (T0 + 900, Status::Up), (T0 + 1000, Status::Up)] {
        ingest_result(&env.state, id, "local", None, at, &outcome(status)).unwrap();
    }
    assert_eq!(uptime(&env.state.db, id, T0, T0 + 1200).unwrap(), Some(75.0));
    assert_eq!(uptime(&env.state.db, id, T0 + 600, T0 + 900).unwrap(), Some(0.0));
    // Before the first result the monitor was pending: nothing to count.
    assert_eq!(uptime(&env.state.db, id, T0 - 600, T0).unwrap(), None);
}
