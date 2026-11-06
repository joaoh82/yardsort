//! The next time a schedule fires.
//!
//! One engine for every kind that names a time of day: `daily`, `weekly` and `monthly` are
//! turned into the cron expression they mean, and cron is asked. That is also where the nights
//! the clocks change are handled, once: a time that does not exist fires at the next minute
//! that does, and a time that happens twice fires the first time only.
//!
//! Generic over the zone, and the time is handed in: the app passes the machine's local time,
//! the tests a fixed zone.

use chrono::{DateTime, TimeZone};
use croner::parser::{CronParser, Seconds, Year};
use croner::Cron;

use super::{ScheduleSpec, TimeOfDay, Weekday};

/// The first time `spec` fires after `after`, or `None` when there is none: a `once` whose
/// time has come.
///
/// `anchor` is the time an `every` last came due, in milliseconds since the epoch. With it, the
/// next time is a whole number of intervals after the anchor, so a schedule acted on a second
/// late does not drift a second later each time. Without it, an `every` counts from `after`.
pub fn next_after<Tz: TimeZone>(
    spec: &ScheduleSpec,
    after: &DateTime<Tz>,
    anchor: Option<i64>,
) -> Result<Option<DateTime<Tz>>, String> {
    match spec {
        ScheduleSpec::Once { at } => {
            let at = after
                .timezone()
                .timestamp_millis_opt(*at)
                .single()
                .ok_or_else(|| "That is not a time.".to_owned())?;
            Ok((at > *after).then_some(at))
        }
        ScheduleSpec::Every { minutes } => {
            let step = i64::from((*minutes).max(1)) * 60_000;
            let now = after.timestamp_millis();
            let next = match anchor.filter(|anchor| *anchor <= now) {
                Some(anchor) => anchor + ((now - anchor) / step + 1) * step,
                None => now + step,
            };
            after
                .timezone()
                .timestamp_millis_opt(next)
                .single()
                .map(Some)
                .ok_or_else(|| "That is too far away.".to_owned())
        }
        ScheduleSpec::Daily { .. }
        | ScheduleSpec::Weekly { .. }
        | ScheduleSpec::Monthly { .. }
        | ScheduleSpec::Cron { .. } => {
            let expr = cron_of(spec).unwrap_or_default();
            let cron = parse(&expr)?;
            match cron.find_next_occurrence(after, false) {
                Ok(next) => Ok(Some(next)),
                // A well-formed expression for a date no calendar has.
                Err(croner::errors::CronError::TimeSearchLimitExceeded) => Ok(None),
                Err(error) => Err(words(&expr, &error)),
            }
        }
    }
}

/// The next `count` times after `after`, fewer when there are fewer: what the dialog and
/// `ys schedule show` print beside a cron expression.
pub fn upcoming<Tz: TimeZone>(
    spec: &ScheduleSpec,
    after: &DateTime<Tz>,
    count: usize,
) -> Result<Vec<DateTime<Tz>>, String> {
    let mut times = Vec::with_capacity(count);
    let mut from = after.clone();
    let mut anchor = None;
    while times.len() < count {
        let Some(next) = next_after(spec, &from, anchor)? else {
            break;
        };
        // Each time found is the anchor of the one after it, as it is when a schedule fires.
        anchor = Some(next.timestamp_millis());
        from = next.clone();
        times.push(next);
    }
    Ok(times)
}

/// The cron expression a spec means; `None` for the kinds that are not a time of day.
pub fn cron_of(spec: &ScheduleSpec) -> Option<String> {
    let at = |time: &TimeOfDay| format!("{} {}", time.minute, time.hour);
    match spec {
        ScheduleSpec::Daily { time } => Some(format!("{} * * *", at(time))),
        ScheduleSpec::Weekly { days, time } => {
            let mut days: Vec<u8> = days.iter().copied().map(Weekday::cron).collect();
            days.sort_unstable();
            days.dedup();
            let days: Vec<String> = days.iter().map(u8::to_string).collect();
            Some(format!("{} * * {}", at(time), days.join(",")))
        }
        ScheduleSpec::Monthly { day, time } => Some(match day {
            Some(day) => format!("{} {day} * *", at(time)),
            None => format!("{} L * *", at(time)),
        }),
        ScheduleSpec::Cron { expr } => Some(expr.trim().to_owned()),
        ScheduleSpec::Once { .. } | ScheduleSpec::Every { .. } => None,
    }
}

