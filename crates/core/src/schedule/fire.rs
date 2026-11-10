//! What becomes of a time that has come, and of the run it started.
//!
//! [`tick`] is the whole of a schedule firing: find what is due, take each time by moving the
//! schedule on to its next, and then either let the time go (it was missed and the schedule
//! skips those) or ask for a run at the door a person would use. Whatever the door says is
//! written down as one line of the schedule's history. [`settle`] is the other half, later:
//! when a run a schedule started has ended, its ending is copied onto that line, and the user
//! is told if the schedule says to.
//!
//! Neither reads the clock, starts a thread or shows anything. The app calls them from its
//! schedules thread and does the showing.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, TimeZone};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::{not_found, when, IfMissed, ScheduleNotify, ScheduleSpec, ScheduleTarget};
use crate::error::{IpcError, IpcResult};
use crate::store::{NewScheduleRun, ScheduleRow, ScheduleRunRow, Store};
use crate::workflow::runs::Look;

/// How late a time can be acted on and still count as on time. Later than this, the app was
/// closed or the machine asleep, and the schedule's own rule for a missed time decides.
pub const LATE_AFTER_MS: i64 = 5 * 60_000;

/// What became of a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleOutcome {
    /// A run was queued.
    Started,
    /// The previous run was still going.
    Skipped,
    /// The time passed while Yardsort was not running, and the schedule skips those.
    Missed,
    /// The run could not be queued.
    Refused,
}

impl ScheduleOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Skipped => "skipped",
            Self::Missed => "missed",
            Self::Refused => "refused",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "started" => Some(Self::Started),
            "skipped" => Some(Self::Skipped),
            "missed" => Some(Self::Missed),
            "refused" => Some(Self::Refused),
            _ => None,
        }
    }
}

/// A time that was acted on: the schedule as it was when its time came, and the line written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fired {
    pub schedule: ScheduleRow,
    pub run: ScheduleRunRow,
    /// What to tell the user about it now: that the schedule could not start its run, said
    /// once for as long as it goes on being refused.
    pub notice: Option<Notice>,
}

/// Something to tell the user about a schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub schedule_id: String,
    pub title: String,
    pub body: String,
}

/// Act on every schedule whose time has come by `now`. Each time is acted on once, by whoever
/// claims it; a second caller, or a second tick, finds nothing left to do.
pub fn tick<Tz: TimeZone>(
    store: &Store,
    data_dir: &Path,
    look: &dyn Look,
    now: &DateTime<Tz>,
) -> IpcResult<Vec<Fired>> {
    let now_ms = now.timestamp_millis();
    let mut fired = Vec::new();
    for schedule in store.due_schedules(now_ms)? {
        let Some(due_at) = schedule.next_at else {
            continue;
        };
        // From now, not from the time that was due: however many times passed while nobody was
        // looking, there is one to act on and the next is still to come.
        let next = match ScheduleSpec::parse(&schedule.spec) {
            Some(spec) => when::next_after(&spec, now, Some(due_at))
                .map(|next| next.map(|next| next.timestamp_millis())),
            None => Err("This Yardsort cannot read when this schedule runs.".to_owned()),
        };
        let taken = match &next {
            Ok(next_at) => store.claim_schedule(&schedule.id, due_at, *next_at)?,
            // Having no next time is how a `once` ends. This is not that, so it is not switched
            // off: it stays on with nothing to come, and says it was refused.
            Err(_) => store.stall_schedule(&schedule.id, due_at)?,
        };
        if !taken {
            continue;
        }
        let late = now_ms - due_at > LATE_AFTER_MS;
        let (outcome, run_id, note) = if let Err(why) = next {
            (
                ScheduleOutcome::Refused,
                None,
                Some(format!(
                    "{why} Its next time cannot be worked out, so it fires nothing until you \
                     edit it."
                )),
            )
        } else if late && IfMissed::parse(&schedule.missed) == Some(IfMissed::Skip) {
            (ScheduleOutcome::Missed, None, None)
        } else {
            match ask_for_run(store, data_dir, look, &schedule, false) {
                Ok(run_id) => (ScheduleOutcome::Started, Some(run_id), None),
                Err(error) if error.code.ends_with("_already_running") => (
                    ScheduleOutcome::Skipped,
                    None,
                    Some("The previous run was still going.".to_owned()),
                ),
                Err(error) => {
                    // No later time can do better with no workspace, so it stops asking.
                    if schedule.workspace_id.is_none() {
                        store.set_schedule_enabled(&schedule.id, false, None)?;
                    }
                    (ScheduleOutcome::Refused, None, Some(error.message))
                }
            }
        };
        // A refusal has no run whose ending could be told later, so it is told now — once,
        // not at every time that comes while its cause is still there.
        let notice = match &note {
            Some(why)
                if outcome == ScheduleOutcome::Refused
                    && ScheduleNotify::parse(&schedule.notify) != Some(ScheduleNotify::Never)
                    && !store
                        .schedule_runs(&schedule.id, 1)?
                        .first()
                        .is_some_and(|last| last.outcome == ScheduleOutcome::Refused.as_str()) =>
            {
                Some(Notice {
                    schedule_id: schedule.id.clone(),
                    title: format!("{} could not start", schedule.name),
                    body: why.clone(),
                })
            }
            _ => None,
        };
        let run = store.record_schedule_run(&NewScheduleRun {
            schedule_id: &schedule.id,
            due_at,
            fired_at: now_ms,
            outcome: outcome.as_str(),
            manual: false,
            run_id: run_id.as_deref(),
            note: note.as_deref(),
        })?;
        fired.push(Fired {
            schedule,
            run,
            notice,
        });
    }
    Ok(fired)
}

