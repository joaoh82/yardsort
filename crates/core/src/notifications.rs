//! The Notifications list: what waits for a person, and what finished while they were not
//! looking. See `docs/design/48-notifications.md`.
//!
//! One row is one thing that happened. A row about an agent's conversation is written here when
//! the thing happens, and it leaves its part of the list by itself:
//!
//! - **Needs you.** The agent asked for a permission ([`read_events`], from its own hook's
//!   `approval.requested`). It leaves when the wait is answered, by the rule the phone's notice
//!   uses ([`crate::notify`]), or when the agent is gone.
//! - **Finished.** The agent's turn ended ([`read_events`] for a harness that reports turns,
//!   [`settled_terminal`] for a terminal gone quiet after a stretch of work; the caller's
//!   [`Turns`] folds the two into one), or its process stopped with an error ([`exited`]). It
//!   leaves when someone looks: [`seen`], [`seen_terminal`], [`seen_all`], or a message typed to
//!   the agent.
//!
//! Everything else that waits is read from where it already stands in the store
//! ([`standing::read`]): a workflow's question or approval, a workflow run that ended, a smoke
//! test waiting to be let run, and a schedule or a trigger that could not start.
//!
//! What has left is kept for a week ([`KEPT_MS`]) and then deleted.
//!
//! Nothing is read from a terminal: a quiet terminal says only that a turn ended. The words a
//! row is shown with are not stored either; [`Lines`] reads them when the list is asked for.
//!
//! Whatever runs the terminals keeps the list with a [`Keeper`]: the app, and a host with no
//! window. It is told what only that process hears, and it is what a device is answered from
//! over Remote (`crate::remote`).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use pty_host::{HostEvent, SessionInfo, SessionState};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::activity::{asks, digest};
use crate::error::IpcResult;
use crate::store::{Store, StoreResult};
use crate::trigger::turns::{Change, Turns};
use crate::usage::logs::Sources;

/// How long a row that left its part of the list is kept under Earlier.
pub const KEPT_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// How many rows of Earlier a listing gives, newest first.
pub const EARLIER_SHOWN: usize = 50;

/// How much of what an agent said last a row shows.
pub const SAID_CHARS: usize = 240;

/// The cursor's name among the event readers' (`trigger_cursors`): where [`read_events`] last
/// stopped in `agent_events`.
const CURSOR: &str = "notifications";

/// How many events one pass of [`read_events`] takes.
const BATCH: usize = 500;

/// Under the same table of cursors: when this list last heard from the terminals, in epoch
/// milliseconds. What printed after that printed while nothing was listening.
const HEARD: &str = "notifications.heard";

/// How long after the last word heard a terminal's last output must be, to count as printed
/// while nothing listened: [`heard`] is written every few seconds, not on every event.
const HEARD_SLACK_MS: i64 = 8_000;

/// How long before a terminal's last output a finished row may have been made and still be
/// about it: a quiet is told a few seconds after the last output.
const QUIET_LAG_MS: i64 = 5_000;

/// What ends a wait for the whole conversation: a person typed, or the turn or session moved on.
const RUN_ANSWERED: [&str; 4] = [
    "prompt.submitted",
    "turn.completed",
    "turn.failed",
    "session.ended",
];

/// What may end a wait for one request. Whether it does is [`crate::notify`]'s to say.
const REQUEST_ANSWERED: [&str; 3] = ["tool.completed", "tool.failed", "approval.resolved"];

/// What a row is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum NotificationKind {
    /// An agent asked for a permission.
    #[serde(rename = "agent.asks")]
    AgentAsks,
    /// An agent finished a turn.
    #[serde(rename = "agent.finished")]
    AgentFinished,
    /// An agent's turn failed.
    #[serde(rename = "agent.failed")]
    AgentFailed,
    /// An agent's process exited with an error.
    #[serde(rename = "agent.stopped")]
    AgentStopped,
    /// A workflow run waits for an approval.
    #[serde(rename = "workflow.approval")]
    WorkflowApproval,
    /// A workflow run waits for the answer to a question.
    #[serde(rename = "workflow.question")]
    WorkflowQuestion,
    /// A workflow run ended well.
    #[serde(rename = "workflow.succeeded")]
    WorkflowSucceeded,
    /// A workflow run failed.
    #[serde(rename = "workflow.failed")]
    WorkflowFailed,
    /// A smoke test's run waits for its file to be approved.
    #[serde(rename = "test.waits")]
    TestWaits,
    /// A schedule's time came and its run could not be started.
    #[serde(rename = "schedule.refused")]
    ScheduleRefused,
    /// A schedule's workspace was deleted.
    #[serde(rename = "schedule.noWorkspace")]
    ScheduleNoWorkspace,
    /// A trigger fired and its run could not be started.
    #[serde(rename = "trigger.refused")]
    TriggerRefused,
    /// A trigger has had the runs its limit allows, and waits to be reset.
    #[serde(rename = "trigger.capped")]
    TriggerCapped,
    /// A trigger's workspace was deleted.
    #[serde(rename = "trigger.noWorkspace")]
    TriggerNoWorkspace,
}

