//! The pure core, no I/O: location state with retries, overall state by quorum, alert rule,
//! uptime from state changes and the planner's slot arithmetic.

use serde::{Deserialize, Serialize};
use uwustatus_probe::Status;

/// Overall state of a monitor. Locations only use `pending`, `up`, `degraded` and `down`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Pending,
    Up,
    Degraded,
    Down,
    Maintenance,
    Paused,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Pending => "pending",
            State::Up => "up",
            State::Degraded => "degraded",
            State::Down => "down",
            State::Maintenance => "maintenance",
            State::Paused => "paused",
        }
    }

    /// Unknown text (a newer build wrote it) reads as `pending`.
    pub fn parse(s: &str) -> Self {
        match s {
            "up" => State::Up,
            "degraded" => State::Degraded,
            "down" => State::Down,
            "maintenance" => State::Maintenance,
            "paused" => State::Paused,
            _ => State::Pending,
        }
    }
}

pub fn status_str(status: Status) -> &'static str {
    match status {
        Status::Up => "up",
        Status::Degraded => "degraded",
        Status::Down => "down",
    }
}

pub fn parse_status(s: &str) -> Option<Status> {
    match s {
        "up" => Some(Status::Up),
        "degraded" => Some(Status::Degraded),
        "down" => Some(Status::Down),
        _ => None,
    }
}

/// How many locations must report down before the monitor is down; fewer is `degraded`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quorum {
    #[default]
    Any,
    Majority,
    All,
}

impl Quorum {
    pub fn reached(self, down: usize, total: usize) -> bool {
        down > 0
            && match self {
                Quorum::Any => true,
                Quorum::Majority => down * 2 > total,
                Quorum::All => down >= total,
            }
    }
}

/// State of one location: `failures` counts consecutive down results.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loc {
    pub state: State,
    pub failures: u32,
}

impl Default for Loc {
    fn default() -> Self {
        Self { state: State::Pending, failures: 0 }
    }
}

/// One result moves a location on. Down needs `retries + 1` failures in a row; until then the
/// location keeps its previous state.
pub fn step(prev: Loc, status: Status, retries: u32) -> Loc {
    match status {
        Status::Up => Loc { state: State::Up, failures: 0 },
        Status::Degraded => Loc { state: State::Degraded, failures: 0 },
        Status::Down => {
            let failures = prev.failures.saturating_add(1);
            Loc { state: if failures > retries { State::Down } else { prev.state }, failures }
        }
    }
}

/// Still confirming a failure: the planner checks again after the retry interval.
pub fn retrying(loc: Loc) -> bool {
    loc.failures > 0 && loc.state != State::Down
}

pub struct LocInput<'a> {
    pub name: &'a str,
    pub loc: Loc,
    pub message: &'a str,
}

pub struct Inputs<'a> {
    /// Every configured location; ones without a result yet are `pending`.
    pub locations: &'a [LocInput<'a>],
    pub quorum: Quorum,
    pub paused: bool,
    pub in_maintenance: bool,
    pub parent_down: bool,
    pub cert_days_left: Option<i64>,
    /// 0 = no certificate warning.
    pub cert_warn_days: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Overall {
    pub state: State,
    pub reason: String,
    /// Down, but the parent is down too: shown as down, never alerted.
    pub via_parent: bool,
}

pub fn overall(i: &Inputs) -> Overall {
    let result = |state, reason: String| Overall { state, reason, via_parent: false };
    if i.paused {
        return result(State::Paused, String::new());
    }
    let total = i.locations.len();
    let label = |l: &LocInput| if total > 1 { format!("{}: {}", l.name, l.message) } else { l.message.to_owned() };
    let down: Vec<&LocInput> = i.locations.iter().filter(|l| l.loc.state == State::Down).collect();
    let known = i.locations.iter().filter(|l| l.loc.state != State::Pending).count();
    let (mut state, mut reason) = if i.quorum.reached(down.len(), total) {
        (State::Down, down.iter().map(|l| label(l)).collect::<Vec<_>>().join("; "))
    } else if !down.is_empty() {
        let names: Vec<&str> = down.iter().map(|l| l.name).collect();
        (State::Degraded, format!("Teilausfall: {} von {total} Standorten down ({})", down.len(), names.join(", ")))
    } else if let Some(l) = i.locations.iter().find(|l| l.loc.state == State::Degraded) {
        (State::Degraded, label(l))
    } else if known == 0 {
        (State::Pending, String::new())
    } else {
        (State::Up, String::new())
    };
    if state == State::Up
        && let Some(days) = i.cert_days_left.filter(|d| i.cert_warn_days > 0 && *d <= i64::from(i.cert_warn_days))
    {
        state = State::Degraded;
        reason =
            if days < 0 { "Zertifikat abgelaufen".into() } else { format!("Zertifikat läuft in {days} Tagen ab") };
    }
    let via_parent = state == State::Down && i.parent_down;
    if i.in_maintenance {
        state = State::Maintenance;
    }
    Overall { state, reason, via_parent }
}

