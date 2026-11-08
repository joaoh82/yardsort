//! Where triggers hear of what happened: rows other parts of Yardsort already write, read from
//! where the trigger thread last stopped, and the host's own activity, which only the app hears.
//!
//! - **`agent_events`**: `turn.completed` (a harness reported a finished turn) and
//!   `process.exited`, by sequence number.
//! - **`done_signals`**: `ys workspace done`, by sequence number.
//! - **`custom_events`**: `ys event emit`, by sequence number.
//! - **Run endings**: workflow and test runs that ended, by time. Read a second behind, so a run
//!   another process ends in the same millisecond as the read is not stepped over.
//! - **The host**: a quiet after real work, and a session starting to work again
//!   ([`settled_terminal`], [`busy_terminal`]).
//!
//! The first time a cursor is read it starts at the newest row: what happened before Yardsort
//! had triggers is never news. Custom events are the exception: every row was written by
//! `ys event emit` for a trigger to hear, so they are read from the first, even one raised before
//! the app ever opened. A trigger saved after one still ignores it, by its `since`.

use std::collections::BTreeMap;
use std::path::Path;

use super::fire::Observation;
use super::turns::Turns;
use super::TriggerEvent;
use crate::error::IpcResult;
use crate::git::Git;
use crate::store::{RunRow, Store};

/// How many rows of a source one look takes. The rest are for the next.
const BATCH: usize = 500;

/// How far behind the clock run endings are read.
const ENDINGS_LAG_MS: i64 = 1_000;

const EVENTS: &str = "agent_events";
const DONE: &str = "done_signals";
const CUSTOM: &str = "custom_events";
const ENDINGS: &str = "run_endings";

/// Where `source` was read up to, or `None` the first time, when the cursor is put at `newest`
/// and there is nothing to read yet.
fn cursor(store: &Store, source: &str, newest: i64) -> IpcResult<Option<i64>> {
    match store.trigger_cursor(source)? {
        Some(position) => Ok(Some(position)),
        None => {
            store.set_trigger_cursor(source, newest)?;
            Ok(None)
        }
    }
}

/// An agent's events since the last look: finished turns, folded with the host's quiets by
/// `turns`, and exits.
pub fn read_events(store: &Store, turns: &Turns, git: &Git) -> IpcResult<Vec<Observation>> {
    let newest = store.newest_event_seq()?.unwrap_or(0);
    let Some(after) = cursor(store, EVENTS, newest)? else {
        return Ok(Vec::new());
    };
    let rows = store.events_after(after, &["turn.completed", "process.exited"], BATCH)?;
    let mut seen = Vec::new();
    for row in &rows {
        let Some(run) = row
            .run_id
            .as_deref()
            .map(|id| store.run(id))
            .transpose()?
            .flatten()
        else {
            continue;
        };
        if run.kind != "harness" {
            continue; // A shell is not an agent.
        }
        match row.kind.as_str() {
            "turn.completed" => {
                let Some(terminal) = run.pty_session_id.as_deref() else {
                    continue;
                };
                if turns.reported(terminal, row.received_at) {
                    seen.push(settled(store, git, &run, row.received_at)?);
                }
            }
            "process.exited" => seen.push(exited(&run, &row.payload, row.occurred_at)),
            _ => {}
        }
    }
    if let Some(last) = rows.last() {
        store.set_trigger_cursor(EVENTS, last.seq)?;
    }
    Ok(seen)
}

/// `ys workspace done` since the last look.
pub fn read_done(store: &Store) -> IpcResult<Vec<Observation>> {
    let newest = store.newest_done_signal()?;
    let Some(after) = cursor(store, DONE, newest)? else {
        return Ok(Vec::new());
    };
    let rows = store.done_signals_after(after, BATCH)?;
    let seen = rows
        .iter()
        .map(|row| {
            let mut facts = BTreeMap::new();
            if let Some(head) = &row.head {
                facts.insert("head".to_owned(), head.clone());
            }
            if let Some(note) = &row.note {
                facts.insert("note".to_owned(), note.clone());
            }
            Observation {
                event: TriggerEvent::WorkspaceDone,
                workspace_id: row.workspace_id.clone(),
                session_id: row.session_id.clone(),
                terminal: None,
                agent_run: None,
                at: row.at,
                facts,
                depth: 0,
                target_id: None,
                status: None,
                post: None,
            }
        })
        .collect();
    if let Some(last) = rows.last() {
        store.set_trigger_cursor(DONE, last.seq)?;
    }
    Ok(seen)
}