/// Every kind, for reading one back by its name.
const KINDS: [NotificationKind; 14] = [
    NotificationKind::AgentAsks,
    NotificationKind::AgentFinished,
    NotificationKind::AgentFailed,
    NotificationKind::AgentStopped,
    NotificationKind::WorkflowApproval,
    NotificationKind::WorkflowQuestion,
    NotificationKind::WorkflowSucceeded,
    NotificationKind::WorkflowFailed,
    NotificationKind::TestWaits,
    NotificationKind::ScheduleRefused,
    NotificationKind::ScheduleNoWorkspace,
    NotificationKind::TriggerRefused,
    NotificationKind::TriggerCapped,
    NotificationKind::TriggerNoWorkspace,
];

impl NotificationKind {
    fn name(self) -> &'static str {
        match self {
            NotificationKind::AgentAsks => "agent.asks",
            NotificationKind::AgentFinished => "agent.finished",
            NotificationKind::AgentFailed => "agent.failed",
            NotificationKind::AgentStopped => "agent.stopped",
            NotificationKind::WorkflowApproval => "workflow.approval",
            NotificationKind::WorkflowQuestion => "workflow.question",
            NotificationKind::WorkflowSucceeded => "workflow.succeeded",
            NotificationKind::WorkflowFailed => "workflow.failed",
            NotificationKind::TestWaits => "test.waits",
            NotificationKind::ScheduleRefused => "schedule.refused",
            NotificationKind::ScheduleNoWorkspace => "schedule.noWorkspace",
            NotificationKind::TriggerRefused => "trigger.refused",
            NotificationKind::TriggerCapped => "trigger.capped",
            NotificationKind::TriggerNoWorkspace => "trigger.noWorkspace",
        }
    }

    fn named(name: &str) -> Option<Self> {
        KINDS.into_iter().find(|kind| kind.name() == name)
    }

    /// What the row is about, which says where its words come from.
    fn about(self) -> About {
        match self {
            NotificationKind::AgentAsks
            | NotificationKind::AgentFinished
            | NotificationKind::AgentFailed
            | NotificationKind::AgentStopped => About::Agent,
            NotificationKind::WorkflowApproval
            | NotificationKind::WorkflowQuestion
            | NotificationKind::WorkflowSucceeded
            | NotificationKind::WorkflowFailed => About::Workflow,
            NotificationKind::TestWaits => About::Test,
            NotificationKind::ScheduleRefused | NotificationKind::ScheduleNoWorkspace => {
                About::Schedule
            }
            NotificationKind::TriggerRefused
            | NotificationKind::TriggerCapped
            | NotificationKind::TriggerNoWorkspace => About::Trigger,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum About {
    Agent,
    Workflow,
    Test,
    Schedule,
    Trigger,
}

/// Where a row is in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum NotificationPart {
    /// Waiting for an answer.
    Needs,
    /// Finished, and not seen yet.
    Finished,
    /// Answered or seen.
    Earlier,
}

/// One row of the list, with what a client needs to show it and to open what it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub id: String,
    pub kind: NotificationKind,
    pub part: NotificationPart,
    pub project_id: String,
    pub project_name: String,
    /// The workspace it happened in. Only a schedule or a trigger can be without one.
    pub workspace_id: Option<String>,
    /// What that workspace is called, or was: a schedule and a trigger keep the name of one
    /// that is gone. Empty for a schedule that makes its own.
    pub workspace_name: String,
    pub branch: Option<String>,
    /// The agent's conversation, by its record's id.
    pub session_id: Option<String>,
    /// What the row is about, within its workspace: the conversation's label or the start of
    /// its first message, or the name of the workflow, the test, the schedule or the trigger.
    pub title: String,
    /// Which agent: `claude`, `codex`…
    pub harness_id: Option<String>,
    /// The workflow's run, the smoke test's run, the schedule or the trigger, by its id.
    pub subject_id: Option<String>,
    /// The workflow of a row about a workflow's run.
    pub workflow_id: Option<String>,
    /// The step that waits, for a workflow's question or approval.
    pub step_id: Option<String>,
    /// The conversation's terminal, while its process runs.
    pub terminal_id: Option<String>,
    /// When it happened, in epoch milliseconds.
    pub at: i64,
    /// When it was answered or seen.
    pub settled_at: Option<i64>,
    pub worked_ms: Option<i64>,
    pub exit_code: Option<i64>,
    /// The command the agent wants to run, while its question is open. See [`Lines`].
    pub command: Option<String>,
    /// The start of what the agent said last (see [`Lines`]), or of what a workflow asks.
    pub said: Option<String>,
    /// Why, for something that went wrong: a failed run's error, or the reason a schedule or a
    /// trigger could not start.
    pub note: Option<String>,
}

struct Raw {
    id: String,
    kind: String,
    part: String,
    workspace_id: Option<String>,
    project_id: Option<String>,
    session_id: Option<String>,
    subject_id: Option<String>,
    step_id: Option<String>,
    worked_ms: Option<i64>,
    exit_code: Option<i64>,
    created_at: i64,
    settled_at: Option<i64>,
}

/// The conversation a row is about, as far as a rule needs it.
struct Agent {
    session: String,
    workspace: String,
    terminal: Option<String>,
}

