//! Agent activity, stage 1: the facts Yardsort itself owns about the processes it starts.
//!
//! A **run** is one process in a workspace's PTY — a harness conversation starting, resuming or
//! forking, a shell, or the project's run command. Its row is written *before* the spawn, so a
//! process that exits before anyone hears of it still has something to be matched to; the PTY
//! id is attached once the spawn returns, and the session record once that exists. **Events**
//! are the facts along the way: started, exited, resumed, forked, or failed to start.
//!
//! Three rules, from [`docs/design/09-agent-events-and-memory.md`]:
//!
//! - **Nothing here reads terminal output.** Every event is something the launcher or the
//!   host already knew: which program, in which workspace, and how it ended.
//! - **Nothing here is on the critical path.** A write that fails is printed, counted under a
//!   diagnostic, and forgotten; the launch goes ahead. See [`Recorder`].
//! - **Delivery is idempotent.** An exit can arrive live from the daemon, from the exit spool
//!   after the window was closed, from settling a record, or from a startup reconciliation; the
//!   unique `(producer, source_key)` makes all of those one row.
//!
//! [`docs/design/09-agent-events-and-memory.md`]: ../../../docs/design/09-agent-events-and-memory.md

use std::path::{Path, PathBuf};

use pty_host::ExitInfo;
use pty_ipc::spool::Spool;
use serde_json::json;

use crate::settings::ActivitySettings;
use crate::store::{NewEvent, NewRun, RunRow, Store, StoreResult};

/// The shape of every event this module writes. Bumped when a payload changes shape.
pub const SCHEMA_VERSION: u16 = 1;
pub const PRODUCER: &str = "yardsort";
pub const METHOD_LIFECYCLE: &str = "lifecycle";
pub const FIDELITY_OBSERVED: &str = "observed";
/// No prompt, no command line, no path beyond ids, no output.
pub const PRIVACY_METADATA: &str = "metadata";

/// Retention: the newest this many events, none older than this.
pub const MAX_EVENTS: usize = 20_000;
pub const MAX_AGE_MS: i64 = 90 * 24 * 60 * 60 * 1000;
/// A run row with no PTY id older than this belongs to a launch that never returned; younger
/// ones may be another client between writing the row and spawning.
pub const PENDING_GRACE_MS: i64 = 60_000;

/// Variables a workspace launch is given, so a program (or, in a later stage, a hook it runs)
/// can say which run it belongs to. Ids only; nothing secret.
pub const RUN_ENV: &str = "YARDSORT_RUN_ID";
pub const WORKSPACE_ENV: &str = "YARDSORT_WORKSPACE_ID";
pub const RECORD_ENV: &str = "YARDSORT_SESSION_RECORD_ID";

/// Where the daemon keeps exits nobody was connected to hear. Under the data directory, so a
/// throwaway profile's activity stays with the profile.
pub fn spool_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("activity").join("spool")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    /// A harness conversation: it has (or will have) a session record.
    Harness,
    /// The user's shell.
    Shell,
    /// Any other program: the project's run command.
    Program,
}

impl RunKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Harness => "harness",
            Self::Shell => "shell",
            Self::Program => "program",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchedBy {
    App,
    Cli,
}

impl LaunchedBy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Cli => "cli",
        }
    }
}

/// How a run relates to a conversation that came before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Continuation {
    Fresh,
    /// The same conversation, in a new process.
    Resumed,
    /// A copy of the conversation `from`, going its own way.
    Forked {
        from: String,
    },
}

impl Continuation {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Resumed => "resumed",
            Self::Forked { .. } => "forked",
        }
    }
}

/// What is known about a run before its process exists.
#[derive(Debug, Clone)]
pub struct RunDraft {
    pub workspace_id: String,
    /// The conversation's record, when it already exists (a resume). A fresh conversation's
    /// record is written after the spawn and linked with [`Recorder::link_session`].
    pub session_id: Option<String>,
    pub kind: RunKind,
    pub harness_id: Option<String>,
    pub harness_session_id: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// The program's name: no directory, no arguments. `shell` for the user's shell.
    pub program: String,
    pub continuation: Continuation,
}

