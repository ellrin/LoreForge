//! Per-member learning state (`progress/<member>/<node-id>.json`, design §4.2) and the
//! familiarity state machine (§4.4):
//!
//! ```text
//! created → learning(streak 0) ─ +1 d check ─ remembered → streak 1 ─ +3 d check ─ remembered → known
//!     ▲                                                                                  │
//!     └──── forgot / marked unfamiliar: streak 0, check again +1 d ◄─────────────────────┘
//! known: maintenance checks +30, +90 d (optional); no activity for 180 d → stale
//! (asked again the next time the term is encountered)
//! ```
//!
//! All intervals are in [`Intervals`]. Every transition takes `now`, so the machine is
//! deterministic (the knowledge base passes its injectable clock).

use chrono::Duration;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::clock::Time;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Learning,
    Known,
    Stale,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Learning => "learning",
            State::Known => "known",
            State::Stale => "stale",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// 記得
    Remembered,
    /// 忘了
    Forgot,
    /// 被標「不熟」
    Unfamiliar,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Remembered => "remembered",
            Outcome::Forgot => "forgot",
            Outcome::Unfamiliar => "unfamiliar",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Intervals {
    /// First check after creation (and after an early `remembered`): days.
    pub first_check_days: i64,
    /// Second check after the first `remembered`.
    pub second_check_days: i64,
    /// After `forgot` / `unfamiliar`.
    pub retry_days: i64,
    /// Number of consecutive `remembered` answers that make a card known.
    pub known_after: u32,
    /// Maintenance checks for known cards (days after becoming known, then after each
    /// maintenance check). Empty or `maintenance = false` turns them off.
    pub maintenance_days: Vec<i64>,
    pub maintenance: bool,
    /// Known cards without any activity for this long become stale.
    pub stale_after_days: i64,
    /// Count a `remembered` given before the check is due. Off by default: only
    /// answers at a due check advance the streak (they are still logged).
    pub count_early: bool,
}

impl Default for Intervals {
    fn default() -> Intervals {
        Intervals { first_check_days: 1, second_check_days: 3, retry_days: 1, known_after: 2, maintenance_days: vec![30, 90], maintenance: true, stale_after_days: 180, count_early: false }
    }
}

impl Intervals {
    fn after_streak(&self, streak: u32) -> i64 {
        if streak == 0 { self.first_check_days } else { self.second_check_days }
    }
    fn maintenance_at(&self, step: u32) -> Option<i64> {
        if !self.maintenance {
            return None;
        }
        self.maintenance_days.get(step as usize).copied()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    pub at: Time,
    /// created | remembered | forgot | unfamiliar | encountered | known | stale | shared
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    /// A `remembered` answered before the check was due (did not advance).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub early: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    pub state: State,
    pub streak: u32,
    pub next_check_at: Option<Time>,
    #[serde(default)]
    pub encounters: u32,
    #[serde(default)]
    pub last_seen_at: Option<Time>,
    /// Maintenance checks passed since the card became known.
    #[serde(default)]
    pub maintenance_step: u32,
    #[serde(default)]
    pub log: Vec<LogEntry>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// What a recall did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Step {
    /// The answer moved the card (streak, state or schedule).
    pub advanced: bool,
    pub became_known: bool,
}

impl Progress {
    /// A card just created and understood: learning, first check tomorrow.
    pub fn new(now: Time, via: Option<&str>, iv: &Intervals) -> Progress {
        Progress {
            state: State::Learning,
            streak: 0,
            next_check_at: Some(now + Duration::days(iv.first_check_days)),
            encounters: 0,
            last_seen_at: Some(now),
            maintenance_step: 0,
            log: vec![LogEntry { at: now, event: "created".into(), via: via.map(str::to_string), early: false }],
            extra: Map::new(),
        }
    }

    /// A card created as already mastered (the author knew the term before adding it):
    /// `created`, then straight to known, as [`Progress::recall`] would after the last
    /// remembered check: maintenance step 0, first maintenance check (none when
    /// maintenance is off), log entry `known`.
    pub fn new_known(now: Time, via: Option<&str>, iv: &Intervals) -> Progress {
        let mut p = Progress::new(now, via, iv);
        p.become_known(now, iv);
        p
    }

    pub fn is_due(&self, now: Time) -> bool {
        self.next_check_at.is_some_and(|t| t <= now)
    }