fn agent(conn: &rusqlite::Connection, session: &str) -> StoreResult<Option<Agent>> {
    Ok(conn
        .query_row(
            "SELECT id, workspace_id, pty_session_id FROM sessions WHERE id = ?",
            [session],
            |row| {
                Ok(Agent {
                    session: row.get(0)?,
                    workspace: row.get(1)?,
                    terminal: row.get(2)?,
                })
            },
        )
        .optional()?)
}

fn agent_in(conn: &rusqlite::Connection, terminal: &str) -> StoreResult<Option<Agent>> {
    Ok(conn
        .query_row(
            "SELECT id, workspace_id, pty_session_id FROM sessions
             WHERE pty_session_id = ? AND state = 'running'",
            [terminal],
            |row| {
                Ok(Agent {
                    session: row.get(0)?,
                    workspace: row.get(1)?,
                    terminal: row.get(2)?,
                })
            },
        )
        .optional()?)
}

/// The row of `session` still in `part`: its id, when it was made, and the run it named.
fn open_row(
    conn: &rusqlite::Connection,
    session: &str,
    part: &str,
) -> StoreResult<Option<(String, i64, Option<String>)>> {
    Ok(conn
        .query_row(
            "SELECT id, created_at, run_id FROM notifications
             WHERE session_id = ?1 AND part = ?2 AND settled_at IS NULL
             ORDER BY created_at DESC LIMIT 1",
            params![session, part],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?)
}

/// Take every row of `session` out of `part`. True when there was one.
fn settle(conn: &rusqlite::Connection, session: &str, part: &str, at: i64) -> StoreResult<bool> {
    let settled = conn.execute(
        "UPDATE notifications SET settled_at = ?3
         WHERE session_id = ?1 AND part = ?2 AND settled_at IS NULL",
        params![session, part, at],
    )?;
    Ok(settled > 0)
}

/// The agent asked for something. One row per wait: a second request before the first was
/// answered adds nothing.
fn asked(
    conn: &rusqlite::Connection,
    agent: &Agent,
    run: Option<&str>,
    at: i64,
) -> StoreResult<bool> {
    if open_row(conn, &agent.session, "needs")?.is_some() {
        return Ok(false);
    }
    let written = conn.execute(
        "INSERT OR IGNORE INTO notifications
         (id, key, kind, part, workspace_id, session_id, run_id, created_at)
         VALUES (?1, ?2, ?3, 'needs', ?4, ?5, ?6, ?7)",
        params![
            uuid::Uuid::new_v4().to_string(),
            format!("agent:{}:asks:{at}", agent.session),
            NotificationKind::AgentAsks.name(),
            agent.workspace,
            agent.session,
            run,
            at,
        ],
    )?;
    Ok(written > 0)
}

/// The agent's turn ended, well or badly, or its process stopped. A conversation has one row
/// that was not seen yet: finishing again before anyone looked moves that row to now. Whatever
/// it was waiting to be answered is over too.
fn finished(
    conn: &rusqlite::Connection,
    agent: &Agent,
    kind: NotificationKind,
    at: i64,
    worked_ms: Option<i64>,
    exit_code: Option<i64>,
) -> StoreResult<bool> {
    let answered = settle(conn, &agent.session, "needs", at)?;
    // A running workflow reads its own agents' turns and goes on by itself: nobody has to
    // look. Its questions and its end are the rows ([`standing::read`]).
    if kind == NotificationKind::AgentFinished && in_running_workflow(conn, agent)? {
        return Ok(answered);
    }
    if let Some((id, _, _)) = open_row(conn, &agent.session, "finished")? {
        conn.execute(
            "UPDATE notifications
             SET kind = ?2, created_at = ?3, worked_ms = COALESCE(?4, worked_ms), exit_code = ?5
             WHERE id = ?1",
            params![id, kind.name(), at, worked_ms, exit_code],
        )?;
        return Ok(true);
    }
    let written = conn.execute(
        "INSERT OR IGNORE INTO notifications
         (id, key, kind, part, workspace_id, session_id, worked_ms, exit_code, created_at)
         VALUES (?1, ?2, ?3, 'finished', ?4, ?5, ?6, ?7, ?8)",
        params![
            uuid::Uuid::new_v4().to_string(),
            format!("agent:{}:{}:{at}", agent.session, kind.name()),
            kind.name(),
            agent.workspace,
            agent.session,
            worked_ms,
            exit_code,
            at,
        ],
    )?;
    Ok(written > 0)
}

/// Whether a workflow run that is still going started this agent: a `start_session` step left
/// its terminal's id as `session`. Nothing on the conversation itself says so. An agent the
/// run only writes to (`session: origin`) is the person's own, and is not one of these.
fn in_running_workflow(conn: &rusqlite::Connection, agent: &Agent) -> StoreResult<bool> {
    let Some(terminal) = &agent.terminal else {
        return Ok(false);
    };
    Ok(conn.query_row(
        "SELECT EXISTS (
           SELECT 1 FROM workflow_step_runs step
           JOIN workflow_runs run ON run.id = step.run_id
           WHERE run.status = 'running' AND step.action = 'start_session'
             AND json_extract(step.outputs, '$.session') = ?1)",
        [terminal],
        |row| row.get(0),
    )?)
}

