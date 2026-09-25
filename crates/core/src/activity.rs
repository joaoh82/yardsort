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

pub mod claude;
pub mod codex;
pub mod hook;
pub mod inbox;

use inbox::{Inbox, InboxEntry};

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

/// Where an agent's hooks leave what they report, for the next client to take in. See
/// [`inbox`].
pub fn inbox_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("activity").join("inbox")
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

    /// The host spawned the process: attach the PTY id and record the start. `capture` names
    /// the native source this run was given — `hook`, for a Claude Code launch with hooks — so
    /// a reader of the timeline can tell a run that could report from one that could not.
    pub fn spawned(
        &self,
        run_id: &str,
        draft: &RunDraft,
        pty_session_id: &str,
        capture: Option<&str>,
    ) {
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
            "capture": capture,
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
    /// When it ended, on the clock of whoever saw it: the daemon's, for an exit it kept while
    /// no window was open. `None` means now — the exit is being seen as it happens.
    pub at: Option<i64>,
}

impl From<&ExitInfo> for ExitFacts {
    fn from(exit: &ExitInfo) -> Self {
        Self {
            code: Some(i64::from(exit.code)),
            success: exit.success,
            signal: exit.signal.clone(),
            at: None,
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
    let at = exit.at.unwrap_or_else(crate::store::now_ms);
    let ended = attempt(store, "end_run_by_pty", || {
        store.end_run_by_pty(pty_session_id, exit.code, reason, at)
    });
    match ended {
        Some(Some(run)) => {
            write_exit(store, &run, exit, reason, via, at);
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

fn write_exit(
    store: &Store,
    run: &RunRow,
    exit: &ExitFacts,
    reason: &str,
    via: Via,
    occurred_at: i64,
) {
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
            occurred_at,
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
        at: None,
    };
    let now = crate::store::now_ms();
    for run in &gone {
        write_exit(store, run, &interrupted, "interrupted", Via::Reconcile, now);
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
        // The daemon's clock: this may be hours after the fact, and the event must not say the
        // agent finished the moment somebody looked.
        let at = i64::try_from(entry.at_ms).unwrap_or(i64::MAX);
        let facts = ExitFacts {
            at: Some(at),
            ..ExitFacts::from(&entry.exit)
        };
        // The session record first, as the app's live handler does: it is what Resume reads.
        let _ = store.end_session_by_pty_at(pty, facts.code, at);
        let mut recorded = record_exit(store, pty, &facts, Via::Spool);
        if recorded == Recorded::NoRun {
            // A process can end before its launcher has attached the PTY id to the run it wrote
            // beforehand — a shell that exits at once, say — and a client draining the spool in
            // that gap would otherwise throw the only copy of the exit away. The entry names
            // the run, so the link is made here instead, and the launcher finds it done.
            recorded = adopt_early_exit(store, &entry.labels, pty, &facts);
        }
        match recorded {
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

/// A spooled exit for a PTY no run has yet: if its labels name a run that is still waiting for
/// its PTY id, attach the id and record the exit against it.
fn adopt_early_exit(
    store: &Store,
    labels: &std::collections::BTreeMap<String, String>,
    pty_session_id: &str,
    exit: &ExitFacts,
) -> Recorded {
    let Some(run_id) = labels.get(crate::launch::RUN_LABEL) else {
        return Recorded::NoRun;
    };
    let pending = match store.run(run_id) {
        Ok(Some(run)) if run.pty_session_id.is_none() && run.ended_at.is_none() => run,
        _ => return Recorded::NoRun,
    };
    if attempt(store, "run_spawned", || {
        store.run_spawned(&pending.id, pty_session_id)
    })
    .is_none()
    {
        return Recorded::NoRun;
    }
    record_exit(store, pty_session_id, exit, Via::Spool)
}

/// What draining the inbox found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InboxReport {
    /// Events recorded for the first time.
    pub imported: usize,
    /// Entries already in the database: another client drained the same file first.
    pub duplicates: usize,
    /// Entries naming no run and no workspace this database knows. Counted, then dropped —
    /// an event that cannot be placed is not shown somewhere it might not belong.
    pub unlinked: usize,
    /// Files that could not be read; they are removed and counted.
    pub unreadable: usize,
    /// Entries a hook had to drop because the inbox was full.
    pub dropped: u64,
    /// The workspaces that gained events, for whoever is showing one.
    pub workspaces: std::collections::BTreeSet<String>,
}

/// Drain the agents' inbox into the store: every event a hook left is linked to its run and
/// written once, and the file removed. Safe from any client at any time; two draining at once
/// make duplicates, not damage. An entry the database refuses is left for the next drain.
pub fn import_inbox(store: &Store, data_dir: &Path) -> InboxReport {
    let inbox = Inbox::new(inbox_dir(data_dir));
    let mut report = InboxReport::default();
    let entries = match inbox.entries() {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("could not read the activity inbox: {error}");
            let _ = store.bump_diagnostic("inbox_unreadable", Some(&error.to_string()));
            return report;
        }
    };
    for path in entries {
        let entry = match Inbox::read(&path) {
            Ok(entry) => entry,
            Err(error) => {
                eprintln!(
                    "dropping unreadable inbox entry {}: {error}",
                    path.display()
                );
                let _ = store.bump_diagnostic("inbox_unreadable", Some(&error.to_string()));
                report.unreadable += 1;
                let _ = std::fs::remove_file(&path);
                continue;
            }
        };
        match link(store, &entry) {
            // A Codex trigger stands for a turn the session file describes; read it from there.
            Some(link)
                if entry.producer == codex::PRODUCER && entry.kind == codex::TRIGGER_KIND =>
            {
                if !import_codex_turn(store, &entry, &link, &mut report) {
                    continue;
                }
            }
            Some(link) => {
                let source_key = format!("inbox:{}", entry.id);
                let payload = entry.payload.to_string();
                let added = attempt(store, &entry.kind, || {
                    store.add_event(&NewEvent {
                        id: &entry.id,
                        schema_version: SCHEMA_VERSION,
                        workspace_id: &link.workspace_id,
                        session_id: link.session_id.as_deref(),
                        run_id: link.run_id.as_deref(),
                        occurred_at: entry.at_ms,
                        kind: &entry.kind,
                        producer: &entry.producer,
                        method: &entry.method,
                        fidelity: &entry.fidelity,
                        source_key: Some(&source_key),
                        privacy_class: &entry.privacy_class,
                        payload: &payload,
                    })
                });
                match added {
                    Some(true) => {
                        report.imported += 1;
                        report.workspaces.insert(link.workspace_id);
                    }
                    Some(false) => report.duplicates += 1,
                    // The database would not take it now; the file stays for the next drain.
                    None => continue,
                }
            }
            None => {
                report.unlinked += 1;
                let _ = store.bump_diagnostic("inbox_unlinked", Some(&entry.kind));
            }
        }
        if let Err(error) = std::fs::remove_file(&path) {
            eprintln!("could not remove inbox entry {}: {error}", path.display());
        }
    }
    report.dropped = inbox.take_dropped();
    if report.dropped > 0 {
        let _ = store.bump_diagnostic("inbox_dropped", Some(&report.dropped.to_string()));
    }
    report
}

/// Expand a Codex turn trigger into the turn's events. Returns whether the trigger's file is
/// done with: a turn the session file has not finished writing is left for the next drain,
/// for a while; after that, or when the file cannot be read at all, the turn is recorded from
/// the trigger alone and the shortfall counted.
fn import_codex_turn(
    store: &Store,
    entry: &InboxEntry,
    link: &Link,
    report: &mut InboxReport,
) -> bool {
    let (events, method) = match codex::expand(entry, link.run_id.as_deref()) {
        Ok(codex::Expanded::Events(events)) => (events, codex::METHOD_SESSION_FILE),
        Ok(codex::Expanded::NotYet(why)) => {
            if crate::store::now_ms() - entry.at_ms < codex::NOT_READY_GRACE_MS {
                return false;
            }
            let _ = store.bump_diagnostic("codex_turn_incomplete", Some(&why));
            (vec![codex::fallback(entry)], codex::METHOD_NOTIFY)
        }
        Err(why) => {
            eprintln!("activity: codex session file: {why}");
            let _ = store.bump_diagnostic("codex_session_file", Some(&why));
            (vec![codex::fallback(entry)], codex::METHOD_NOTIFY)
        }
    };
    let mut any_new = false;
    for event in events {
        let payload = event.payload.to_string();
        let added = attempt(store, event.kind, || {
            store.add_event(&NewEvent {
                id: &uuid::Uuid::new_v4().to_string(),
                schema_version: SCHEMA_VERSION,
                workspace_id: &link.workspace_id,
                session_id: link.session_id.as_deref(),
                run_id: link.run_id.as_deref(),
                occurred_at: event.occurred_at,
                kind: event.kind,
                producer: codex::PRODUCER,
                method,
                fidelity: codex::FIDELITY,
                source_key: Some(&event.source_key),
                privacy_class: PRIVACY_METADATA,
                payload: &payload,
            })
        });
        match added {
            Some(true) => {
                report.imported += 1;
                any_new = true;
            }
            Some(false) => report.duplicates += 1,
            None => return false,
        }
    }
    if any_new {
        report.workspaces.insert(link.workspace_id.clone());
    }
    true
}

/// Where a reported event belongs.
struct Link {
    workspace_id: String,
    session_id: Option<String>,
    run_id: Option<String>,
}

/// Place an entry: by the run the launcher named, if its environment reached the hook; else by
/// the agent's own session id, which the launcher recorded when it chose it; else by the
/// workspace alone. Never by a working directory — a path is not an identity.
fn link(store: &Store, entry: &InboxEntry) -> Option<Link> {
    let record = entry
        .session_record_id
        .as_deref()
        .filter(|id| matches!(store.session(id), Ok(Some(_))))
        .map(str::to_owned);
    let from_run = |run: RunRow| Link {
        workspace_id: run.workspace_id,
        session_id: run.session_id.or_else(|| record.clone()),
        run_id: Some(run.id),
    };
    if let Some(run_id) = &entry.run_id {
        if let Ok(Some(run)) = store.run(run_id) {
            return Some(from_run(run));
        }
    }
    if let Some(native) = &entry.native_session_id {
        if let Ok(Some(run)) = store.run_by_harness_session(native) {
            return Some(from_run(run));
        }
    }
    let workspace_id = entry.workspace_id.as_deref()?;
    if !matches!(store.workspace(workspace_id), Ok(Some(_))) {
        return None;
    }
    Some(Link {
        workspace_id: workspace_id.to_owned(),
        session_id: record,
        run_id: None,
    })
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

/// A path an agent reported, relative to the workspace — or nothing, and a flag, when it is
/// somewhere else. `.` and `..` are resolved lexically first, so a path that climbs out of the
/// workspace and back into somewhere else is seen for where it ends up, not for how it was
/// spelled; without a workspace to measure against, no path can be called inside one.
pub(crate) fn workspace_relative(path: &str, cwd: Option<&str>) -> (Option<String>, bool) {
    let Some(cwd) = cwd else {
        return (None, true);
    };
    let root = normalized(std::iter::empty(), cwd);
    let full = if is_absolute(path) {
        normalized(std::iter::empty(), path)
    } else {
        normalized(root.iter().map(String::as_str), path)
    };
    // Only a drive-lettered path is compared without case: Windows does not distinguish
    // `C:\Repo` from `C:\repo`, but on Linux `/work/REPO` is another directory altogether.
    let windows = root.first().is_some_and(|first| first.ends_with(':'));
    let same = |a: &String, b: &String| {
        if windows {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    };
    let inside = full.len() >= root.len() && root.iter().zip(&full).all(|(a, b)| same(a, b));
    if !inside {
        return (None, true);
    }
    let relative = full[root.len()..].join("/");
    if relative.is_empty() {
        return (Some(".".to_owned()), false);
    }
    (Some(relative), false)
}

fn is_absolute(path: &str) -> bool {
    path.starts_with('/')
        || path.starts_with('\\')
        || path.get(1..3).is_some_and(|s| s == ":\\" || s == ":/")
}

/// The components of `path` after `base`, with `.` dropped and `..` resolved. Both separators
/// count; a drive letter is a component, and the floor on Windows as the root is elsewhere.
fn normalized<'a>(base: impl Iterator<Item = &'a str>, path: &'a str) -> Vec<String> {
    let mut parts: Vec<String> = base.map(str::to_owned).collect();
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|last| !last.ends_with(':')) {
                    parts.pop();
                }
            }
            other => parts.push(other.to_owned()),
        }
    }
    parts
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

        recorder.spawned(&run_id, &draft, "pty-1", None);
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
        recorder.spawned(&run_id, &draft, "pty-1", None);

        let exit = ExitFacts {
            code: Some(3),
            success: false,
            signal: None,
            at: None,
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
        recorder.spawned(&run, &resume, "pty-2", None);
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
        recorder.spawned(&run, &fork, "pty-3", None);

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
                ..Default::default()
            },
            LaunchedBy::Cli,
        );
        assert_eq!(off.begin(&harness_draft(&ws)), None);
        assert_eq!(store.activity_counts().unwrap(), (0, 0));
        let exit = ExitFacts {
            code: Some(0),
            success: true,
            signal: None,
            at: None,
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
            at: None,
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
        recorder.spawned(&run_id, &draft, "pty-1", None);
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

    /// An agent that finishes overnight finished overnight, not when somebody opened the window
    /// the next morning: the daemon's clock is what the event, the run and the record keep.
    #[test]
    fn a_spooled_exit_keeps_the_time_the_daemon_saw_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let ws = workspace(&store);
        let recorder = recorder(&store);
        let draft = harness_draft(&ws);
        let run_id = recorder.begin(&draft).unwrap();
        recorder.spawned(&run_id, &draft, "pty-1", None);
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

        let overnight: u64 = 1_700_000_000_000;
        spool_entry(dir.path(), overnight, "pty-1", 0);
        assert_eq!(import_spool(&store, dir.path()).imported, 1);

        let exited = store.events(&ws, None, 1).unwrap().remove(0);
        assert_eq!(exited.kind, "process.exited");
        assert_eq!(exited.occurred_at, overnight as i64);
        assert!(
            exited.received_at > exited.occurred_at,
            "received now, occurred then"
        );
        assert_eq!(
            store.run(&run_id).unwrap().unwrap().ended_at,
            Some(overnight as i64)
        );
        assert_eq!(
            store.session("rec-1").unwrap().unwrap().ended_at,
            Some(overnight as i64)
        );
    }

    /// The launcher writes the run, spawns, and only then attaches the PTY id. A process that
    /// exits inside that gap, with another client draining the spool at that moment, must not
    /// lose its only copy of the exit: the entry names the run, so the link is made here.
    #[test]
    fn an_exit_spooled_before_the_pty_was_linked_still_settles_its_run() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let ws = workspace(&store);
        let recorder = recorder(&store);
        let draft = harness_draft(&ws);
        let run_id = recorder.begin(&draft).unwrap();

        let spool = Spool::new(spool_dir(dir.path()));
        let entry = |at, pty: &str, run: &str, code| SpoolEntry {
            version: SPOOL_VERSION,
            at_ms: at,
            daemon_pid: 1,
            session: SessionId(pty.into()),
            labels: [(crate::launch::RUN_LABEL.to_owned(), run.to_owned())]
                .into_iter()
                .collect(),
            exit: ExitInfo {
                code,
                success: code == 0,
                signal: None,
            },
        };
        spool.record(&entry(10, "pty-early", &run_id, 7)).unwrap();
        spool
            .record(&entry(11, "pty-orphan", "no-such-run", 1))
            .unwrap();

        let report = import_spool(&store, dir.path());
        assert_eq!((report.imported, report.unmatched), (1, 1));
        assert!(spool.entries().unwrap().is_empty(), "both consumed");
        let run = store.run(&run_id).unwrap().unwrap();
        assert_eq!(run.pty_session_id.as_deref(), Some("pty-early"));
        assert_eq!(
            (run.exit_code, run.end_reason.as_deref()),
            (Some(7), Some("exited"))
        );

        // The launcher now catches up: its own link is a no-op, and its settle a duplicate.
        recorder.spawned(&run_id, &draft, "pty-early", None);
        let settle = ExitFacts {
            code: Some(7),
            success: false,
            signal: None,
            at: None,
        };
        assert_eq!(
            record_exit(&store, "pty-early", &settle, Via::Settle),
            Recorded::Duplicate
        );
        let mut kinds = kinds(&store, &ws);
        kinds.sort();
        assert_eq!(kinds, ["process.exited", "process.started"]);
    }