    /// Last time anything happened to this card for this member.
    pub fn last_activity(&self) -> Option<Time> {
        let log = self.log.iter().map(|e| e.at).max();
        match (log, self.last_seen_at) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }
    }

    /// Known cards untouched for `stale_after_days` become stale. Returns true when the
    /// state changed (the caller decides whether to save).
    pub fn refresh(&mut self, now: Time, iv: &Intervals) -> bool {
        if self.state != State::Known || self.is_due(now) {
            return false;
        }
        let Some(last) = self.last_activity() else { return false };
        if now - last >= Duration::days(iv.stale_after_days) {
            self.state = State::Stale;
            self.next_check_at = None;
            self.log.push(LogEntry { at: now, event: "stale".into(), via: None, early: false });
            return true;
        }
        false
    }

    fn log(&mut self, now: Time, event: &str, via: Option<&str>, early: bool) {
        self.log.push(LogEntry { at: now, event: event.into(), via: via.map(str::to_string), early });
    }

    /// Apply an answer (記得 / 忘了 / 不熟).
    pub fn recall(&mut self, outcome: Outcome, now: Time, via: Option<&str>, iv: &Intervals) -> Step {
        self.refresh(now, iv);
        self.last_seen_at = Some(now);
        match outcome {
            Outcome::Forgot | Outcome::Unfamiliar => {
                self.log(now, outcome.as_str(), via, false);
                self.state = State::Learning;
                self.streak = 0;
                self.maintenance_step = 0;
                self.next_check_at = Some(now + Duration::days(iv.retry_days));
                Step { advanced: true, became_known: false }
            }
            Outcome::Remembered => {
                let due = self.is_due(now) || self.next_check_at.is_none();
                match self.state {
                    State::Learning => {
                        if !due && !iv.count_early {
                            self.log(now, "remembered", via, true);
                            return Step { advanced: false, became_known: false };
                        }
                        self.log(now, "remembered", via, false);
                        self.streak += 1;
                        if self.streak >= iv.known_after.max(1) {
                            self.become_known(now, iv);
                            Step { advanced: true, became_known: true }
                        } else {
                            self.next_check_at = Some(now + Duration::days(iv.after_streak(self.streak)));
                            Step { advanced: true, became_known: false }
                        }
                    }
                    State::Known => {
                        if !self.is_due(now) {
                            self.log(now, "remembered", via, true);
                            return Step { advanced: false, became_known: false };
                        }
                        self.log(now, "remembered", via, false);
                        self.maintenance_step += 1;
                        self.next_check_at = iv.maintenance_at(self.maintenance_step).map(|d| now + Duration::days(d));
                        Step { advanced: true, became_known: false }
                    }
                    State::Stale => {
                        self.log(now, "remembered", via, false);
                        self.become_known(now, iv);
                        Step { advanced: true, became_known: true }
                    }
                }
            }
        }
    }

    fn become_known(&mut self, now: Time, iv: &Intervals) {
        self.state = State::Known;
        self.maintenance_step = 0;
        self.next_check_at = iv.maintenance_at(0).map(|d| now + Duration::days(d));
        self.log(now, "known", None, false);
    }

    /// The term was met somewhere (reader, slide, paper). A stale card becomes due now.
    pub fn encounter(&mut self, now: Time, via: Option<&str>, iv: &Intervals) {
        self.refresh(now, iv);
        self.encounters += 1;
        self.last_seen_at = Some(now);
        self.log(now, "encountered", via, false);
        if self.state == State::Stale {
            self.next_check_at = Some(now);
        }
    }

