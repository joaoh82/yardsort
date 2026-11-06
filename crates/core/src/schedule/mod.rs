//! Schedules: a standing instruction to start a workflow or a smoke test in a workspace, once
//! at a set time or again and again. See `docs/design/34-schedules.md`.
//!
//! A schedule adds nothing to a run. It is a row that says what, where and when, and when its
//! time comes it goes through the door a person's click goes through — [`crate::workflow::runs`]
//! or [`crate::smoke::runs`] — so everything those doors check is checked for it too. What is
//! here is the part before a run: what a schedule may hold ([`check`], [`add`], [`change`]),
//! when its next time is ([`when`]), and what becomes of a time that has come ([`fire`]).
//!
//! Nothing here reads the clock but [`clock`], which is where the app and `ys` get the time they
//! hand in. Every rule takes the time, in the zone the times are meant in, so every rule is
//! tested with a made-up clock and a fixed zone.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::path::Path;

use chrono::{DateTime, TimeZone};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::{IpcError, IpcResult};
use crate::store::{ScheduleFields, ScheduleRow, ScheduleRunRow, Store, WorkspaceRow};
use crate::workflow::runs::Look;

pub mod clock;
pub mod fire;
pub mod when;

/// When a schedule fires. Kept as the user chose it, so the list reads the way it was written:
/// the daily, weekly and monthly kinds are cron underneath (see [`when`]) but are not stored as
/// cron.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScheduleSpec {
    /// Once, at this time: milliseconds since the Unix epoch.
    Once {
        #[specta(type = f64)]
        at: i64,
    },
    /// Again every so many minutes, counted from the last time it fired.
    Every {
        minutes: u32,
    },
    Daily {
        time: TimeOfDay,
    },
    /// On these weekdays. At least one.
    Weekly {
        days: Vec<Weekday>,
        time: TimeOfDay,
    },
    /// On this day of the month, 1 to 28, or on its last day when there is none.
    Monthly {
        day: Option<u8>,
        time: TimeOfDay,
    },
    /// Five fields: minute, hour, day of month, month, day of week.
    Cron {
        expr: String,
    },
}

/// A time of day on the machine's own clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TimeOfDay {
    pub hour: u8,
    pub minute: u8,
}

impl Display for TimeOfDay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)
    }
}

impl TimeOfDay {
    /// `9:00`, `09:00` or `18:30`.
    pub fn parse(text: &str) -> Option<Self> {
        let (hour, minute) = text.trim().split_once(':')?;
        let digits = |part: &str, most: usize| {
            (!part.is_empty() && part.len() <= most && part.bytes().all(|b| b.is_ascii_digit()))
                .then(|| part.parse::<u8>().ok())
                .flatten()
        };
        let time = Self {
            hour: digits(hour, 2)?,
            minute: (minute.len() == 2).then(|| digits(minute, 2)).flatten()?,
        };
        (time.hour < 24 && time.minute < 60).then_some(time)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Weekday {
    pub const ALL: [Self; 7] = [
        Self::Mon,
        Self::Tue,
        Self::Wed,
        Self::Thu,
        Self::Fri,
        Self::Sat,
        Self::Sun,
    ];

    pub fn short(self) -> &'static str {
        match self {
            Self::Mon => "Mon",
            Self::Tue => "Tue",
            Self::Wed => "Wed",
            Self::Thu => "Thu",
            Self::Fri => "Fri",
            Self::Sat => "Sat",
            Self::Sun => "Sun",
        }
    }

    /// As cron counts them: Sunday is 0.
    fn cron(self) -> u8 {
        match self {
            Self::Sun => 0,
            Self::Mon => 1,
            Self::Tue => 2,
            Self::Wed => 3,
            Self::Thu => 4,
            Self::Fri => 5,
            Self::Sat => 6,
        }
    }

    /// `mon`, `Monday`, `MON`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|day| {
            let short = day.short().to_ascii_lowercase();
            text == short || (text.starts_with(&short) && text.ends_with("day"))
        })
    }
}

/// What a schedule starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleTarget {
    Workflow,
    Test,
}

impl ScheduleTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Workflow => "workflow",
            Self::Test => "test",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "workflow" => Some(Self::Workflow),
            "test" => Some(Self::Test),
            _ => None,
        }
    }

    /// What to call it in a sentence.
    pub fn words(self) -> &'static str {
        match self {
            Self::Workflow => "workflow",
            Self::Test => "smoke test",
        }
    }
}