    #[test]
    fn runs_whose_process_is_gone_are_interrupted_and_young_pending_ones_are_left_alone() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        let recorder = recorder(&store);
        let draft = harness_draft(&ws);
        let dead = recorder.begin(&draft).unwrap();
        recorder.spawned(&dead, &draft, "pty-dead", None);
        let alive = recorder.begin(&draft).unwrap();
        recorder.spawned(&alive, &draft, "pty-alive", None);
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
            recorder.spawned(&id, &draft, &format!("pty-{n}"), None);
            let exit = ExitFacts {
                code: Some(0),
                success: true,
                signal: None,
                at: None,
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
        recorder.spawned(&open, &draft, "pty-open", None);
        let done = recorder.begin(&draft).unwrap();
        recorder.spawned(&done, &draft, "pty-done", None);
        let exit = ExitFacts {
            code: Some(0),
            success: true,
            signal: None,
            at: None,
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
        recorder.spawned(&run, &draft, "pty-1", None);
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
            recorder.spawned(&id, &draft, &format!("pty-{n}"), None);
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

    fn inbox_entry(id: &str, kind: &str) -> InboxEntry {
        InboxEntry {
            version: inbox::INBOX_VERSION,
            id: id.into(),
            at_ms: 1_790_000_000_000,
            producer: claude::PRODUCER.into(),
            method: claude::METHOD.into(),
            fidelity: claude::FIDELITY.into(),
            run_id: None,
            workspace_id: None,
            session_record_id: None,
            native_session_id: None,
            kind: kind.into(),
            privacy_class: PRIVACY_METADATA.into(),
            payload: json!({ "tool": "Read", "path": "a.rs" }),
        }
    }

    #[test]
    fn inbox_entries_are_placed_by_run_then_by_the_agents_session_then_by_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let ws = workspace(&store);
        let recorder = recorder(&store);
        let draft = harness_draft(&ws);
        let run_id = recorder.begin(&draft).unwrap();
        recorder.spawned(&run_id, &draft, "pty-1", Some(claude::METHOD));
        store
            .add_session(&NewSession {
                id: "rec-1",
                workspace_id: &ws,
                harness_id: "claude",
                harness_session_id: Some("h-1"),
                title: "",
                pty_session_id: "pty-1",
                ..Default::default()
            })
            .unwrap();
        recorder.link_session(&run_id, "rec-1");
        let inbox = Inbox::new(inbox_dir(dir.path()));

        // The environment made it through: the run is named.
        let mut by_run = inbox_entry("e-run", "tool.started");
        by_run.run_id = Some(run_id.clone());
        inbox.record(&by_run).unwrap();
        // It did not, but the agent's own session id is one the launcher recorded.
        let mut by_native = inbox_entry("e-native", "tool.completed");
        by_native.native_session_id = Some("h-1".into());
        inbox.record(&by_native).unwrap();
        // Neither, but the workspace is known: placed there, without a run.
        let mut by_ws = inbox_entry("e-ws", "turn.completed");
        by_ws.workspace_id = Some(ws.clone());
        inbox.record(&by_ws).unwrap();
        // Nothing this database knows: dropped and counted, never guessed.
        let mut stray = inbox_entry("e-stray", "session.ended");
        stray.run_id = Some("run-from-another-profile".into());
        stray.workspace_id = Some("ws-from-another-profile".into());
        stray.native_session_id = Some("h-unknown".into());
        inbox.record(&stray).unwrap();

        let report = import_inbox(&store, dir.path());
        assert_eq!(
            (
                report.imported,
                report.duplicates,
                report.unlinked,
                report.unreadable
            ),
            (3, 0, 1, 0)
        );
        assert_eq!(report.workspaces.iter().collect::<Vec<_>>(), [&ws]);
        assert!(
            inbox.entries().unwrap().is_empty(),
            "drained, strays included"
        );

        let events = store.all_events(Some(&ws)).unwrap();
        let find = |id: &str| events.iter().find(|e| e.id == id).unwrap();
        let placed = find("e-run");
        assert_eq!(placed.run_id.as_deref(), Some(run_id.as_str()));
        assert_eq!(placed.session_id.as_deref(), Some("rec-1"));
        assert_eq!(
            (
                placed.producer.as_str(),
                placed.method.as_str(),
                placed.fidelity.as_str()
            ),
            ("claude", "hook", "reported")
        );
        assert_eq!(
            placed.occurred_at, 1_790_000_000_000,
            "the hook's clock, not the drain's"
        );
        assert_eq!(placed.source_key.as_deref(), Some("inbox:e-run"));
        assert_eq!(find("e-native").run_id.as_deref(), Some(run_id.as_str()));
        assert_eq!(find("e-ws").run_id, None);
        assert_eq!(find("e-ws").session_id, None);
        assert!(events.iter().all(|e| e.id != "e-stray"));
        assert!(store
            .diagnostics()
            .unwrap()
            .iter()
            .any(|d| d.name == "inbox_unlinked" && d.count == 1));

        // The run's start says it could report.
        let started = events.iter().find(|e| e.kind == "process.started").unwrap();
        assert_eq!(payload(started)["capture"], "hook");
    }

    #[test]
    fn an_inbox_entry_drained_twice_is_one_event_and_a_bad_file_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let ws = workspace(&store);
        let inbox = Inbox::new(inbox_dir(dir.path()));
        let mut entry = inbox_entry("e-1", "tool.started");
        entry.workspace_id = Some(ws.clone());
        inbox.record(&entry).unwrap();
        assert_eq!(import_inbox(&store, dir.path()).imported, 1);
        // Another client read the same file before this one removed it.
        inbox.record(&entry).unwrap();
        let again = import_inbox(&store, dir.path());
        assert_eq!((again.imported, again.duplicates), (0, 1));
        assert_eq!(store.all_events(Some(&ws)).unwrap().len(), 1);

        std::fs::write(inbox.dir().join("0000000000001-junk.json"), b"{not json").unwrap();
        let junk = import_inbox(&store, dir.path());
        assert_eq!(junk.unreadable, 1);
        assert!(inbox.entries().unwrap().is_empty());
    }