/// The name a program is recorded under: its last path component, without a Windows extension.
pub fn program_name(program: Option<&str>) -> String {
    let Some(program) = program else {
        return "shell".to_owned();
    };
    let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let lower = name.to_ascii_lowercase();
    for ext in [".exe", ".cmd", ".bat"] {
        if let Some(stem) = lower.strip_suffix(ext) {
            return name[..stem.len()].to_owned();
        }
    }
    name.to_owned()
}

/// Records a launch's lifecycle, and never lets a failure out. Every method is best effort: an
/// error is printed, counted under `write_failed`, and swallowed, so the process being
/// launched is never the one to pay for a database problem.
pub struct Recorder<'a> {
    store: &'a Store,
    enabled: bool,
    launched_by: LaunchedBy,
}

impl<'a> Recorder<'a> {
    pub fn new(store: &'a Store, settings: &ActivitySettings, launched_by: LaunchedBy) -> Self {
        Self {
            store,
            enabled: settings.record_lifecycle,
            launched_by,
        }
    }

    /// Write the pending run row. Returns its id, or `None` when recording is off or the write
    /// failed — in which case nothing else about this run is recorded either.
    pub fn begin(&self, draft: &RunDraft) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let id = uuid::Uuid::new_v4().to_string();
        attempt(self.store, "add_run", || {
            self.store.add_run(&NewRun {
                id: &id,
                workspace_id: &draft.workspace_id,
                session_id: draft.session_id.as_deref(),
                kind: draft.kind.as_str(),
                harness_id: draft.harness_id.as_deref(),
                harness_session_id: draft.harness_session_id.as_deref(),
                launched_by: self.launched_by.as_str(),
            })
        })
        .map(|()| id)
    }

    /// The host spawned the process: attach the PTY id and record the start.
    pub fn spawned(&self, run_id: &str, draft: &RunDraft, pty_session_id: &str) {
        attempt(self.store, "run_spawned", || {
            self.store.run_spawned(run_id, pty_session_id)
        });
        let payload = json!({
            "kind": draft.kind.as_str(),
            "harnessId": draft.harness_id,
            "model": draft.model,
            "effort": draft.effort,
            "program": draft.program,
            "continuation": draft.continuation.as_str(),
            "launchedBy": self.launched_by.as_str(),
            "ptySessionId": pty_session_id,
        });
        self.event(
            draft,
            Some(run_id),
            "process.started",
            &format!("run:{run_id}:started"),
            &payload,
        );
        match &draft.continuation {
            Continuation::Fresh => {}
            Continuation::Resumed => self.event(
                draft,
                Some(run_id),
                "session.resumed",
                &format!("run:{run_id}:resumed"),
                &json!({ "sessionId": draft.session_id }),
            ),
            Continuation::Forked { from } => self.event(
                draft,
                Some(run_id),
                "session.forked",
                &format!("run:{run_id}:forked"),
                &json!({ "sessionId": draft.session_id, "fromSessionId": from }),
            ),
        }
    }

    /// The host could not start the program. The run ends here, having never had a PTY.
    pub fn spawn_failed(&self, run_id: &str, draft: &RunDraft, reason: &str) {
        attempt(self.store, "end_run", || {
            self.store.end_run(run_id, None, "spawn_failed")
        });
        let payload = json!({
            "kind": draft.kind.as_str(),
            "harnessId": draft.harness_id,
            "program": draft.program,
            "reason": reason,
        });
        self.event(
            draft,
            Some(run_id),
            "process.spawn_failed",
            &format!("run:{run_id}:spawn_failed"),
            &payload,
        );
    }

    /// A fresh conversation's record now exists.
    pub fn link_session(&self, run_id: &str, session_id: &str) {
        attempt(self.store, "link_run_session", || {
            self.store.link_run_session(run_id, session_id)
        });
    }

    fn event(
        &self,
        draft: &RunDraft,
        run_id: Option<&str>,
        kind: &str,
        source_key: &str,
        payload: &serde_json::Value,
    ) {
        attempt(self.store, kind, || {
            self.store.add_event(&NewEvent {
                id: &uuid::Uuid::new_v4().to_string(),
                schema_version: SCHEMA_VERSION,
                workspace_id: &draft.workspace_id,
                session_id: draft.session_id.as_deref(),
                run_id,
                occurred_at: crate::store::now_ms(),
                kind,
                producer: PRODUCER,
                method: METHOD_LIFECYCLE,
                fidelity: FIDELITY_OBSERVED,
                source_key: Some(source_key),
                privacy_class: PRIVACY_METADATA,
                payload: &payload.to_string(),
            })
        });
    }
}

