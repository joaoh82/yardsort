//! Who may type into a session, and for how long.
//!
//! A session has one controller at a time. The person at the computer is [`Controller::Local`]:
//! anything typed through [`crate::PtyHost::write`] or [`crate::PtyHost::paste`] takes control
//! at once, whoever held it, because the person in front of the machine always wins. Otherwise
//! control is a **lease** that someone asks for, renews, and gives back — in practice a device
//! reached through the remote connector, which is why the host knows a holder only as an opaque
//! name. Each change of hands bumps the session's generation, so a holder that lost control
//! cannot type on with the lease it had, even over a connection that is still open.
//!
//! Local control lapses after [`LOCAL_HOLD`] without local input, and a lease after
//! [`LEASE_TTL`] without renewal. Both lapse lazily: whoever asks next finds them gone.
//!
//! The host does not decide who *may* ask for a lease. Anything that can reach the host is
//! already the local user: the daemon's socket is private to them. Whether a device may control
//! a terminal is the remote layer's question, answered by its grant before it asks here.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// How long a lease lasts without renewal.
pub const LEASE_TTL: Duration = Duration::from_secs(30);
/// How often a holder should renew: three chances before the lease lapses.
pub const RENEW_EVERY: Duration = Duration::from_secs(10);
/// How long local control lasts after the last thing typed locally. Until then nobody else can
/// take a lease; the person may only have looked away.
pub const LOCAL_HOLD: Duration = Duration::from_secs(30);
/// How long each kind of hold lasts: [`LOCAL_HOLD`] and [`LEASE_TTL`], unless a host is given
/// others with [`crate::PtyHost::with_holds`]. A holder is asked to renew three times a lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Holds {
    pub local: Duration,
    pub lease: Duration,
}

impl Default for Holds {
    fn default() -> Self {
        Self {
            local: LOCAL_HOLD,
            lease: LEASE_TTL,
        }
    }
}

/// A holder's name is a device id in practice; this is far more than one needs.
const MAX_HOLDER: usize = 128;

/// Who may type into a session right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum Controller {
    /// Nobody holds it: the next to ask gets it.
    Nobody,
    /// The person at this computer, through the app or `ys attach`.
    Local,
    /// Whoever holds the lease, by the name they asked with.
    Leased { holder: String },
}

/// A lease handed out by [`crate::PtyHost::acquire`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lease {
    pub id: String,
    /// Increases every time control of this session changes hands.
    pub generation: u64,
    pub ttl_ms: u32,
    pub renew_every_ms: u32,
}

impl Lease {
    pub fn reference(&self) -> LeaseRef {
        LeaseRef {
            id: self.id.clone(),
            generation: self.generation,
        }
    }
}

/// Names a lease in every call that needs one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaseRef {
    pub id: String,
    pub generation: u64,
}

/// Why a call under a lease was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Someone else holds control, or the lease lapsed: it is not this lease any more.
    Lost,
    /// Input skipped a number. Nothing after a gap is written: the missing input may have been
    /// the half that made the rest safe.
    OutOfOrder { expected: u64, got: u64 },
    /// Somebody else controls the session.
    Held(Controller),
    /// A holder's name that is empty, too long, or not printable.
    BadHolder,
}

/// What [`Control::input`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Delivery {
    /// The next input: write it.
    Fresh,
    /// One already written under this lease, sent again: drop it.
    Repeat,
}

/// A session's control state. Lives under the session's view lock, beside its viewers, because
/// who controls the session also decides who answers its terminal queries.
#[derive(Debug)]
pub(crate) struct Control {
    generation: u64,
    state: State,
    holds: Holds,
}

#[derive(Debug)]
enum State {
    Nobody,
    Local {
        last: Instant,
    },
    Leased {
        holder: String,
        id: String,
        expires: Instant,
        /// The last input number written under this lease; 0 before any.
        input: u64,
    },
}

impl Default for Control {
    fn default() -> Self {
        Self::new(Holds::default())
    }
}

impl Control {
    pub(crate) fn new(holds: Holds) -> Self {
        Self {
            generation: 0,
            state: State::Nobody,
            holds,
        }
    }

    /// Who controls the session at `now`.
    pub(crate) fn controller(&mut self, now: Instant) -> Controller {
        self.lapse(now);
        match &self.state {
            State::Nobody => Controller::Nobody,
            State::Local { .. } => Controller::Local,
            State::Leased { holder, .. } => Controller::Leased {
                holder: holder.clone(),
            },
        }
    }

    /// Whether a lease holder controls the session at `now`.
    pub(crate) fn leased(&mut self, now: Instant) -> bool {
        self.lapse(now);
        matches!(self.state, State::Leased { .. })
    }

