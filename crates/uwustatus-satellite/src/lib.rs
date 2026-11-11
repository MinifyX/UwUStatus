//! UwUStatus satellite: runs the checks the main instance assigns, keeps their results while the
//! main instance is away, alarms when it stays away (dead man's switch) and can serve a small
//! emergency status page.

pub mod agent;
pub mod config;
pub mod deadman;
pub mod page;
pub mod state;

use std::collections::HashMap;
use std::sync::Mutex;

use deadman::DeadManConfig;
use uwustatus_probe::Status;

/// What the emergency page shows; kept up to date by the agent.
#[derive(Default)]
pub struct Live(Mutex<LiveState>);

#[derive(Default, Clone)]
pub struct LiveState {
    pub config: Option<DeadManConfig>,
    pub connected: bool,
    /// Unix seconds of the last connect or loss.
    pub since: Option<i64>,
    /// Latest own result per assignment id.
    pub latest: HashMap<String, Latest>,
}

#[derive(Debug, Clone)]
pub struct Latest {
    pub at: i64,
    pub status: Status,
    pub latency_ms: u64,
}

impl Live {
    pub fn get(&self) -> std::sync::MutexGuard<'_, LiveState> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub fn unix_now() -> i64 {
    jiff::Timestamp::now().as_second()
}

/// `2026-11-11 08:30 UTC`.
pub fn utc(seconds: i64) -> String {
    jiff::Timestamp::from_second(seconds).map(|t| t.strftime("%Y-%m-%d %H:%M UTC").to_string()).unwrap_or_default()
}
