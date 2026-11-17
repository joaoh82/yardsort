//! The Notifications list: what waits for a person, and what finished while they were not
//! looking. See `docs/design/46-notifications.md`.
//!
//! One row is one thing that happened to an agent's conversation. It is written here when the
//! thing happens, and it leaves its part of the list by itself:
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
//! What has left is kept for a week ([`KEPT_MS`]) and then deleted.
//!
//! Nothing is read from a terminal: a quiet terminal says only that a turn ended. The words a
//! row is shown with are not stored either; [`Lines`] reads them when the list is asked for.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::activity::{asks, digest};
use crate::store::{Store, StoreResult};
use crate::trigger::turns::Turns;
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
}

impl NotificationKind {
    fn name(self) -> &'static str {
        match self {
            NotificationKind::AgentAsks => "agent.asks",
            NotificationKind::AgentFinished => "agent.finished",
            NotificationKind::AgentFailed => "agent.failed",
            NotificationKind::AgentStopped => "agent.stopped",
        }
    }

    fn named(name: &str) -> Option<Self> {
        [
            NotificationKind::AgentAsks,
            NotificationKind::AgentFinished,
            NotificationKind::AgentFailed,
            NotificationKind::AgentStopped,
        ]
        .into_iter()
        .find(|kind| kind.name() == name)
    }
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
    pub workspace_id: String,
    pub workspace_name: String,
    pub branch: Option<String>,
    /// The agent's conversation, by its record's id.
    pub session_id: Option<String>,
    /// What the conversation is called: its label, or the start of its first message.
    pub session_title: String,
    /// Which agent: `claude`, `codex`…
    pub harness_id: Option<String>,
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
    /// The start of what the agent said last. See [`Lines`].
    pub said: Option<String>,
}

struct Raw {
    id: String,
    kind: String,
    part: String,
    workspace_id: String,
    session_id: Option<String>,
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
    settle(conn, &agent.session, "needs", at)?;
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
                "SELECT id, kind, part, workspace_id, session_id, worked_ms, exit_code, created_at,
                        settled_at
                 FROM notifications WHERE settled_at IS NULL
                 ORDER BY created_at DESC, id LIMIT ?",
                i64::MAX,
            ),
            (
                "SELECT id, kind, part, workspace_id, session_id, worked_ms, exit_code, created_at,
                        settled_at
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
                    session_id: row.get(4)?,
                    worked_ms: row.get(5)?,
                    exit_code: row.get(6)?,
                    created_at: row.get(7)?,
                    settled_at: row.get(8)?,
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
        let Some(kind) = NotificationKind::named(&raw.kind) else {
            continue; // Written by a later Yardsort.
        };
        let Some(workspace) = store.workspace(&raw.workspace_id)? else {
            continue;
        };
        if workspace.forgotten {
            continue;
        }
        let Some(project) = store.project(&workspace.project_id)? else {
            continue;
        };
        let session = match &raw.session_id {
            Some(id) => store.session(id)?,
            None => None,
        };
        let part = match (raw.settled_at, raw.part.as_str()) {
            (Some(_), _) => NotificationPart::Earlier,
            (None, "needs") => NotificationPart::Needs,
            (None, _) => NotificationPart::Finished,
        };
        out.push(Notification {
            id: raw.id,
            kind,
            part,
            project_id: project.id,
            project_name: project.name,
            workspace_id: workspace.id,
            workspace_name: workspace.name,
            branch: workspace.branch,
            session_title: session.as_ref().map_or_else(String::new, |session| {
                if session.label.is_empty() {
                    session.title.clone()
                } else {
                    session.label.clone()
                }
            }),
            harness_id: session.as_ref().map(|session| session.harness_id.clone()),
            terminal_id: session
                .as_ref()
                .filter(|session| session.running)
                .and_then(|session| session.pty_session_id.clone()),
            session_id: raw.session_id,
            at: raw.created_at,
            settled_at: raw.settled_at,
            worked_ms: raw.worked_ms,
            exit_code: raw.exit_code,
            command: None,
            said: None,
        });
    }
    Ok(out)
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
                        let read = last_said(store, sources, &row.workspace_id, &session, now);
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

#[cfg(test)]
mod tests;
