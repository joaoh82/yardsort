//! What each account uses, a day at a time (`docs/design/46-remote-r7.md` §11): the bytes the
//! relay carried for it, the most connections it had open to its hosts at once, the bytes of
//! synced bodies its computers sent, and how many of its devices made a request. Days are UTC.
//!
//! Each instance counts in memory and adds what it counted to the records every
//! [`FLUSH_EVERY`], and once more as it stops; a crash loses at most that much. The peak of
//! connections is the most open through one instance, which is every one of the account's when
//! it runs as one. Which devices were active is kept only for the day before and the day itself,
//! to count each once; a day's row keeps the count.
//!
//! `yardsort-server usage` prints the rows. Nothing here is asked before an action: the counts
//! entitlements need are the records' own (`crate::entitlements`).

use std::collections::{BTreeSet, HashMap};
use std::sync::{Mutex, PoisonError, Weak};
use std::time::Duration;

use remote_proto::DeviceId;

use crate::http::now_ms;
use crate::records::{AccountId, Records};
use crate::ControlService;

/// How often each instance adds what it counted to the records.
pub const FLUSH_EVERY: Duration = Duration::from_secs(60);
/// How long a day's row is kept.
pub const USAGE_RETENTION_DAYS: i64 = 400;

const DAY_MS: i64 = 24 * 60 * 60_000;

/// The UTC day `ms` falls on, as days since 1970-01-01.
pub fn day_of(ms: i64) -> i64 {
    ms.div_euclid(DAY_MS)
}

/// One account's counts for one day, as an instance adds them: bytes and devices are added to
/// what is kept, the peak replaces it if higher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageDelta {
    pub account: AccountId,
    pub day: i64,
    pub relay_bytes: u64,
    pub peak_streams: u64,
    pub sync_bytes: u64,
    pub devices: BTreeSet<DeviceId>,
}

/// One account's day, as the records keep it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageDay {
    pub account: AccountId,
    pub day: i64,
    pub relay_bytes: u64,
    pub peak_streams: u64,
    pub sync_bytes: u64,
    pub active_devices: u64,
}

#[derive(Default)]
struct Tally {
    relay_bytes: u64,
    peak_streams: u64,
    sync_bytes: u64,
    devices: BTreeSet<DeviceId>,
}

/// One instance's counts since they were last taken.
#[derive(Default)]
pub struct Meter {
    tallies: Mutex<HashMap<(AccountId, i64), Tally>>,
}

impl Meter {
    fn with(&self, account: &AccountId, now: i64, f: impl FnOnce(&mut Tally)) {
        let mut tallies = self.tallies.lock().unwrap_or_else(PoisonError::into_inner);
        f(tallies.entry((account.clone(), day_of(now))).or_default());
    }

    /// The relay carried `bytes` for `account`, either way.
    pub fn relayed(&self, account: &AccountId, bytes: usize, now: i64) {
        self.with(account, now, |t| {
            t.relay_bytes = t.relay_bytes.saturating_add(bytes as u64)
        });
    }

    /// `account` has `open` connections to its hosts through this instance.
    pub fn streams(&self, account: &AccountId, open: usize, now: i64) {
        self.with(account, now, |t| {
            t.peak_streams = t.peak_streams.max(open as u64)
        });
    }

    /// `account`'s computers sent `bytes` of synced bodies.
    pub fn synced(&self, account: &AccountId, bytes: usize, now: i64) {
        self.with(account, now, |t| {
            t.sync_bytes = t.sync_bytes.saturating_add(bytes as u64)
        });
    }

    /// `device` of `account` made a request.
    pub fn active(&self, account: &AccountId, device: &DeviceId, now: i64) {
        self.with(account, now, |t| {
            if !t.devices.contains(device) {
                t.devices.insert(device.clone());
            }
        });
    }

    /// Everything counted, emptied.
    pub fn take(&self) -> Vec<UsageDelta> {
        let taken =
            std::mem::take(&mut *self.tallies.lock().unwrap_or_else(PoisonError::into_inner));
        let mut deltas: Vec<UsageDelta> = taken
            .into_iter()
            .map(|((account, day), tally)| UsageDelta {
                account,
                day,
                relay_bytes: tally.relay_bytes,
                peak_streams: tally.peak_streams,
                sync_bytes: tally.sync_bytes,
                devices: tally.devices,
            })
            .collect();
        deltas.sort_by(|a, b| (a.day, &a.account.0).cmp(&(b.day, &b.account.0)));
        deltas
    }