/// How long the agent had been at it when its turn ended at `at`: since the last message typed
/// to it, when its hooks said so.
fn worked_since_prompt(
    conn: &rusqlite::Connection,
    session: &str,
    at: i64,
) -> StoreResult<Option<i64>> {
    let prompted: Option<i64> = conn
        .query_row(
            "SELECT MAX(occurred_at) FROM agent_events
             WHERE session_id = ?1 AND kind = 'prompt.submitted' AND occurred_at <= ?2",
            params![session, at],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    Ok(prompted.map(|from| (at - from).max(0)))
}

/// Take in the agents' events since the last look: what they asked for, what answered it, and
/// the turns their harnesses reported. `turns` is the caller's fold of those reports with the
/// host's quiets, so one turn is one row. True when the list changed.
///
/// The first look starts at the newest event: what happened before there was a list is not
/// news.
pub fn read_events(store: &Store, turns: &Turns) -> StoreResult<bool> {
    let Some(mut after) = store.trigger_cursor(CURSOR)? else {
        let newest = store.newest_event_seq()?.unwrap_or(0);
        store.set_trigger_cursor(CURSOR, newest)?;
        return Ok(false);
    };
    let kinds: Vec<&str> = ["approval.requested"]
        .into_iter()
        .chain(RUN_ANSWERED)
        .chain(REQUEST_ANSWERED)
        .collect();
    let mut changed = false;
    loop {
        let rows = store.events_after(after, &kinds, BATCH)?;
        let conn = store.conn();
        for row in &rows {
            let Some(agent) = row
                .session_id
                .as_deref()
                .map(|id| agent(&conn, id))
                .transpose()?
                .flatten()
            else {
                continue; // A shell, or a conversation that is gone.
            };
            let kind = row.kind.as_str();
            if kind == "approval.requested" {
                changed |= asked(&conn, &agent, row.run_id.as_deref(), row.occurred_at)?;
                continue;
            }
            if RUN_ANSWERED.contains(&kind) {
                changed |= settle(&conn, &agent.session, "needs", row.occurred_at)?;
            } else if let Some((_, since, run)) = open_row(&conn, &agent.session, "needs")? {
                let answered = match run {
                    Some(run) => crate::notify::wait_answered(&conn, &run, since)?,
                    None => true,
                };
                if answered {
                    changed |= settle(&conn, &agent.session, "needs", row.occurred_at)?;
                }
            }
            match kind {
                // Someone typed to it, here or from a phone: they have seen where it stood.
                "prompt.submitted" => {
                    changed |= settle(&conn, &agent.session, "finished", row.occurred_at)?;
                }
                "turn.completed" | "turn.failed" => {
                    let failed = kind == "turn.failed";
                    // A quiet terminal may have told this turn's end already. A failure is
                    // still worth saying over the row it made.
                    let first = agent
                        .terminal
                        .as_deref()
                        .is_none_or(|terminal| turns.reported(terminal, row.received_at));
                    if first || failed {
                        let worked = worked_since_prompt(&conn, &agent.session, row.occurred_at)?;
                        let what = if failed {
                            NotificationKind::AgentFailed
                        } else {
                            NotificationKind::AgentFinished
                        };
                        changed |= finished(&conn, &agent, what, row.occurred_at, worked, None)?;
                    }
                }
                _ => {}
            }
        }
        drop(conn);
        let Some(last) = rows.last() else {
            break;
        };
        after = last.seq;
        store.set_trigger_cursor(CURSOR, after)?;
        if rows.len() < BATCH {
            break;
        }
    }
    Ok(changed)
}

/// The agent in `terminal` went quiet after `busy_ms` of work, and the caller's [`Turns`] says
/// this is the first word of that turn's end. True when the list changed. A shell is not an
/// agent, and adds nothing.
pub fn settled_terminal(
    store: &Store,
    terminal: &str,
    busy_ms: u32,
    now: i64,
) -> StoreResult<bool> {
    let conn = store.conn();
    let Some(agent) = agent_in(&conn, terminal)? else {
        return Ok(false);
    };
    let worked = worked_since_prompt(&conn, &agent.session, now)?.or(Some(i64::from(busy_ms)));
    finished(
        &conn,
        &agent,
        NotificationKind::AgentFinished,
        now,
        worked,
        None,
    )
}

/// The list is hearing from the terminals at `now`. Call it every few seconds while it is,
/// and [`catch_up`] knows from when nothing was.
pub fn heard(store: &Store, now: i64) -> StoreResult<()> {
    store.set_trigger_cursor(HEARD, now)
}

/// A terminal that is alive when the list starts listening: its id, and how long ago it last
/// printed. Only one that has printed something and is quiet now is worth giving.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    pub terminal: String,
    pub idle_ms: u32,
}

