//! One `agent.settled` per turn.
//!
//! An agent's turn ends in up to two signals: the host's quiet after enough output, and, for a
//! harness that reports turns, a `turn.completed` row. The workflow driver takes the later of
//! the two as the moment a session settled. A trigger must not fire twice for the one turn, so
//! [`Turns`] folds them: a turn ends once between one burst of work and the next.
//!
//! The quiet half is held in memory only. After a restart, a turn that ended while nobody
//! watched is known by the row its harness wrote, when its harness writes one; a quiet while
//! the app was closed is not a row anywhere and is not recovered.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use pty_host::HostEvent;

/// What one host event meant for a session's turns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// This terminal's turn ended now, and it is the first word of that turn's end.
    Settled { terminal: String },
    /// This terminal started working again: whatever was gathering on its settling is off.
    Busy { terminal: String },
}

#[derive(Debug, Default, Clone, Copy)]
struct Turn {
    /// When it last started working.
    busy_at: Option<i64>,
    /// When its latest turn's end was told.
    told_at: Option<i64>,
}

/// The turns of every terminal the app has heard from, by terminal id.
#[derive(Debug)]
pub struct Turns {
    /// A quiet after at least this much output counts as a turn's end; less is the echo of a
    /// keystroke, or of a paste.
    threshold_ms: u32,
    seen: Mutex<HashMap<String, Turn>>,
}

impl Default for Turns {
    fn default() -> Self {
        Self::new(crate::workflow::driver::SETTLE_BUSY_MS)
    }
}

impl Turns {
    pub fn new(threshold_ms: u32) -> Self {
        Self {
            threshold_ms,
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// Take in a host event at `now`.
    pub fn observe(&self, event: &HostEvent, now: i64) -> Option<Change> {
        match event {
            HostEvent::Busy { id } => {
                self.seen
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .entry(id.0.clone())
                    .or_default()
                    .busy_at = Some(now);
                Some(Change::Busy {
                    terminal: id.0.clone(),
                })
            }
            HostEvent::Quiet { id, busy_ms } if *busy_ms >= self.threshold_ms => {
                self.ended(&id.0, now).then(|| Change::Settled {
                    terminal: id.0.clone(),
                })
            }
            HostEvent::Quiet { .. } => None,
            HostEvent::Exited { id, .. } => {
                self.seen
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&id.0);
                None
            }
        }
    }

    /// A harness reported that the turn in `terminal` ended at `at`. True when nothing had said
    /// so yet for this turn.
    pub fn reported(&self, terminal: &str, at: i64) -> bool {
        self.ended(terminal, at)
    }

    fn ended(&self, terminal: &str, at: i64) -> bool {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        let turn = seen.entry(terminal.to_owned()).or_default();
        // Told already, and the agent has not started working since: the same turn's other
        // signal.
        let told = turn
            .told_at
            .is_some_and(|told| turn.busy_at.is_none_or(|busy| told >= busy));
        if told {
            return false;
        }
        turn.told_at = Some(at);
        true
    }
}

#[cfg(test)]
mod tests {
    use pty_host::SessionId;

    use super::*;

    fn quiet(busy_ms: u32) -> HostEvent {
        HostEvent::Quiet {
            id: SessionId("t".into()),
            busy_ms,
        }
    }

    fn busy() -> HostEvent {
        HostEvent::Busy {
            id: SessionId("t".into()),
        }
    }

    #[test]
    fn a_quiet_and_a_reported_turn_are_one_turn() {
        let turns = Turns::new(8_000);
        assert_eq!(
            turns.observe(&busy(), 0),
            Some(Change::Busy {
                terminal: "t".into()
            })
        );
        assert_eq!(
            turns.observe(&quiet(9_000), 10_000),
            Some(Change::Settled {
                terminal: "t".into()
            })
        );
        assert!(!turns.reported("t", 10_500), "the same turn, said twice");
        assert_eq!(
            turns.observe(&busy(), 20_000),
            Some(Change::Busy {
                terminal: "t".into()
            })
        );
        assert!(turns.reported("t", 30_000), "the next turn, reported first");
        assert_eq!(turns.observe(&quiet(9_000), 30_100), None);
    }

    #[test]
    fn a_short_burst_is_not_a_turn() {
        let turns = Turns::new(8_000);
        turns.observe(&busy(), 0);
        assert_eq!(turns.observe(&quiet(200), 1_000), None);
        assert_eq!(
            turns.observe(&quiet(8_000), 2_000),
            Some(Change::Settled {
                terminal: "t".into()
            })
        );
    }

    #[test]
    fn after_a_restart_a_reported_turn_counts() {
        let turns = Turns::new(8_000);
        assert!(turns.reported("t", 5_000));
        assert!(!turns.reported("t", 5_000), "the same row read twice");
    }
}