/// How an exit came to be known. Recorded with the event, because "the daemon told a window"
/// and "found in the spool a day later" are different kinds of evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// The daemon's event, heard by a connected client.
    Live,
    /// Asked of the host right after a launch, in case the process was already gone.
    Settle,
    /// The daemon's exit spool, drained on a later connection.
    Spool,
    /// A run still open at startup whose process the daemon does not have.
    Reconcile,
}

impl Via {
    fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Settle => "settle",
            Self::Spool => "spool",
            Self::Reconcile => "reconcile",
        }
    }
}

/// An exit as the host reported it, or the absence of one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitFacts {
    /// `None` when the process was not seen to exit: the run was interrupted.
    pub code: Option<i64>,
    pub success: bool,
    pub signal: Option<String>,
}

impl From<&ExitInfo> for ExitFacts {
    fn from(exit: &ExitInfo) -> Self {
        Self {
            code: Some(i64::from(exit.code)),
            success: exit.success,
            signal: exit.signal.clone(),
        }
    }
}

/// What recording an exit did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    /// The run is now ended and the event written.
    New,
    /// The run had ended already, from an earlier delivery of the same exit.
    Duplicate,
    /// No run is recorded for that PTY: recording was off, or it was not a workspace launch.
    NoRun,
}

/// The process behind a PTY session ended. Ends its run and records the exit, once, whichever
/// way the news arrived. Best effort, like everything here.
pub fn record_exit(store: &Store, pty_session_id: &str, exit: &ExitFacts, via: Via) -> Recorded {
    let reason = if exit.code.is_some() {
        "exited"
    } else {
        "interrupted"
    };
    let ended = attempt(store, "end_run_by_pty", || {
        store.end_run_by_pty(pty_session_id, exit.code, reason)
    });
    match ended {
        Some(Some(run)) => {
            write_exit(store, &run, exit, reason, via);
            Recorded::New
        }
        Some(None) => match store.run_by_pty(pty_session_id) {
            Ok(Some(_)) => {
                let _ = store.bump_diagnostic("duplicate_exit", Some(via.as_str()));
                Recorded::Duplicate
            }
            _ => Recorded::NoRun,
        },
        None => Recorded::NoRun,
    }
}

fn write_exit(store: &Store, run: &RunRow, exit: &ExitFacts, reason: &str, via: Via) {
    let source_key = match &run.pty_session_id {
        Some(pty) => format!("pty:{pty}:exited"),
        None => format!("run:{}:exited", run.id),
    };
    let payload = json!({
        "exitCode": exit.code,
        "success": exit.success,
        "signal": exit.signal,
        "reason": reason,
        "via": via.as_str(),
    });
    attempt(store, "process.exited", || {
        store.add_event(&NewEvent {
            id: &uuid::Uuid::new_v4().to_string(),
            schema_version: SCHEMA_VERSION,
            workspace_id: &run.workspace_id,
            session_id: run.session_id.as_deref(),
            run_id: Some(&run.id),
            occurred_at: crate::store::now_ms(),
            kind: "process.exited",
            producer: PRODUCER,
            method: METHOD_LIFECYCLE,
            fidelity: FIDELITY_OBSERVED,
            source_key: Some(&source_key),
            privacy_class: PRIVACY_METADATA,
            payload: &payload.to_string(),
        })
    });
}