/// Whether a change from `from` to `next` should reach the alert channels.
pub fn alerts(from: State, from_via_parent: bool, next: &Overall, muted: bool) -> bool {
    let quiet_target = matches!(next.state, State::Pending | State::Paused | State::Maintenance);
    // Coming back to up from a state that never alerted is no news.
    let silent_recovery = next.state == State::Up
        && (matches!(from, State::Pending | State::Paused | State::Maintenance) || from_via_parent);
    !muted && !next.via_parent && !quiet_target && !silent_recovery
}

/// Percent of `[from, to)` spent up or degraded, counting only time that was up, degraded or down
/// (pending, paused and maintenance are left out). `changes` are `(at, new state)`, sorted.
pub fn uptime(initial: State, changes: &[(i64, State)], from: i64, to: i64) -> Option<f64> {
    let (mut good, mut counted) = (0i64, 0i64);
    let mut tally = |state: State, span: i64| match state {
        State::Up | State::Degraded => {
            good += span;
            counted += span;
        }
        State::Down => counted += span,
        _ => {}
    };
    let (mut current, mut t) = (initial, from);
    for &(at, next) in changes.iter().filter(|(at, _)| *at > from && *at < to) {
        tally(current, at - t);
        (current, t) = (next, at);
    }
    tally(current, to - t);
    (counted > 0).then(|| good as f64 * 100.0 / counted as f64)
}

/// Fixed offset of a monitor inside its interval, so monitors spread out (jitter without drift).
pub fn phase(id: i64, interval: i64) -> i64 {
    (id.wrapping_mul(2_654_435_761).unsigned_abs() % interval.max(1) as u64) as i64
}