    /// Put back what could not be added to the records, to try again with the next.
    pub fn restore(&self, deltas: Vec<UsageDelta>) {
        let mut tallies = self.tallies.lock().unwrap_or_else(PoisonError::into_inner);
        for delta in deltas {
            let tally = tallies.entry((delta.account, delta.day)).or_default();
            tally.relay_bytes = tally.relay_bytes.saturating_add(delta.relay_bytes);
            tally.peak_streams = tally.peak_streams.max(delta.peak_streams);
            tally.sync_bytes = tally.sync_bytes.saturating_add(delta.sync_bytes);
            tally.devices.extend(delta.devices);
        }
    }
}

/// Add what was counted to the records every [`FLUSH_EVERY`], for as long as the service lives.
pub(crate) fn start_flushing<R: Records + 'static>(service: Weak<ControlService<R>>) {
    tokio::spawn(async move {
        let mut every = tokio::time::interval(FLUSH_EVERY);
        every.tick().await;
        loop {
            every.tick().await;
            let Some(service) = service.upgrade() else {
                return;
            };
            service.flush_usage(now_ms()).await;
        }
    });
}

/// The days as `yardsort-server usage` prints them: a header, then a line each, tab-separated.
pub fn table(days: &[UsageDay]) -> String {
    let mut out =
        String::from("day\taccount\tactive devices\tpeak connections\trelay bytes\tsync bytes\n");
    for day in days {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\n",
            date(day.day),
            day.account.0,
            day.active_devices,
            day.peak_streams,
            day.relay_bytes,
            day.sync_bytes
        ));
    }
    out
}

/// `day` as `YYYY-MM-DD`.
pub fn date(day: i64) -> String {
    // Howard Hinnant's civil_from_days.
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The day a `YYYY-MM-DD` date names.
pub fn parse_date(text: &str) -> Option<i64> {
    let mut parts = text.trim().splitn(3, '-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let year = if m <= 2 { y - 1 } else { y };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let day = era * 146_097 + doe - 719_468;
    // 2026-02-31 is not a day: it would come back as another.
    (date(day) == format!("{y:04}-{m:02}-{d:02}")).then_some(day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str) -> AccountId {
        AccountId(id.into())
    }

    fn device(id: &str) -> DeviceId {
        DeviceId::new(id).unwrap()
    }

    #[test]
    fn days_are_utc_dates_both_ways() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(day_of(1_763_337_600_000)), "2025-11-17");
        assert_eq!(parse_date("2025-11-17"), Some(day_of(1_763_337_600_000)));
        assert_eq!(
            parse_date("2024-02-29").map(date).as_deref(),
            Some("2024-02-29")
        );
        assert_eq!(parse_date("2026-02-29"), None);
        assert_eq!(parse_date("2026-13-01"), None);
        assert_eq!(parse_date("yesterday"), None);
        assert_eq!(day_of(-1), -1);
    }

    #[test]
    fn a_meter_adds_bytes_keeps_the_peak_and_counts_each_device_once_a_day() {
        let meter = Meter::default();
        let (a, b) = (account("a"), account("b"));
        let day = 20_000 * DAY_MS;
        meter.relayed(&a, 100, day);
        meter.relayed(&a, 50, day + 1);
        meter.streams(&a, 3, day);
        meter.streams(&a, 1, day);
        meter.synced(&a, 7, day);
        meter.active(&a, &device("d1"), day);
        meter.active(&a, &device("d1"), day + 2);
        meter.active(&a, &device("d2"), day);
        meter.relayed(&b, 1, day);
        // The next day is a row of its own.
        meter.relayed(&a, 9, day + DAY_MS);
        let taken = meter.take();
        assert_eq!(taken.len(), 3);
        assert_eq!(
            taken[0],
            UsageDelta {
                account: a.clone(),
                day: 20_000,
                relay_bytes: 150,
                peak_streams: 3,
                sync_bytes: 7,
                devices: [device("d1"), device("d2")].into(),
            }
        );
        assert_eq!((taken[1].account.clone(), taken[1].relay_bytes), (b, 1));
        assert_eq!((taken[2].day, taken[2].relay_bytes), (20_001, 9));
        assert!(meter.take().is_empty(), "taking empties it");

        meter.relayed(&a, 1, day);
        meter.restore(taken);
        let again = meter.take();
        assert_eq!(
            again[0].relay_bytes, 151,
            "put back adds to what came since"
        );
    }
}
