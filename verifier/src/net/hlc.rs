//! Hybrid logical clock. Stamps are totally ordered by (wall_ms, counter,
//! replica), so every replica sorts the same set of ops the same way, and a
//! stamp issued after observing another is always greater than it.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Stamp {
    pub wall_ms: u64,
    pub counter: u32,
    pub replica: String,
}

impl Ord for Stamp {
    fn cmp(&self, o: &Self) -> Ordering {
        (self.wall_ms, self.counter, &self.replica).cmp(&(o.wall_ms, o.counter, &o.replica))
    }
}

impl PartialOrd for Stamp {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Stamp {
    /// Compact sortable text form, `wall.counter@replica`.
    pub fn text(&self) -> String {
        format!("{:013}.{:04}@{}", self.wall_ms, self.counter, self.replica)
    }
}

#[derive(Debug, Clone)]
pub struct Clock {
    replica: String,
    wall_ms: u64,
    counter: u32,
}

impl Clock {
    pub fn new(replica: impl Into<String>) -> Self {
        Clock { replica: replica.into(), wall_ms: 0, counter: 0 }
    }

    pub fn replica(&self) -> &str {
        &self.replica
    }

    /// Issue a stamp for a local event at physical time `now_ms`.
    pub fn tick(&mut self, now_ms: u64) -> Stamp {
        if now_ms > self.wall_ms {
            self.wall_ms = now_ms;
            self.counter = 0;
        } else {
            self.counter += 1;
        }
        Stamp { wall_ms: self.wall_ms, counter: self.counter, replica: self.replica.clone() }
    }

    /// Fold in a stamp seen from any replica, so later ticks sort after it.
    pub fn observe(&mut self, s: &Stamp) {
        if s.wall_ms > self.wall_ms {
            self.wall_ms = s.wall_ms;
            self.counter = s.counter;
        } else if s.wall_ms == self.wall_ms && s.counter > self.counter {
            self.counter = s.counter;
        }
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_are_monotonic_even_if_wall_clock_stalls_or_regresses() {
        let mut c = Clock::new("a");
        let s1 = c.tick(100);
        let s2 = c.tick(100);
        let s3 = c.tick(50);
        let s4 = c.tick(200);
        assert!(s1 < s2 && s2 < s3 && s3 < s4);
        assert_eq!(s4.counter, 0);
    }

    #[test]
    fn observing_a_remote_stamp_orders_later_ticks_after_it() {
        let mut a = Clock::new("a");
        let mut b = Clock::new("b");
        let remote = b.tick(1_000);
        a.tick(10);
        a.observe(&remote);
        let local = a.tick(10);
        assert!(local > remote);
    }

    #[test]
    fn replica_breaks_ties() {
        let x = Stamp { wall_ms: 1, counter: 0, replica: "a".into() };
        let y = Stamp { wall_ms: 1, counter: 0, replica: "b".into() };
        assert!(x < y);
        assert!(x.text() < y.text());
    }
}