/// The list starts listening at `now`, with these agents' terminals alive and quiet. One that
/// last printed after anything was last heard did so while Yardsort was closed: it finished
/// something nobody saw, at the time it stopped printing. True when the list changed.
///
/// The very first time there is nothing to compare with, and nothing is news.
pub fn catch_up(store: &Store, waiting: &[Waiting], now: i64) -> StoreResult<bool> {
    let last = store.trigger_cursor(HEARD)?;
    heard(store, now)?;
    let Some(last) = last else {
        return Ok(false);
    };
    let conn = store.conn();
    let mut changed = false;
    for terminal in waiting {
        let stopped = now - i64::from(terminal.idle_ms);
        if stopped <= last + HEARD_SLACK_MS {
            continue; // It was quiet already when the list last listened.
        }
        let Some(agent) = agent_in(&conn, &terminal.terminal)? else {
            continue;
        };
        // Its harness may have reported the turn itself, in a row made from its event.
        let told: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM notifications
                            WHERE session_id = ?1 AND part = 'finished' AND created_at >= ?2)",
            params![agent.session, stopped - QUIET_LAG_MS],
            |row| row.get(0),
        )?;
        if !told {
            let worked = worked_since_prompt(&conn, &agent.session, stopped)?;
            changed |= finished(
                &conn,
                &agent,
                NotificationKind::AgentFinished,
                stopped,
                worked,
                None,
            )?;
        }
    }
    Ok(changed)
}

/// The process of conversation `session` exited. One that ended with an error is worth
/// telling; one that was closed, or left with `0`, is not. Either way nothing waits on it any
/// more. True when the list changed.
pub fn exited(store: &Store, session: &str, exit_code: Option<i64>, now: i64) -> StoreResult<bool> {
    let conn = store.conn();
    let Some(agent) = agent(&conn, session)? else {
        return Ok(false);
    };
    match exit_code {
        Some(code) if code != 0 => finished(
            &conn,
            &agent,
            NotificationKind::AgentStopped,
            now,
            None,
            Some(code),
        ),
        _ => settle(&conn, &agent.session, "needs", now),
    }
}

/// These rows were looked at. Only a finished row can be: one that needs an answer leaves when
/// it gets one. How many left.
pub fn seen(store: &Store, ids: &[String], now: i64) -> StoreResult<usize> {
    let conn = store.conn();
    let mut left = 0;
    for id in ids {
        left += conn.execute(
            "UPDATE notifications SET settled_at = ?2
             WHERE id = ?1 AND part = 'finished' AND settled_at IS NULL",
            params![id, now],
        )?;
    }
    Ok(left)
}

/// The terminal `terminal` is in front of someone: what its agent finished is seen.
pub fn seen_terminal(store: &Store, terminal: &str, now: i64) -> StoreResult<usize> {
    Ok(store.conn().execute(
        "UPDATE notifications SET settled_at = ?2
         WHERE part = 'finished' AND settled_at IS NULL
           AND session_id IN (SELECT id FROM sessions WHERE pty_session_id = ?1)",
        params![terminal, now],
    )?)
}

/// Everything finished is seen.
pub fn seen_all(store: &Store, now: i64) -> StoreResult<usize> {
    Ok(store.conn().execute(
        "UPDATE notifications SET settled_at = ?1 WHERE part = 'finished' AND settled_at IS NULL",
        [now],
    )?)
}

/// The list: everything still in its part, newest first, then the newest of Earlier. Rows that
/// left more than a week ago are deleted on the way, and a question whose agent is gone has
/// nobody to answer it and leaves.
///
/// `command` and `said` are left empty: see [`Lines`].
pub fn list(store: &Store, now: i64) -> StoreResult<Vec<Notification>> {
    let raws = {
        let conn = store.conn();
        conn.execute(
            "DELETE FROM notifications WHERE settled_at IS NOT NULL AND settled_at < ?",
            [now.saturating_sub(KEPT_MS)],
        )?;
        conn.execute(
            "UPDATE notifications SET settled_at = ?1
             WHERE part = 'needs' AND settled_at IS NULL AND session_id IS NOT NULL
               AND session_id NOT IN (SELECT id FROM sessions WHERE state = 'running')",
            [now],
        )?;
        let mut raws = Vec::new();
        for (sql, limit) in [
            (
                "SELECT id, kind, part, workspace_id, project_id, session_id, subject_id, step_id,
                        worked_ms, exit_code, created_at, settled_at
                 FROM notifications WHERE settled_at IS NULL
                 ORDER BY created_at DESC, id LIMIT ?",
                i64::MAX,
            ),
            (
                "SELECT id, kind, part, workspace_id, project_id, session_id, subject_id, step_id,
                        worked_ms, exit_code, created_at, settled_at
                 FROM notifications WHERE settled_at IS NOT NULL
                 ORDER BY settled_at DESC, id LIMIT ?",
                EARLIER_SHOWN as i64,
            ),
        ] {
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map([limit], |row| {
                Ok(Raw {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    part: row.get(2)?,
                    workspace_id: row.get(3)?,
                    project_id: row.get(4)?,
                    session_id: row.get(5)?,
                    subject_id: row.get(6)?,
                    step_id: row.get(7)?,
                    worked_ms: row.get(8)?,
                    exit_code: row.get(9)?,
                    created_at: row.get(10)?,
                    settled_at: row.get(11)?,
                })
            })?;
            for row in rows {
                raws.push(row?);
            }
        }
        raws
    };
    let mut out = Vec::with_capacity(raws.len());
    for raw in raws {
        if let Some(row) = shown(store, raw)? {
            out.push(row);
        }
    }
    Ok(out)
}