/// `ys event emit` since the last look. One raised by a smoke test run's `run` step is one
/// deeper than that run, so a chain of tests raising each other's events stops.
pub fn read_custom(store: &Store) -> IpcResult<Vec<Observation>> {
    // From the first row, not the newest: see the module's note.
    let after = store.trigger_cursor(CUSTOM)?.unwrap_or(0);
    let rows = store.custom_events_after(after, BATCH)?;
    let mut seen = Vec::new();
    for row in &rows {
        let mut facts = BTreeMap::new();
        facts.insert("name".to_owned(), row.name.clone());
        if let Some(head) = &row.head {
            facts.insert("head".to_owned(), head.clone());
        }
        if let Some(note) = &row.note {
            facts.insert("note".to_owned(), note.clone());
        }
        let depth = match row.run_id.as_deref() {
            Some(run) if store.test_run(run)?.is_some() => run_depth(store, "test", run)? + 1,
            _ => 0,
        };
        seen.push(Observation {
            event: TriggerEvent::Custom,
            workspace_id: row.workspace_id.clone(),
            session_id: row.session_id.clone(),
            terminal: None,
            agent_run: None,
            at: row.at,
            facts,
            depth,
            target_id: None,
            status: None,
            post: None,
        });
    }
    if let Some(last) = rows.last() {
        store.set_trigger_cursor(CUSTOM, last.seq)?;
    }
    Ok(seen)
}

/// Workflow and test runs that ended since the last look, up to a second before `now`.
pub fn read_endings(store: &Store, now: i64) -> IpcResult<Vec<Observation>> {
    let until = now - ENDINGS_LAG_MS;
    let Some(after) = cursor(store, ENDINGS, until)? else {
        return Ok(Vec::new());
    };
    if until <= after {
        return Ok(Vec::new());
    }
    let ended = store.runs_ended_between(after, until)?;
    let mut seen = Vec::new();
    for run in &ended {
        let (event, facts) = match run.kind {
            "workflow" => {
                let mut facts = BTreeMap::new();
                facts.insert("summary".to_owned(), workflow_summary(store, &run.id)?);
                (TriggerEvent::WorkflowFinished, facts)
            }
            _ => {
                let mut facts = BTreeMap::new();
                facts.insert("verdict".to_owned(), run.status.clone());
                if let Some(row) = store.test_run(&run.id)? {
                    let steps = store.test_steps(&run.id)?;
                    facts.insert(
                        "summary".to_owned(),
                        crate::smoke::report::summary(&row, &steps),
                    );
                }
                (TriggerEvent::TestFinished, facts)
            }
        };
        seen.push(Observation {
            event,
            workspace_id: run.workspace_id.clone(),
            session_id: None,
            terminal: None,
            agent_run: None,
            at: run.ended_at,
            facts,
            depth: run_depth(store, run.kind, &run.id)? + 1,
            target_id: Some(run.target_id.clone()),
            status: Some(run.status.clone()),
            post: None,
        });
    }
    store.set_trigger_cursor(ENDINGS, until)?;
    Ok(seen)
}

/// The host heard a terminal go quiet after real work, and `turns` says it is the first word
/// of that turn's end: an agent settled, if the terminal is an agent's.
pub fn settled_terminal(
    store: &Store,
    git: &Git,
    terminal: &str,
    at: i64,
) -> IpcResult<Option<Observation>> {
    match store.run_by_pty(terminal)? {
        Some(run) if run.kind == "harness" => Ok(Some(settled(store, git, &run, at)?)),
        _ => Ok(None),
    }
}

/// The host heard a terminal start working again: an agent's settling, still gathering, is
/// off. Returns how many fires went.
pub fn busy_terminal(store: &Store, terminal: &str) -> IpcResult<usize> {
    match store.run_by_pty(terminal)?.and_then(|run| run.session_id) {
        Some(session_id) => super::fire::went_busy(store, &session_id),
        None => Ok(0),
    }
}

fn settled(store: &Store, git: &Git, run: &RunRow, at: i64) -> IpcResult<Observation> {
    let mut facts = BTreeMap::new();
    if let Some(workspace) = store.workspace(&run.workspace_id)? {
        if let Ok(Some(head)) = git.commit_id(Path::new(&workspace.path), "HEAD") {
            facts.insert("head".to_owned(), head);
        }
    }
    Ok(Observation {
        event: TriggerEvent::AgentSettled,
        workspace_id: run.workspace_id.clone(),
        session_id: run.session_id.clone(),
        terminal: run.pty_session_id.clone(),
        agent_run: Some(run.id.clone()),
        at,
        facts,
        depth: 0,
        target_id: None,
        status: None,
        post: None,
    })
}