    /// Something was typed at this computer. Returns whether that took control from a lease.
    pub(crate) fn local_input(&mut self, now: Instant) -> bool {
        self.lapse(now);
        let took = matches!(self.state, State::Leased { .. });
        if !matches!(self.state, State::Local { .. }) {
            self.generation += 1;
        }
        self.state = State::Local { last: now };
        took
    }

    /// Hand `holder` a lease, if nobody else has control. With `take_over`, take it from another
    /// lease holder — but never from the person at the computer.
    pub(crate) fn acquire(
        &mut self,
        holder: &str,
        take_over: bool,
        now: Instant,
    ) -> Result<Lease, Refusal> {
        if holder.is_empty() || holder.len() > MAX_HOLDER || holder.chars().any(char::is_control) {
            return Err(Refusal::BadHolder);
        }
        match self.controller(now) {
            Controller::Local => return Err(Refusal::Held(Controller::Local)),
            Controller::Leased { holder: other } if other != holder && !take_over => {
                return Err(Refusal::Held(Controller::Leased { holder: other }));
            }
            // Nobody, a take-over, or the same holder asking again — after a reconnect, say.
            // Each is a new lease, and the old one's input stops counting.
            _ => {}
        }
        self.generation += 1;
        let id = uuid::Uuid::new_v4().to_string();
        self.state = State::Leased {
            holder: holder.to_owned(),
            id: id.clone(),
            expires: now + self.holds.lease,
            input: 0,
        };
        Ok(self.lease(id))
    }

    /// Extend a lease that is still held.
    pub(crate) fn renew(&mut self, lease: &LeaseRef, now: Instant) -> Result<Lease, Refusal> {
        self.check(lease, now)?;
        if let State::Leased { expires, .. } = &mut self.state {
            *expires = now + self.holds.lease;
        }
        Ok(self.lease(lease.id.clone()))
    }

    /// Give control back. Giving back a lease that is already gone is not an error: it is
    /// gone either way.
    pub(crate) fn release(&mut self, lease: &LeaseRef, now: Instant) -> bool {
        if self.check(lease, now).is_err() {
            return false;
        }
        self.generation += 1;
        self.state = State::Nobody;
        true
    }

    /// Whether input number `seq` under `lease` should be written.
    pub(crate) fn input(
        &mut self,
        lease: &LeaseRef,
        seq: u64,
        now: Instant,
    ) -> Result<Delivery, Refusal> {
        self.check(lease, now)?;
        let State::Leased { input, .. } = &mut self.state else {
            return Err(Refusal::Lost);
        };
        if seq <= *input {
            return Ok(Delivery::Repeat);
        }
        if seq != *input + 1 {
            return Err(Refusal::OutOfOrder {
                expected: *input + 1,
                got: seq,
            });
        }
        *input = seq;
        Ok(Delivery::Fresh)
    }

    /// Whether `lease` is the one in force at `now`.
    pub(crate) fn check(&mut self, lease: &LeaseRef, now: Instant) -> Result<(), Refusal> {
        self.lapse(now);
        match &self.state {
            State::Leased { id, .. } if *id == lease.id && self.generation == lease.generation => {
                Ok(())
            }
            _ => Err(Refusal::Lost),
        }
    }

    fn lease(&self, id: String) -> Lease {
        Lease {
            id,
            generation: self.generation,
            ttl_ms: millis(self.holds.lease),
            renew_every_ms: millis(self.holds.lease / 3),
        }
    }

    /// When the hold in force lapses, if nothing renews it first.
    pub(crate) fn lapses_at(&self) -> Option<Instant> {
        match &self.state {
            State::Nobody => None,
            State::Local { last } => Some(*last + self.holds.local),
            State::Leased { expires, .. } => Some(*expires),
        }
    }

    fn lapse(&mut self, now: Instant) {
        let lapsed = match &self.state {
            State::Nobody => false,
            State::Local { last } => now.saturating_duration_since(*last) >= self.holds.local,
            State::Leased { expires, .. } => now >= *expires,
        };
        if lapsed {
            self.generation += 1;
            self.state = State::Nobody;
        }
    }
}