/// A stored row with what a client shows it with. Nothing when what it is about is gone, or
/// when a later Yardsort wrote it.
fn shown(store: &Store, raw: Raw) -> StoreResult<Option<Notification>> {
    let Some(kind) = NotificationKind::named(&raw.kind) else {
        return Ok(None);
    };
    let workspace = match &raw.workspace_id {
        Some(id) => match store.workspace(id)? {
            Some(workspace) if !workspace.forgotten => Some(workspace),
            _ => return Ok(None),
        },
        None => None,
    };
    let project = match workspace
        .as_ref()
        .map(|workspace| workspace.project_id.clone())
        .or(raw.project_id)
    {
        Some(id) => store.project(&id)?,
        None => None,
    };
    let Some(project) = project else {
        return Ok(None);
    };
    let part = match (raw.settled_at, raw.part.as_str()) {
        (Some(_), _) => NotificationPart::Earlier,
        (None, "needs") => NotificationPart::Needs,
        (None, _) => NotificationPart::Finished,
    };
    let mut row = Notification {
        id: raw.id,
        kind,
        part,
        project_id: project.id,
        project_name: project.name,
        workspace_id: workspace.as_ref().map(|workspace| workspace.id.clone()),
        workspace_name: workspace
            .as_ref()
            .map_or_else(String::new, |workspace| workspace.name.clone()),
        branch: workspace.and_then(|workspace| workspace.branch),
        session_id: raw.session_id,
        title: String::new(),
        harness_id: None,
        subject_id: raw.subject_id,
        workflow_id: None,
        step_id: raw.step_id,
        terminal_id: None,
        at: raw.created_at,
        settled_at: raw.settled_at,
        worked_ms: raw.worked_ms,
        exit_code: raw.exit_code,
        command: None,
        said: None,
        note: None,
    };
    let subject = row.subject_id.clone().unwrap_or_default();
    match kind.about() {
        About::Agent => {
            let session = match &row.session_id {
                Some(id) => store.session(id)?,
                None => None,
            };
            if let Some(session) = session {
                row.title = if session.label.is_empty() {
                    session.title
                } else {
                    session.label
                };
                row.harness_id = Some(session.harness_id);
                row.terminal_id = session.pty_session_id.filter(|_| session.running);
            }
        }
        About::Workflow => {
            let Some(run) = store.workflow_run(&subject)? else {
                return Ok(None);
            };
            row.title = run.workflow_name;
            row.workflow_id = Some(run.workflow_id);
            match (kind, &row.step_id) {
                (NotificationKind::WorkflowFailed, _) => {
                    // The run's own error, or else the note of the step that failed.
                    let why = match run.error {
                        Some(error) => Some(error),
                        None => store
                            .workflow_steps(&subject)?
                            .into_iter()
                            .find(|step| step.status == "failed")
                            .and_then(|step| step.note),
                    };
                    row.note = why.map(|why| one_line(&why)).filter(|why| !why.is_empty());
                }
                (
                    NotificationKind::WorkflowApproval | NotificationKind::WorkflowQuestion,
                    Some(step),
                ) => {
                    row.said = store
                        .workflow_wait(&subject, step)?
                        .and_then(|wait| wait.prompt)
                        .map(|prompt| one_line(&prompt))
                        .filter(|prompt| !prompt.is_empty());
                }
                _ => {}
            }
        }
        About::Test => {
            let Some(run) = store.test_run(&subject)? else {
                return Ok(None);
            };
            row.title = run.test_name;
        }
        About::Schedule => {
            let Some(schedule) = store.schedule(&subject)? else {
                return Ok(None);
            };
            row.title = schedule.name;
            if row.workspace_id.is_none() {
                row.workspace_name = schedule.workspace_name;
            }
            // Why it could not start is read from its newest time, which says so only while
            // nothing has happened since.
            if kind == NotificationKind::ScheduleRefused && part == NotificationPart::Needs {
                row.note = store
                    .schedule_runs(&subject, 1)?
                    .into_iter()
                    .next()
                    .filter(|line| line.outcome == "refused")
                    .and_then(|line| line.note)
                    .map(|why| one_line(&why));
            }
        }
        About::Trigger => {
            let Some(trigger) = store.trigger(&subject)? else {
                return Ok(None);
            };
            row.title = trigger.name;
            if row.workspace_id.is_none() {
                row.workspace_name = trigger.workspace_name;
            }
            if kind != NotificationKind::TriggerNoWorkspace && part == NotificationPart::Needs {
                row.note = store
                    .trigger_fires(&subject, standing::FIRES)?
                    .into_iter()
                    .find(|fire| fire.outcome != "gathering")
                    .filter(|fire| matches!(fire.outcome.as_str(), "refused" | "capped"))
                    .and_then(|fire| fire.note)
                    .map(|why| one_line(&why));
            }
        }
    }
    Ok(Some(row))
}

