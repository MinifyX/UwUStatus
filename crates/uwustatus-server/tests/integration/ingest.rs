//! `ingest_result` and the state machine end to end: retries, quorum, seq dedup, parents,
//! maintenance, reminders, version and certificate events.

use crate::monitors::{outcome, tcp};
use crate::support::*;
use serde_json::json;
use tokio::sync::broadcast;
use uwustatus_probe::{Outcome, Status};
use uwustatus_server::engine::State;
use uwustatus_server::events::Event;
use uwustatus_server::ingest::{Ingested, ingest_result, tick_states};

fn drain(rx: &mut broadcast::Receiver<Event>) -> Vec<Event> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

/// `(from, to, alert, via_parent)` of the state changes.
fn changes(events: &[Event]) -> Vec<(State, State, bool, bool)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::StateChanged { from, to, alert, unreachable_via_parent, .. } => {
                Some((*from, *to, *alert, *unreachable_via_parent))
            }
            _ => None,
        })
        .collect()
}

fn state_of(env: &Env, id: i64) -> String {
    env.state
        .db
        .with(|c| c.query_row("SELECT state FROM monitor_state WHERE monitor_id = ?1", [id], |r| r.get(0)))
        .unwrap()
}

#[tokio::test]
async fn retries_then_down() {
    let env = env().await;
    let cookie = env.admin().await;
    let mut m = tcp("Web");
    m["retries"] = json!(1);
    let id = env.monitor(&cookie, m).await["id"].as_i64().unwrap();
    let mut rx = env.state.events.subscribe();
    let ingest = |at, status| ingest_result(&env.state, id, "local", None, at, &outcome(status)).unwrap();

    assert_eq!(ingest(100, Status::Up), Ingested { duplicate: false, retrying: false });
    assert_eq!(ingest(160, Status::Down), Ingested { duplicate: false, retrying: true });
    assert_eq!(state_of(&env, id), "up");
    assert_eq!(ingest(170, Status::Down), Ingested { duplicate: false, retrying: false });
    // An older result is stored but does not move the state.
    ingest(50, Status::Up);
    assert_eq!(state_of(&env, id), "down");
    ingest(230, Status::Up);
    assert_eq!(
        changes(&drain(&mut rx)),
        [
            (State::Pending, State::Up, false, false),
            (State::Up, State::Down, true, false),
            (State::Down, State::Up, true, false)
        ]
    );
}

#[tokio::test]
async fn quorum_and_seq_dedup() {
    let env = env().await;
    let cookie = env.admin().await;
    env.state
        .db
        .with(|c| {
            c.execute_batch("INSERT INTO satellites (id, name, created) VALUES ('sat-a', 'A', 0), ('sat-b', 'B', 0);")
        })
        .unwrap();
    let mut m = tcp("Web");
    m["locations"] = json!(["local", "sat-a", "sat-b"]);
    m["quorum"] = json!("majority");
    let id = env.monitor(&cookie, m).await["id"].as_i64().unwrap();
    let ingest = |loc, seq, at, status| ingest_result(&env.state, id, loc, seq, at, &outcome(status)).unwrap();

    ingest("local", None, 100, Status::Up);
    ingest("sat-a", Some(1), 100, Status::Up);
    // One location is enough for up, the others are still pending.
    assert_eq!(state_of(&env, id), "up");
    ingest("sat-b", Some(1), 100, Status::Up);
    ingest("sat-a", Some(2), 160, Status::Down);
    assert_eq!(state_of(&env, id), "degraded");
    let reason: String = env
        .state
        .db
        .with(|c| c.query_row("SELECT reason FROM monitor_state WHERE monitor_id = ?1", [id], |r| r.get(0)))
        .unwrap();
    assert!(reason.contains("1 von 3") && reason.contains("sat-a"), "{reason}");
    // The same (location, seq) again is a duplicate, even with another result.
    assert!(ingest("sat-a", Some(2), 170, Status::Up).duplicate);
    assert_eq!(state_of(&env, id), "degraded");
    ingest("sat-b", Some(2), 160, Status::Down);
    assert_eq!(state_of(&env, id), "down");
}