/// What to do with a time that passed while Yardsort was not running.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum IfMissed {
    /// One run when Yardsort is next open, however many times were missed.
    #[default]
    RunOnce,
    Skip,
}

impl IfMissed {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RunOnce => "run_once",
            Self::Skip => "skip",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "run_once" => Some(Self::RunOnce),
            "skip" => Some(Self::Skip),
            _ => None,
        }
    }
}

/// When the user is told how a scheduled run ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleNotify {
    #[default]
    Failure,
    Always,
    Never,
}

impl ScheduleNotify {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Failure => "failure",
            Self::Always => "always",
            Self::Never => "never",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "failure" => Some(Self::Failure),
            "always" => Some(Self::Always),
            "never" => Some(Self::Never),
            _ => None,
        }
    }
}

impl ScheduleSpec {
    /// The spec a row holds; `None` for one this Yardsort cannot read.
    pub fn parse(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_owned())
    }

    /// Whether it fires more than once.
    pub fn repeats(&self) -> bool {
        !matches!(self, Self::Once { .. })
    }
}

/// The longest `every`: a year of minutes. Past that it is a `once`.
const MOST_MINUTES: u32 = 366 * 24 * 60;

/// What is wrong with a spec, in words, or nothing. A `once` must still be to come at `now`.
pub fn check<Tz: TimeZone>(spec: &ScheduleSpec, now: &DateTime<Tz>) -> Result<(), String> {
    let time_ok = |time: &TimeOfDay| {
        (time.hour < 24 && time.minute < 60)
            .then_some(())
            .ok_or_else(|| {
                format!(
                    "`{:02}:{:02}` is not a time of day.",
                    time.hour, time.minute
                )
            })
    };
    match spec {
        ScheduleSpec::Once { at } => {
            if *at <= now.timestamp_millis() {
                return Err("That time has already passed.".to_owned());
            }
        }
        ScheduleSpec::Every { minutes } => {
            if *minutes == 0 {
                return Err("The shortest a schedule repeats is every minute.".to_owned());
            }
            if *minutes > MOST_MINUTES {
                return Err("The longest a schedule repeats is every year.".to_owned());
            }
        }
        ScheduleSpec::Daily { time } => time_ok(time)?,
        ScheduleSpec::Weekly { days, time } => {
            if days.is_empty() {
                return Err("Pick at least one day of the week.".to_owned());
            }
            time_ok(time)?;
        }
        ScheduleSpec::Monthly { day, time } => {
            if day.is_some_and(|day| !(1..=28).contains(&day)) {
                return Err(
                    "Pick a day from 1 to 28, or the last day, so no month is left out.".to_owned(),
                );
            }
            time_ok(time)?;
        }
        ScheduleSpec::Cron { .. } => {}
    }
    // Whatever it is, it has to have a next time: this is where a cron expression is read, and
    // where one that never comes (the 30th of February) is caught.
    match when::next_after(spec, now, None)? {
        Some(_) => Ok(()),
        None => Err("That never comes.".to_owned()),
    }
}

/// A spec in words: "Weekdays at 09:00", "Every 2 hours", "Cron `0 9 * * 1-5`". `zone` is the
/// zone a `once` is told in.
pub fn describe<Tz: TimeZone>(spec: &ScheduleSpec, zone: &Tz) -> String
where
    Tz::Offset: Display,
{
    match spec {
        ScheduleSpec::Once { at } => match zone.timestamp_millis_opt(*at).single() {
            Some(at) => at.format("%-d %b %Y at %H:%M").to_string(),
            None => "Once".to_owned(),
        },
        ScheduleSpec::Every { minutes } => match *minutes {
            1 => "Every minute".to_owned(),
            60 => "Every hour".to_owned(),
            1440 => "Every day".to_owned(),
            minutes if minutes % 1440 == 0 => format!("Every {} days", minutes / 1440),
            minutes if minutes % 60 == 0 => format!("Every {} hours", minutes / 60),
            minutes => format!("Every {minutes} minutes"),
        },
        ScheduleSpec::Daily { time } => format!("Daily at {time}"),
        ScheduleSpec::Weekly { days, time } => {
            let mut days = days.clone();
            days.sort();
            days.dedup();
            let weekdays = &Weekday::ALL[..5];
            let weekend = &Weekday::ALL[5..];
            if days == Weekday::ALL {
                format!("Daily at {time}")
            } else if days == weekdays {
                format!("Weekdays at {time}")
            } else if days == weekend {
                format!("Weekends at {time}")
            } else {
                let names: Vec<&str> = days.iter().map(|day| day.short()).collect();
                format!("{} at {time}", names.join(", "))
            }
        }
        ScheduleSpec::Monthly { day: None, time } => format!("Monthly on the last day at {time}"),
        ScheduleSpec::Monthly {
            day: Some(day),
            time,
        } => {
            let suffix = match day {
                1 | 21 => "st",
                2 | 22 => "nd",
                3 | 23 => "rd",
                _ => "th",
            };
            format!("Monthly on the {day}{suffix} at {time}")
        }
        ScheduleSpec::Cron { expr } => format!("Cron `{}`", expr.trim()),
    }
}

