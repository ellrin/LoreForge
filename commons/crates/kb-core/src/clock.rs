//! Injectable clock so the state machine and timestamps are deterministic in tests.

use std::sync::Mutex;

use chrono::{DateTime, Duration, FixedOffset, Local};

pub type Time = DateTime<FixedOffset>;

pub trait Clock: Send + Sync {
    fn now(&self) -> Time;
}

/// The computer's clock in its local offset.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Time {
        Local::now().fixed_offset()
    }
}

/// A clock that only moves when told to.
#[derive(Debug)]
pub struct FixedClock(Mutex<Time>);

impl FixedClock {
    pub fn new(t: Time) -> FixedClock {
        FixedClock(Mutex::new(t))
    }
    /// `FixedClock::at("2026-11-08T21:10:00+08:00")`
    pub fn at(rfc3339: &str) -> FixedClock {
        FixedClock::new(DateTime::parse_from_rfc3339(rfc3339).expect("RFC 3339 time"))
    }
    pub fn set(&self, t: Time) {
        *self.0.lock().unwrap() = t;
    }
    pub fn advance(&self, d: Duration) {
        let mut t = self.0.lock().unwrap();
        *t += d;
    }
    pub fn advance_days(&self, days: i64) {
        self.advance(Duration::days(days));
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Time {
        *self.0.lock().unwrap()
    }
}
