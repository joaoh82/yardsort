//! How often one address may start a sign-in or ask for credentials: a token bucket per address,
//! in this process. Credentials are 256-bit random, so this is not what stops guessing; it keeps
//! one address from filling the records with sign-ins in progress or keeping the server busy.
//!
//! Each instance counts on its own, so two instances allow twice the rate. An IPv6 address counts
//! with the rest of its /64, which one subscriber usually holds whole.
//!
//! At most [`MAX_TRACKED`] addresses are counted one by one. When that many are, full buckets
//! (which say nothing) are swept out at most once every [`SWEEP_EVERY`], and an address that
//! still finds no room shares one overflow bucket with every other such address. So a burst from
//! many addresses costs bounded memory and no more than one sweep a second, and the addresses
//! already counted keep their own allowance; only newcomers during the burst share.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Addresses counted one by one.
const MAX_TRACKED: usize = 10_000;
/// The least time between two sweeps of a full map.
const SWEEP_EVERY: Duration = Duration::from_secs(1);

pub struct Limiter {
    /// The burst allowed, and the refill over a minute.
    per_minute: f64,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    buckets: HashMap<IpAddr, Bucket>,
    /// Shared by the addresses that found no room in `buckets`.
    overflow: Option<Bucket>,
    swept_at: Option<Instant>,
    sweeps: usize,
}

#[derive(Clone, Copy)]
struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Limiter {
    pub fn per_minute(count: u32) -> Self {
        Self {
            per_minute: f64::from(count.max(1)),
            state: Mutex::new(State::default()),
        }
    }

    /// Take one for `address`, or say how long until one is free.
    pub fn check(&self, address: IpAddr, now: Instant) -> Result<(), Duration> {
        let key = grouped(address);
        let Ok(mut state) = self.state.lock() else {
            // A panic elsewhere while holding it: let requests through rather than refuse all.
            return Ok(());
        };
        let state = &mut *state;
        if !state.buckets.contains_key(&key) && state.buckets.len() >= MAX_TRACKED {
            let due = state
                .swept_at
                .is_none_or(|at| now.saturating_duration_since(at) >= SWEEP_EVERY);
            if due {
                state
                    .buckets
                    .retain(|_, b| self.refilled(*b, now) < self.per_minute);
                state.swept_at = Some(now);
                state.sweeps += 1;
            }
        }
        let full = Bucket {
            tokens: self.per_minute,
            at: now,
        };
        let bucket = if state.buckets.len() < MAX_TRACKED || state.buckets.contains_key(&key) {
            state.buckets.entry(key).or_insert(full)
        } else {
            state.overflow.get_or_insert(full)
        };
        let tokens = self.refilled(*bucket, now);
        if tokens >= 1.0 {
            *bucket = Bucket {
                tokens: tokens - 1.0,
                at: now,
            };
            Ok(())
        } else {
            *bucket = Bucket { tokens, at: now };
            Err(Duration::from_secs_f64(
                (1.0 - tokens) * 60.0 / self.per_minute,
            ))
        }
    }

    fn refilled(&self, bucket: Bucket, now: Instant) -> f64 {
        let elapsed = now.saturating_duration_since(bucket.at).as_secs_f64();
        (bucket.tokens + elapsed * self.per_minute / 60.0).min(self.per_minute)
    }
}

/// What an address is counted as: itself, or its /64 for IPv6.
fn grouped(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V4(_) => address,
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let s = v6.segments();
                IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn a_burst_then_the_refill_rate() {
        let limiter = Limiter::per_minute(3);
        let t = Instant::now();
        for _ in 0..3 {
            assert!(limiter.check(ip("192.0.2.1"), t).is_ok());
        }
        let wait = limiter.check(ip("192.0.2.1"), t).unwrap_err();
        assert_eq!(
            wait.as_secs(),
            20,
            "one every twenty seconds at three a minute"
        );
        // Another address is counted on its own.
        assert!(limiter.check(ip("192.0.2.2"), t).is_ok());
        assert!(limiter
            .check(ip("192.0.2.1"), t + Duration::from_secs(19))
            .is_err());
        assert!(limiter
            .check(ip("192.0.2.1"), t + Duration::from_secs(40))
            .is_ok());
    }

    #[test]
    fn an_ipv6_address_counts_with_its_64() {
        let limiter = Limiter::per_minute(1);
        let t = Instant::now();
        assert!(limiter.check(ip("2001:db8:1:2::1"), t).is_ok());
        assert!(limiter.check(ip("2001:db8:1:2:ffff::9"), t).is_err());
        assert!(limiter.check(ip("2001:db8:1:3::1"), t).is_ok());
        // An IPv4 address seen as IPv6 is still that IPv4 address.
        assert!(limiter.check(ip("198.51.100.7"), t).is_ok());
        assert!(limiter.check(ip("::ffff:198.51.100.7"), t).is_err());
    }

    #[test]
    fn a_burst_from_many_addresses_is_bounded_and_sweeps_at_most_once_a_second() {
        let limiter = Limiter::per_minute(10);
        let t = Instant::now();
        let known = ip("203.0.113.1");
        assert!(limiter.check(known, t).is_ok());
        let mut refused = 0;
        for i in 0..(2 * MAX_TRACKED as u32) {
            if limiter.check(IpAddr::V4(i.into()), t).is_err() {
                refused += 1;
            }
        }
        let tracked = |l: &Limiter| l.state.lock().unwrap().buckets.len();
        let sweeps = |l: &Limiter| l.state.lock().unwrap().sweeps;
        assert_eq!(tracked(&limiter), MAX_TRACKED);
        assert_eq!(sweeps(&limiter), 1, "one sweep for the whole burst");
        // The newcomers that found no room shared one allowance of ten.
        assert_eq!(refused, MAX_TRACKED + 1 - 10);
        // An address counted before the burst keeps its own.
        assert!(limiter.check(known, t).is_ok());

        // Once the burst has refilled, a sweep a second later makes room again.
        let later = t + Duration::from_secs(60);
        assert!(limiter.check(ip("198.51.100.9"), later).is_ok());
        assert_eq!(sweeps(&limiter), 2);
        assert!(tracked(&limiter) < 100);
    }
}