/// First slot strictly after `after` on the grid `phase + k * interval`.
pub fn next_slot(after: i64, interval: i64, phase: i64) -> i64 {
    let interval = interval.max(1);
    phase + ((after - phase).div_euclid(interval) + 1) * interval
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(results: &[Status], retries: u32) -> Loc {
        results.iter().fold(Loc::default(), |l, s| step(l, *s, retries))
    }

    #[test]
    fn retries_before_down() {
        use Status::*;
        assert_eq!(run(&[Up, Down], 2).state, State::Up);
        assert!(retrying(run(&[Up, Down], 2)));
        assert_eq!(run(&[Up, Down, Down], 2).state, State::Up);
        assert_eq!(run(&[Up, Down, Down, Down], 2), Loc { state: State::Down, failures: 3 });
        assert!(!retrying(run(&[Up, Down, Down, Down], 2)));
        assert_eq!(run(&[Down], 0).state, State::Down);
        assert_eq!(run(&[Down, Down, Up], 1), Loc { state: State::Up, failures: 0 });
        assert_eq!(run(&[Down], 1).state, State::Pending);
        assert_eq!(run(&[Up, Degraded], 1).state, State::Degraded);
    }

    fn locs(states: &[State]) -> Vec<LocInput<'static>> {
        const NAMES: [&str; 4] = ["local", "a", "b", "c"];
        states
            .iter()
            .zip(NAMES)
            .map(|(s, name)| LocInput { name, loc: Loc { state: *s, failures: 0 }, message: "x" })
            .collect()
    }

    fn eval(states: &[State], quorum: Quorum) -> Overall {
        overall(&Inputs {
            locations: &locs(states),
            quorum,
            paused: false,
            in_maintenance: false,
            parent_down: false,
            cert_days_left: None,
            cert_warn_days: 14,
        })
    }

    #[test]
    fn quorum_and_partial_outage() {
        use State::*;
        assert_eq!(eval(&[Up, Down, Up], Quorum::Any).state, Down);
        assert_eq!(eval(&[Up, Down, Up], Quorum::Majority).state, Degraded);
        assert!(eval(&[Up, Down, Up], Quorum::Majority).reason.starts_with("Teilausfall: 1 von 3"));
        assert_eq!(eval(&[Down, Down, Up], Quorum::Majority).state, Down);
        assert_eq!(eval(&[Down, Down, Up, Up], Quorum::Majority).state, Degraded);
        assert_eq!(eval(&[Down, Down, Up], Quorum::All).state, Degraded);
        assert_eq!(eval(&[Down, Down, Down], Quorum::All).state, Down);
        assert_eq!(eval(&[Down, Pending], Quorum::All).state, Degraded);
        assert_eq!(eval(&[Pending, Pending], Quorum::Any).state, Pending);
        assert_eq!(eval(&[Up, Pending], Quorum::Any).state, Up);
        assert_eq!(
            eval(&[Up, Degraded], Quorum::Any),
            Overall { state: Degraded, reason: "a: x".into(), via_parent: false }
        );
        assert_eq!(eval(&[Down], Quorum::Any).reason, "x");
    }

    #[test]
    fn paused_maintenance_parent_cert() {
        let l = locs(&[State::Down]);
        let base = Inputs {
            locations: &l,
            quorum: Quorum::Any,
            paused: false,
            in_maintenance: false,
            parent_down: true,
            cert_days_left: None,
            cert_warn_days: 14,
        };
        let o = overall(&base);
        assert_eq!((o.state, o.via_parent), (State::Down, true));
        assert!(!alerts(State::Up, false, &o, false));
        assert_eq!(overall(&Inputs { in_maintenance: true, ..base }).state, State::Maintenance);
        assert_eq!(overall(&Inputs { in_maintenance: true, paused: true, ..base }).state, State::Paused);

        let up = locs(&[State::Up]);
        let cert =
            |days, warn| overall(&Inputs { locations: &up, cert_days_left: Some(days), cert_warn_days: warn, ..base });
        assert_eq!(cert(30, 14).state, State::Up);
        assert_eq!(
            cert(5, 14),
            Overall { state: State::Degraded, reason: "Zertifikat läuft in 5 Tagen ab".into(), via_parent: false }
        );
        assert_eq!(cert(-1, 14).reason, "Zertifikat abgelaufen");
        assert_eq!(cert(5, 0).state, State::Up);
    }

    #[test]
    fn alert_rule() {
        let o = |state| Overall { state, reason: String::new(), via_parent: false };
        assert!(alerts(State::Up, false, &o(State::Down), false));
        assert!(!alerts(State::Up, false, &o(State::Down), true));
        assert!(alerts(State::Pending, false, &o(State::Down), false));
        assert!(alerts(State::Down, false, &o(State::Up), false));
        assert!(!alerts(State::Down, true, &o(State::Up), false));
        assert!(!alerts(State::Pending, false, &o(State::Up), false));
        assert!(!alerts(State::Up, false, &o(State::Maintenance), false));
        assert!(alerts(State::Up, false, &o(State::Degraded), false));
    }

    #[test]
    fn uptime_from_changes() {
        use State::*;
        assert_eq!(uptime(Up, &[], 0, 100), Some(100.0));
        assert_eq!(uptime(Up, &[(25, Down), (50, Up)], 0, 100), Some(75.0));
        // Maintenance and pending are left out of the total.
        assert_eq!(uptime(Pending, &[(50, Up), (75, Maintenance)], 0, 100), Some(100.0));
        assert_eq!(uptime(Down, &[(25, Maintenance), (75, Up)], 0, 100), Some(50.0));
        assert_eq!(uptime(Paused, &[], 0, 100), None);
        // Changes outside the range are ignored.
        assert_eq!(uptime(Down, &[(-5, Down), (100, Up)], 0, 100), Some(0.0));
    }

    #[test]
    fn slots_do_not_drift() {
        let p = phase(7, 60);
        assert!((0..60).contains(&p));
        assert_ne!(phase(1, 60), phase(2, 60));
        assert_eq!(next_slot(1000, 60, 10), 1030);
        assert_eq!(next_slot(1030, 60, 10), 1090);
        assert_eq!(next_slot(1089, 60, 10), 1090);
        // However late a check finished, the next slot stays on the grid.
        assert_eq!(next_slot(1095, 60, 10) % 60, 10);
        assert_eq!(next_slot(-5, 60, 10), 10);
    }
}