    /// A throwaway CODEX_HOME holding the fixture session file where Codex would keep it.
    fn codex_home_with_fixture(dir: &Path) -> PathBuf {
        let home = dir.join("codex");
        let day = home.join("sessions/2026/09/25");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/codex")
                .join(codex::FIXTURE_VERSION)
                .join("rollout.jsonl"),
            day.join("rollout-2026-09-25T13-07-32-01a0d83f-c3ea-7ae0-88df-82c9431d3f8b.jsonl"),
        )
        .unwrap();
        home
    }

    fn codex_trigger(id: &str, home: &Path, run_id: &str, at_ms: i64) -> InboxEntry {
        let mut entry = inbox_entry(id, codex::TRIGGER_KIND);
        entry.producer = codex::PRODUCER.into();
        entry.method = codex::METHOD_NOTIFY.into();
        entry.run_id = Some(run_id.into());
        entry.at_ms = at_ms;
        entry.payload = json!({
            "threadId": "01a0d83f-c3ea-7ae0-88df-82c9431d3f8b",
            "turnId": "01a0d83f-c42b-7fd3-b4af-c46bc45458cc",
            "client": "codex_exec",
            "codexHome": home.to_string_lossy(),
        });
        entry
    }

    #[test]
    fn a_codex_turn_trigger_is_expanded_from_the_session_file_once_however_often_it_is_drained() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let ws = workspace(&store);
        let recorder = recorder(&store);
        let mut draft = harness_draft(&ws);
        draft.harness_id = Some("codex".into());
        draft.harness_session_id = None;
        let run_id = recorder.begin(&draft).unwrap();
        recorder.spawned(&run_id, &draft, "pty-c", Some(codex::METHOD_NOTIFY));
        let home = codex_home_with_fixture(dir.path());
        let inbox = Inbox::new(inbox_dir(dir.path()));

        inbox
            .record(&codex_trigger(
                "t-1",
                &home,
                &run_id,
                crate::store::now_ms(),
            ))
            .unwrap();
        let report = import_inbox(&store, dir.path());
        assert_eq!(
            (report.imported, report.duplicates, report.unlinked),
            (8, 0, 0)
        );
        assert!(
            inbox.entries().unwrap().is_empty(),
            "the trigger is done with"
        );
        let events = store.all_events(Some(&ws)).unwrap();
        let of = |kind: &str| events.iter().filter(|e| e.kind == kind).count();
        assert_eq!(of("tool.completed"), 2);
        assert_eq!(of("tool.failed"), 1);
        assert_eq!(of("file.reported_write"), 1);
        assert_eq!(of("usage.reported"), 1);
        assert_eq!(of("turn.completed"), 1);
        let turn = events.iter().find(|e| e.kind == "turn.completed").unwrap();
        assert_eq!(turn.run_id.as_deref(), Some(run_id.as_str()));
        assert_eq!(
            (
                turn.producer.as_str(),
                turn.method.as_str(),
                turn.fidelity.as_str()
            ),
            ("codex", "session_file", "reported")
        );
        assert_eq!(
            turn.occurred_at,
            codex::iso_to_ms("2026-09-25T11:07:43.814Z").unwrap()
        );
        assert_eq!(turn.source_key.as_deref(), Some("codex:01a0d83f-c3ea-7ae0-88df-82c9431d3f8b:01a0d83f-c42b-7fd3-b4af-c46bc45458cc:turn"));

        // The same turn, delivered again (a second notify for it, or a second client's drain).
        inbox
            .record(&codex_trigger(
                "t-2",
                &home,
                &run_id,
                crate::store::now_ms(),
            ))
            .unwrap();
        let again = import_inbox(&store, dir.path());
        assert_eq!((again.imported, again.duplicates), (0, 8));
        assert_eq!(
            store.all_events(Some(&ws)).unwrap().len(),
            9,
            "plus process.started"
        );
    }

    #[test]
    fn a_codex_turn_the_file_has_not_finished_waits_and_a_lost_file_still_records_the_turn() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let ws = workspace(&store);
        let recorder = recorder(&store);
        let mut draft = harness_draft(&ws);
        draft.harness_id = Some("codex".into());
        let run_id = recorder.begin(&draft).unwrap();
        recorder.spawned(&run_id, &draft, "pty-c", Some(codex::METHOD_NOTIFY));
        let home = codex_home_with_fixture(dir.path());
        let inbox = Inbox::new(inbox_dir(dir.path()));

        // A turn the session file has not ended: fresh, so it waits.
        let mut early = codex_trigger("t-early", &home, &run_id, crate::store::now_ms());
        early.payload["turnId"] = json!("turn-still-running");
        inbox.record(&early).unwrap();
        let report = import_inbox(&store, dir.path());
        assert_eq!(report.imported, 0);
        assert_eq!(inbox.entries().unwrap().len(), 1, "left for the next drain");

        // The same, but older than the grace: recorded from the trigger alone, and counted.
        let mut late = codex_trigger(
            "t-late",
            &home,
            &run_id,
            crate::store::now_ms() - codex::NOT_READY_GRACE_MS - 1,
        );
        late.payload["turnId"] = json!("turn-still-running");
        inbox.record(&late).unwrap();
        let report = import_inbox(&store, dir.path());
        assert_eq!(report.imported, 1);
        let events = store.all_events(Some(&ws)).unwrap();
        let fallback = events.iter().find(|e| e.kind == "turn.completed").unwrap();
        assert_eq!(fallback.method, "notify");
        assert!(store
            .diagnostics()
            .unwrap()
            .iter()
            .any(|d| d.name == "codex_turn_incomplete"));

        // No session file at all: the same, under its own counter, and the file is done with.
        let mut lost = codex_trigger("t-lost", &home, &run_id, crate::store::now_ms());
        lost.payload["threadId"] = json!("no-such-thread");
        inbox.record(&lost).unwrap();
        let report = import_inbox(&store, dir.path());
        assert_eq!(report.imported, 1);
        assert_eq!(
            inbox.entries().unwrap().len(),
            1,
            "only the fresh one waits"
        );
        assert!(store
            .diagnostics()
            .unwrap()
            .iter()
            .any(|d| d.name == "codex_session_file"));
    }
}