fn exited(run: &RunRow, payload: &str, at: i64) -> Observation {
    let payload: serde_json::Value = serde_json::from_str(payload).unwrap_or_default();
    let code = payload.get("exitCode").and_then(serde_json::Value::as_i64);
    let success = payload.get("success").and_then(serde_json::Value::as_bool);
    let reason = payload.get("reason").and_then(serde_json::Value::as_str);
    let summary = match (reason, code, success) {
        (Some("interrupted"), _, _) => "It ended without Yardsort hearing how.".to_owned(),
        (_, _, Some(true)) => "It exited normally.".to_owned(),
        (_, Some(code), _) => format!("It exited with code {code}."),
        _ => "It was stopped.".to_owned(),
    };
    let mut facts = BTreeMap::new();
    facts.insert("summary".to_owned(), summary);
    Observation {
        event: TriggerEvent::AgentExited,
        workspace_id: run.workspace_id.clone(),
        session_id: run.session_id.clone(),
        terminal: run.pty_session_id.clone(),
        agent_run: Some(run.id.clone()),
        at,
        facts,
        depth: 0,
        target_id: None,
        status: Some(
            if success == Some(true) {
                "exited"
            } else {
                "failed"
            }
            .to_owned(),
        ),
        post: None,
    }
}

/// A workflow run's ending in one line.
fn workflow_summary(store: &Store, run_id: &str) -> IpcResult<String> {
    let Some(run) = store.workflow_run(run_id)? else {
        return Ok(String::new());
    };
    let why = run.error.clone().or_else(|| {
        store
            .workflow_steps(run_id)
            .unwrap_or_default()
            .iter()
            .find(|step| step.status == "failed")
            .map(|step| match step.note.as_deref() {
                Some(note) => format!("`{}`: {note}", step.step_id),
                None => format!("The step `{}` failed.", step.step_id),
            })
    });
    Ok(match (run.status.as_str(), why) {
        ("succeeded", _) => format!("`{}` succeeded.", run.workflow_id),
        (status, Some(why)) => format!("`{}` {status}. {why}", run.workflow_id),
        (status, None) => format!("`{}` {status}.", run.workflow_id),
    })
}