/// The one line of substance a row is shown with, read when the list is asked for and never
/// stored: the command an agent wants to run, from its open question, and the start of what it
/// said last, from its own session file.
///
/// A session file can be large and the list is read often, so what an agent said is read once
/// for each time it finished and remembered for as long as this process runs.
#[derive(Debug, Default)]
pub struct Lines {
    /// By row id: when the row was made, and what was read then.
    said: Mutex<HashMap<String, (i64, Option<String>)>>,
}

impl Lines {
    /// Fill `command` and `said` on `rows`.
    pub fn fill(
        &self,
        store: &Store,
        sources: &Sources,
        data_dir: &Path,
        rows: &mut [Notification],
        now: i64,
    ) {
        let asks_dir = asks::asks_dir(data_dir);
        let mut said = self.said.lock().unwrap_or_else(PoisonError::into_inner);
        // What a row that is gone said is not needed again.
        said.retain(|id, _| rows.iter().any(|row| row.id == *id));
        for row in rows.iter_mut() {
            let Some(session) = row.session_id.clone() else {
                continue;
            };
            match (row.part, row.kind) {
                (NotificationPart::Needs, NotificationKind::AgentAsks) => {
                    row.command = asks::open(&asks_dir, &session, now)
                        .into_iter()
                        .next()
                        .and_then(|ask| ask.command.or(ask.path).or(Some(ask.tool)))
                        .map(|line| one_line(&line));
                }
                (
                    NotificationPart::Finished,
                    NotificationKind::AgentFinished | NotificationKind::AgentFailed,
                ) => {
                    let fresh = said.get(&row.id).is_some_and(|(at, _)| *at == row.at);
                    if !fresh {
                        let read = row.workspace_id.as_deref().and_then(|workspace| {
                            last_said(store, sources, workspace, &session, now)
                        });
                        said.insert(row.id.clone(), (row.at, read));
                    }
                    row.said = said.get(&row.id).and_then(|(_, line)| line.clone());
                }
                // What was read while it was unseen is still worth showing; nothing is read
                // again for a row that left.
                (
                    NotificationPart::Earlier,
                    NotificationKind::AgentFinished | NotificationKind::AgentFailed,
                ) => {
                    row.said = said.get(&row.id).and_then(|(_, line)| line.clone());
                }
                _ => {}
            }
        }
    }
}

fn last_said(
    store: &Store,
    sources: &Sources,
    workspace: &str,
    session: &str,
    now: i64,
) -> Option<String> {
    let workspace = store.workspace(workspace).ok().flatten()?;
    let conversation = digest::one(store, &workspace, session, sources, now)
        .ok()
        .flatten()?;
    conversation
        .turns
        .last()
        .and_then(|turn| turn.reply())
        .map(one_line)
        .filter(|line| !line.is_empty())
}

/// The first line of `text` with anything on it, cut to what a row has room for.
fn one_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if line.chars().count() <= SAID_CHARS {
        return line.to_owned();
    }
    let mut cut: String = line.chars().take(SAID_CHARS).collect();
    cut.push('…');
    cut
}

/// How often, at most, the store is told the list is still hearing from the terminals.
const HEARD_EVERY: Duration = Duration::from_secs(5);

/// What a process that runs the terminals keeps, to keep the list: the app, or a host with no
/// window. It is told what only that process hears — a terminal going quiet, an agent's process
/// ending, the activity inbox drained — and it says when the list changed, to whoever it was
/// made for.
///
/// Nothing here fails its caller: the list is not worth stopping a terminal's event for. What
/// the store would not do is written to the log.
pub struct Keeper {
    /// Each terminal's turns, so a quiet terminal and its harness's own report of the same turn
    /// are one row.
    turns: Turns,
    /// What rows are shown with, read when the list is asked for.
    lines: Lines,
    /// When the store was last told the list is hearing from the terminals.
    heard: Mutex<Option<Instant>>,
    /// Terminals that were asked to close. An agent ended that way did not stop with an error,
    /// whatever its process says on the way out.
    closing: Mutex<HashSet<String>>,
    /// When [`Self::look`] last ran.
    looked: Mutex<Option<Instant>>,
    /// Called when the list changed: a row was added, answered or seen.
    changed: Box<dyn Fn() + Send + Sync>,
}

impl std::fmt::Debug for Keeper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keeper").finish_non_exhaustive()
    }
}

