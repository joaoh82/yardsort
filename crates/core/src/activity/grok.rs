//! Grok, as a source of activity: its own session directory, read as it grows.
//!
//! Grok keeps, per session, a directory under `$GROK_HOME/sessions/<encoded cwd>/<id>/` with an
//! `events.jsonl` that is metadata by Grok's own design — which tool ran, how long it took, how
//! it came out, whether a permission was asked and how long the user took to answer, when a
//! turn started and ended and on which model — a `usage.json` with tokens and cost per turn,
//! and a `summary.json`. Yardsort chooses Grok's session id at launch, so it knows exactly
//! which directory is which run's. Nothing is installed, no hook is given, nothing of the user's
//! is written: the files are read, from where they were last read, whenever the drain runs.
//!
//! Grok also has hooks in Claude Code's shape, always trusted from `$GROK_HOME/hooks/`, but its
//! per-launch configuration overlay is barred from adding them, so using them would mean writing
//! a file of ours into the user's home. The session directory says the same things without that.
//! See `docs/design/14-agent-events-stage-2-grok.md` and the fixtures under
//! `crates/core/fixtures/grok/`.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::codex::Derived;
use super::{attempt, iso_to_ms, PRIVACY_METADATA, SCHEMA_VERSION};
use crate::store::{NewEvent, Store};

/// The built-in harness this adapter belongs to.
pub const HARNESS_ID: &str = "grok";
pub const PRODUCER: &str = "grok";
pub const METHOD: &str = "session_file";
pub const FIDELITY: &str = "reported";
/// The Grok version the fixtures — and so the mapping — come from.
pub const FIXTURE_VERSION: &str = "1.0.41";
/// A run that ended this long ago is still read once more: Grok writes its last lines and the
/// usage after the process is gone.
pub const AFTER_END_MS: i64 = 10 * 60 * 1000;
/// Read no event log larger than this in one go; a long session's is a few megabytes.
pub const MAX_EVENTS_BYTES: u64 = 64 * 1024 * 1024;

/// Where Grok keeps its state, from the environment a launch is given: `GROK_HOME`, else
/// `.grok` under the home directory.
pub fn grok_home(var: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(home) = var("GROK_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(home));
    }
    var("HOME")
        .or_else(|| var("USERPROFILE"))
        .filter(|v| !v.is_empty())
        .map(|home| PathBuf::from(home).join(".grok"))
}

/// `$GROK_HOME/sessions/<encoded cwd>/<session id>/`, found by the id alone: the encoding of
/// the working directory is Grok's business, and the id is a UUID.
pub fn find_session_dir(grok_home: &Path, session_id: &str) -> Option<PathBuf> {
    let sessions = grok_home.join("sessions");
    let entries = std::fs::read_dir(&sessions).ok()?;
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path().join(session_id))
        .filter(|p| p.is_dir())
        .collect();
    found.sort();
    found.pop()
}

/// Where reading left off, per run: the number of event lines already taken. Held by whoever
/// drains — the app's watcher keeps one across ticks; `ys` starts empty and lets the unique
/// source keys turn re-reads into duplicates.
pub type Cursors = HashMap<String, usize>;

/// What reading the session directories found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub imported: usize,
    pub duplicates: usize,
    /// Runs still going, or just ended: worth looking at again soon.
    pub watching: usize,
    pub workspaces: BTreeSet<String>,
    /// Pull requests newly seen created, for the drainer to track.
    pub pull_requests: Vec<crate::tracked::Seen>,
}