    /// 0–100 familiarity for sorting and graph colour.
    pub fn score(&self) -> u32 {
        match self.state {
            State::Learning => (10 + 25 * self.streak).min(60),
            State::Known => (70 + 10 * self.maintenance_step).min(100),
            State::Stale => 40,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;

    fn t0() -> Time {
        DateTime::parse_from_rfc3339("2026-11-08T21:10:00+08:00").unwrap()
    }
    fn d(n: i64) -> Time {
        t0() + Duration::days(n)
    }

    #[test]
    fn created_as_known() {
        let iv = Intervals::default();
        let p = Progress::new_known(t0(), Some("reader"), &iv);
        assert_eq!((p.state, p.streak, p.maintenance_step), (State::Known, 0, 0));
        assert_eq!(p.next_check_at, Some(d(30)));
        assert_eq!(p.log.iter().map(|e| e.event.as_str()).collect::<Vec<_>>(), ["created", "known"]);
        let off = Intervals { maintenance: false, ..Intervals::default() };
        assert_eq!(Progress::new_known(t0(), None, &off).next_check_at, None);
    }

    #[test]
    fn two_remembered_make_known() {
        let iv = Intervals::default();
        let mut p = Progress::new(t0(), Some("reader"), &iv);
        assert_eq!(p.state, State::Learning);
        assert_eq!(p.next_check_at, Some(d(1)));
        assert!(!p.is_due(t0()));
        assert!(p.recall(Outcome::Remembered, d(1), Some("desk"), &iv).advanced);
        assert_eq!((p.state, p.streak, p.next_check_at), (State::Learning, 1, Some(d(4))));
        let s = p.recall(Outcome::Remembered, d(4), None, &iv);
        assert!(s.became_known);
        assert_eq!(p.state, State::Known);
        assert_eq!(p.next_check_at, Some(d(34)));
        assert_eq!(p.score(), 70);
        // Maintenance +30 then +90, then none.
        p.recall(Outcome::Remembered, d(34), None, &iv);
        assert_eq!(p.next_check_at, Some(d(124)));
        p.recall(Outcome::Remembered, d(124), None, &iv);
        assert_eq!(p.next_check_at, None);
        assert_eq!(p.state, State::Known);
    }

    #[test]
    fn early_remember_does_not_count() {
        let iv = Intervals::default();
        let mut p = Progress::new(t0(), None, &iv);
        let s = p.recall(Outcome::Remembered, t0() + Duration::hours(1), None, &iv);
        assert!(!s.advanced);
        assert_eq!(p.streak, 0);
        assert!(p.log.last().unwrap().early);
        let iv2 = Intervals { count_early: true, ..Intervals::default() };
        p.recall(Outcome::Remembered, t0() + Duration::hours(2), None, &iv2);
        assert_eq!(p.streak, 1);
    }

    #[test]
    fn forgot_and_unfamiliar_reset() {
        let iv = Intervals::default();
        let mut p = Progress::new(t0(), None, &iv);
        p.recall(Outcome::Remembered, d(1), None, &iv);
        p.recall(Outcome::Forgot, d(4), None, &iv);
        assert_eq!((p.state, p.streak, p.next_check_at), (State::Learning, 0, Some(d(5))));
        p.recall(Outcome::Remembered, d(5), None, &iv);
        p.recall(Outcome::Remembered, d(8), None, &iv);
        assert_eq!(p.state, State::Known);
        p.recall(Outcome::Unfamiliar, d(9), None, &iv);
        assert_eq!((p.state, p.streak, p.next_check_at), (State::Learning, 0, Some(d(10))));
    }

    #[test]
    fn stale_after_180_days_and_encounter_asks_again() {
        let iv = Intervals { maintenance: false, ..Intervals::default() };
        let mut p = Progress::new(t0(), None, &iv);
        p.recall(Outcome::Remembered, d(1), None, &iv);
        p.recall(Outcome::Remembered, d(4), None, &iv);
        assert_eq!((p.state, p.next_check_at), (State::Known, None));
        assert!(!p.refresh(d(183), &iv));
        assert!(p.refresh(d(184), &iv));
        assert_eq!(p.state, State::Stale);
        p.encounter(d(200), Some("reader"), &iv);
        assert!(p.is_due(d(200)));
        assert_eq!(p.encounters, 1);
        p.recall(Outcome::Remembered, d(200), None, &iv);
        assert_eq!(p.state, State::Known);
    }

    #[test]
    fn configurable_intervals() {
        let iv = Intervals { first_check_days: 2, second_check_days: 5, known_after: 3, ..Intervals::default() };
        let mut p = Progress::new(t0(), None, &iv);
        assert_eq!(p.next_check_at, Some(d(2)));
        p.recall(Outcome::Remembered, d(2), None, &iv);
        assert_eq!(p.next_check_at, Some(d(7)));
        p.recall(Outcome::Remembered, d(7), None, &iv);
        assert_eq!(p.state, State::Learning);
        p.recall(Outcome::Remembered, d(12), None, &iv);
        assert_eq!(p.state, State::Known);
    }
}