/// Start a schedule's run now, because a person asked. Its next time is left where it is. A
/// refusal is the person's to read, so it is returned and not written to the history.
pub fn run_now(
    store: &Store,
    data_dir: &Path,
    look: &dyn Look,
    id: &str,
    now_ms: i64,
) -> IpcResult<Fired> {
    let schedule = store.schedule(id)?.ok_or_else(not_found)?;
    // A person asked, so a person is there: a smoke test the workflow runs may wait for them.
    let run_id = ask_for_run(store, data_dir, look, &schedule, true)?;
    let run = store.record_schedule_run(&NewScheduleRun {
        schedule_id: &schedule.id,
        due_at: now_ms,
        fired_at: now_ms,
        outcome: ScheduleOutcome::Started.as_str(),
        manual: true,
        run_id: Some(&run_id),
        note: None,
    })?;
    Ok(Fired {
        schedule,
        run,
        notice: None,
    })
}

/// Ask for the schedule's run at the door a person would use, and return the run's id.
/// `attended` is whether a person asked for it just now, rather than the clock.
fn ask_for_run(
    store: &Store,
    data_dir: &Path,
    look: &dyn Look,
    schedule: &ScheduleRow,
    attended: bool,
) -> IpcResult<String> {
    let Some(workspace_id) = schedule.workspace_id.as_deref() else {
        return Err(IpcError::new(
            "workspace_not_found",
            format!(
                "`{}` was deleted, so this has nowhere to run. Edit it to pick a workspace.",
                schedule.workspace_name
            ),
        ));
    };
    let inputs: BTreeMap<String, String> =
        serde_json::from_str(&schedule.inputs).unwrap_or_default();
    match ScheduleTarget::parse(&schedule.target_kind) {
        Some(ScheduleTarget::Workflow) => {
            if !attended {
                // A smoke test it runs may have gained a step a person does, or changed, since
                // the schedule was saved. Whatever is missing or broken is the door's to say.
                let workflow = crate::workflow::find(data_dir, &schedule.target_id);
                let workflow = workflow.as_ref().and_then(|entry| entry.workflow.as_ref());
                if let (Some(workflow), Some(workspace)) =
                    (workflow, store.workspace(workspace_id)?)
                {
                    if let Some(why) = super::workflow_needs_a_person(store, &workspace, workflow)?
                    {
                        return Err(IpcError::new("schedule_needs_person", why));
                    }
                }
            }
            crate::workflow::runs::queue(
                store,
                data_dir,
                &crate::workflow::runs::Request {
                    workflow_id: &schedule.target_id,
                    workspace_id,
                    inputs: &inputs,
                    // The app is what queues it. Which schedule asked is in the schedule's
                    // history.
                    requested_by: "app",
                    unattended: !attended,
                    origin_record: None,
                    depth: 0,
                    event: None,
                    tracked: None,
                },
                look,
            )
        }
        Some(ScheduleTarget::Test) => {
            // The file may have gained a step a person does since the schedule was saved.
            let needs_person = store
                .workspace(workspace_id)?
                .and_then(|workspace| {
                    crate::smoke::find(Path::new(&workspace.path), &schedule.target_id)
                })
                .and_then(|entry| entry.test.as_ref().and_then(super::needs_a_person));
            if let Some(why) = needs_person {
                return Err(IpcError::new("schedule_needs_person", why));
            }
            crate::smoke::runs::queue(
                store,
                &crate::smoke::runs::Request {
                    test_id: &schedule.target_id,
                    workspace_id,
                    inputs: &inputs,
                    requested_by: "app",
                    session_record_id: None,
                    // The schedule's own test: a file nobody approved waits in the open, with
                    // the schedule saying so and the user told (see `settle`).
                    unattended: false,
                },
            )
            .map(|queued| queued.id)
        }
        None => Err(IpcError::new(
            "schedule_target",
            format!(
                "This Yardsort does not know how to start a `{}`.",
                schedule.target_kind
            ),
        )),
    }
}