/// Read every live Grok run's session directory into the store. Best effort throughout: a
/// directory that is not there yet (Grok has not started writing) or a line that cannot be
/// parsed is skipped, never raised.
pub fn import(store: &Store, grok_home: &Path, cursors: &mut Cursors) -> Report {
    let mut report = Report::default();
    let since = crate::store::now_ms() - AFTER_END_MS;
    let Some(runs) = attempt(store, "live_runs", || store.live_runs(HARNESS_ID, since)) else {
        return report;
    };
    for run in runs {
        let Some(session_id) = run.harness_session_id.as_deref() else {
            continue;
        };
        // Every run the query returned is worth another look: one still going, or one that
        // ended within the grace and may yet gain its last lines and its usage.
        report.watching += 1;
        let Some(dir) = find_session_dir(grok_home, session_id) else {
            continue;
        };
        let mut derived = Vec::new();
        if let Some(started) = read_summary(&dir, session_id, &run.id) {
            derived.push(started);
        }
        let from = cursors.get(&run.id).copied().unwrap_or(0);
        let (events, lines) = read_events(&dir, session_id, from);
        derived.extend(events);
        cursors.insert(run.id.clone(), lines);
        derived.extend(read_usage(&dir, session_id));
        let mut any = false;
        for event in derived {
            let payload = event.payload.to_string();
            let added = attempt(store, event.kind, || {
                store.add_event(&NewEvent {
                    id: &uuid::Uuid::new_v4().to_string(),
                    schema_version: SCHEMA_VERSION,
                    workspace_id: &run.workspace_id,
                    session_id: run.session_id.as_deref(),
                    run_id: Some(&run.id),
                    occurred_at: event.occurred_at,
                    kind: event.kind,
                    producer: PRODUCER,
                    method: METHOD,
                    fidelity: FIDELITY,
                    source_key: Some(&event.source_key),
                    privacy_class: PRIVACY_METADATA,
                    payload: &payload,
                })
            });
            match added {
                Some(true) => {
                    report.imported += 1;
                    any = true;
                    if let Some(url) = event.payload["pullRequestUrl"].as_str() {
                        report.pull_requests.push(crate::tracked::Seen {
                            workspace_id: run.workspace_id.clone(),
                            session_id: run.session_id.clone(),
                            url: url.to_owned(),
                        });
                    }
                }
                Some(false) => report.duplicates += 1,
                None => {}
            }
        }
        if any {
            report.workspaces.insert(run.workspace_id.clone());
        }
    }
    report
}

/// The session's start, from `summary.json`: once per run, so a resumed session says so again.
pub fn read_summary(dir: &Path, session_id: &str, run_id: &str) -> Option<Derived> {
    let text = std::fs::read_to_string(dir.join("summary.json")).ok()?;
    let summary: Value = serde_json::from_str(&text).ok()?;
    let s = |key: &str| summary.get(key).and_then(Value::as_str).map(str::to_owned);
    Some(Derived {
        kind: "session.started",
        source_key: format!("grok:{session_id}:{run_id}:session"),
        occurred_at: s("created_at")
            .as_deref()
            .and_then(iso_to_ms)
            .unwrap_or_else(crate::store::now_ms),
        payload: json!({
            "sessionId": session_id,
            "model": s("current_model_id"),
            "reasoningEffort": s("reasoning_effort"),
            "agent": s("agent_name"),
        }),
    })
}