/// Read a five-field expression. No seconds and no year: a schedule's finest grain is a
/// minute, and the sixth field means different things to different crons.
fn parse(expr: &str) -> Result<Cron, String> {
    let fields = expr.split_whitespace().count();
    if expr.trim().is_empty() {
        return Err("Write a cron expression: minute, hour, day, month, day of week.".to_owned());
    }
    if !expr.trim_start().starts_with('@') && fields != 5 {
        return Err(format!(
            "`{}` has {fields} {}; a cron expression here has five: minute, hour, day of the \
             month, month, day of the week.",
            expr.trim(),
            if fields == 1 { "field" } else { "fields" },
        ));
    }
    CronParser::builder()
        .seconds(Seconds::Disallowed)
        .year(Year::Disallowed)
        .build()
        .parse(expr)
        .map_err(|error| words(expr, &error))
}

fn words(expr: &str, error: &croner::errors::CronError) -> String {
    format!("`{}` is not a cron expression: {error}", expr.trim())
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, Duration, NaiveDate, Timelike, Utc};
    use chrono_tz::Tz;

    use super::*;

    const LISBON: Tz = chrono_tz::Europe::Lisbon;
    const NEW_YORK: Tz = chrono_tz::America::New_York;

    fn at(zone: Tz, y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Tz> {
        zone.from_local_datetime(
            &NaiveDate::from_ymd_opt(y, mo, d)
                .unwrap()
                .and_hms_opt(h, mi, 0)
                .unwrap(),
        )
        .earliest()
        .unwrap()
    }

    fn time(hour: u8, minute: u8) -> TimeOfDay {
        TimeOfDay { hour, minute }
    }

    fn next(spec: &ScheduleSpec, after: &DateTime<Tz>) -> DateTime<Tz> {
        next_after(spec, after, None).unwrap().unwrap()
    }

    /// `2026-11-05 09:00`, as the zone's clock shows it.
    fn clock(when: &DateTime<Tz>) -> String {
        when.format("%Y-%m-%d %H:%M").to_string()
    }

    #[test]
    fn daily_is_today_when_the_time_is_still_to_come_and_tomorrow_when_it_is_not() {
        let spec = ScheduleSpec::Daily { time: time(9, 0) };
        assert_eq!(
            clock(&next(&spec, &at(LISBON, 2026, 11, 5, 8, 59))),
            "2026-11-05 09:00"
        );
        // At the time itself, the next one is tomorrow's: a time is never its own next.
        assert_eq!(
            clock(&next(&spec, &at(LISBON, 2026, 11, 5, 9, 0))),
            "2026-11-06 09:00"
        );
        assert_eq!(next(&spec, &at(LISBON, 2026, 11, 5, 9, 0)).second(), 0);
    }

    #[test]
    fn weekly_fires_on_its_days_only() {
        let weekdays = ScheduleSpec::Weekly {
            days: Weekday::ALL[..5].to_vec(),
            time: time(9, 0),
        };
        // 2026-11-06 is a Friday; the next weekday after it is Monday the 9th.
        assert_eq!(
            clock(&next(&weekdays, &at(LISBON, 2026, 11, 6, 9, 0))),
            "2026-11-09 09:00"
        );
        let sunday = ScheduleSpec::Weekly {
            days: vec![Weekday::Sun, Weekday::Sun],
            time: time(23, 30),
        };
        let found = next(&sunday, &at(LISBON, 2026, 11, 5, 12, 0));
        assert_eq!(clock(&found), "2026-11-08 23:30");
        assert_eq!(found.weekday(), chrono::Weekday::Sun);
    }

    #[test]
    fn monthly_on_a_day_and_on_the_last_day() {
        let first = ScheduleSpec::Monthly {
            day: Some(1),
            time: time(6, 0),
        };
        assert_eq!(
            clock(&next(&first, &at(LISBON, 2026, 11, 5, 0, 0))),
            "2026-12-01 06:00"
        );
        let last = ScheduleSpec::Monthly {
            day: None,
            time: time(6, 0),
        };
        // February, in a year that is not a leap year and in one that is.
        assert_eq!(
            clock(&next(&last, &at(LISBON, 2027, 2, 1, 0, 0))),
            "2027-02-28 06:00"
        );
        assert_eq!(
            clock(&next(&last, &at(LISBON, 2028, 2, 1, 0, 0))),
            "2028-02-29 06:00"
        );
        assert_eq!(
            clock(&next(&last, &at(LISBON, 2026, 11, 30, 6, 0))),
            "2026-12-31 06:00"
        );
    }

    #[test]
    fn every_counts_from_now_or_in_whole_steps_from_when_it_last_came_due() {
        let spec = ScheduleSpec::Every { minutes: 15 };
        let now = at(LISBON, 2026, 11, 5, 10, 0);
        assert_eq!(clock(&next(&spec, &now)), "2026-11-05 10:15");

        // Due at 10:00:00, acted on two seconds late: the next is 10:15:00, not 10:15:02.
        let due = now.timestamp_millis();
        let late = now + Duration::seconds(2);
        let found = next_after(&spec, &late, Some(due)).unwrap().unwrap();
        assert_eq!(found.timestamp_millis(), due + 15 * 60_000);

        // Due at 10:00, acted on at 11:07 after the machine slept: the next whole step is
        // 11:15, not a burst of the four that were missed.
        let woke = at(LISBON, 2026, 11, 5, 11, 7);
        let found = next_after(&spec, &woke, Some(due)).unwrap().unwrap();
        assert_eq!(clock(&found), "2026-11-05 11:15");
    }

    #[test]
    fn once_has_a_next_time_until_it_has_come() {
        let moment = at(LISBON, 2026, 11, 5, 18, 0);
        let spec = ScheduleSpec::Once {
            at: moment.timestamp_millis(),
        };
        assert_eq!(
            next_after(&spec, &at(LISBON, 2026, 11, 5, 17, 59), None).unwrap(),
            Some(moment)
        );
        assert_eq!(next_after(&spec, &moment, None).unwrap(), None);
        assert_eq!(
            next_after(&spec, &at(LISBON, 2026, 11, 6, 0, 0), None).unwrap(),
            None
        );
    }

    #[test]
    fn cron_reads_lists_ranges_steps_and_names() {
        let cron = |expr: &str| ScheduleSpec::Cron {
            expr: expr.to_owned(),
        };
        let now = at(LISBON, 2026, 11, 5, 10, 7);
        assert_eq!(
            clock(&next(&cron("*/15 * * * *"), &now)),
            "2026-11-05 10:15"
        );
        assert_eq!(clock(&next(&cron("0 9 * * 1-5"), &now)), "2026-11-06 09:00");
        assert_eq!(
            clock(&next(&cron("30 8,20 * * *"), &now)),
            "2026-11-05 20:30"
        );
        assert_eq!(clock(&next(&cron("0 0 1 jan *"), &now)), "2027-01-01 00:00");
        assert_eq!(
            clock(&next(&cron("0 12 * * sun"), &now)),
            "2026-11-08 12:00"
        );
        assert_eq!(clock(&next(&cron(" @daily "), &now)), "2026-11-06 00:00");
    }

    #[test]
    fn a_bad_cron_expression_is_refused_in_words() {
        let problem = |expr: &str| {
            next_after(
                &ScheduleSpec::Cron {
                    expr: expr.to_owned(),
                },
                &at(LISBON, 2026, 11, 5, 10, 0),
                None,
            )
            .unwrap_err()
        };
        assert!(problem("").starts_with("Write a cron expression"));
        assert!(problem("0 9 * *").contains("has 4 fields"));
        // Six fields is a seconds field somewhere else; here it is one too many.
        assert!(problem("0 0 9 * * *").contains("has 6 fields"));
        assert!(problem("61 9 * * *").starts_with("`61 9 * * *` is not a cron expression"));
        assert!(problem("0 9 * * cat").contains("is not a cron expression"));
        assert!(problem("@reboot").contains("is not a cron expression"));
    }

    #[test]
    fn a_date_no_calendar_has_never_comes() {
        let spec = ScheduleSpec::Cron {
            expr: "0 0 30 2 *".to_owned(),
        };
        assert_eq!(
            next_after(&spec, &at(LISBON, 2026, 11, 5, 10, 0), None).unwrap(),
            None
        );
    }

    #[test]
    fn the_night_the_clocks_go_forward_a_missing_time_fires_at_the_next_that_exists() {
        // New York, 2027-03-14: 02:00 becomes 03:00, so 02:30 does not happen.
        let spec = ScheduleSpec::Daily { time: time(2, 30) };
        let found = next(&spec, &at(NEW_YORK, 2027, 3, 13, 12, 0));
        assert_eq!(clock(&found), "2027-03-14 03:00");
        // Once that day, and the day after is back at 02:30.
        assert_eq!(clock(&next(&spec, &found)), "2027-03-15 02:30");
    }

    #[test]
    fn the_night_the_clocks_go_back_a_repeated_time_fires_once() {
        // New York, 2026-11-01: 02:00 becomes 01:00, so 01:30 happens twice.
        let spec = ScheduleSpec::Daily { time: time(1, 30) };
        let first = next(&spec, &at(NEW_YORK, 2026, 10, 31, 12, 0));
        assert_eq!(clock(&first), "2026-11-01 01:30");
        assert_eq!(first.with_timezone(&Utc).hour(), 5, "the first 01:30, EDT");
        assert_eq!(clock(&next(&spec, &first)), "2026-11-02 01:30");
    }

    #[test]
    fn a_wall_clock_time_keeps_its_hour_across_the_change() {
        let spec = ScheduleSpec::Daily { time: time(9, 0) };
        let before = next(&spec, &at(NEW_YORK, 2026, 10, 31, 0, 0));
        let after = next(&spec, &at(NEW_YORK, 2026, 11, 1, 12, 0));
        assert_eq!((before.hour(), after.hour()), (9, 9));
        // Nine in the morning is an hour later in UTC once the clocks have gone back.
        assert_eq!(
            (
                before.with_timezone(&Utc).hour(),
                after.with_timezone(&Utc).hour()
            ),
            (13, 14)
        );
    }

    #[test]
    fn upcoming_lists_the_next_times_in_order() {
        let now = at(LISBON, 2026, 11, 5, 10, 0);
        let list = |spec: &ScheduleSpec, count| -> Vec<String> {
            upcoming(spec, &now, count)
                .unwrap()
                .iter()
                .map(clock)
                .collect()
        };
        assert_eq!(
            list(&ScheduleSpec::Every { minutes: 30 }, 3),
            ["2026-11-05 10:30", "2026-11-05 11:00", "2026-11-05 11:30"]
        );
        assert_eq!(
            list(&ScheduleSpec::Daily { time: time(9, 0) }, 2),
            ["2026-11-06 09:00", "2026-11-07 09:00"]
        );
        let once = ScheduleSpec::Once {
            at: at(LISBON, 2026, 11, 5, 18, 0).timestamp_millis(),
        };
        assert_eq!(list(&once, 5), ["2026-11-05 18:00"]);
    }

    #[test]
    fn the_presets_are_the_cron_they_mean() {
        assert_eq!(
            cron_of(&ScheduleSpec::Daily { time: time(9, 5) }).as_deref(),
            Some("5 9 * * *")
        );
        assert_eq!(
            cron_of(&ScheduleSpec::Weekly {
                days: vec![Weekday::Sun, Weekday::Wed, Weekday::Mon],
                time: time(18, 30)
            })
            .as_deref(),
            Some("30 18 * * 0,1,3")
        );
        assert_eq!(
            cron_of(&ScheduleSpec::Monthly {
                day: None,
                time: time(0, 0)
            })
            .as_deref(),
            Some("0 0 L * *")
        );
        assert_eq!(cron_of(&ScheduleSpec::Every { minutes: 5 }), None);
    }
}
