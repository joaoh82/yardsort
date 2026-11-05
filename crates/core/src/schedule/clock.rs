//! The machine's clock and zone: the one place a schedule's code reads them.
//!
//! Everything else in [`crate::schedule`] is handed the time. The app and `ys` get it here and
//! pass it in, which also keeps the time library out of both of them.

use chrono::{DateTime, Local, NaiveDateTime, Offset, TimeZone};

/// A moment, in the machine's own zone.
pub type Now = DateTime<Local>;

pub fn now() -> Now {
    Local::now()
}

/// The machine's offset from UTC at `now`, in seconds. When this changes — the clocks changed,
/// or the machine moved — times of day are worked out again.
pub fn utc_offset_seconds(now: &Now) -> i32 {
    now.offset().fix().local_minus_utc()
}

/// A moment as the machine's clock shows it: `Thu 5 Nov 2026 18:00`.
pub fn show(at_ms: i64) -> String {
    match Local.timestamp_millis_opt(at_ms).single() {
        Some(at) => at.format("%a %-d %b %Y %H:%M").to_string(),
        None => "never".to_owned(),
    }
}

/// A date and a time on the machine's clock, as milliseconds since the epoch: `2026-11-06 18:00`
/// or `2026-11-06T18:00`. `None` for anything else, and for a time the clock skips that night.
/// A time the clock shows twice means the first.
pub fn parse_local(text: &str) -> Option<i64> {
    let text = text.trim();
    let naive = ["%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M"]
        .iter()
        .find_map(|format| NaiveDateTime::parse_from_str(text, format).ok())?;
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|at| at.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_and_time_is_read_in_either_spelling_and_shown_back() {
        let spaced = parse_local("2026-11-06 18:00").unwrap();
        assert_eq!(parse_local(" 2026-11-06T18:00 "), Some(spaced));
        assert_eq!(show(spaced), "Fri 6 Nov 2026 18:00");
        for bad in [
            "",
            "tomorrow",
            "2026-11-06",
            "18:00",
            "2026-13-01 10:00",
            "2026-11-06 25:00",
        ] {
            assert_eq!(parse_local(bad), None, "{bad}");
        }
    }
}