/// A schedule as it is asked for, before anything is checked.
#[derive(Debug, Clone)]
pub struct Draft<'a> {
    /// What the list calls it. The target's own name when not given.
    pub name: Option<&'a str>,
    pub target: ScheduleTarget,
    pub target_id: &'a str,
    pub workspace_id: &'a str,
    /// As given, by input id. Checked against what the target asks for.
    pub inputs: &'a BTreeMap<String, String>,
    pub spec: &'a ScheduleSpec,
    pub missed: IfMissed,
    pub notify: ScheduleNotify,
}

/// A draft that passed every check, ready to be written.
struct Checked {
    name: String,
    workspace: WorkspaceRow,
    inputs: String,
    spec: String,
    next_at: Option<i64>,
}

/// Everything that can be known to be wrong before the first time comes: the workspace is
/// there, the target is there and ready, its inputs fit, nothing in it needs a person, and the
/// spec has a next time. The doors check all of it again when the time comes, since any of it
/// can change in between; this is so a mistake is told to the person making it.
fn check_draft<Tz: TimeZone>(
    store: &Store,
    data_dir: &Path,
    look: &dyn Look,
    draft: &Draft<'_>,
    now: &DateTime<Tz>,
) -> IpcResult<Checked> {
    let workspace = store
        .workspace(draft.workspace_id)?
        .filter(|w| !w.forgotten)
        .ok_or_else(|| IpcError::new("workspace_not_found", "That workspace is not there."))?;
    if workspace.archived {
        return Err(IpcError::new(
            "workspace_archived",
            format!(
                "`{}` is archived. Restore it before scheduling something in it.",
                workspace.name
            ),
        ));
    }
    let target_name = match draft.target {
        ScheduleTarget::Workflow => {
            let entry = crate::workflow::find(data_dir, draft.target_id).ok_or_else(|| {
                IpcError::new(
                    "workflow_not_found",
                    format!("There is no workflow `{}`.", draft.target_id),
                )
            })?;
            let Some(workflow) = entry.workflow.as_ref().filter(|_| entry.runnable()) else {
                return Err(IpcError::new(
                    "workflow_invalid",
                    format!("`{}` has problems and cannot run.", entry.id),
                ));
            };
            let harness = |wanted: &str| look.harness(wanted);
            crate::workflow::runs::check_inputs(workflow, draft.inputs, &harness)
                .map_err(|why| IpcError::new("workflow_inputs", why))?;
            if let Some(why) = workflow_needs_a_person(store, &workspace, workflow)? {
                return Err(IpcError::new("schedule_needs_person", why));
            }
            workflow.name.clone()
        }
        ScheduleTarget::Test => {
            let worktree = Path::new(&workspace.path);
            let entry = crate::smoke::find(worktree, draft.target_id).ok_or_else(|| {
                IpcError::new(
                    "test_not_found",
                    format!(
                        "There is no smoke test `{}` in `{}`.",
                        draft.target_id, workspace.name
                    ),
                )
            })?;
            let Some(test) = entry.test.as_ref().filter(|_| entry.runnable()) else {
                return Err(IpcError::new(
                    "test_invalid",
                    format!("`{}` has problems and cannot run.", entry.id),
                ));
            };
            if let Some(why) = needs_a_person(test) {
                return Err(IpcError::new("schedule_needs_person", why));
            }
            crate::smoke::runs::check_inputs(test, draft.inputs)
                .map_err(|why| IpcError::new("test_inputs", why))?;
            test.name.clone()
        }
    };
    check(draft.spec, now).map_err(|why| IpcError::new("schedule_when", why))?;
    let next_at = when::next_after(draft.spec, now, None)
        .map_err(|why| IpcError::new("schedule_when", why))?
        .map(|at| at.timestamp_millis());
    let name = draft
        .name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or(target_name);
    Ok(Checked {
        name,
        workspace,
        inputs: serde_json::to_string(draft.inputs).unwrap_or_else(|_| "{}".to_owned()),
        spec: draft.spec.to_json(),
        next_at,
    })
}

