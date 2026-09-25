//! Stage 3 of agent events: which of a workspace's files an agent *said* it wrote.
//!
//! Git shows what changed; the timeline holds what the agents reported. This joins the two at
//! read time, per path, and says no more than either side does: a file with a report was
//! named by that agent's own hooks or session file as one it wrote, and a file without one
//! changed with no such report — the user, a script, or an agent whose capture was off. Which
//! lines of a diff came from which report is never claimed: git shows the sum of every change
//! since the last commit, and no adapter records a line.
//!
//! Nothing here is stored. A `workspace.changed` event, which the stage 1 report deferred to
//! this stage, is still not recorded: the join needs git's answer at the moment of reading, and
//! a row per file-system signal would only restate the Changes panel in the store.

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

/// Every report for one workspace-relative path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileReports {
    pub path: String,
    /// Oldest first.
    pub reports: Vec<Report>,
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

/// The event kinds a write is reported through. Everything else is read for nothing.
const KINDS: [&str; 3] = ["file.reported_write", "tool.completed", "process.started"];

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
pub fn of(store: &Store, workspace_id: &str) -> StoreResult<Provenance> {
    let runs = store.runs(workspace_id)?;
    let events = store.events_of_kinds(workspace_id, &KINDS)?;
    let native = store.native_methods_by_run(workspace_id)?;
    Ok(join(&runs, &events, &native))
}

/// The pure part: reports by path from the events, coverage from the runs. `native` is which
/// runs have events of the agent's own, and through what — the fallback for a run whose start
/// event is gone.
pub fn join(
    runs: &[crate::store::RunRow],
    events: &[EventRow],
    native: &[(String, String)],
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
    let mut files: BTreeMap<String, BTreeMap<(Option<String>, String), Report>> = BTreeMap::new();

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
        let report = files
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

    Provenance {
        files: files
            .into_iter()
            .map(|(path, by_run)| {
                let mut reports: Vec<Report> = by_run.into_values().collect();
                reports.sort_by_key(|report| report.first_at);
                FileReports { path, reports }
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

        let p = of(&store, &ws).unwrap();
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

        let p = of(&store, &ws).unwrap();
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
        let p = of(&store, &ws).unwrap();
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
        let p = of(&store, &ws).unwrap();
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

        let p = of(&store, &ws).unwrap();
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
        assert!(of(&store, &ws).unwrap().files.is_empty());
        event(
            &store,
            &ws,
            Some("r1"),
            "codex",
            "file.reported_write",
            3_000,
            json!({ "path": "src/lib.rs", "kind": "update", "status": "completed" }),
        );
        let p = of(&store, &ws).unwrap();
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

        let p = of(&store, &ws).unwrap();
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
        let p = of(&store, &ws).unwrap();
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
        assert_eq!(rows.len(), 1, "process.started only");
        assert!(store.events_of_kinds(&ws, &[]).unwrap().is_empty());
        assert!(
            of(&store, &ws).unwrap().files.is_empty(),
            "a started tool has not written"
        );
    }
}
