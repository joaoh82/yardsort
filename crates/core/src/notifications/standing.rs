//! Rows for what already stands in the store, waiting on a person: a workflow's question or
//! approval, a smoke test's run waiting for its file to be approved, and a schedule or a trigger
//! that could not start. And one for each workflow run that ended.
//!
//! Nothing here is told when the thing happens. The app and `ys` are two processes that both
//! open a wait, queue a test and answer either; what both leave is the store. So [`read`] looks
//! at where each stands and makes the list agree: a row for what waits now, and whatever
//! stopped waiting leaves its part. Reading twice changes nothing.
//!
//! A schedule or a trigger whose run waits for a smoke test's approval is not a row of its own.
//! The test's run is, and it is the thing to answer.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Mutex, PoisonError};

use rusqlite::params;

use super::NotificationKind;
use crate::error::IpcResult;
use crate::schedule::{self, ScheduleStanding};
use crate::store::{Store, StoreResult};
use crate::trigger::{self, TriggerStanding};
use crate::workflow::waits;

/// Under the event readers' cursors (`trigger_cursors`): the newest ending of a workflow run
/// this list has taken in, in epoch milliseconds.
const RUNS: &str = "notifications.runs";

/// How many of a trigger's fires say where it stands: enough to reach its newest decided one.
pub(super) const FIRES: usize = 20;

/// One pass at a time. Two that crossed could each act on what the other had already changed:
/// a wait answered between them would be listed again by the slower one.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Something that waits on a person now.
struct Waits {
    /// Stable for as long as it waits, and not used again once it has stopped.
    key: String,
    kind: NotificationKind,
    workspace: Option<String>,
    project: Option<String>,
    /// The run, the schedule or the trigger.
    subject: String,
    step: Option<String>,
    /// When it started waiting.
    at: i64,
}

/// Make the list agree with where workflows, smoke tests, schedules and triggers stand at
/// `now`. True when the list changed.
///
/// What waits when the list is first read is listed: it needs its person now, whenever it
/// began. A run that ended before the first read is not news.
pub fn read(store: &Store, now: i64) -> IpcResult<bool> {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let mut waiting = Vec::new();
    workflows(store, &mut waiting)?;
    tests(store, &mut waiting)?;
    schedules(store, now, &mut waiting)?;
    triggers(store, now, &mut waiting)?;
    let mut changed = mirror(store, &waiting, now)?;
    changed |= ended_runs(store, now)?;
    Ok(changed)
}

/// Runs blocked on an approval or on the answer to a question. One the person has decided and
/// the engine has not applied yet is not waiting on them any more.
fn workflows(store: &Store, waiting: &mut Vec<Waits>) -> StoreResult<()> {
    for wait in waits::list_open(store)? {
        let Some(run) = store.workflow_run(&wait.run_id)? else {
            continue;
        };
        let Some(workspace) = run.workspace_id else {
            continue;
        };
        let kind = if wait.kind == waits::KIND_QUESTION {
            NotificationKind::WorkflowQuestion
        } else {
            NotificationKind::WorkflowApproval
        };
        waiting.push(Waits {
            // A step entered again opens a new wait, at a new time.
            key: format!(
                "workflow:{}:{}:{}",
                wait.run_id, wait.step_id, wait.created_at
            ),
            kind,
            workspace: Some(workspace),
            project: None,
            subject: wait.run_id,
            step: Some(wait.step_id),
            at: wait.created_at,
        });
    }
    Ok(())
}

/// Smoke test runs that wait for their file to be approved.
fn tests(store: &Store, waiting: &mut Vec<Waits>) -> StoreResult<()> {
    for run in store.active_test_runs()? {
        if run.status != "awaiting_approval" {
            continue;
        }
        let Some(workspace) = run.workspace_id else {
            continue;
        };
        waiting.push(Waits {
            key: format!("test:{}:waits", run.id),
            kind: NotificationKind::TestWaits,
            workspace: Some(workspace),
            project: None,
            subject: run.id,
            step: None,
            at: run.created_at,
        });
    }
    Ok(())
}

/// Schedules that cannot do their job until someone changes something: the last time was
/// refused, or the workspace is gone.
fn schedules(store: &Store, now: i64, waiting: &mut Vec<Waits>) -> IpcResult<()> {
    let latest: BTreeMap<String, _> = store
        .latest_schedule_runs()?
        .into_iter()
        .map(|line| (line.schedule_id.clone(), line))
        .collect();
    let going = schedule::going_runs(store)?;
    for row in store.schedules()? {
        let last = latest.get(&row.id);
        // What the row is told apart by: the line that was refused, or else the schedule as it
        // was last changed. Changing it and getting into the same trouble is a new row.
        let changed = format!("at{}", row.updated_at);
        let (kind, stamp, at) = match schedule::standing(&row, last, going.get(&row.id)) {
            ScheduleStanding::Refused => match last.filter(|line| line.outcome == "refused") {
                Some(line) => (
                    NotificationKind::ScheduleRefused,
                    line.id.clone(),
                    line.fired_at,
                ),
                None => (NotificationKind::ScheduleRefused, changed, now),
            },
            ScheduleStanding::NoWorkspace => (NotificationKind::ScheduleNoWorkspace, changed, now),
            _ => continue,
        };
        waiting.push(Waits {
            key: format!("schedule:{}:{}:{stamp}", row.id, kind.name()),
            kind,
            workspace: row.workspace_id,
            project: Some(row.project_id),
            subject: row.id,
            step: None,
            at,
        });
    }
    Ok(())
}