/// End every run whose process is not among `alive` — the PTY sessions the daemon is actually
/// running — as interrupted. Call this *after* [`import_spool`], so an exit the daemon kept is
/// recorded with its code rather than as a loss. Returns how many runs were ended.
pub fn end_interrupted(store: &Store, alive: &[String]) -> usize {
    let Some(gone) = attempt(store, "end_interrupted_runs", || {
        store.end_interrupted_runs(alive, PENDING_GRACE_MS)
    }) else {
        return 0;
    };
    let interrupted = ExitFacts {
        code: None,
        success: false,
        signal: None,
    };
    for run in &gone {
        write_exit(store, run, &interrupted, "interrupted", Via::Reconcile);
    }
    gone.len()
}

/// What draining the spool found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Exits recorded for the first time.
    pub imported: usize,
    /// Exits already known — heard live, or imported by another client.
    pub duplicates: usize,
    /// Entries for sessions no run is recorded for.
    pub unmatched: usize,
    /// Files that could not be read; they are removed and counted.
    pub unreadable: usize,
    /// Exits the daemon had to drop because the spool was full.
    pub dropped: u64,
}

/// Drain the daemon's exit spool into the store. Each entry settles the run *and* the session
/// record it belonged to — a conversation that ended while the window was closed gets its real
/// exit code instead of "interrupted" — and the file is then removed. Safe to call from any
/// client at any time; two draining at once make duplicates, not damage.
pub fn import_spool(store: &Store, data_dir: &Path) -> ImportReport {
    let spool = Spool::new(spool_dir(data_dir));
    let mut report = ImportReport::default();
    let entries = match spool.entries() {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("could not read the exit spool: {error}");
            let _ = store.bump_diagnostic("spool_unreadable", Some(&error.to_string()));
            return report;
        }
    };
    for path in entries {
        let entry = match Spool::read(&path) {
            Ok(entry) => entry,
            Err(error) => {
                eprintln!(
                    "dropping unreadable spool entry {}: {error}",
                    path.display()
                );
                let _ = store.bump_diagnostic("spool_unreadable", Some(&error.to_string()));
                report.unreadable += 1;
                let _ = std::fs::remove_file(&path);
                continue;
            }
        };
        let pty = entry.session.0.as_str();
        // The session record first, as the app's live handler does: it is what Resume reads.
        let _ = store.end_session_by_pty(pty, Some(i64::from(entry.exit.code)));
        match record_exit(store, pty, &ExitFacts::from(&entry.exit), Via::Spool) {
            Recorded::New => report.imported += 1,
            Recorded::Duplicate => report.duplicates += 1,
            Recorded::NoRun => report.unmatched += 1,
        }
        // Only once it is in the database; a removal that fails is re-read next time and
        // counted as a duplicate, which is the cheap kind of mistake.
        if let Err(error) = std::fs::remove_file(&path) {
            eprintln!("could not remove spool entry {}: {error}", path.display());
        }
    }
    report.dropped = spool.take_dropped();
    if report.dropped > 0 {
        let _ = store.bump_diagnostic("spool_dropped", Some(&report.dropped.to_string()));
    }
    report
}

/// Keep the tables bounded. Returns how many events were removed.
pub fn prune(store: &Store) -> usize {
    let removed = attempt(store, "prune_activity", || {
        store.prune_activity(MAX_EVENTS, MAX_AGE_MS)
    })
    .unwrap_or(0);
    if removed > 0 {
        let _ = store.bump_diagnostic("pruned", Some(&removed.to_string()));
    }
    removed
}