/// The events of `events.jsonl` from line `from` on, and the number of lines now read. Each
/// event is keyed by its line, so a line read twice is one row.
pub fn read_events(dir: &Path, session_id: &str, from: usize) -> (Vec<Derived>, usize) {
    let path = dir.join("events.jsonl");
    let Ok(metadata) = std::fs::metadata(&path) else {
        return (vec![], from);
    };
    if metadata.len() > MAX_EVENTS_BYTES {
        return (vec![], from);
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
        return (vec![], from);
    };
    let mut events = Vec::new();
    let mut lines = 0;
    let mut turn: Option<i64> = None;
    // Read only once a finished tool call needs it.
    let mut opened: Option<HashMap<String, String>> = None;
    // Only a line Grok has finished writing counts: a fragment at the end — a record caught
    // mid-append — is left for the next read, when it will be whole.
    for (index, line) in text.split_inclusive('\n').enumerate() {
        let Some(line) = line.strip_suffix('\n') else {
            break;
        };
        lines = index + 1;
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let text = |key: &str| record.get(key).and_then(Value::as_str).map(str::to_owned);
        let number = |key: &str| record.get(key).and_then(Value::as_i64);
        if text("type").as_deref() == Some("turn_started") {
            turn = number("turn_number");
        }
        if index < from {
            continue;
        }
        let at = text("ts").as_deref().and_then(iso_to_ms).unwrap_or(0);
        let key = |what: &str| format!("grok:{session_id}:{index}:{what}");
        let (kind, payload) = match text("type").as_deref() {
            Some("turn_started") => (
                "turn.started",
                json!({
                    "sessionId": session_id,
                    "turnNumber": number("turn_number"),
                    "model": text("model_id"),
                    "relationship": text("session_relationship"),
                }),
            ),
            Some("turn_ended") => {
                let outcome = text("outcome");
                (
                    if outcome.as_deref() == Some("completed") {
                        "turn.completed"
                    } else {
                        "turn.failed"
                    },
                    json!({ "sessionId": session_id, "turnNumber": turn, "outcome": outcome }),
                )
            }
            Some("tool_started") => (
                "tool.started",
                json!({ "tool": text("tool_name"), "sessionId": session_id, "turnNumber": turn }),
            ),
            Some("tool_completed") => {
                let success = text("outcome").as_deref() == Some("success");
                let mut payload = json!({
                    "tool": text("tool_name"),
                    "toolUseId": text("tool_call_id"),
                    "sessionId": session_id,
                    "turnNumber": turn,
                    "durationMs": number("duration_ms"),
                    "outcome": text("outcome"),
                });
                // A pull request the call opened keeps its link and nothing else: the one
                // exception to metadata only (docs/design/38-pull-requests-elsewhere.md).
                if let (true, Some(id)) = (success, text("tool_call_id")) {
                    let opened = opened.get_or_insert_with(|| opened_in_history(dir));
                    if let Some(url) = opened.get(&id) {
                        payload["pullRequestUrl"] = Value::String(url.clone());
                    }
                }
                (
                    if success {
                        "tool.completed"
                    } else {
                        "tool.failed"
                    },
                    payload,
                )
            }
            // A permission the user was actually asked for: one that took time to answer, or
            // was refused. The ones Grok resolved by itself at once are not events to a reader.
            Some("permission_resolved") => {
                let waited = number("wait_ms").unwrap_or(0) > 0;
                let refused = text("decision").as_deref() != Some("allow");
                if !waited && !refused {
                    continue;
                }
                (
                    "approval.resolved",
                    json!({
                        "tool": text("tool_name"),
                        "sessionId": session_id,
                        "turnNumber": turn,
                        "decision": text("decision"),
                        "waitMs": number("wait_ms"),
                    }),
                )
            }
            _ => continue,
        };
        events.push(Derived {
            kind,
            source_key: key(kind),
            occurred_at: at,
            payload,
        });
    }
    (events, lines)
}

/// The pull requests the session's tool calls opened, by call id, from `chat_history.jsonl`,
/// where Grok keeps each call beside its result: a terminal command running `gh pr create`, by
/// what it printed; a GitHub create-pull-request tool, by its answer. Read for that alone — the
/// event log has no output — and nothing else of the conversation is kept.
pub fn opened_in_history(dir: &Path) -> HashMap<String, String> {
    let mut opened = HashMap::new();
    let path = dir.join("chat_history.jsonl");
    if std::fs::metadata(&path).map_or(true, |m| m.len() > MAX_EVENTS_BYTES) {
        return opened;
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
        return opened;
    };
    // The calls that may have opened one, by id: a terminal command's line, or none for a tool.
    let mut calls: HashMap<String, Option<String>> = HashMap::new();
    for record in text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
    {
        match record.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                for call in record["tool_calls"].as_array().into_iter().flatten() {
                    let (Some(id), Some(name)) = (
                        call.get("id").and_then(Value::as_str),
                        call.get("name").and_then(Value::as_str),
                    ) else {
                        continue;
                    };
                    if name == "run_terminal_command" {
                        let arguments = call
                            .get("arguments")
                            .and_then(Value::as_str)
                            .and_then(|a| serde_json::from_str::<Value>(a).ok());
                        if let Some(command) = arguments
                            .as_ref()
                            .and_then(|a| a.get("command"))
                            .and_then(Value::as_str)
                        {
                            calls.insert(id.to_owned(), Some(command.to_owned()));
                        }
                    } else if crate::tracked::is_create_pull_request("", name) {
                        calls.insert(id.to_owned(), None);
                    }
                }
            }
            Some("tool_result") => {
                let Some(call) = record
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .and_then(|id| calls.get(id).map(|call| (id, call)))
                else {
                    continue;
                };
                let content = &record["content"];
                let url = match call.1 {
                    Some(command) => {
                        let printed = match content {
                            Value::String(text) => text.clone(),
                            other => other.to_string(),
                        };
                        crate::tracked::created_by_command(command, &printed)
                    }
                    None => crate::tracked::link_in_tool_result(content),
                };
                if let Some(url) = url {
                    opened.insert(call.0.to_owned(), url);
                }
            }
            _ => {}
        }
    }
    opened
}