/// Why a smoke test cannot run with nobody there, when it cannot: it has a step a person does.
pub fn needs_a_person(test: &crate::smoke::Test) -> Option<String> {
    let (_, step) = test
        .all_steps()
        .find(|(_, step)| matches!(step.action, crate::smoke::Action::Manual { .. }))?;
    Some(format!(
        "`{}` has a step a person does (`{}`), so it cannot run on a schedule: nobody would be \
         there to answer it.",
        test.id, step.id
    ))
}

/// Why a workflow cannot run with nobody there, as far as can be known before it starts: one
/// of its `run_test` steps names a smoke test that has a step a person does, or whose file
/// would wait to be approved. Such a step would wait for a person who is not coming — for ever
/// when it has no timeout — and every later time of the schedule would find the run still
/// going.
///
/// A step whose test is a `{{ variable }}` cannot be known until it runs. The run a schedule
/// starts is marked as unattended, and fails that step when its time comes (see
/// [`crate::smoke::runs::Request::unattended`]).
pub(crate) fn workflow_needs_a_person(
    store: &Store,
    workspace: &WorkspaceRow,
    workflow: &crate::workflow::Workflow,
) -> IpcResult<Option<String>> {
    for step in &workflow.steps {
        let crate::workflow::Action::RunTest { test, .. } = &step.action else {
            continue;
        };
        if test.contains("{{") {
            continue;
        }
        if let Some(why) = crate::smoke::runs::waits_for_a_person(store, workspace, test)? {
            return Ok(Some(format!(
                "`{}` cannot run on a schedule: its step `{}` runs a smoke test that would \
                 wait for you. {why}",
                workflow.id, step.id
            )));
        }
    }
    Ok(None)
}

fn fields<'a>(draft: &'a Draft<'_>, checked: &'a Checked) -> ScheduleFields<'a> {
    ScheduleFields {
        name: &checked.name,
        target_kind: draft.target.as_str(),
        target_id: draft.target_id,
        project_id: &checked.workspace.project_id,
        workspace_id: &checked.workspace.id,
        workspace_name: &checked.workspace.name,
        inputs: &checked.inputs,
        spec: &checked.spec,
        missed: draft.missed.as_str(),
        notify: draft.notify.as_str(),
        next_at: checked.next_at,
    }
}

/// Add a schedule, switched on, with its first time worked out from `now`.
pub fn add<Tz: TimeZone>(
    store: &Store,
    data_dir: &Path,
    look: &dyn Look,
    draft: &Draft<'_>,
    created_by: &str,
    now: &DateTime<Tz>,
) -> IpcResult<ScheduleRow> {
    let checked = check_draft(store, data_dir, look, draft, now)?;
    Ok(store.add_schedule(&fields(draft, &checked), created_by)?)
}

/// Change a schedule. Its next time is worked out again from `now`; whether it is on stays.
pub fn change<Tz: TimeZone>(
    store: &Store,
    data_dir: &Path,
    look: &dyn Look,
    id: &str,
    draft: &Draft<'_>,
    now: &DateTime<Tz>,
) -> IpcResult<ScheduleRow> {
    let checked = check_draft(store, data_dir, look, draft, now)?;
    if !store.update_schedule(id, &fields(draft, &checked))? {
        return Err(not_found());
    }
    store.schedule(id)?.ok_or_else(not_found)
}