/// What [`settle`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settled {
    /// Whether any line of any history changed, so lists should be read again.
    pub changed: bool,
    pub notices: Vec<Notice>,
}

/// Look at the runs schedules started and have not seen end. One that ended has its ending
/// copied onto its line, and a notice when the schedule asks for one. One that waits for the
/// user to approve its file gets a notice, once.
pub fn settle(store: &Store) -> IpcResult<Settled> {
    let mut settled = Settled::default();
    for line in store.unsettled_schedule_runs()? {
        let Some(schedule) = store.schedule(&line.schedule_id)? else {
            continue;
        };
        let notify = ScheduleNotify::parse(&schedule.notify).unwrap_or_default();
        let Some(run_id) = line.run_id.as_deref() else {
            settled.changed |= store.settle_schedule_run(&line.id, "gone", false)?;
            continue;
        };
        // The run's status, why it failed if it did, and whether it waits for approval.
        let (status, reason, waiting) = match ScheduleTarget::parse(&schedule.target_kind) {
            Some(ScheduleTarget::Workflow) => match store.workflow_run(run_id)? {
                Some(run) if run.active() => continue,
                Some(run) => {
                    let reason = run.error.clone().or_else(|| {
                        let steps = store.workflow_steps(run_id).unwrap_or_default();
                        steps
                            .iter()
                            .find(|step| step.status == "failed")
                            .map(|step| failed_step(&step.step_id, step.note.as_deref()))
                    });
                    (run.status, reason, false)
                }
                None => ("gone".to_owned(), None, false),
            },
            Some(ScheduleTarget::Test) => match store.test_run(run_id)? {
                Some(run) if run.status == "awaiting_approval" => (run.status, None, true),
                Some(run) if run.active() => continue,
                Some(run) => {
                    let reason = run.error.clone().or_else(|| {
                        let steps = store.test_steps(run_id).unwrap_or_default();
                        steps
                            .iter()
                            .find(|step| step.status == "failed")
                            .map(|step| failed_step(&step.step_id, step.note.as_deref()))
                    });
                    (run.status, reason, false)
                }
                None => ("gone".to_owned(), None, false),
            },
            None => ("gone".to_owned(), None, false),
        };
        if waiting {
            if !line.told_waiting && store.mark_schedule_run_told_waiting(&line.id)? {
                settled.changed = true;
                if notify != ScheduleNotify::Never {
                    settled.notices.push(Notice {
                        schedule_id: schedule.id.clone(),
                        title: format!("{} is waiting for your approval", schedule.name),
                        body: format!(
                            "The smoke test `{}` is new, or has changed since you approved it. \
                             Open Yardsort to look at it and run it.",
                            schedule.target_id
                        ),
                    });
                }
            }
            continue;
        }
        let failed = matches!(status.as_str(), "failed" | "errored");
        let tell = status != "gone"
            && match notify {
                ScheduleNotify::Always => true,
                ScheduleNotify::Failure => failed,
                ScheduleNotify::Never => false,
            };
        if !store.settle_schedule_run(&line.id, &status, tell)? {
            continue;
        }
        settled.changed = true;
        if tell {
            let what =
                ScheduleTarget::parse(&schedule.target_kind).map_or("run", ScheduleTarget::words);
            let place = format!(
                "The {what} `{}` in `{}`.",
                schedule.target_id, schedule.workspace_name
            );
            settled.notices.push(Notice {
                schedule_id: schedule.id.clone(),
                title: format!("{} {}", schedule.name, ending(&status)),
                body: match reason.filter(|_| failed) {
                    Some(reason) => failure_notice(&reason),
                    None => place,
                },
            });
        }
    }
    Ok(settled)
}

