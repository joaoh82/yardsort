//! Stage 3 of agent events: which of a workspace's files an agent *said* it wrote.
//!
//! Git shows what changed; the timeline holds what the agents reported. This joins the two at
//! read time, per path, and says no more than either side does: a file with a report was
//! named by that agent's own hooks or session file as one it wrote, and a file without one
//! changed with no such report — the user, a script, or an agent whose capture was off. Which
//! lines of a diff came from which report is never claimed: git shows the sum of every change
//! since the last commit, and no adapter records a line.
//!
//! A file the agent made with a shell command has no report: a command names no file, and the
//! command is never read. For those, the second join is an *observation*: the file's own
//! modification time against the windows in which the run's tools were executing. A write
//! that falls inside `Bash started … Bash done` was made while that command ran — by the
//! command, or by whoever else wrote the file in that second, which is why it is a lower
//! fidelity than a report and is worded as one. Only a tool call is a window; a run as a whole
//! is not, because an agent writes through its tools, and a file written while it sat idle
//! is more likely the user's.
//!
//! Nothing here is stored. A `workspace.changed` event, which the stage 1 report deferred to
//! this stage, is still not recorded: the join needs git's answer at the moment of reading, and
//! the file's clock is a better witness than a debounced watcher's.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::store::{EventRow, Store, StoreResult};

/// What one run reported about one path, summed over its reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// The run the reports were linked to; `None` when an adapter could not link them.
    pub run_id: Option<String>,
    /// The harness of that run, as Yardsort launched it.
    pub harness_id: Option<String>,
    /// Who said so, how, and how sure to be — the events' own provenance columns.
    pub producer: String,
    pub method: String,
    pub fidelity: String,
    pub first_at: i64,
    pub last_at: i64,
    /// How many writes were reported: tool calls for the hook adapters, file changes for the
    /// session-file ones. A count of reports, not of lines or of edits.
    pub writes: u32,
}

/// One tool call whose execution window contains the file's last write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedMatch {
    pub run_id: String,
    pub harness_id: Option<String>,
    /// The tool that was running, as the agent named it (`Bash`, `shell`, …).
    pub tool: Option<String>,
    pub from: i64,
    pub to: i64,
}

/// The file's last write on disk fell inside the execution window of one or more tool calls.
/// The file's own clock against the timeline: an observation, not a report, and worded as one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Observed {
    /// When the file was last written, as the file system has it.
    pub at: i64,
    /// Every tool call that was executing then. More than one means two agents at once, and the
    /// observation cannot say which.
    pub matches: Vec<ObservedMatch>,
}

/// Every report for one workspace-relative path, and what was observed of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileReports {
    pub path: String,
    /// Oldest first.
    pub reports: Vec<Report>,
    /// Set when the file's last write fell inside a tool call's window, whether or not it was
    /// also reported. Only for files whose modification time the caller supplied.
    pub observed: Option<Observed>,
}

/// One agent run, and whether it was in a position to report anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunCoverage {
    pub run_id: String,
    pub harness_id: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    /// How the run was asked to report (`hook`, `notify`, `plugin`, `session_file`,
    /// `extension`), or `None` for a run that was not: capture was off, or refused.
    pub capture: Option<String>,
    /// Whether `None` means that. The start event says what a run was asked; when it has been
    /// cleared or pruned, a report linked to the run says as much; with neither, nothing is
    /// known, and the run is not one to count as silent.
    pub capture_known: bool,
}

/// The join for one workspace: reported writes by path, and the runs that could have reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    /// Sorted by path.
    pub files: Vec<FileReports>,
    /// The workspace's agent runs that were spawned, oldest first. Shells cannot report and
    /// are left out.
    pub runs: Vec<RunCoverage>,
}

impl Provenance {
    /// How many runs reported through some capture.
    pub fn reporting_runs(&self) -> usize {
        self.runs.iter().filter(|run| run.capture.is_some()).count()
    }
}

/// The event kinds a write is reported through, plus the ones that bound a tool call's
/// window. Everything else is read for nothing.
const KINDS: [&str; 5] = [
    "file.reported_write",
    "tool.completed",
    "tool.started",
    "tool.failed",
    "process.started",
];

/// A tool call's execution window, for the observed join.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Window {
    run_id: String,
    tool: Option<String>,
    /// A tool that reported its own file is a window for that file alone; a command is a window
    /// for any file.
    path: Option<String>,
    from: i64,
    to: i64,
}