fn millis(duration: Duration) -> u32 {
    u32::try_from(duration.as_millis()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn later(start: Instant, by: Duration) -> Instant {
        start + by
    }

    #[test]
    fn a_lease_is_had_by_asking_and_kept_by_renewing() {
        let mut control = Control::default();
        let t0 = Instant::now();
        let lease = control.acquire("phone", false, t0).unwrap();
        assert_eq!(lease.ttl_ms, 30_000);
        assert_eq!(lease.renew_every_ms, 10_000);
        assert_eq!(
            control.controller(t0),
            Controller::Leased {
                holder: "phone".into()
            }
        );
        // Renewed every twenty seconds, it outlives its thirty.
        let mut now = t0;
        for _ in 0..5 {
            now = later(now, Duration::from_secs(20));
            control.renew(&lease.reference(), now).unwrap();
        }
        // Left alone, it lapses.
        now = later(now, LEASE_TTL);
        assert_eq!(control.renew(&lease.reference(), now), Err(Refusal::Lost));
        assert_eq!(control.controller(now), Controller::Nobody);
    }

    #[test]
    fn input_is_written_once_in_order_and_never_after_a_gap() {
        let mut control = Control::default();
        let t0 = Instant::now();
        let lease = control.acquire("phone", false, t0).unwrap().reference();
        assert_eq!(control.input(&lease, 1, t0), Ok(Delivery::Fresh));
        assert_eq!(control.input(&lease, 2, t0), Ok(Delivery::Fresh));
        assert_eq!(control.input(&lease, 2, t0), Ok(Delivery::Repeat));
        assert_eq!(control.input(&lease, 1, t0), Ok(Delivery::Repeat));
        assert_eq!(
            control.input(&lease, 4, t0),
            Err(Refusal::OutOfOrder {
                expected: 3,
                got: 4
            })
        );
        assert_eq!(control.input(&lease, 3, t0), Ok(Delivery::Fresh));
    }

    #[test]
    fn typing_here_takes_control_at_once_and_the_old_lease_types_no_more() {
        let mut control = Control::default();
        let t0 = Instant::now();
        let lease = control.acquire("phone", false, t0).unwrap();
        assert!(control.local_input(t0), "it was taken from the lease");
        assert_eq!(control.controller(t0), Controller::Local);
        assert_eq!(control.input(&lease.reference(), 1, t0), Err(Refusal::Lost));
        assert_eq!(control.renew(&lease.reference(), t0), Err(Refusal::Lost));
        assert!(!control.release(&lease.reference(), t0));
        // And nobody can lease it while the person here is typing, take-over or not.
        assert_eq!(
            control.acquire("phone", true, later(t0, Duration::from_secs(29))),
            Err(Refusal::Held(Controller::Local))
        );
        // Once they stop for long enough, it is free again.
        let free = later(t0, LOCAL_HOLD);
        assert_eq!(control.controller(free), Controller::Nobody);
        let again = control.acquire("phone", false, free).unwrap();
        assert!(again.generation > lease.generation);
        assert!(!control.local_input(later(free, LEASE_TTL)), "lapsed first");
    }

    #[test]
    fn another_holder_must_ask_to_take_over_and_then_the_first_is_out() {
        let mut control = Control::default();
        let t0 = Instant::now();
        let first = control.acquire("phone", false, t0).unwrap();
        assert_eq!(
            control.acquire("tablet", false, t0),
            Err(Refusal::Held(Controller::Leased {
                holder: "phone".into()
            }))
        );
        let second = control.acquire("tablet", true, t0).unwrap();
        assert_eq!(control.input(&first.reference(), 1, t0), Err(Refusal::Lost));
        assert_eq!(
            control.input(&second.reference(), 1, t0),
            Ok(Delivery::Fresh)
        );
        // The same holder asking again gets a new lease, and its old one stops counting.
        let third = control.acquire("tablet", false, t0).unwrap();
        assert_eq!(
            control.input(&second.reference(), 2, t0),
            Err(Refusal::Lost)
        );
        assert_eq!(
            control.input(&third.reference(), 1, t0),
            Ok(Delivery::Fresh)
        );
    }

    #[test]
    fn a_released_lease_frees_the_session_and_a_stale_reference_does_nothing() {
        let mut control = Control::default();
        let t0 = Instant::now();
        let lease = control.acquire("phone", false, t0).unwrap().reference();
        let forged = LeaseRef {
            id: lease.id.clone(),
            generation: lease.generation + 1,
        };
        assert!(!control.release(&forged, t0));
        assert!(control.leased(t0));
        assert!(control.release(&lease, t0));
        assert_eq!(control.controller(t0), Controller::Nobody);
        assert!(!control.release(&lease, t0), "already given back");
    }

    #[test]
    fn a_holder_must_have_a_printable_name() {
        let mut control = Control::default();
        let t0 = Instant::now();
        for bad in ["", "a\nb", "\u{1b}[31m", &"x".repeat(MAX_HOLDER + 1)] {
            assert_eq!(
                control.acquire(bad, false, t0),
                Err(Refusal::BadHolder),
                "{bad:?}"
            );
        }
        assert_eq!(control.controller(t0), Controller::Nobody);
    }
}
