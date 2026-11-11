//! Settings stored in SQLite as one JSON document.

use crate::app::AppState;
use crate::auth::Authed;
use crate::db::Db;
use crate::error::{ApiError, ApiResult, Body};
use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};

/// Later phases add their fields here, each with a default so old rows keep loading.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Days to keep raw check results, hourly and daily aggregates.
    pub retention_raw_days: u32,
    pub retention_hourly_days: u32,
    pub retention_daily_days: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { retention_raw_days: 7, retention_hourly_days: 90, retention_daily_days: 365 }
    }
}

pub fn load(db: &Db) -> Settings {
    db.setting("settings").ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(db: &Db, settings: &Settings) -> ApiResult<()> {
    db.set_setting("settings", &serde_json::to_string(settings).map_err(ApiError::internal)?)?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Update {
    retention_raw_days: Option<u32>,
    retention_hourly_days: Option<u32>,
    retention_daily_days: Option<u32>,
}

pub async fn get(State(state): State<AppState>, _: Authed) -> Json<Settings> {
    Json(load(&state.db))
}

pub async fn put(State(state): State<AppState>, user: Authed, Body(update): Body<Update>) -> ApiResult<Json<Settings>> {
    let mut s = load(&state.db);
    for (value, target, max, what) in [
        (update.retention_raw_days, &mut s.retention_raw_days, 90, "Rohdaten"),
        (update.retention_hourly_days, &mut s.retention_hourly_days, 730, "Stundenwerte"),
        (update.retention_daily_days, &mut s.retention_daily_days, 3650, "Tageswerte"),
    ] {
        if let Some(days) = value {
            if !(1..=max).contains(&days) {
                return Err(ApiError::invalid(format!("Aufbewahrung {what}: 1 bis {max} Tage")));
            }
            *target = days;
        }
    }
    if !(s.retention_raw_days <= s.retention_hourly_days && s.retention_hourly_days <= s.retention_daily_days) {
        return Err(ApiError::invalid("Aufbewahrung: Rohdaten ≤ Stundenwerte ≤ Tageswerte"));
    }
    save(&state.db, &s)?;
    tracing::info!("settings changed by {}", user.actor());
    Ok(Json(s))
}