impl Keeper {
    /// A keeper that calls `changed` whenever the list changed.
    pub fn new(changed: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            turns: Turns::default(),
            lines: Lines::default(),
            heard: Mutex::new(None),
            closing: Mutex::new(HashSet::new()),
            looked: Mutex::new(None),
            changed: Box::new(changed),
        }
    }

    /// The process has just started and found these terminals alive. What agents' hooks
    /// reported meanwhile is read first, so a turn told that way is not told again; then an
    /// agent that stopped printing while nothing listened finished something nobody saw; then
    /// what else waits on this computer.
    pub fn start(&self, store: &Store, alive: &[SessionInfo]) {
        self.read_events(store);
        let waiting: Vec<Waiting> = alive
            .iter()
            .filter(|session| {
                session.has_output
                    && !session.busy
                    && matches!(session.state, SessionState::Running)
            })
            .map(|session| Waiting {
                terminal: session.id.0.clone(),
                idle_ms: session.idle_ms,
            })
            .collect();
        self.tell(catch_up(store, &waiting, crate::store::now_ms()));
        self.look(store);
    }

    /// Every event of the terminals passes through here, after an exit's record was settled.
    /// `ended` is the conversation whose process an exit ended, when it was an agent's.
    pub fn observe(&self, store: &Store, event: &HostEvent, ended: Option<&str>) {
        let now = crate::store::now_ms();
        self.hearing(store, now);
        let settled = matches!(self.turns.observe(event, now), Some(Change::Settled { .. }));
        let changed = match event {
            HostEvent::Quiet { id, busy_ms } if settled => {
                settled_terminal(store, &id.0, *busy_ms, now)
            }
            HostEvent::Exited { id, exit } => {
                let closed = self
                    .closing
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&id.0);
                let Some(session) = ended else {
                    return;
                };
                // A signal is a person, or the program that runs it, ending it: not a failure
                // to tell of.
                let error = (!exit.success && exit.signal.is_none() && !closed)
                    .then(|| i64::from(exit.code));
                exited(store, session, error, now)
            }
            _ => return,
        };
        self.tell(changed);
    }

    /// Say, every few seconds at most, that the list is hearing from the terminals: what
    /// prints after the last such word printed while nothing listened.
    fn hearing(&self, store: &Store, now: i64) {
        let mut last = self.heard.lock().unwrap_or_else(PoisonError::into_inner);
        if last.is_some_and(|at| at.elapsed() < HEARD_EVERY) {
            return;
        }
        *last = Some(Instant::now());
        if let Err(error) = heard(store, now) {
            eprintln!("notifications: {error}");
        }
    }

    /// `terminal` is being closed on purpose.
    pub fn closing(&self, terminal: &str) {
        self.closing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(terminal.to_owned());
    }

    /// Take in what agents' hooks reported since the last look. Call it after every drain of
    /// the activity inbox.
    pub fn read_events(&self, store: &Store) {
        self.tell(read_events(store, &self.turns));
    }

    /// A workflow's run, a smoke test's run, a schedule or a trigger changed, here or from
    /// `ys`: have the list agree with where they stand now ([`standing::read`]).
    pub fn look(&self, store: &Store) {
        *self.looked.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
        match standing::read(store, crate::store::now_ms()) {
            Ok(changed) => self.tell(Ok(changed)),
            Err(error) => eprintln!("notifications: {}", error.message),
        }
    }

    /// [`Self::look`], unless it ran within `every`: for a caller that asks often and is told
    /// nothing, as a host with no window is.
    pub fn look_within(&self, store: &Store, every: Duration) {
        let recent = self
            .looked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some_and(|at| at.elapsed() < every);
        if !recent {
            self.look(store);
        }
    }

    /// The list as it stands now, with the lines its rows are shown with. `conversations` is
    /// where agents keep theirs: the sources of their session files, and this profile's data
    /// folder. Without it rows come without those lines.
    pub fn list(
        &self,
        store: &Store,
        conversations: Option<(&Sources, &Path)>,
        now: i64,
    ) -> IpcResult<Vec<Notification>> {
        // Whoever asks is shown where things stand now, whether or not anyone said they moved.
        standing::read(store, now)?;
        let mut rows = list(store, now)?;
        if let Some((sources, data_dir)) = conversations {
            self.lines.fill(store, sources, data_dir, &mut rows, now);
        }
        Ok(rows)
    }

    /// These finished rows were looked at.
    pub fn seen(&self, store: &Store, ids: &[String]) -> StoreResult<()> {
        let left = seen(store, ids, crate::store::now_ms())?;
        self.tell(Ok(left > 0));
        Ok(())
    }

    /// `terminal` is in front of its person: what its agent finished is seen.
    pub fn seen_terminal(&self, store: &Store, terminal: &str) -> StoreResult<()> {
        let left = seen_terminal(store, terminal, crate::store::now_ms())?;
        self.tell(Ok(left > 0));
        Ok(())
    }

    /// Everything finished is seen.
    pub fn seen_all(&self, store: &Store) -> StoreResult<()> {
        let left = seen_all(store, crate::store::now_ms())?;
        self.tell(Ok(left > 0));
        Ok(())
    }

    /// A workspace or a project went away, and its rows with it; a schedule or a trigger that
    /// ran there now has nowhere to. Whoever shows the list reads it again.
    pub fn removed(&self) {
        (self.changed)();
    }

    fn tell(&self, changed: StoreResult<bool>) {
        match changed {
            Ok(true) => (self.changed)(),
            Ok(false) => {}
            Err(error) => eprintln!("notifications: {error}"),
        }
    }
}

/// What tells one state of the list from another: enough that any row added, answered, seen or
/// deleted changes it. For telling a device that follows the list that it changed.
pub fn stamp(store: &Store) -> StoreResult<(i64, i64, i64, i64)> {
    Ok(store.conn().query_row(
        "SELECT count(*), count(settled_at), COALESCE(max(created_at), 0),
                COALESCE(max(settled_at), 0)
         FROM notifications",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?)
}

pub mod standing;

#[cfg(test)]
mod tests;