/// Tokens and cost per turn, from `usage.json`. Keyed by turn, so a file read again adds only
/// the turns it did not have.
pub fn read_usage(dir: &Path, session_id: &str) -> Vec<Derived> {
    let Ok(text) = std::fs::read_to_string(dir.join("usage.json")) else {
        return vec![];
    };
    let Ok(usage) = serde_json::from_str::<Value>(&text) else {
        return vec![];
    };
    let Some(turns) = usage.get("turns").and_then(Value::as_array) else {
        return vec![];
    };
    turns
        .iter()
        .filter_map(|turn| {
            let n = |key: &str| turn.get(key).and_then(Value::as_i64);
            let number = n("turnNumber")?;
            Some(Derived {
                kind: "usage.reported",
                source_key: format!("grok:{session_id}:usage:{number}"),
                occurred_at: turn
                    .get("endedAt")
                    .and_then(Value::as_str)
                    .and_then(iso_to_ms)
                    .unwrap_or_else(crate::store::now_ms),
                payload: json!({
                    "sessionId": session_id,
                    "turnNumber": number,
                    "model": turn.get("primaryModelId").and_then(Value::as_str),
                    "inputTokens": n("inputTokens"),
                    "outputTokens": n("outputTokens"),
                    "reasoningOutputTokens": n("reasoningTokens"),
                    "cachedInputTokens": n("cachedReadTokens"),
                    "cacheWriteTokens": n("cacheCreationTokens"),
                    "totalTokens": n("totalTokens"),
                    "modelCalls": n("modelCalls"),
                    // Grok's own unit, kept as it is; the guide says what it is not.
                    "costUsdTicks": n("costUsdTicks"),
                }),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::{Continuation, LaunchedBy, Recorder, RunDraft, RunKind};

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/grok")
            .join(FIXTURE_VERSION)
    }

    /// A throwaway GROK_HOME holding the fixture session where Grok would keep it.
    fn grok_home_with_fixture(dir: &Path, session_id: &str) -> PathBuf {
        let home = dir.join("grok");
        let session = home
            .join("sessions/%2Ftmp%2Fyardsort-fixture%2Frepo")
            .join(session_id);
        std::fs::create_dir_all(&session).unwrap();
        for file in ["events.jsonl", "usage.json", "summary.json"] {
            std::fs::copy(fixtures().join(file), session.join(file)).unwrap();
        }
        home
    }

    const SESSION: &str = "11111111-1111-4111-8111-111111111111";

    /// Not recorded yet — Grok was not signed in on the machine that recorded the others — so
    /// the shapes are the recorded ones of `grok/1.0.41` (a `run_terminal_command` and its
    /// `tool_completed`) and `grok/1.0.70` (the chat history's calls and results), with a
    /// `gh pr create` in them. `scripts/record-pr-creation.sh grok` records the real thing.
    #[test]
    fn a_pull_request_a_terminal_command_opened_keeps_its_link_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let line = |value: Value| format!("{value}\n");
        let call = |id: &str, name: &str, arguments: Value| {
            line(json!({ "type": "assistant", "content": "",
                "tool_calls": [{ "id": id, "name": name, "arguments": arguments.to_string() }] }))
        };
        let result = |id: &str, content: &str| {
            line(json!({ "type": "tool_result", "tool_call_id": id, "content": content }))
        };
        let printed = "exit: 0\n\nCreating pull request for bump into main in o/lib\n\nhttps://github.com/o/lib/pull/41\n";
        let history = [
            call(
                "call-1",
                "run_terminal_command",
                json!({ "command": "gh pr create --fill", "description": "Open it" }),
            ),
            result("call-1", printed),
            call(
                "call-2",
                "run_terminal_command",
                json!({ "command": "gh pr view 41" }),
            ),
            result("call-2", printed),
            call(
                "call-3",
                "run_terminal_command",
                json!({ "command": "gh pr create --fill" }),
            ),
            result("call-3", printed),
        ]
        .concat();
        std::fs::write(dir.path().join("chat_history.jsonl"), history).unwrap();
        let completed = |id: &str, outcome: &str| {
            line(
                json!({ "ts": "2026-09-25T16:33:17.286Z", "type": "tool_completed",
                "tool_name": "run_terminal_command", "duration_ms": 9, "outcome": outcome,
                "tool_call_id": id }),
            )
        };
        let log = [
            completed("call-1", "success"),
            completed("call-2", "success"),
            completed("call-3", "error"),
        ]
        .concat();
        std::fs::write(dir.path().join("events.jsonl"), log).unwrap();

        let (events, _) = read_events(dir.path(), SESSION, 0);
        let links: Vec<Option<&str>> = events
            .iter()
            .map(|e| e.payload["pullRequestUrl"].as_str())
            .collect();
        assert_eq!(
            links,
            [Some("https://github.com/o/lib/pull/41"), None, None],
            "the one that created it; not a view, not a failure"
        );
        for event in &events {
            let text = event.payload.to_string();
            for content in ["gh pr", "Creating pull request", "Open it"] {
                assert!(!text.contains(content), "{content} in {text}");
            }
        }
    }

    #[test]
    fn the_event_log_reads_as_tools_turns_and_the_permissions_that_were_really_asked() {
        let (events, lines) = read_events(&fixtures(), SESSION, 0);
        assert_eq!(lines, 140);
        let kinds: Vec<&str> = events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            [
                "turn.started",
                "tool.started",   // write
                "tool.started",   // read_file
                "tool.started",   // run_terminal_command
                "tool.completed", // write
                "tool.failed",    // read_file, of a file that is not there
                "tool.completed", // run_terminal_command
                "tool.started",   // read_file
                "tool.completed",
                "turn.completed",
            ],
            "permissions Grok granted itself at once are not events; phases never are"
        );
        let failed = events.iter().find(|e| e.kind == "tool.failed").unwrap();
        assert_eq!(failed.payload["tool"], "read_file");
        assert_eq!(failed.payload["durationMs"], 8);
        assert_eq!(failed.payload["turnNumber"], 0);
        assert_eq!(
            failed.occurred_at,
            iso_to_ms("2026-09-25T16:33:17.300Z").unwrap()
        );
        let turn = events.iter().find(|e| e.kind == "turn.started").unwrap();
        assert_eq!(turn.payload["model"], "grok-4.7");
        assert!(events.iter().all(|e| e.source_key.starts_with("grok:1111")));
        let text = events
            .iter()
            .map(|e| e.payload.to_string())
            .collect::<String>();
        for content in ["hello", "missing.txt", "/tmp", "marvin", "http"] {
            assert!(!text.contains(content), "leaked {content:?}");
        }

        // From a cursor: only what came after.
        let (rest, lines_again) = read_events(&fixtures(), SESSION, lines - 1);
        assert_eq!(lines_again, lines);
        assert_eq!(
            rest.iter().map(|e| e.kind).collect::<Vec<_>>(),
            ["turn.completed"]
        );
        assert_eq!(
            rest[0].payload["turnNumber"], 0,
            "the turn is known even from a cursor"
        );
    }

    #[test]
    fn a_record_caught_mid_append_waits_for_the_next_read() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("events.jsonl");
        let whole =
            r#"{"ts":"2026-09-25T16:33:17.216Z","type":"tool_started","tool_name":"write"}"#;
        let (head, tail) = whole.split_at(40);
        std::fs::write(
            &log,
            format!(
                "{}\n{head}",
                r#"{"ts":"2026-09-25T16:33:12.317Z","type":"turn_started","turn_number":0,"model_id":"grok-4.7"}"#
            ),
        )
        .unwrap();
        let (events, cursor) = read_events(dir.path(), "s", 0);
        assert_eq!(
            events.iter().map(|e| e.kind).collect::<Vec<_>>(),
            ["turn.started"]
        );
        assert_eq!(cursor, 1, "the fragment is not counted");

        let mut text = std::fs::read_to_string(&log).unwrap();
        text.push_str(tail);
        text.push('\n');
        std::fs::write(&log, text).unwrap();
        let (events, cursor) = read_events(dir.path(), "s", cursor);
        assert_eq!(
            events.iter().map(|e| e.kind).collect::<Vec<_>>(),
            ["tool.started"]
        );
        assert_eq!(events[0].payload["tool"], "write");
        assert_eq!(cursor, 2);
    }

    #[test]
    fn a_permission_that_waited_or_was_refused_is_an_event() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("events.jsonl"),
            concat!(
                r#"{"ts":"2026-09-25T16:33:12.317Z","type":"turn_started","turn_number":2,"model_id":"grok-4.7"}"#, "\n",
                r#"{"ts":"2026-09-25T16:33:17.230Z","type":"permission_resolved","tool_name":"run_terminal_command","decision":"allow","wait_ms":4210}"#, "\n",
                r#"{"ts":"2026-09-25T16:33:18.000Z","type":"permission_resolved","tool_name":"write","decision":"deny","wait_ms":0}"#, "\n",
                r#"{"ts":"2026-09-25T16:33:19.000Z","type":"permission_resolved","tool_name":"read_file","decision":"allow","wait_ms":0}"#, "\n",
                r#"{"ts":"2026-09-25T16:33:20.000Z","type":"turn_ended","outcome":"interrupted"}"#, "\n",
                "not json at all\n",
            ),
        )
        .unwrap();
        let (events, lines) = read_events(dir.path(), "s", 0);
        assert_eq!(lines, 6);
        let kinds: Vec<&str> = events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            [
                "turn.started",
                "approval.resolved",
                "approval.resolved",
                "turn.failed"
            ]
        );
        assert_eq!(events[1].payload["waitMs"], 4210);
        assert_eq!(events[2].payload["decision"], "deny");
        assert_eq!(events[3].payload["outcome"], "interrupted");
    }

    #[test]
    fn usage_and_the_summary_read_as_tokens_per_turn_and_the_sessions_start() {
        let usage = read_usage(&fixtures(), SESSION);
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0].kind, "usage.reported");
        assert_eq!(
            usage[0].source_key,
            "grok:11111111-1111-4111-8111-111111111111:usage:1"
        );
        assert_eq!(usage[0].payload["totalTokens"], 57107);
        assert_eq!(usage[0].payload["cachedInputTokens"], 43648);
        assert_eq!(usage[0].payload["model"], "grok-4.7-build");
        assert_eq!(
            usage[0].occurred_at,
            iso_to_ms("2026-09-25T16:33:21.149881834+00:00").unwrap()
        );

        let started = read_summary(&fixtures(), SESSION, "run-1").unwrap();
        assert_eq!(started.kind, "session.started");
        assert_eq!(
            started.source_key,
            "grok:11111111-1111-4111-8111-111111111111:run-1:session"
        );
        assert_eq!(started.payload["model"], "grok-4.7");
        assert_eq!(started.payload["reasoningEffort"], "high");
        assert!(!started.payload.to_string().contains("stripped"));
        assert_eq!(read_usage(Path::new("/nowhere"), SESSION), vec![]);
        assert!(read_summary(Path::new("/nowhere"), SESSION, "r").is_none());
    }

    #[test]
    fn the_session_directory_is_found_by_id_under_any_encoded_working_directory() {
        let dir = tempfile::tempdir().unwrap();
        let home = grok_home_with_fixture(dir.path(), SESSION);
        assert!(find_session_dir(&home, SESSION).unwrap().ends_with(SESSION));
        assert!(find_session_dir(&home, "no-such-session").is_none());
        assert!(find_session_dir(Path::new("/nowhere"), SESSION).is_none());
        let at = |vars: &[(&str, &str)]| {
            grok_home(&|k| {
                vars.iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| (*v).to_owned())
            })
        };
        assert_eq!(
            at(&[("GROK_HOME", "/g"), ("HOME", "/h")]),
            Some(PathBuf::from("/g"))
        );
        assert_eq!(at(&[("HOME", "/h")]), Some(PathBuf::from("/h/.grok")));
        assert_eq!(at(&[]), None);
    }

    #[test]
    fn a_live_run_is_read_into_the_store_once_and_then_only_what_is_new() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let project = store.add_project("app", "/code/app").unwrap();
        let ws = store
            .add_worktree(&project.id, "fix", "/wt/fix", Some("ys/fix"), Some("main"))
            .unwrap()
            .id;
        let recorder = Recorder::new(&store, &Default::default(), LaunchedBy::App);
        let draft = RunDraft {
            workspace_id: ws.clone(),
            session_id: None,
            kind: RunKind::Harness,
            harness_id: Some(HARNESS_ID.into()),
            harness_session_id: Some(SESSION.into()),
            model: None,
            effort: None,
            program: "grok".into(),
            continuation: Continuation::Fresh,
        };
        let run_id = recorder.begin(&draft).unwrap();
        recorder.spawned(&run_id, &draft, "pty-g", Some(METHOD));
        let home = grok_home_with_fixture(dir.path(), SESSION);
        let mut cursors = Cursors::new();

        let report = import(&store, &home, &mut cursors);
        assert_eq!(
            report.imported,
            1 + 10 + 1,
            "session, the log, the turn's usage"
        );
        assert_eq!(report.duplicates, 0);
        assert_eq!(report.watching, 1);
        assert_eq!(report.workspaces.iter().collect::<Vec<_>>(), [&ws]);
        assert_eq!(cursors[&run_id], 140);
        let events = store.all_events(Some(&ws)).unwrap();
        let usage = events.iter().find(|e| e.kind == "usage.reported").unwrap();
        assert_eq!(usage.run_id.as_deref(), Some(run_id.as_str()));
        assert_eq!(
            (
                usage.producer.as_str(),
                usage.method.as_str(),
                usage.fidelity.as_str()
            ),
            ("grok", "session_file", "reported")
        );

        // Nothing new: nothing added, and the summary and usage are duplicates, not repeats.
        let again = import(&store, &home, &mut cursors);
        assert_eq!((again.imported, again.duplicates), (0, 2));

        // Grok appends a line: only that line is read.
        let log = find_session_dir(&home, SESSION)
            .unwrap()
            .join("events.jsonl");
        let mut text = std::fs::read_to_string(&log).unwrap();
        text.push_str(r#"{"ts":"2026-09-25T16:40:00.000Z","type":"turn_started","turn_number":1,"model_id":"grok-4.7"}"#);
        text.push('\n');
        std::fs::write(&log, text).unwrap();
        let more = import(&store, &home, &mut cursors);
        assert_eq!(more.imported, 1);
        assert_eq!(cursors[&run_id], 141);
        assert!(more.pull_requests.is_empty());

        // A terminal command opens a pull request: its link is handed on with the run's
        // workspace, once.
        let dir_of = find_session_dir(&home, SESSION).unwrap();
        std::fs::write(
            dir_of.join("chat_history.jsonl"),
            format!(
                "{}\n{}\n",
                json!({ "type": "assistant", "tool_calls": [{ "id": "call-pr",
                    "name": "run_terminal_command",
                    "arguments": json!({ "command": "gh pr create --fill" }).to_string() }] }),
                json!({ "type": "tool_result", "tool_call_id": "call-pr",
                    "content": "exit: 0\nhttps://github.com/o/lib/pull/41\n" }),
            ),
        )
        .unwrap();
        let mut text = std::fs::read_to_string(&log).unwrap();
        text.push_str(r#"{"ts":"2026-09-25T16:40:01.000Z","type":"tool_completed","tool_name":"run_terminal_command","duration_ms":9,"outcome":"success","tool_call_id":"call-pr"}"#);
        text.push('\n');
        std::fs::write(&log, text).unwrap();
        let opened = import(&store, &home, &mut cursors);
        assert_eq!(
            opened.pull_requests,
            [crate::tracked::Seen {
                workspace_id: ws.clone(),
                session_id: None,
                url: "https://github.com/o/lib/pull/41".into(),
            }]
        );

        // Without a cursor — `ys`, say — the whole log is duplicates, not damage, and nothing
        // is handed on twice.
        let fresh = import(&store, &home, &mut Cursors::new());
        assert_eq!((fresh.imported, fresh.duplicates), (0, 12 + 2));
        assert!(fresh.pull_requests.is_empty());

        // A run that just ended is read once more, in case the last lines landed late.
        store.end_run(&run_id, Some(0), "exited").unwrap();
        let ended = import(&store, &home, &mut cursors);
        assert_eq!(ended.watching, 1, "just ended: still worth looking at");
        assert_eq!(ended.duplicates, 2);

        // One that ended long ago is left alone.
        const OLD: &str = "22222222-2222-4222-8222-222222222222";
        grok_home_with_fixture(dir.path(), OLD);
        let old = RunDraft {
            harness_session_id: Some(OLD.into()),
            ..draft.clone()
        };
        let old_id = recorder.begin(&old).unwrap();
        recorder.spawned(&old_id, &old, "pty-old", Some(METHOD));
        let long_ago = crate::store::now_ms() - AFTER_END_MS - 1;
        store
            .end_run_by_pty("pty-old", Some(0), "exited", long_ago)
            .unwrap();
        let stale = import(&store, &home, &mut Cursors::new());
        assert_eq!(
            (stale.imported, stale.duplicates),
            (0, 12 + 2),
            "only the recent run"
        );
        assert_eq!(stale.watching, 1, "the old one is not watched either");
        assert!(store
            .all_events(Some(&ws))
            .unwrap()
            .iter()
            .filter(|e| e.producer == PRODUCER)
            .all(|e| e.run_id.as_deref() != Some(old_id.as_str())));
    }
}