/// The tool calls' windows, per run. A `tool.started` is paired with the next `tool.completed`
/// or `tool.failed` of the same run and, when both carry one, the same `toolUseId` — else the
/// same tool name, in order. A completion with no start is bounded by its own `durationMs`;
/// without either, it is no window. A start with no end is open until the run's end.
fn windows(runs: &[crate::store::RunRow], events: &[EventRow]) -> Vec<Window> {
    let run_end: BTreeMap<&str, i64> = runs
        .iter()
        .map(|run| (run.id.as_str(), run.ended_at.unwrap_or(i64::MAX)))
        .collect();
    /// A `tool.started` waiting for its end.
    struct Started {
        run_id: String,
        tool: Option<String>,
        use_id: Option<String>,
        path: Option<String>,
        from: i64,
    }
    let mut open: Vec<Started> = vec![];
    let mut windows = vec![];
    for event in events {
        if !matches!(
            event.kind.as_str(),
            "tool.started" | "tool.completed" | "tool.failed"
        ) {
            continue;
        }
        let Some(run_id) = event.run_id.as_deref() else {
            continue;
        };
        let payload: Value = serde_json::from_str(&event.payload).unwrap_or(Value::Null);
        let text = |key: &str| payload.get(key).and_then(Value::as_str).map(str::to_owned);
        let (tool, use_id, path) = (text("tool"), text("toolUseId"), text("path"));
        if event.kind == "tool.started" {
            open.push(Started {
                run_id: run_id.to_owned(),
                tool,
                use_id,
                path,
                from: event.occurred_at,
            });
            continue;
        }
        let matching = open.iter().position(|started| {
            started.run_id == run_id
                && match (&started.use_id, &use_id) {
                    (Some(a), Some(b)) => a == b,
                    _ => started.tool == tool,
                }
        });
        match matching {
            Some(index) => {
                let started = open.remove(index);
                windows.push(Window {
                    run_id: started.run_id,
                    tool: started.tool,
                    path: path.or(started.path),
                    from: started.from,
                    to: event.occurred_at,
                });
            }
            None => {
                if let Some(duration) = payload.get("durationMs").and_then(Value::as_i64) {
                    windows.push(Window {
                        run_id: run_id.to_owned(),
                        tool,
                        path,
                        from: event.occurred_at - duration.max(0),
                        to: event.occurred_at,
                    });
                }
            }
        }
    }
    for started in open {
        let to = run_end
            .get(started.run_id.as_str())
            .copied()
            .unwrap_or(i64::MAX);
        windows.push(Window {
            run_id: started.run_id,
            tool: started.tool,
            path: started.path,
            from: started.from,
            to,
        });
    }
    windows.sort_by_key(|window| (window.from, window.to));
    windows
}

/// Whether an event says a file was written. `file.reported_write` is the contract's own kind
/// for it (Codex, OpenCode, Cursor) — unless the report itself says the change did not land:
/// Codex writes a `FileChange` item's `status` through, and a failed patch is not a write. The
/// hook and extension adapters that report a tool call instead name the harness's writing
/// tools. Grok's log carries a tool name and no path, so it never reaches here with one — and a
/// tool the list does not know is not a write.
fn is_write(producer: &str, kind: &str, tool: Option<&str>, status: Option<&str>) -> bool {
    match kind {
        "file.reported_write" => status.is_none_or(|status| status == "completed"),
        "tool.completed" => match (producer, tool) {
            ("claude", Some(tool)) => {
                matches!(tool, "Write" | "Edit" | "MultiEdit" | "NotebookEdit")
            }
            ("pi" | "omp", Some(tool)) => matches!(tool, "write" | "edit"),
            _ => false,
        },
        _ => false,
    }
}

/// The join for `workspace_id`, from the store as it is now.
/// `files` is each changed file the caller wants observed, with its modification time in
/// epoch milliseconds; a file that is gone from disk is simply left out.
pub fn of(store: &Store, workspace_id: &str, files: &[(String, i64)]) -> StoreResult<Provenance> {
    let runs = store.runs(workspace_id)?;
    let events = store.events_of_kinds(workspace_id, &KINDS)?;
    let native = store.native_methods_by_run(workspace_id)?;
    Ok(join(&runs, &events, &native, files))
}