/// Triggers that cannot do their job until someone changes something: the last fire was
/// refused, a subject has had the runs its limit allows, or the workspace is gone.
fn triggers(store: &Store, now: i64, waiting: &mut Vec<Waits>) -> IpcResult<()> {
    for row in store.triggers()? {
        let fires = store.trigger_fires(&row.id, FIRES)?;
        let decided = fires.iter().find(|fire| fire.outcome != "gathering");
        let kind = match trigger::standing_now(store, &row, &fires)? {
            TriggerStanding::Refused => NotificationKind::TriggerRefused,
            TriggerStanding::Capped => NotificationKind::TriggerCapped,
            TriggerStanding::NoWorkspace => NotificationKind::TriggerNoWorkspace,
            _ => continue,
        };
        let (stamp, at) = match decided {
            Some(fire) if kind != NotificationKind::TriggerNoWorkspace => {
                (fire.id.clone(), fire.decided_at.unwrap_or(fire.due_at))
            }
            _ => (format!("at{}", row.updated_at), now),
        };
        waiting.push(Waits {
            key: format!("trigger:{}:{}:{stamp}", row.id, kind.name()),
            kind,
            workspace: row.workspace_id,
            project: Some(row.project_id),
            subject: row.id,
            step: None,
            at,
        });
    }
    Ok(())
}

/// Make the rows that wait be exactly `waiting`: one that is not there yet is added, and one
/// that is there and waits no more leaves, answered. An agent's question is not one of these;
/// its own events say when it is answered.
fn mirror(store: &Store, waiting: &[Waits], now: i64) -> StoreResult<bool> {
    let conn = store.conn();
    let listed: Vec<(String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, key FROM notifications
             WHERE settled_at IS NULL AND part = 'needs' AND kind NOT LIKE 'agent.%'",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let wanted: HashSet<&str> = waiting.iter().map(|waits| waits.key.as_str()).collect();
    let mut changed = false;
    for (id, key) in &listed {
        if !wanted.contains(key.as_str()) {
            changed |= conn.execute(
                "UPDATE notifications SET settled_at = ?2 WHERE id = ?1 AND settled_at IS NULL",
                params![id, now],
            )? > 0;
        }
    }
    let have: HashSet<&str> = listed.iter().map(|(_, key)| key.as_str()).collect();
    for waits in waiting {
        if have.contains(waits.key.as_str()) {
            continue;
        }
        changed |= conn.execute(
            "INSERT OR IGNORE INTO notifications
             (id, key, kind, part, workspace_id, project_id, subject_id, step_id, created_at)
             VALUES (?1, ?2, ?3, 'needs', ?4, ?5, ?6, ?7, ?8)",
            params![
                uuid::Uuid::new_v4().to_string(),
                waits.key,
                waits.kind.name(),
                waits.workspace,
                waits.project,
                waits.subject,
                waits.step,
                waits.at,
            ],
        )? > 0;
    }
    Ok(changed)
}

/// A finished row for each workflow run that succeeded or failed since the last read. One a
/// person ended is not news to them: a run that was cancelled, and one that stopped because
/// its approval was rejected or sent back for changes.
///
/// The first read starts from `now`: a run that ended before there was a list is not news.
fn ended_runs(store: &Store, now: i64) -> StoreResult<bool> {
    let Some(after) = store.trigger_cursor(RUNS)? else {
        store.set_trigger_cursor(RUNS, now)?;
        return Ok(false);
    };
    let conn = store.conn();
    // A failed run that was taken up again is going: what was said of its failing is over.
    let mut changed = conn.execute(
        "UPDATE notifications SET settled_at = ?1
         WHERE settled_at IS NULL AND kind IN ('workflow.succeeded', 'workflow.failed')
           AND subject_id IN (SELECT id FROM workflow_runs WHERE status IN ('queued', 'running'))",
        [now],
    )? > 0;
    let ended: Vec<(String, String, String, i64)> = {
        let mut stmt = conn.prepare(
            "SELECT id, status, workspace_id, ended_at FROM workflow_runs
             WHERE status IN ('succeeded', 'failed') AND workspace_id IS NOT NULL
               AND ended_at > ?1
               AND NOT EXISTS (
                 SELECT 1 FROM workflow_step_runs step
                 JOIN workflow_approval_decisions decided
                   ON decided.run_id = step.run_id AND decided.step_id = step.step_id
                 WHERE step.run_id = workflow_runs.id AND step.status = 'failed'
                   AND decided.decision IN ('rejected', 'changes_requested'))
             ORDER BY ended_at, id",
        )?;
        let rows = stmt.query_map([after], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        rows.collect::<Result<_, _>>()?
    };
    let mut newest = after;
    for (run, status, workspace, ended_at) in &ended {
        let kind = if status == "failed" {
            NotificationKind::WorkflowFailed
        } else {
            NotificationKind::WorkflowSucceeded
        };
        changed |= conn.execute(
            "INSERT OR IGNORE INTO notifications
             (id, key, kind, part, workspace_id, subject_id, created_at)
             VALUES (?1, ?2, ?3, 'finished', ?4, ?5, ?6)",
            params![
                uuid::Uuid::new_v4().to_string(),
                // A failed run taken up again ends a second time, at a later time.
                format!("workflow:{run}:ended:{ended_at}"),
                kind.name(),
                workspace,
                run,
                ended_at,
            ],
        )? > 0;
        newest = newest.max(*ended_at);
    }
    drop(conn);
    // Never up to this very millisecond: another run may still end in it, and would be passed
    // over. One read twice is one row, by its key.
    let newest = newest.min(now - 1);
    if newest > after {
        store.set_trigger_cursor(RUNS, newest)?;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests;