/// What a notification says when a run failed: the first line of the reason, kept short.
/// When the reason continues on another line, or that line was cut short, it says where
/// to read the rest. A one-line reason is sent as it is. The full text stays on the run.
pub fn failure_notice(reason: &str) -> String {
    const CAP: usize = 200;
    let mut lines = reason.lines();
    let first = lines.next().unwrap_or("").trim();
    let more = lines.any(|line| !line.trim().is_empty());
    let mut line = if first.is_empty() {
        "The run failed.".to_owned()
    } else {
        first.to_owned()
    };
    let cut = line.chars().count() > CAP;
    if cut {
        line = line.chars().take(CAP).collect();
        line.push_str("...");
    }
    if more || cut {
        format!("{line}\nOpen Yardsort for the report.")
    } else {
        line
    }
}

fn failed_step(step_id: &str, note: Option<&str>) -> String {
    match note {
        Some(note) => format!("`{step_id}`: {note}"),
        None => format!("The step `{step_id}` failed."),
    }
}

/// A run's ending as the end of a sentence about it: "Nightly smoke failed".
pub fn ending(status: &str) -> &str {
    match status {
        "succeeded" => "finished",
        "passed" => "passed",
        "failed" => "failed",
        "errored" => "could not finish",
        "cancelled" => "was cancelled",
        "refused" => "was not approved",
        "gone" => "is gone",
        other => other,
    }
}

/// Work out again the next time of every waiting schedule that names a time of day, for when
/// the machine's zone is not the one they were worked out in: at start, and when the offset
/// from UTC changes. A time that has already come is left for [`tick`]. Returns whether any
/// moved.
pub fn rezone<Tz: TimeZone>(store: &Store, now: &DateTime<Tz>) -> IpcResult<bool> {
    let now_ms = now.timestamp_millis();
    let mut moved = false;
    for schedule in store.waiting_schedules()? {
        let Some(at) = schedule.next_at.filter(|at| *at > now_ms) else {
            continue;
        };
        let Some(spec) =
            ScheduleSpec::parse(&schedule.spec).filter(|spec| when::cron_of(spec).is_some())
        else {
            continue;
        };
        let Ok(Some(next)) = when::next_after(&spec, now, None) else {
            continue;
        };
        let next = next.timestamp_millis();
        if next != at {
            moved |= store.move_schedule(&schedule.id, at, Some(next))?;
        }
    }
    Ok(moved)
}

#[cfg(test)]
mod tests;