/// How many triggered runs led to a run: a workflow run keeps its own, a test run a trigger
/// queued has its fire's, and one a workflow's `run_test` queued has that workflow run's.
fn run_depth(store: &Store, kind: &str, run_id: &str) -> IpcResult<i64> {
    let workflow_depth = |id: &str| -> IpcResult<i64> {
        Ok(store
            .workflow_run(id)?
            .map(|run| crate::workflow::runs::Facts::parse(&run.context).depth)
            .unwrap_or(0))
    };
    if kind == "workflow" {
        return workflow_depth(run_id);
    }
    if let Some(fire) = store.trigger_fire_of_run(run_id)? {
        return Ok(fire.depth);
    }
    match store.workflow_run_of_test(run_id)? {
        Some(workflow_run) => workflow_depth(&workflow_run),
        None => Ok(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trigger::tests::Tx;

    fn now() -> i64 {
        crate::store::now_ms()
    }

    #[test]
    fn a_cursor_starts_at_the_newest_so_what_came_before_is_never_news() {
        let tx = Tx::new();
        tx.store()
            .add_done_signal(tx.workspace_id(), None, None, Some("before"))
            .unwrap();
        assert!(read_done(tx.store()).unwrap().is_empty());
        tx.store()
            .add_done_signal(tx.workspace_id(), None, Some("abc"), Some("after"))
            .unwrap();
        let seen = read_done(tx.store()).unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].event, TriggerEvent::WorkspaceDone);
        assert_eq!(seen[0].facts.get("note").map(String::as_str), Some("after"));
        assert_eq!(seen[0].facts.get("head").map(String::as_str), Some("abc"));
        assert!(read_done(tx.store()).unwrap().is_empty(), "read once");
    }

    #[test]
    fn a_custom_event_is_read_once_from_the_first_and_one_a_test_raised_is_one_deeper_than_it() {
        let tx = Tx::new();
        let raise = |name: &str, run_id: Option<&str>| {
            tx.store()
                .add_custom_event(&crate::store::NewCustomEvent {
                    workspace_id: tx.workspace_id(),
                    session_id: None,
                    run_id,
                    name,
                    head: Some("abc"),
                    note: Some("ready"),
                })
                .unwrap();
        };
        // Raised before the trigger thread ever looked: still news.
        raise("before", None);
        raise("deploy", None);
        let seen = read_custom(tx.store()).unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].kind(), "custom.before");
        let seen = &seen[1..];
        assert_eq!(seen[0].event, TriggerEvent::Custom);
        assert_eq!(seen[0].kind(), "custom.deploy");
        assert_eq!(seen[0].facts.get("note").map(String::as_str), Some("ready"));
        assert_eq!(seen[0].facts.get("head").map(String::as_str), Some("abc"));
        assert_eq!(seen[0].depth, 0);
        assert!(read_custom(tx.store()).unwrap().is_empty(), "read once");

        // A smoke test a trigger started at depth 2 raises one from its `run` step.
        let events = [TriggerEvent::WorkspaceDone];
        let filters = crate::trigger::Filters::default();
        let inputs = BTreeMap::new();
        tx.try_add(&tx.draft(
            &events,
            &filters,
            crate::schedule::ScheduleTarget::Test,
            "demo",
            &inputs,
        ))
        .unwrap();
        let at = crate::trigger::tests::NOW + 1;
        let mut done = read_done_as(&tx, at);
        done.depth = 2;
        super::super::fire::observe(tx.store(), &done).unwrap();
        let git = crate::git::testing::git();
        let fired = super::super::fire::tick(
            tx.store(),
            &tx.fx.data_dir,
            &crate::workflow::runs::Accepting::default(),
            &git,
            at,
        )
        .unwrap();
        let test_run = fired[0].fire.run_id.clone().unwrap();
        raise("deploy", Some(&test_run));
        // A run id that is not a test's counts for nothing.
        raise("deploy", Some("not-a-run"));
        let seen = read_custom(tx.store()).unwrap();
        assert_eq!(seen.iter().map(|s| s.depth).collect::<Vec<_>>(), [3, 0]);
    }

    /// `ys workspace done`, as `read_done` would see it at `at`.
    fn read_done_as(tx: &Tx, at: i64) -> Observation {
        Observation {
            event: TriggerEvent::WorkspaceDone,
            workspace_id: tx.workspace_id().to_owned(),
            session_id: None,
            terminal: None,
            agent_run: None,
            at,
            facts: BTreeMap::new(),
            depth: 0,
            target_id: None,
            status: None,
            post: None,
        }
    }

    #[test]
    fn a_run_that_ended_is_seen_once_with_the_depth_a_run_it_starts_would_have() {
        let tx = Tx::new();
        assert!(read_endings(tx.store(), now()).unwrap().is_empty());
        let facts = crate::workflow::runs::Facts {
            depth: 2,
            ..Default::default()
        };
        let run = crate::workflow::runs::queue(
            tx.store(),
            &tx.fx.data_dir,
            &crate::workflow::runs::Request {
                workflow_id: "tell",
                workspace_id: tx.workspace_id(),
                inputs: &BTreeMap::new(),
                requested_by: "app",
                unattended: false,
                origin_record: None,
                depth: facts.depth,
                event: None,
            },
            &crate::workflow::runs::Accepting::default(),
        )
        .unwrap();
        tx.store().start_workflow_run(&run, None).unwrap();
        tx.store()
            .finish_workflow_run(&run, "failed", Some("It broke."))
            .unwrap();
        // Read a second behind the clock.
        assert!(read_endings(tx.store(), now()).unwrap().is_empty());
        let seen = read_endings(tx.store(), now() + 2_000).unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].event, TriggerEvent::WorkflowFinished);
        assert_eq!(seen[0].depth, 3);
        assert_eq!(seen[0].status.as_deref(), Some("failed"));
        assert_eq!(
            seen[0].facts.get("summary").map(String::as_str),
            Some("`tell` failed. It broke.")
        );
        assert!(read_endings(tx.store(), now() + 4_000).unwrap().is_empty());
    }

    #[test]
    fn a_reported_turn_of_an_agent_is_one_settling_and_a_shell_is_none() {
        let tx = Tx::new();
        tx.agent("rec-1", "pty-1");
        let turns = Turns::default();
        let git = crate::git::testing::git();
        assert!(read_events(tx.store(), &turns, &git).unwrap().is_empty());
        let event = |kind: &str, run: &str, id: &str| {
            tx.store()
                .add_event(&crate::store::NewEvent {
                    id,
                    schema_version: 1,
                    workspace_id: tx.workspace_id(),
                    session_id: Some("rec-1"),
                    run_id: Some(run),
                    occurred_at: now(),
                    kind,
                    producer: "claude",
                    method: "hook",
                    fidelity: "reported",
                    source_key: None,
                    privacy_class: "metadata",
                    payload: r#"{"exitCode":1,"success":false,"reason":"exited"}"#,
                })
                .unwrap();
        };
        event("turn.completed", "run-rec-1", "e1");
        event("process.exited", "run-rec-1", "e2");
        let seen = read_events(tx.store(), &turns, &git).unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].event, TriggerEvent::AgentSettled);
        assert_eq!(seen[0].session_id.as_deref(), Some("rec-1"));
        assert_eq!(seen[0].terminal.as_deref(), Some("pty-1"));
        assert_eq!(seen[1].event, TriggerEvent::AgentExited);
        assert_eq!(seen[1].status.as_deref(), Some("failed"));
        assert_eq!(
            seen[1].facts.get("summary").map(String::as_str),
            Some("It exited with code 1.")
        );
        // The host's quiet for the turn already reported says nothing new.
        assert!(!turns.reported("pty-1", now()));
    }
}