#[tokio::test]
async fn parent_down_masks_child() {
    let env = env().await;
    let cookie = env.admin().await;
    let parent = env.monitor(&cookie, tcp("Router")).await["id"].as_i64().unwrap();
    let mut child = tcp("Web");
    child["parent_id"] = json!(parent);
    let child = env.monitor(&cookie, child).await["id"].as_i64().unwrap();
    let ingest = |id, at, status| ingest_result(&env.state, id, "local", None, at, &outcome(status)).unwrap();
    ingest(parent, 100, Status::Up);
    ingest(child, 100, Status::Up);
    let mut rx = env.state.events.subscribe();

    ingest(parent, 160, Status::Down);
    ingest(child, 161, Status::Down);
    assert_eq!(
        changes(&drain(&mut rx)),
        [(State::Up, State::Down, true, false), (State::Up, State::Down, false, true)]
    );
    // Parent back, child still down: now that is news.
    ingest(parent, 220, Status::Up);
    assert_eq!(
        changes(&drain(&mut rx)),
        [(State::Down, State::Up, true, false), (State::Down, State::Down, true, false)]
    );

    // A child failing while the parent is down is masked, and so is its recovery.
    ingest(child, 230, Status::Up);
    ingest(parent, 280, Status::Down);
    ingest(child, 290, Status::Down);
    ingest(child, 300, Status::Up);
    let events = drain(&mut rx);
    assert_eq!(
        changes(&events)[1..],
        [
            (State::Up, State::Down, true, false),
            (State::Up, State::Down, false, true),
            (State::Down, State::Up, false, false)
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn maintenance_and_reminders() {
    let t0 = 1_800_000_000;
    let env = env_at(t0).await;
    let cookie = env.admin().await;
    let mut m = tcp("Web");
    m["reminder_min"] = json!(5);
    let id = env.monitor(&cookie, m).await["id"].as_i64().unwrap();
    let mut rx = env.state.events.subscribe();
    ingest_result(&env.state, id, "local", None, t0, &outcome(Status::Down)).unwrap();
    assert_eq!(changes(&drain(&mut rx)), [(State::Pending, State::Down, true, false)]);

    tokio::time::advance(std::time::Duration::from_secs(299)).await;
    tick_states(&env.state);
    assert!(drain(&mut rx).is_empty());
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    tick_states(&env.state);
    assert!(matches!(drain(&mut rx)[..], [Event::Reminder { since, .. }] if since == t0));
    tick_states(&env.state);
    assert!(drain(&mut rx).is_empty(), "one reminder per interval");

    let now = env.state.clock.now();
    let window =
        json!({ "title": "Update", "schedule": { "type": "once", "start": now, "end": now + 600 }, "monitors": [id] });
    assert_eq!(
        env.send("POST", "/api/maintenance", Some(&cookie), Some(window)).await.status,
        axum::http::StatusCode::CREATED
    );
    assert_eq!(changes(&drain(&mut rx)), [(State::Down, State::Maintenance, false, false)]);
    // Results inside the window change nothing visible.
    ingest_result(&env.state, id, "local", None, now + 10, &outcome(Status::Up)).unwrap();
    assert!(changes(&drain(&mut rx)).is_empty());
    tokio::time::advance(std::time::Duration::from_secs(600)).await;
    tick_states(&env.state);
    assert_eq!(changes(&drain(&mut rx)), [(State::Maintenance, State::Up, false, false)]);
}

#[tokio::test]
async fn version_and_certificate() {
    let env = env().await;
    let cookie = env.admin().await;
    let id = env.monitor(&cookie, tcp("Web")).await["id"].as_i64().unwrap();
    let mut rx = env.state.events.subscribe();
    let ingest = |at, version: &str, cert_days| {
        let mut o: Outcome = outcome(Status::Up);
        o.details.version = Some(version.into());
        o.details.cert_days_left = cert_days;
        ingest_result(&env.state, id, "local", None, at, &o).unwrap();
    };
    ingest(100, "1.0", Some(60));
    ingest(160, "1.1", Some(60));
    assert!(
        drain(&mut rx)
            .iter()
            .any(|e| matches!(e, Event::VersionChanged { from, to, .. } if from == "1.0" && to == "1.1"))
    );
    ingest(220, "1.1", Some(5));
    ingest(280, "1.1", Some(4));
    let events = drain(&mut rx);
    assert_eq!(
        events.iter().filter(|e| matches!(e, Event::CertExpiring { days_left: 5, .. })).count(),
        1,
        "{events:?}"
    );
    assert!(!events.iter().any(|e| matches!(e, Event::CertExpiring { days_left: 4, .. })));
    assert_eq!(state_of(&env, id), "degraded");
    // Renewed: up again, and the next expiry warns again.
    ingest(340, "1.1", Some(90));
    assert_eq!(state_of(&env, id), "up");
    ingest(400, "1.1", Some(3));
    assert!(drain(&mut rx).iter().any(|e| matches!(e, Event::CertExpiring { days_left: 3, .. })));
}