/// Switch a schedule on or off. Switched on, its next time is worked out from `now`, so times
/// that passed while it was off are not made up for.
pub fn set_enabled<Tz: TimeZone>(
    store: &Store,
    id: &str,
    enabled: bool,
    now: &DateTime<Tz>,
) -> IpcResult<ScheduleRow> {
    let schedule = store.schedule(id)?.ok_or_else(not_found)?;
    let next_at = if enabled {
        if schedule.workspace_id.is_none() {
            return Err(IpcError::new(
                "workspace_not_found",
                format!(
                    "`{}` was deleted, so this has nowhere to run. Edit it to pick a workspace.",
                    schedule.workspace_name
                ),
            ));
        }
        let spec = ScheduleSpec::parse(&schedule.spec).ok_or_else(|| {
            IpcError::new(
                "schedule_when",
                "This Yardsort cannot read when this schedule runs. Edit it to say when.",
            )
        })?;
        let next = when::next_after(&spec, now, None)
            .map_err(|why| IpcError::new("schedule_when", why))?
            .ok_or_else(|| {
                IpcError::new(
                    "schedule_when",
                    "Its time has passed. Edit it to give it a new one.",
                )
            })?;
        Some(next.timestamp_millis())
    } else {
        None
    };
    store.set_schedule_enabled(id, enabled, next_at)?;
    store.schedule(id)?.ok_or_else(not_found)
}

/// Where a schedule stands, for the list: what a person needs to know at a glance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleStanding {
    /// On, with a time to come.
    Waiting,
    /// Its run is going.
    Running,
    /// Its run waits for the user to approve the test file.
    AwaitingApproval,
    /// Its last time could not be queued.
    Refused,
    /// Its workspace was deleted.
    NoWorkspace,
    /// Switched off by the user.
    Off,
    /// A `once` that has had its time.
    Done,
    /// A `once` whose time passed while Yardsort was not running, and that skips those: it
    /// never ran.
    Missed,
}

impl ScheduleStanding {
    /// Whether the user has something to do before this schedule can do its job.
    pub fn needs_attention(self) -> bool {
        matches!(
            self,
            Self::AwaitingApproval | Self::Refused | Self::NoWorkspace
        )
    }
}

/// The run each schedule started that has not ended, by the schedule's id: the newest, should
/// one have more than one. What [`standing`] is given as `going`.
pub fn going_runs(store: &Store) -> IpcResult<BTreeMap<String, ScheduleRunRow>> {
    // Oldest first, so a newer line takes an older one's place.
    Ok(store
        .unsettled_schedule_runs()?
        .into_iter()
        .map(|line| (line.schedule_id.clone(), line))
        .collect())
}

/// Where `schedule` stands, given the newest line of its history and the line of the run it
/// started that has not ended, from [`going_runs`].
///
/// The two are different lines once a later time was skipped because that run was still going.
/// The run is what counts then: a schedule whose run waits for approval goes on saying so,
/// however many times have been skipped behind it.
pub fn standing(
    schedule: &ScheduleRow,
    last: Option<&ScheduleRunRow>,
    going: Option<&ScheduleRunRow>,
) -> ScheduleStanding {
    if schedule.workspace_id.is_none() {
        return ScheduleStanding::NoWorkspace;
    }
    // A run that is still going is said first: a `once` that has fired is not done with while
    // its run waits for the user.
    let going = going.filter(|line| line.outcome == "started" && line.result.is_none());
    if let Some(line) = going {
        return if line.told_waiting {
            ScheduleStanding::AwaitingApproval
        } else {
            ScheduleStanding::Running
        };
    }
    if !schedule.enabled {
        if schedule.next_at.is_some() {
            return ScheduleStanding::Off;
        }
        // A `once` that has had its time. What became of that time is said, not assumed: one
        // that was missed or refused never ran.
        return match last.map(|line| line.outcome.as_str()) {
            Some("missed") => ScheduleStanding::Missed,
            Some("refused") => ScheduleStanding::Refused,
            _ => ScheduleStanding::Done,
        };
    }
    // On, with no time to come: its next time could not be worked out when it last fired.
    if schedule.next_at.is_none() {
        return ScheduleStanding::Refused;
    }
    match last {
        Some(line) if line.outcome == "refused" => ScheduleStanding::Refused,
        _ => ScheduleStanding::Waiting,
    }
}

pub(crate) fn not_found() -> IpcError {
    IpcError::new("schedule_not_found", "That schedule is not there.")
}

#[cfg(test)]
pub(crate) mod tests;