/// The pure part: reports by path from the events, coverage from the runs, and for each of
/// `files` (path, modification time) the tool calls that were executing when it was last
/// written. `native` is which runs have events of the agent's own, and through what — the
/// fallback for a run whose start event is gone.
pub fn join(
    runs: &[crate::store::RunRow],
    events: &[EventRow],
    native: &[(String, String)],
    files: &[(String, i64)],
) -> Provenance {
    let harness_of: BTreeMap<&str, Option<&str>> = runs
        .iter()
        .map(|run| (run.id.as_str(), run.harness_id.as_deref()))
        .collect();
    let mut capture_of: BTreeMap<&str, Option<String>> = BTreeMap::new();
    let mut reported_through: BTreeMap<&str, &str> = BTreeMap::new();
    for (run_id, method) in native {
        reported_through
            .entry(run_id.as_str())
            .or_insert(method.as_str());
    }
    // Path → (run id, producer) → the report so far. A run's reports of one path are one row.
    let mut files_by_path: BTreeMap<String, BTreeMap<(Option<String>, String), Report>> =
        BTreeMap::new();

    for event in events {
        let payload: Value = serde_json::from_str(&event.payload).unwrap_or(Value::Null);
        if event.kind == "process.started" {
            if let Some(run_id) = event.run_id.as_deref() {
                capture_of.insert(
                    run_id,
                    payload
                        .get("capture")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                );
            }
            continue;
        }
        let tool = payload.get("tool").and_then(Value::as_str);
        let status = payload.get("status").and_then(Value::as_str);
        if !is_write(&event.producer, &event.kind, tool, status) {
            continue;
        }
        // A path the adapter could not place inside the workspace was never recorded; a write
        // without one says nothing about any file the diff shows.
        let Some(path) = payload.get("path").and_then(Value::as_str) else {
            continue;
        };
        let key = (event.run_id.clone(), event.producer.clone());
        let report = files_by_path
            .entry(path.to_owned())
            .or_default()
            .entry(key)
            .or_insert_with(|| Report {
                run_id: event.run_id.clone(),
                harness_id: event
                    .run_id
                    .as_deref()
                    .and_then(|id| harness_of.get(id).copied().flatten())
                    .map(str::to_owned),
                producer: event.producer.clone(),
                method: event.method.clone(),
                fidelity: event.fidelity.clone(),
                first_at: event.occurred_at,
                last_at: event.occurred_at,
                writes: 0,
            });
        report.first_at = report.first_at.min(event.occurred_at);
        report.last_at = report.last_at.max(event.occurred_at);
        report.writes += 1;
    }

    // The observed join: only harness runs have tools, and only a tool that named no file (or
    // named this one) is a window for this file.
    let harness_runs: BTreeMap<&str, Option<&str>> = runs
        .iter()
        .filter(|run| run.kind == "harness")
        .map(|run| (run.id.as_str(), run.harness_id.as_deref()))
        .collect();
    let windows = windows(runs, events);
    let mut observed: BTreeMap<String, Observed> = BTreeMap::new();
    for (path, at) in files {
        let matches: Vec<ObservedMatch> = windows
            .iter()
            .filter(|window| window.from <= *at && *at <= window.to)
            .filter(|window| window.path.as_deref().is_none_or(|p| p == path))
            .filter_map(|window| {
                harness_runs
                    .get(window.run_id.as_str())
                    .map(|harness| ObservedMatch {
                        run_id: window.run_id.clone(),
                        harness_id: harness.map(str::to_owned),
                        tool: window.tool.clone(),
                        from: window.from,
                        to: window.to,
                    })
            })
            .collect();
        if !matches.is_empty() {
            observed.insert(path.clone(), Observed { at: *at, matches });
        }
    }
    for path in observed.keys() {
        files_by_path.entry(path.clone()).or_default();
    }

    Provenance {
        files: files_by_path
            .into_iter()
            .map(|(path, by_run)| {
                let mut reports: Vec<Report> = by_run.into_values().collect();
                reports.sort_by_key(|report| report.first_at);
                let observed = observed.remove(&path);
                FileReports {
                    path,
                    reports,
                    observed,
                }
            })
            .collect(),
        runs: runs
            .iter()
            .filter(|run| run.kind == "harness" && run.pty_session_id.is_some())
            // The store lists runs newest first; a reader of coverage wants the story in order.
            .rev()
            .map(|run| {
                // The start event is the word on what the run was asked. Without it — cleared
                // while the run was live, or pruned — a report the agent made through the run
                // says it was asked, and through what, since capture is named by its method.
                let (capture, capture_known) = match capture_of.get(run.id.as_str()) {
                    Some(capture) => (capture.clone(), true),
                    None => match reported_through.get(run.id.as_str()) {
                        Some(method) => (Some((*method).to_owned()), true),
                        None => (None, false),
                    },
                };
                RunCoverage {
                    run_id: run.id.clone(),
                    harness_id: run.harness_id.clone(),
                    started_at: run.started_at,
                    ended_at: run.ended_at,
                    capture,
                    capture_known,
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{NewEvent, NewRun};
    use serde_json::json;

    fn workspace(store: &Store) -> String {
        let project = store.add_project("app", "/code/app").unwrap();
        store
            .add_worktree(&project.id, "fix", "/wt/fix", Some("ys/fix"), Some("main"))
            .unwrap()
            .id
    }

    fn run(
        store: &Store,
        ws: &str,
        id: &str,
        kind: &str,
        harness: Option<&str>,
        capture: Option<&str>,
    ) {
        store
            .add_run(&NewRun {
                id,
                workspace_id: ws,
                session_id: None,
                kind,
                harness_id: harness,
                harness_session_id: None,
                launched_by: "app",
            })
            .unwrap();
        store.run_spawned(id, &format!("pty-{id}")).unwrap();
        event(
            store,
            ws,
            Some(id),
            "yardsort",
            "process.started",
            1_000,
            json!({ "kind": kind, "harnessId": harness, "capture": capture }),
        );
    }

    fn event(
        store: &Store,
        ws: &str,
        run_id: Option<&str>,
        producer: &str,
        kind: &str,
        at: i64,
        payload: Value,
    ) {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!("e-{n}");
        let payload = payload.to_string();
        assert!(store
            .add_event(&NewEvent {
                id: &id,
                schema_version: 1,
                workspace_id: ws,
                session_id: None,
                run_id,
                occurred_at: at,
                kind,
                producer,
                method: if producer == "yardsort" {
                    "lifecycle"
                } else {
                    "hook"
                },
                fidelity: if producer == "yardsort" {
                    "observed"
                } else {
                    "reported"
                },
                source_key: None,
                privacy_class: "metadata",
                payload: &payload,
            })
            .unwrap());
    }

    fn paths(p: &Provenance) -> Vec<&str> {
        p.files.iter().map(|f| f.path.as_str()).collect()
    }

    /// Claude's hooks report a tool call, not a file: only its writing tools count, and only
    /// with a path inside the workspace. Reading a file is not writing it, and a command is
    /// nothing the diff can be joined to.
    #[test]
    fn a_hook_adapter_counts_its_writing_tools_and_nothing_else() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        run(&store, &ws, "r1", "harness", Some("claude"), Some("hook"));
        let tool = |name: &str, path: Option<&str>, at: i64| {
            let mut payload = json!({ "tool": name });
            if let Some(path) = path {
                payload["path"] = json!(path);
            }
            event(
                &store,
                &ws,
                Some("r1"),
                "claude",
                "tool.completed",
                at,
                payload,
            );
        };
        tool("Read", Some("src/lib.rs"), 2_000);
        tool("Write", Some("src/lib.rs"), 3_000);
        tool("Edit", Some("src/lib.rs"), 4_000);
        tool("Bash", None, 5_000);
        event(
            &store,
            &ws,
            Some("r1"),
            "claude",
            "tool.completed",
            6_000,
            json!({ "tool": "Write", "pathOutsideWorkspace": true }),
        );

        let p = of(&store, &ws, &[]).unwrap();
        assert_eq!(paths(&p), ["src/lib.rs"]);
        let report = &p.files[0].reports[0];
        assert_eq!(
            (report.writes, report.first_at, report.last_at),
            (2, 3_000, 4_000),
            "the read is not a write; the write and the edit are two"
        );
        assert_eq!(report.run_id.as_deref(), Some("r1"));
        assert_eq!(report.harness_id.as_deref(), Some("claude"));
        assert_eq!(
            (report.producer.as_str(), report.method.as_str()),
            ("claude", "hook")
        );
    }

    /// OpenCode reports an edit twice — the tool call and the file event — and that is one
    /// write, counted through the contract's own kind. Codex and Cursor have only that kind.
    #[test]
    fn a_reported_write_is_the_contracts_kind_and_a_tool_call_beside_it_is_not_counted_twice() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        run(
            &store,
            &ws,
            "r1",
            "harness",
            Some("opencode"),
            Some("plugin"),
        );
        event(
            &store,
            &ws,
            Some("r1"),
            "opencode",
            "tool.completed",
            2_000,
            json!({ "tool": "edit", "path": "README.md" }),
        );
        event(
            &store,
            &ws,
            Some("r1"),
            "opencode",
            "file.reported_write",
            2_001,
            json!({ "path": "README.md", "kind": "changed" }),
        );
        run(&store, &ws, "r2", "harness", Some("codex"), Some("notify"));
        event(
            &store,
            &ws,
            Some("r2"),
            "codex",
            "file.reported_write",
            9_000,
            json!({ "path": "README.md", "kind": "update" }),
        );

        let p = of(&store, &ws, &[]).unwrap();
        assert_eq!(paths(&p), ["README.md"]);
        let reports = &p.files[0].reports;
        assert_eq!(reports.len(), 2, "one row per run");
        assert_eq!(
            (reports[0].producer.as_str(), reports[0].writes),
            ("opencode", 1)
        );
        assert_eq!(
            (reports[1].producer.as_str(), reports[1].writes),
            ("codex", 1)
        );
        assert!(reports[0].first_at < reports[1].first_at, "oldest first");
    }

    /// Grok's log names the tool and never the file: a `write` from it is not a claim about
    /// any path, and the join says nothing rather than guessing.
    #[test]
    fn a_write_without_a_path_is_no_claim_about_any_file() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        run(
            &store,
            &ws,
            "r1",
            "harness",
            Some("grok"),
            Some("session_file"),
        );
        event(
            &store,
            &ws,
            Some("r1"),
            "grok",
            "tool.completed",
            2_000,
            json!({ "tool": "write", "durationMs": 3 }),
        );
        let p = of(&store, &ws, &[]).unwrap();
        assert!(p.files.is_empty());
        assert_eq!(p.reporting_runs(), 1, "the run did report — just not files");
    }

    /// The pi family reports tool calls like Claude, under its own tool names.
    #[test]
    fn the_pi_family_counts_write_and_edit() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        run(&store, &ws, "r1", "harness", Some("omp"), Some("extension"));
        for (tool, at) in [
            ("read", 2_000),
            ("write", 3_000),
            ("edit", 4_000),
            ("bash", 5_000),
        ] {
            event(
                &store,
                &ws,
                Some("r1"),
                "omp",
                "tool.completed",
                at,
                json!({ "tool": tool, "path": "hello.txt" }),
            );
        }
        let p = of(&store, &ws, &[]).unwrap();
        assert_eq!(p.files[0].reports[0].writes, 2);
    }

    /// Coverage is per run: which agent runs could have reported, and which did not. A shell
    /// cannot report, a run that never spawned never ran, and a run whose capture was refused
    /// is listed with none — that is what tells a reader why a changed file has no report.
    #[test]
    fn coverage_lists_the_agent_runs_and_how_each_was_asked_to_report() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        run(&store, &ws, "r1", "harness", Some("claude"), Some("hook"));
        run(&store, &ws, "r2", "harness", Some("claude"), None);
        run(&store, &ws, "sh", "shell", None, None);
        store
            .add_run(&NewRun {
                id: "never",
                workspace_id: &ws,
                session_id: None,
                kind: "harness",
                harness_id: Some("codex"),
                harness_session_id: None,
                launched_by: "app",
            })
            .unwrap();

        let p = of(&store, &ws, &[]).unwrap();
        let ids: Vec<&str> = p.runs.iter().map(|r| r.run_id.as_str()).collect();
        assert_eq!(ids, ["r1", "r2"]);
        assert_eq!(p.runs[0].capture.as_deref(), Some("hook"));
        assert_eq!(
            (p.runs[1].capture.as_deref(), p.runs[1].capture_known),
            (None, true)
        );
        assert_eq!(p.reporting_runs(), 1);
    }

    /// Codex reports a `FileChange` item whether or not the patch landed, and says which in
    /// `status`. A failed patch against a file changed some other way must not badge that
    /// file as Codex's, nor count as a write.
    #[test]
    fn a_reported_write_that_says_it_failed_is_not_a_write() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        run(&store, &ws, "r1", "harness", Some("codex"), Some("notify"));
        event(
            &store,
            &ws,
            Some("r1"),
            "codex",
            "file.reported_write",
            2_000,
            json!({ "path": "src/lib.rs", "kind": "update", "status": "failed" }),
        );
        assert!(of(&store, &ws, &[]).unwrap().files.is_empty());
        event(
            &store,
            &ws,
            Some("r1"),
            "codex",
            "file.reported_write",
            3_000,
            json!({ "path": "src/lib.rs", "kind": "update", "status": "completed" }),
        );
        let p = of(&store, &ws, &[]).unwrap();
        assert_eq!(paths(&p), ["src/lib.rs"]);
        assert_eq!(
            p.files[0].reports[0].writes, 1,
            "the failed one is not counted"
        );
    }

    /// Clearing a workspace's activity while an agent is live takes its start event and leaves
    /// the run; the reports that follow still arrive. Coverage must not then call that run
    /// silent: what it reported through says it was reporting. A run with neither a start
    /// event nor a report is unknown, not silent.
    #[test]
    fn a_run_whose_start_event_is_gone_is_known_by_what_it_reported_through() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        run(&store, &ws, "r1", "harness", Some("claude"), Some("hook"));
        run(&store, &ws, "r2", "harness", Some("codex"), None);
        store.clear_activity(Some(&ws)).unwrap();
        assert_eq!(store.runs(&ws).unwrap().len(), 2, "clearing keeps the runs");
        event(
            &store,
            &ws,
            Some("r1"),
            "claude",
            "tool.completed",
            5_000,
            json!({ "tool": "Write", "path": "hello.txt" }),
        );

        let p = of(&store, &ws, &[]).unwrap();
        assert_eq!(paths(&p), ["hello.txt"]);
        assert_eq!(
            p.reporting_runs(),
            1,
            "the report says the run was reporting"
        );
        let r1 = p.runs.iter().find(|r| r.run_id == "r1").unwrap();
        assert_eq!(
            (r1.capture.as_deref(), r1.capture_known),
            (Some("hook"), true)
        );
        let r2 = p.runs.iter().find(|r| r.run_id == "r2").unwrap();
        assert_eq!(
            (r2.capture.as_deref(), r2.capture_known),
            (None, false),
            "nothing is known of r2"
        );
    }

    /// A report an adapter could not link to a run still names a file; it is shown without a
    /// run, never attached to whichever run happened to be live.
    #[test]
    fn an_unlinked_report_keeps_its_file_and_has_no_run() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        event(
            &store,
            &ws,
            None,
            "cursor",
            "file.reported_write",
            2_000,
            json!({ "path": "a.ts", "kind": "changed" }),
        );
        let p = of(&store, &ws, &[]).unwrap();
        assert_eq!(paths(&p), ["a.ts"]);
        assert_eq!(p.files[0].reports[0].run_id, None);
        assert_eq!(p.files[0].reports[0].harness_id, None);
    }

    /// Only the kinds that can carry a write are read: the store is asked for those and no
    /// other, so a workspace with a long timeline costs the same as a short one.
    #[test]
    fn only_the_kinds_that_carry_a_write_are_read() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        run(&store, &ws, "r1", "harness", Some("claude"), Some("hook"));
        event(
            &store,
            &ws,
            Some("r1"),
            "claude",
            "turn.completed",
            2_000,
            json!({}),
        );
        event(
            &store,
            &ws,
            Some("r1"),
            "claude",
            "tool.started",
            3_000,
            json!({ "tool": "Write", "path": "x" }),
        );
        let rows = store.events_of_kinds(&ws, &KINDS).unwrap();
        assert_eq!(
            rows.len(),
            2,
            "process.started and the tool's start; not the turn"
        );
        assert!(store.events_of_kinds(&ws, &[]).unwrap().is_empty());
        assert!(
            of(&store, &ws, &[]).unwrap().files.is_empty(),
            "a started tool has not written"
        );
    }

    /// The file an agent made with `echo > hello.txt` has no report, but its clock says when it
    /// was last written, and the Bash call was executing then: observed, named with its tool,
    /// while a write outside every window is nothing at all.
    #[test]
    fn a_file_written_while_a_command_ran_is_observed_with_that_command() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        run(&store, &ws, "r1", "harness", Some("claude"), Some("hook"));
        let bash = |kind: &str, at: i64| {
            event(
                &store,
                &ws,
                Some("r1"),
                "claude",
                kind,
                at,
                json!({ "tool": "Bash", "toolUseId": "t1" }),
            )
        };
        bash("tool.started", 2_000);
        bash("tool.completed", 2_400);

        let p = of(
            &store,
            &ws,
            &[("hello.txt".into(), 2_100), ("mine.txt".into(), 9_000)],
        )
        .unwrap();
        assert_eq!(
            paths(&p),
            ["hello.txt"],
            "mine.txt was written outside every window"
        );
        let file = &p.files[0];
        assert!(file.reports.is_empty(), "nothing was reported");
        let observed = file.observed.as_ref().unwrap();
        assert_eq!(observed.at, 2_100);
        assert_eq!(observed.matches.len(), 1);
        let m = &observed.matches[0];
        assert_eq!(
            (
                m.run_id.as_str(),
                m.harness_id.as_deref(),
                m.tool.as_deref(),
                m.from,
                m.to
            ),
            ("r1", Some("claude"), Some("Bash"), 2_000, 2_400)
        );
    }

    /// A tool that named its own file is a window for that file alone: hello4.txt written by
    /// `Write hello4.txt` is reported, and a write to another file in the same instant is not
    /// laid at that tool's door. A file tool's report and its window meet on the same row.
    #[test]
    fn a_tool_that_named_its_file_is_a_window_for_that_file_only() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        run(&store, &ws, "r1", "harness", Some("claude"), Some("hook"));
        let write = |kind: &str, at: i64| {
            event(
                &store,
                &ws,
                Some("r1"),
                "claude",
                kind,
                at,
                json!({ "tool": "Write", "toolUseId": "t1", "path": "hello4.txt" }),
            )
        };
        write("tool.started", 2_000);
        write("tool.completed", 2_050);

        let files = [
            ("hello4.txt".to_owned(), 2_020),
            ("other.txt".to_owned(), 2_020),
        ];
        let p = of(&store, &ws, &files).unwrap();
        assert_eq!(paths(&p), ["hello4.txt"]);
        let file = &p.files[0];
        assert_eq!(file.reports.len(), 1, "reported");
        assert_eq!(
            file.observed.as_ref().unwrap().matches[0].tool.as_deref(),
            Some("Write"),
            "and observed, in the same row"
        );
    }

    /// Two agents with commands running at once: the observation names both and decides
    /// nothing. A completion with only a duration is a window too (Codex's session file); a
    /// start with no end is open until the run ends; a shell's tools are no window at all.
    #[test]
    fn windows_come_from_pairs_durations_and_open_starts_and_never_from_a_shell() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        run(&store, &ws, "r1", "harness", Some("claude"), Some("hook"));
        run(&store, &ws, "r2", "harness", Some("codex"), Some("notify"));
        run(&store, &ws, "sh", "shell", None, None);
        event(
            &store,
            &ws,
            Some("r1"),
            "claude",
            "tool.started",
            2_000,
            json!({ "tool": "Bash", "toolUseId": "a" }),
        );
        // Codex: a completion with a duration and no start.
        event(
            &store,
            &ws,
            Some("r2"),
            "codex",
            "tool.completed",
            2_500,
            json!({ "tool": "shell", "durationMs": 1_000 }),
        );
        // A shell run's events (none exist today) would be no window: it is not a harness.
        event(
            &store,
            &ws,
            Some("sh"),
            "yardsort",
            "tool.started",
            1_000,
            json!({ "tool": "Bash" }),
        );
        store.end_run("r1", Some(0), "exited").unwrap();

        let ended = store.run("r1").unwrap().unwrap().ended_at.unwrap();
        let p = of(
            &store,
            &ws,
            &[("both.txt".into(), 2_200), ("late.txt".into(), ended + 1)],
        )
        .unwrap();
        assert_eq!(
            paths(&p),
            ["both.txt"],
            "after r1 ended, its open Bash is no window"
        );
        let matches = &p.files[0].observed.as_ref().unwrap().matches;
        let who: Vec<&str> = matches.iter().map(|m| m.run_id.as_str()).collect();
        assert_eq!(
            who,
            ["r2", "r1"],
            "both, by when each began; the shell never"
        );
        assert_eq!(
            (matches[0].from, matches[0].to),
            (1_500, 2_500),
            "from the duration"
        );
    }
}