/// Run one store operation, and turn a failure into a printed line and a counter.
fn attempt<T>(store: &Store, what: &str, op: impl FnOnce() -> StoreResult<T>) -> Option<T> {
    match op() {
        Ok(value) => Some(value),
        Err(error) => {
            eprintln!("activity: {what} failed: {error}");
            // The diagnostics table may be the one thing still writable; if not, so be it.
            let _ = store.bump_diagnostic("write_failed", Some(&format!("{what}: {error}")));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::NewSession;
    use pty_host::SessionId;
    use pty_ipc::spool::{SpoolEntry, SPOOL_VERSION};

    fn workspace(store: &Store) -> String {
        let project = store.add_project("app", "/code/app").unwrap();
        store
            .add_worktree(&project.id, "fix", "/wt/fix", Some("ys/fix"), Some("main"))
            .unwrap()
            .id
    }

    fn harness_draft(workspace_id: &str) -> RunDraft {
        RunDraft {
            workspace_id: workspace_id.to_owned(),
            session_id: None,
            kind: RunKind::Harness,
            harness_id: Some("claude".into()),
            harness_session_id: Some("h-1".into()),
            model: Some("opus".into()),
            effort: None,
            program: "claude".into(),
            continuation: Continuation::Fresh,
        }
    }

    fn recorder(store: &Store) -> Recorder<'_> {
        Recorder {
            store,
            enabled: true,
            launched_by: LaunchedBy::App,
        }
    }

    fn kinds(store: &Store, workspace_id: &str) -> Vec<String> {
        let mut events = store.events(workspace_id, None, 100).unwrap();
        events.reverse();
        events.into_iter().map(|e| e.kind).collect()
    }

    fn payload(event: &crate::store::EventRow) -> serde_json::Value {
        serde_json::from_str(&event.payload).unwrap()
    }

    #[test]
    fn a_program_is_named_without_its_directory_or_extension() {
        assert_eq!(program_name(None), "shell");
        assert_eq!(program_name(Some("/usr/bin/claude")), "claude");
        assert_eq!(
            program_name(Some(r"C:\Users\x\AppData\npm\codex.CMD")),
            "codex"
        );
        assert_eq!(program_name(Some("cmd.exe")), "cmd");
        assert_eq!(program_name(Some("omp")), "omp");
    }

    #[test]
    fn a_run_is_pending_before_the_spawn_and_linked_to_its_record_after() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        let recorder = recorder(&store);
        let draft = harness_draft(&ws);

        let run_id = recorder.begin(&draft).unwrap();
        let pending = store.run(&run_id).unwrap().unwrap();
        assert_eq!((pending.pty_session_id, pending.session_id), (None, None));
        assert!(kinds(&store, &ws).is_empty(), "nothing has happened yet");

        recorder.spawned(&run_id, &draft, "pty-1");
        store
            .add_session(&NewSession {
                id: "rec-1",
                workspace_id: &ws,
                harness_id: "claude",
                title: "",
                pty_session_id: "pty-1",
                ..Default::default()
            })
            .unwrap();
        recorder.link_session(&run_id, "rec-1");

        let run = store.run(&run_id).unwrap().unwrap();
        assert_eq!(run.pty_session_id.as_deref(), Some("pty-1"));
        assert_eq!(run.session_id.as_deref(), Some("rec-1"));
        assert_eq!(run.launched_by, "app");
        assert_eq!(kinds(&store, &ws), ["process.started"]);
        let started = &store.events(&ws, None, 1).unwrap()[0];
        assert_eq!(
            started.session_id.as_deref(),
            Some("rec-1"),
            "the event written before the record existed is linked too"
        );
        let body = payload(started);
        assert_eq!(body["harnessId"], "claude");
        assert_eq!(body["continuation"], "fresh");
        assert_eq!(body["launchedBy"], "app");
        assert!(
            started.payload.contains("opus") && !started.payload.contains("fix the"),
            "model yes, prompt never"
        );
    }

    #[test]
    fn an_exit_is_recorded_once_however_many_times_it_is_delivered() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        let recorder = recorder(&store);
        let draft = harness_draft(&ws);
        let run_id = recorder.begin(&draft).unwrap();
        recorder.spawned(&run_id, &draft, "pty-1");

        let exit = ExitFacts {
            code: Some(3),
            success: false,
            signal: None,
        };
        assert_eq!(
            record_exit(&store, "pty-1", &exit, Via::Live),
            Recorded::New
        );
        assert_eq!(
            record_exit(&store, "pty-1", &exit, Via::Settle),
            Recorded::Duplicate
        );
        assert_eq!(
            record_exit(&store, "pty-1", &exit, Via::Spool),
            Recorded::Duplicate
        );
        assert_eq!(
            record_exit(&store, "pty-unknown", &exit, Via::Live),
            Recorded::NoRun
        );

        let run = store.run(&run_id).unwrap().unwrap();
        assert_eq!(
            (run.exit_code, run.end_reason.as_deref()),
            (Some(3), Some("exited"))
        );
        assert_eq!(kinds(&store, &ws), ["process.started", "process.exited"]);
        let exited = &store.events(&ws, None, 1).unwrap()[0];
        assert_eq!(payload(exited)["via"], "live", "the first delivery wins");
        let duplicates = store
            .diagnostics()
            .unwrap()
            .into_iter()
            .find(|d| d.name == "duplicate_exit")
            .unwrap();
        assert_eq!(duplicates.count, 2);
    }

    #[test]
    fn a_spawn_that_fails_ends_the_run_without_a_pty() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        let recorder = recorder(&store);
        let draft = harness_draft(&ws);
        let run_id = recorder.begin(&draft).unwrap();
        recorder.spawn_failed(&run_id, &draft, "`claude` was not found on PATH");

        let run = store.run(&run_id).unwrap().unwrap();
        assert_eq!(run.end_reason.as_deref(), Some("spawn_failed"));
        assert!(run.pty_session_id.is_none() && run.ended_at.is_some());
        assert_eq!(kinds(&store, &ws), ["process.spawn_failed"]);
        assert!(store.events(&ws, None, 1).unwrap()[0]
            .payload
            .contains("not found"));
    }

    #[test]
    fn resumes_and_forks_say_which_conversation_they_continue() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        store
            .add_session(&NewSession {
                id: "rec-1",
                workspace_id: &ws,
                harness_id: "claude",
                title: "",
                pty_session_id: "pty-1",
                ..Default::default()
            })
            .unwrap();
        let recorder = recorder(&store);

        let resume = RunDraft {
            session_id: Some("rec-1".into()),
            continuation: Continuation::Resumed,
            ..harness_draft(&ws)
        };
        let run = recorder.begin(&resume).unwrap();
        recorder.spawned(&run, &resume, "pty-2");
        assert_eq!(
            store.run(&run).unwrap().unwrap().session_id.as_deref(),
            Some("rec-1")
        );

        let fork = RunDraft {
            continuation: Continuation::Forked {
                from: "rec-1".into(),
            },
            ..harness_draft(&ws)
        };
        let run = recorder.begin(&fork).unwrap();
        recorder.spawned(&run, &fork, "pty-3");

        assert_eq!(
            kinds(&store, &ws),
            [
                "process.started",
                "session.resumed",
                "process.started",
                "session.forked"
            ]
        );
        let forked = &store.events(&ws, None, 1).unwrap()[0];
        assert_eq!(payload(forked)["fromSessionId"], "rec-1");
    }

    #[test]
    fn recording_can_be_switched_off_and_then_writes_nothing() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        let off = Recorder::new(
            &store,
            &ActivitySettings {
                record_lifecycle: false,
                show_timeline: false,
            },
            LaunchedBy::Cli,
        );
        assert_eq!(off.begin(&harness_draft(&ws)), None);
        assert_eq!(store.activity_counts().unwrap(), (0, 0));
        let exit = ExitFacts {
            code: Some(0),
            success: true,
            signal: None,
        };
        assert_eq!(
            record_exit(&store, "pty-1", &exit, Via::Live),
            Recorded::NoRun
        );
    }

    /// The point of the whole module: a database that cannot take the row must not take the
    /// launch down with it. The caller gets `None` and carries on; the failure is counted.
    #[test]
    fn a_broken_table_is_counted_and_swallowed() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        store.break_activity_tables();
        let recorder = recorder(&store);
        assert_eq!(recorder.begin(&harness_draft(&ws)), None);
        let exit = ExitFacts {
            code: Some(0),
            success: true,
            signal: None,
        };
        assert_eq!(
            record_exit(&store, "pty-1", &exit, Via::Live),
            Recorded::NoRun
        );
        let failed = store
            .diagnostics()
            .unwrap()
            .into_iter()
            .find(|d| d.name == "write_failed")
            .expect("counted");
        assert!(failed.count >= 2, "{failed:?}");
        assert!(failed.last_detail.unwrap().contains("no such table"));
    }

    fn spool_entry(dir: &Path, at: u64, pty: &str, code: u32) {
        let spool = Spool::new(spool_dir(dir));
        spool
            .record(&SpoolEntry {
                version: SPOOL_VERSION,
                at_ms: at,
                daemon_pid: 1,
                session: SessionId(pty.into()),
                labels: Default::default(),
                exit: ExitInfo {
                    code,
                    success: code == 0,
                    signal: None,
                },
            })
            .unwrap();
    }

    /// The window was closed; the agent finished; the daemon spooled the exit and went away.
    /// The next start must find a clean exit with its code, not an interruption.
    #[test]
    fn a_spooled_exit_settles_the_run_and_the_record_and_is_then_gone() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let ws = workspace(&store);
        let recorder = recorder(&store);
        let draft = harness_draft(&ws);
        let run_id = recorder.begin(&draft).unwrap();
        recorder.spawned(&run_id, &draft, "pty-1");
        store
            .add_session(&NewSession {
                id: "rec-1",
                workspace_id: &ws,
                harness_id: "claude",
                title: "",
                pty_session_id: "pty-1",
                ..Default::default()
            })
            .unwrap();
        recorder.link_session(&run_id, "rec-1");

        spool_entry(dir.path(), 10, "pty-1", 0);
        spool_entry(dir.path(), 11, "pty-nobody-knows", 1);
        std::fs::write(spool_dir(dir.path()).join("12-junk.json"), b"{").unwrap();

        let report = import_spool(&store, dir.path());
        assert_eq!(
            report,
            ImportReport {
                imported: 1,
                duplicates: 0,
                unmatched: 1,
                unreadable: 1,
                dropped: 0
            }
        );
        let run = store.run(&run_id).unwrap().unwrap();
        assert_eq!(
            (run.exit_code, run.end_reason.as_deref()),
            (Some(0), Some("exited"))
        );
        let record = store.session("rec-1").unwrap().unwrap();
        assert!(!record.running);
        assert_eq!(record.exit_code, Some(0), "not interrupted: it exited");
        assert!(
            Spool::new(spool_dir(dir.path()))
                .entries()
                .unwrap()
                .is_empty(),
            "drained"
        );

        // Reconciling afterwards has nothing left to interrupt.
        assert_eq!(end_interrupted(&store, &[]), 0);
        assert_eq!(store.end_interrupted_sessions(&[]).unwrap(), 0);

        // The same exit turning up again is a duplicate, not a second event.
        spool_entry(dir.path(), 13, "pty-1", 0);
        assert_eq!(import_spool(&store, dir.path()).duplicates, 1);
        assert_eq!(kinds(&store, &ws), ["process.started", "process.exited"]);
    }

    #[test]
    fn runs_whose_process_is_gone_are_interrupted_and_young_pending_ones_are_left_alone() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        let recorder = recorder(&store);
        let draft = harness_draft(&ws);
        let dead = recorder.begin(&draft).unwrap();
        recorder.spawned(&dead, &draft, "pty-dead");
        let alive = recorder.begin(&draft).unwrap();
        recorder.spawned(&alive, &draft, "pty-alive");
        let pending = recorder.begin(&draft).unwrap();

        assert_eq!(end_interrupted(&store, &["pty-alive".to_owned()]), 1);
        let run = store.run(&dead).unwrap().unwrap();
        assert_eq!(
            (run.exit_code, run.end_reason.as_deref()),
            (None, Some("interrupted"))
        );
        assert!(store.run(&alive).unwrap().unwrap().ended_at.is_none());
        assert!(
            store.run(&pending).unwrap().unwrap().ended_at.is_none(),
            "somebody may be spawning it right now"
        );
        let exited = store
            .events(&ws, None, 10)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == "process.exited")
            .unwrap();
        assert_eq!(payload(&exited)["reason"], "interrupted");
        assert_eq!(payload(&exited)["via"], "reconcile");
    }

    #[test]
    fn pruning_keeps_the_newest_and_drops_runs_left_without_events() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        let recorder = recorder(&store);
        let mut runs = Vec::new();
        for n in 0..5 {
            let draft = harness_draft(&ws);
            let id = recorder.begin(&draft).unwrap();
            recorder.spawned(&id, &draft, &format!("pty-{n}"));
            let exit = ExitFacts {
                code: Some(0),
                success: true,
                signal: None,
            };
            record_exit(&store, &format!("pty-{n}"), &exit, Via::Live);
            runs.push(id);
        }
        assert_eq!(store.activity_counts().unwrap(), (10, 5));

        assert_eq!(store.prune_activity(4, MAX_AGE_MS).unwrap(), 6);
        assert_eq!(store.activity_counts().unwrap(), (4, 2));
        assert!(
            store.run(&runs[0]).unwrap().is_none(),
            "no events left for it"
        );
        assert!(store.run(&runs[4]).unwrap().is_some());

        // A negative age: even a row written this very millisecond is older than "now + 1".
        assert_eq!(
            store.prune_activity(100, -1).unwrap(),
            4,
            "everything is too old"
        );
        assert_eq!(store.activity_counts().unwrap(), (0, 0));
    }

    #[test]
    fn clearing_forgets_ended_runs_but_never_an_open_one() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        let recorder = recorder(&store);
        let draft = harness_draft(&ws);
        let open = recorder.begin(&draft).unwrap();
        recorder.spawned(&open, &draft, "pty-open");
        let done = recorder.begin(&draft).unwrap();
        recorder.spawned(&done, &draft, "pty-done");
        let exit = ExitFacts {
            code: Some(0),
            success: true,
            signal: None,
        };
        record_exit(&store, "pty-done", &exit, Via::Live);

        assert_eq!(store.clear_activity(Some(&ws)).unwrap(), 3);
        assert!(store.run(&done).unwrap().is_none());
        assert!(
            store.run(&open).unwrap().is_some(),
            "its exit still has to be matched"
        );
        assert_eq!(
            record_exit(&store, "pty-open", &exit, Via::Live),
            Recorded::New
        );
    }

    #[test]
    fn activity_goes_with_its_workspace_and_survives_a_forgotten_record() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        let recorder = recorder(&store);
        let draft = harness_draft(&ws);
        let run = recorder.begin(&draft).unwrap();
        recorder.spawned(&run, &draft, "pty-1");
        store
            .add_session(&NewSession {
                id: "rec-1",
                workspace_id: &ws,
                harness_id: "claude",
                title: "",
                pty_session_id: "pty-1",
                ..Default::default()
            })
            .unwrap();
        recorder.link_session(&run, "rec-1");

        assert!(store.remove_session("rec-1").unwrap());
        let kept = store.run(&run).unwrap().unwrap();
        assert_eq!(kept.session_id, None, "the link is cleared, the run stays");
        assert_eq!(store.events(&ws, None, 10).unwrap()[0].session_id, None);

        assert!(store.remove_worktree(&ws).unwrap());
        assert!(store.run(&run).unwrap().is_none());
        assert_eq!(store.activity_counts().unwrap(), (0, 0));
    }

    #[test]
    fn the_timeline_pages_newest_first() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        let recorder = recorder(&store);
        for n in 0..6 {
            let draft = harness_draft(&ws);
            let id = recorder.begin(&draft).unwrap();
            recorder.spawned(&id, &draft, &format!("pty-{n}"));
        }
        let first = store.events(&ws, None, 4).unwrap();
        assert_eq!(first.len(), 4);
        assert!(first[0].seq > first[3].seq);
        let rest = store.events(&ws, Some(first[3].seq), 4).unwrap();
        assert_eq!(rest.len(), 2);
        assert!(rest[0].seq < first[3].seq);
        assert_eq!(store.all_events(Some(&ws)).unwrap().len(), 6);
        assert_eq!(
            store.all_events(None).unwrap()[0].seq,
            rest[1].seq,
            "oldest first"
        );
    }
}
