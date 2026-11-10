//! Persistent app state in SQLite. Pure storage: no git, no filesystem, no policy.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

/// Applied in order; a database's `user_version` is how many it has had. Never edit a shipped
/// migration — add a new one.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_unique_workspace_path.sql"),
    include_str!("../migrations/0003_sessions.sql"),
    include_str!("../migrations/0004_session_prompt.sql"),
    include_str!("../migrations/0005_forgotten_workspaces.sql"),
    include_str!("../migrations/0006_removed_projects.sql"),
    include_str!("../migrations/0007_project_automation.sql"),
    include_str!("../migrations/0008_workspace_preparation.sql"),
    include_str!("../migrations/0009_agent_runs_and_events.sql"),
    include_str!("../migrations/0010_project_memory.sql"),
    include_str!("../migrations/0011_workspace_outcomes.sql"),
    include_str!("../migrations/0012_workflow_runs.sql"),
    include_str!("../migrations/0013_workflow_run_context.sql"),
    include_str!("../migrations/0014_workspace_tasks.sql"),
    include_str!("../migrations/0015_outcome_began.sql"),
    include_str!("../migrations/0016_project_conversations.sql"),
    include_str!("../migrations/0017_session_began.sql"),
    include_str!("../migrations/0018_remote_host.sql"),
    include_str!("../migrations/0019_smoke_tests.sql"),
    include_str!("../migrations/0020_smoke_test_settings.sql"),
    include_str!("../migrations/0021_smoke_test_browser.sql"),
    include_str!("../migrations/0022_schedules.sql"),
    include_str!("../migrations/0023_triggers.sql"),
    include_str!("../migrations/0024_pr_watches.sql"),
    include_str!("../migrations/0025_custom_events.sql"),
    include_str!("../migrations/0026_remote_computer.sql"),
    include_str!("../migrations/0027_tracked_pull_requests.sql"),
    include_str!("../migrations/0028_tracked_pr_watches.sql"),
    include_str!("../migrations/0029_workflow_step_outcome.sql"),
    include_str!("../migrations/0030_workflow_artifacts.sql"),
    include_str!("../migrations/0031_workflow_approval_decisions.sql"),
];

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("cannot create the data directory: {0}")]
    Io(#[from] std::io::Error),
    #[error("this database was written by a newer version of Yardsort (schema {found}, this build knows {known})")]
    TooNew { found: usize, known: usize },
}

pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRow {
    pub id: String,
    pub name: String,
    pub root_path: String,
}

/// This machine as a host on a Yardsort server. See `migrations/0018_remote_host.sql`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteHostRow {
    pub server_url: String,
    pub server_id: String,
    pub host_id: String,
    pub device_id: String,
    pub name: String,
    pub enabled_at: i64,
}

/// This machine as a client of other hosts. See `migrations/0026_remote_computer.sql`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteComputerRow {
    pub server_url: String,
    pub server_id: String,
    pub device_id: String,
    pub name: String,
    pub signed_in_at: i64,
}

/// A device this host's person let in. `scopes` and `projects` are JSON, as on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteGrantRow {
    pub id: String,
    pub device_id: String,
    pub device_name: String,
    pub scopes: String,
    pub projects: String,
    pub approved_at: i64,
}

/// A pull request in another repository that a workspace follows. See
/// `migrations/0023_tracked_pull_requests.sql`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedPullRequestRow {
    pub id: String,
    pub workspace_id: String,
    /// The conversation that opened it, when that is known.
    pub session_id: Option<String>,
    /// `host/owner/name`.
    pub repo: String,
    pub number: u32,
    pub url: String,
    /// `app`, `cli` or `hook`.
    pub tracked_by: String,
    pub created_at: i64,
}

/// What [`Store::track_pull_request`] is given.
#[derive(Debug, Clone, Copy)]
pub struct NewTrackedPullRequest<'a> {
    pub workspace_id: &'a str,
    pub session_id: Option<&'a str>,
    pub repo: &'a str,
    pub number: u32,
    pub url: &'a str,
    pub tracked_by: &'a str,
}

const TRACKED_COLUMNS: &str =
    "id, workspace_id, session_id, repo, number, url, tracked_by, created_at";

fn tracked_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TrackedPullRequestRow> {
    Ok(TrackedPullRequestRow {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        session_id: row.get(2)?,
        repo: row.get(3)?,
        number: row.get(4)?,
        url: row.get(5)?,
        tracked_by: row.get(6)?,
        created_at: row.get(7)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceTaskRow {
    /// Where the task is kept: `github`.
    pub source: String,
    /// Which of that source's projects: `host/owner/name`.
    pub repo: String,
    /// What a person calls it there: `#91`.
    pub key: String,
    pub url: String,
    /// As it was when the workspace was started.
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRow {
    pub id: String,
    pub project_id: String,
    pub kind: String,
    pub name: String,
    pub path: String,
    pub branch: Option<String>,
    /// The branch this workspace's branch was created from. `None` when Yardsort did not
    /// create the branch (an existing branch was opened, or a worktree was adopted) — which also
    /// means the branch is not ours to delete when undoing.
    pub base_branch: Option<String>,
    /// Archived: the worktree is gone from disk but the row, its branch and its session history
    /// are kept, so it can be restored.
    pub archived: bool,
    /// Forgotten: hidden from every list, kept only so its session history survives until the
    /// worktree is imported again. Folder and branch are untouched. Never set on `local`.
    pub forgotten: bool,
}

/// One harness conversation. See `migrations/0003_sessions.sql`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub id: String,
    pub workspace_id: String,
    pub harness_id: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub harness_session_id: Option<String>,
    pub title: String,
    pub forked_from: Option<String>,
    pub running: bool,
    pub exit_code: Option<i64>,
    pub pty_session_id: Option<String>,
    /// Milliseconds since the Unix epoch. The latest start: a resume moves it.
    pub started_at: i64,
    /// When the conversation first began, which a resume leaves alone.
    pub began_at: i64,
    pub ended_at: Option<i64>,
}

/// What is known about a session when it starts.
#[derive(Debug, Clone, Default)]
pub struct NewSession<'a> {
    pub id: &'a str,
    pub workspace_id: &'a str,
    pub harness_id: &'a str,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub harness_session_id: Option<&'a str>,
    pub title: &'a str,
    pub forked_from: Option<&'a str>,
    pub pty_session_id: &'a str,
    /// The whole first message, when this conversation started with one.
    pub prompt: Option<&'a str>,
}

/// One process in a workspace. See `migrations/0009_agent_runs_and_events.sql`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRow {
    pub id: String,
    pub workspace_id: String,
    pub session_id: Option<String>,
    /// `harness`, `shell` or `program`.
    pub kind: String,
    pub harness_id: Option<String>,
    pub harness_session_id: Option<String>,
    /// `None` until the process was spawned, and for ever if it never was.
    pub pty_session_id: Option<String>,
    /// `app` or `cli`.
    pub launched_by: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub exit_code: Option<i64>,
    /// `exited`, `interrupted` or `spawn_failed`, once ended.
    pub end_reason: Option<String>,
    pub collection: String,
}

/// What is known about a run before its process exists.
#[derive(Debug, Clone, Default)]
pub struct NewRun<'a> {
    pub id: &'a str,
    pub workspace_id: &'a str,
    pub session_id: Option<&'a str>,
    pub kind: &'a str,
    pub harness_id: Option<&'a str>,
    pub harness_session_id: Option<&'a str>,
    pub launched_by: &'a str,
}

/// One memory entry. See `migrations/0010_project_memory.sql`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRow {
    pub id: String,
    pub project_id: String,
    pub text: String,
    /// `candidate`, `approved`, `rejected` or `revoked`.
    pub state: String,
    /// `user` or `agent`.
    pub author: String,
    pub harness_id: Option<String>,
    pub workspace_id: Option<String>,
    pub workspace_name: Option<String>,
    pub run_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// One change to a memory entry. See `migrations/0010_project_memory.sql`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryHistoryRow {
    pub at: i64,
    pub action: String,
    /// `user` or `agent`.
    pub by: String,
    /// The text an edit replaced.
    pub previous_text: Option<String>,
}

/// One attempt's outcome and its evidence. See `migrations/0011_workspace_outcomes.sql`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OutcomeRow {
    pub id: String,
    pub project_id: String,
    pub workspace_id: Option<String>,
    pub workspace_name: String,
    pub branch: Option<String>,
    pub base_branch: Option<String>,
    pub task: Option<String>,
    /// Comma-separated, first-seen order.
    pub harnesses: String,
    pub label: Option<String>,
    pub labeled_at: Option<i64>,
    pub ahead_at: Option<i64>,
    /// The branch's commit when last seen ahead of its base.
    pub ahead_tip: Option<String>,
    pub merged_at: Option<i64>,
    pub pr_number: Option<i64>,
    pub pr_state: Option<String>,
    pub ended: Option<String>,
    pub ended_at: Option<i64>,
    /// When the workspace was created. `None` for a row first written before this was kept,
    /// whose workspace was already gone.
    pub began_at: Option<i64>,
    /// When this row was first written, which may be as late as the workspace's deletion.
    pub created_at: i64,
    pub updated_at: i64,
}

/// What an outcome describes, copied from its workspace while the workspace is still there.
#[derive(Debug, Clone, Copy)]
pub struct OutcomeSnapshot<'a> {
    pub id: &'a str,
    pub project_id: &'a str,
    pub workspace_name: &'a str,
    pub branch: Option<&'a str>,
    pub base_branch: Option<&'a str>,
    pub task: Option<&'a str>,
    /// Comma-separated harness ids, first-seen order; empty keeps what was recorded.
    pub harnesses: &'a str,
}

/// What a proposal came to, decided inside the transaction that would add it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryInsert {
    Added(String),
    /// The same text, ignoring ASCII case, is already in the project.
    Known {
        id: String,
        state: String,
    },
    /// As many candidates as are allowed already wait.
    Full,
}

/// A memory entry about to be written.
pub struct NewMemory<'a> {
    pub project_id: &'a str,
    pub text: &'a str,
    pub state: &'a str,
    pub author: &'a str,
    pub harness_id: Option<&'a str>,
    pub workspace_id: Option<&'a str>,
    pub workspace_name: Option<&'a str>,
    pub run_id: Option<&'a str>,
}

/// One recorded fact. See `migrations/0009_agent_runs_and_events.sql`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRow {
    pub seq: i64,
    pub id: String,
    pub schema_version: i64,
    pub workspace_id: String,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub occurred_at: i64,
    pub received_at: i64,
    pub kind: String,
    pub producer: String,
    pub method: String,
    pub fidelity: String,
    pub source_key: Option<String>,
    pub privacy_class: String,
    /// JSON, whose shape depends on `kind` and `schema_version`.
    pub payload: String,
}

/// Which events [`Store::events_window`] reads.
#[derive(Debug, Clone, Copy)]
pub struct EventFilter<'a> {
    /// Any of these workspaces'. None is none at all.
    pub workspace_ids: &'a [String],
    pub privacy_class: &'a str,
    /// Only this session's, when given.
    pub session_id: Option<&'a str>,
}

/// Where [`Store::events_window`] starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventBound {
    /// The newest.
    Newest,
    /// The newest of those with a smaller sequence number.
    Before(i64),
    /// The oldest of those with a larger sequence number.
    After(i64),
}

#[derive(Debug, Clone, Default)]
pub struct NewEvent<'a> {
    pub id: &'a str,
    pub schema_version: u16,
    pub workspace_id: &'a str,
    pub session_id: Option<&'a str>,
    pub run_id: Option<&'a str>,
    pub occurred_at: i64,
    pub kind: &'a str,
    pub producer: &'a str,
    pub method: &'a str,
    pub fidelity: &'a str,
    /// What makes a second delivery of the same fact a no-op. `None` never deduplicates.
    pub source_key: Option<&'a str>,
    pub privacy_class: &'a str,
    pub payload: &'a str,
}

/// A counter of something that went wrong, or was merely noticed, while recording activity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticRow {
    pub name: String,
    pub count: i64,
    pub last_at: i64,
    pub last_detail: Option<String>,
}

/// One time a workflow was asked to run. See `migrations/0012_workflow_runs.sql`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRunRow {
    pub id: String,
    pub workflow_id: String,
    pub workflow_name: String,
    /// The workflow file as it was when the run was asked for.
    pub definition: String,
    pub project_id: Option<String>,
    /// `None` once the workspace is deleted; `workspace_name` keeps what it was called.
    pub workspace_id: Option<String>,
    pub workspace_name: String,
    /// A JSON object of strings.
    pub inputs: String,
    /// `queued`, `running`, `succeeded`, `failed` or `cancelled`.
    pub status: String,
    /// `app` or `cli`.
    pub requested_by: String,
    pub error: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    /// What the run found out about where it runs, as a JSON object. See
    /// `workflow::runs::Facts`.
    pub context: String,
}

impl WorkflowRunRow {
    pub fn active(&self) -> bool {
        matches!(self.status.as_str(), "queued" | "running")
    }
}

/// One step of a workflow run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowStepRow {
    pub run_id: String,
    pub step_id: String,
    pub position: i64,
    pub action: String,
    /// `pending`, `running`, `waiting`, `succeeded`, `failed`, `skipped` or `cancelled`.
    pub status: String,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    /// A JSON object of strings.
    pub outputs: String,
    pub note: Option<String>,
    /// A step outcome as JSON, when the step judged something. Null on older rows and on
    /// steps that produced none. See `workflow::engine::StepOutcome`.
    pub outcome: Option<String>,
}

/// A named document stored on a run, as a later step and the run view read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowArtifactRow {
    pub run_id: String,
    pub name: String,
    pub step_id: String,
    /// `file`, `text`, `output` or `transcript`.
    pub source: String,
    /// The path, the field, or `digest` / `raw`. Empty for text written in the file.
    pub detail: Option<String>,
    pub body: String,
    /// The body was cut at the size cap.
    pub truncated: bool,
    pub updated_at: i64,
}

/// What recording an approval did, or why it did not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalWrite {
    Recorded,
    MissingRun,
    MissingStep,
    /// The run is not running. `status` is the word the row has.
    RunEnded {
        status: String,
    },
    NotAnApproval,
    NotWaiting,
    AlreadyDecided,
    FeedbackRequired,
    NotADecision,
}

/// A decision stored for one approval step, before the engine has applied it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowApproval {
    /// `approved`, `rejected` or `changes_requested`.
    pub decision: String,
    pub feedback: Option<String>,
    pub actor: String,
    pub decided_at: i64,
}

/// An artifact about to be stored. The body is already capped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewWorkflowArtifact {
    pub name: String,
    pub source: String,
    pub detail: Option<String>,
    pub body: String,
    pub truncated: bool,
}

/// A run about to be queued, with its steps in file order as `(step id, action)`.
#[derive(Debug, Clone)]
pub struct NewWorkflowRun<'a> {
    pub id: &'a str,
    pub workflow_id: &'a str,
    pub workflow_name: &'a str,
    pub definition: &'a str,
    pub project_id: &'a str,
    pub workspace_id: &'a str,
    pub workspace_name: &'a str,
    pub inputs: &'a str,
    /// What was found out before queuing, as a JSON object.
    pub context: &'a str,
    pub requested_by: &'a str,
    pub steps: &'a [(&'a str, &'a str)],
}

/// Whether a run was queued, or refused because the same workflow is already running there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Queued {
    Queued,
    AlreadyRunning(String),
}

/// What a step moves to. Terminal states set its end time; the others its start time, once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Pending,
    Running,
    Waiting,
    Succeeded,
    Failed,
    Skipped,
    Cancelled,
}

impl StepStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            StepStatus::Pending => "pending",
            StepStatus::Running => "running",
            StepStatus::Waiting => "waiting",
            StepStatus::Succeeded => "succeeded",
            StepStatus::Failed => "failed",
            StepStatus::Skipped => "skipped",
            StepStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "pending" => StepStatus::Pending,
            "running" => StepStatus::Running,
            "waiting" => StepStatus::Waiting,
            "succeeded" => StepStatus::Succeeded,
            "failed" => StepStatus::Failed,
            "skipped" => StepStatus::Skipped,
            "cancelled" => StepStatus::Cancelled,
            _ => return None,
        })
    }

    pub fn is_final(self) -> bool {
        matches!(
            self,
            StepStatus::Succeeded
                | StepStatus::Failed
                | StepStatus::Skipped
                | StepStatus::Cancelled
        )
    }
}

mod smoke;
pub use smoke::*;

mod schedules;
pub use schedules::*;

mod pr_watches;
pub use pr_watches::*;

mod triggers;
pub use triggers::*;

pub struct Store {
    conn: Mutex<Connection>,
}

/// How long to wait for another process to finish writing before giving up. Generous: the writes
/// here are single statements or small transactions, so anything approaching this means something
/// is wrong rather than merely busy.
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Beside the database: a copy of it from just before the last upgrade. The version that was
/// running before the upgrade can still open it; the upgraded database it cannot.
pub fn copy_before_upgrade(database: &Path) -> std::path::PathBuf {
    let mut name = database.file_name().unwrap_or_default().to_os_string();
    name.push(".before-upgrade");
    database.with_file_name(name)
}

/// [`copy_before_upgrade`], when there is one this build can open.
pub fn readable_copy_before_upgrade(database: &Path) -> Option<std::path::PathBuf> {
    let copy = copy_before_upgrade(database);
    let conn =
        Connection::open_with_flags(&copy, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let schema: u32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .ok()?;
    (schema > 0 && schema as usize <= MIGRATIONS.len()).then_some(copy)
}

/// Writes [`copy_before_upgrade`], replacing the one an earlier upgrade left. Written under
/// another name first, so a copy cut short never takes the place of a whole one.
fn keep_a_copy(conn: &Connection, database: &Path) -> StoreResult<()> {
    let copy = copy_before_upgrade(database);
    let mut name = copy.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".partial-{}", std::process::id()));
    let partial = copy.with_file_name(name);
    let _ = std::fs::remove_file(&partial);
    let written = conn
        .execute("VACUUM INTO ?1", [partial.to_string_lossy()])
        .map_err(StoreError::from)
        .and_then(|_| std::fs::rename(&partial, &copy).map_err(StoreError::from));
    if written.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    written
}

/// Brings the database up to `migrations`, which is [`MIGRATIONS`] everywhere but in tests.
///
/// All of them in one transaction: an upgrade that stops partway leaves the schema it started
/// from, never one between. That matters for the copy. A retry copies again, and a copy of an
/// in-between schema would replace the only one the version before the upgrade can open.
fn upgrade(conn: &mut Connection, migrations: &[&str], path: Option<&Path>) -> StoreResult<()> {
    let applied: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let applied = applied as usize;
    if applied > migrations.len() {
        return Err(StoreError::TooNew {
            found: applied,
            known: migrations.len(),
        });
    }
    if applied == migrations.len() {
        return Ok(());
    }
    // An upgrade cannot be taken back: the version that was running before it will refuse the
    // database from now on. Going back to that version then means going back to this.
    if applied > 0 {
        if let Some(path) = path {
            if let Err(error) = keep_a_copy(conn, path) {
                eprintln!("yardsort: no copy of the database before upgrading it: {error}");
            }
        }
    }
    let tx = conn.transaction()?;
    for sql in &migrations[applied..] {
        tx.execute_batch(sql)?;
    }
    tx.pragma_update(
        None,
        "user_version",
        u32::try_from(migrations.len()).unwrap_or(u32::MAX),
    )?;
    tx.commit()?;
    Ok(())
}

impl Store {
    pub fn open(path: &Path) -> StoreResult<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        Self::prepare(Connection::open(path)?, Some(path))
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::prepare(Connection::open_in_memory().unwrap(), None).unwrap()
    }

    fn prepare(mut conn: Connection, path: Option<&Path>) -> StoreResult<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // More than one process opens this database — the app and the `ys` CLI. WAL lets them
        // read side by side, but only one may write at a time, so the loser of a race has to
        // wait its turn. rusqlite already defaults to a five-second busy timeout; it is set
        // here so the value is ours rather than a dependency's default.
        conn.busy_timeout(BUSY_TIMEOUT)?;
        // The timeout alone is not enough. A `BEGIN DEFERRED` that reads first and writes second
        // — which is what every write in this file does — cannot wait for the write lock without
        // risking deadlock, so SQLite refuses it outright rather than calling the busy handler.
        // Taking the lock up front is what actually lets two processes write.
        conn.set_transaction_behavior(TransactionBehavior::Immediate);

        upgrade(&mut conn, MIGRATIONS, path)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn mark_workspace_preparation(&self, id: &str) -> StoreResult<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO workspace_preparation (workspace_id) VALUES (?)",
            [id],
        )?;
        Ok(())
    }

    pub fn prepares_workspace(&self, id: &str) -> StoreResult<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS(SELECT 1 FROM workspace_preparation WHERE workspace_id = ?)",
            [id],
            |row| row.get(0),
        )?)
    }

    pub fn project_automation(&self, id: &str) -> StoreResult<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT config FROM project_automation WHERE project_id = ?",
                [id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn set_project_automation(&self, id: &str, config: &str) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO project_automation (project_id, config) VALUES (?, ?) ON CONFLICT(project_id) DO UPDATE SET config = excluded.config",
            params![id, config],
        )?;
        Ok(())
    }

    /// Insert a project together with its `local` workspace, at the end of the list.
    pub fn add_project(&self, name: &str, root_path: &str) -> StoreResult<ProjectRow> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let project = ProjectRow {
            id: new_id(),
            name: name.to_owned(),
            root_path: root_path.to_owned(),
        };
        let next: i64 = tx.query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM projects",
            [],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT INTO projects (id, name, root_path, sort_order, created_at) VALUES (?, ?, ?, ?, ?)",
            params![project.id, project.name, project.root_path, next, now_ms()],
        )?;
        tx.execute(
            "INSERT INTO workspaces (id, project_id, kind, name, path, created_at)
             VALUES (?, ?, 'local', 'local', ?, ?)",
            params![new_id(), project.id, project.root_path, now_ms()],
        )?;
        tx.commit()?;
        Ok(project)
    }

    /// The projects on show. A removed one is left out; see [`Self::removed_project_by_root`].
    pub fn projects(&self) -> StoreResult<Vec<ProjectRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, name, root_path FROM projects WHERE removed = 0
             ORDER BY sort_order, created_at",
        )?;
        let rows = stmt.query_map([], project_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// A project removed with its history kept, if that folder was one: opening it again is a
    /// revival, not a fresh add.
    pub fn removed_project_by_root(&self, root_path: &str) -> StoreResult<Option<ProjectRow>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, name, root_path FROM projects WHERE root_path = ? AND removed = 1",
                [root_path],
                project_from_row,
            )
            .optional()?)
    }

    /// Show a removed project again, at the end of the list like a project just added.
    pub fn revive_project(&self, id: &str) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE projects SET removed = 0,
                 sort_order = (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM projects)
             WHERE id = ? AND removed = 1",
            [id],
        )? > 0)
    }

    pub fn project_by_root(&self, root_path: &str) -> StoreResult<Option<ProjectRow>> {
        Ok(self
            .projects()?
            .into_iter()
            .find(|project| project.root_path == root_path))
    }

    /// Workspaces of every project on show: `local` first, then active ones, then archived ones.
    /// Forgotten ones, and those of removed projects, are left out; [`Self::workspace`] still
    /// finds them by id.
    pub fn workspaces(&self) -> StoreResult<Vec<WorkspaceRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {WORKSPACE_COLUMNS} FROM workspaces
             WHERE forgotten = 0 AND project_id IN (SELECT id FROM projects WHERE removed = 0)
             ORDER BY project_id, kind = 'local' DESC, status = 'active' DESC, sort_order, created_at",
        ))?;
        let rows = stmt.query_map([], workspace_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Every path Yardsort has a row for, forgotten ones included: what adoption must not touch.
    pub fn workspace_paths(&self) -> StoreResult<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT path FROM workspaces")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn workspace(&self, id: &str) -> StoreResult<Option<WorkspaceRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE id = ?"),
                [id],
                workspace_from_row,
            )
            .optional()?)
    }

    /// Record that a workspace was started from a task. Saying so twice changes nothing.
    pub fn link_task(&self, workspace_id: &str, task: &WorkspaceTaskRow) -> StoreResult<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO workspace_tasks
                 (workspace_id, source, repo, key, url, title, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                workspace_id,
                task.source,
                task.repo,
                task.key,
                task.url,
                task.title,
                now_ms()
            ],
        )?;
        Ok(())
    }

    /// The tasks a workspace was started from, oldest first.
    pub fn workspace_tasks(&self, workspace_id: &str) -> StoreResult<Vec<WorkspaceTaskRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT source, repo, key, url, title FROM workspace_tasks
             WHERE workspace_id = ? ORDER BY created_at, rowid",
        )?;
        let rows = stmt.query_map([workspace_id], |row| {
            Ok(WorkspaceTaskRow {
                source: row.get(0)?,
                repo: row.get(1)?,
                key: row.get(2)?,
                url: row.get(3)?,
                title: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Follow a pull request in another repository with a workspace. Saying so twice changes
    /// nothing: the row already there is returned, with `false`. A session the first time did
    /// not know is filled in by a later one that does.
    pub fn track_pull_request(
        &self,
        new: &NewTrackedPullRequest<'_>,
    ) -> StoreResult<(TrackedPullRequestRow, bool)> {
        let conn = self.conn();
        let added = conn.execute(
            "INSERT OR IGNORE INTO tracked_pull_requests
                 (id, workspace_id, session_id, repo, number, url, tracked_by, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                uuid::Uuid::new_v4().to_string(),
                new.workspace_id,
                new.session_id,
                new.repo,
                new.number,
                new.url,
                new.tracked_by,
                now_ms()
            ],
        )? > 0;
        if !added && new.session_id.is_some() {
            conn.execute(
                "UPDATE tracked_pull_requests SET session_id = ?
                 WHERE workspace_id = ? AND url = ? AND session_id IS NULL",
                params![new.session_id, new.workspace_id, new.url],
            )?;
        }
        let row = conn.query_row(
            &format!(
                "SELECT {TRACKED_COLUMNS} FROM tracked_pull_requests
                 WHERE workspace_id = ? AND url = ?"
            ),
            params![new.workspace_id, new.url],
            tracked_from_row,
        )?;
        Ok((row, added))
    }

    /// The pull requests a workspace follows elsewhere, newest tracked first.
    pub fn tracked_pull_requests(
        &self,
        workspace_id: &str,
    ) -> StoreResult<Vec<TrackedPullRequestRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {TRACKED_COLUMNS} FROM tracked_pull_requests
             WHERE workspace_id = ? ORDER BY created_at DESC, rowid DESC"
        ))?;
        let rows = stmt.query_map([workspace_id], tracked_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Every pull request the project's shown workspaces follow elsewhere, newest tracked
    /// first. A forgotten workspace's are kept for when it comes back, and not listed.
    pub fn tracked_pull_requests_in_project(
        &self,
        project_id: &str,
    ) -> StoreResult<Vec<TrackedPullRequestRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT t.id, t.workspace_id, t.session_id, t.repo, t.number, t.url, t.tracked_by,
                    t.created_at
             FROM tracked_pull_requests t JOIN workspaces w ON w.id = t.workspace_id
             WHERE w.project_id = ? AND w.forgotten = 0
             ORDER BY t.created_at DESC, t.rowid DESC",
        )?;
        let rows = stmt.query_map([project_id], tracked_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// When a tracked pull request was tracked, for tests that keep a made-up clock.
    #[cfg(test)]
    pub(crate) fn set_tracked_at(&self, id: &str, at: i64) -> StoreResult<()> {
        self.conn().execute(
            "UPDATE tracked_pull_requests SET created_at = ? WHERE id = ?",
            params![at, id],
        )?;
        Ok(())
    }

    /// Stop following one, by its id or its link. Returns whether there was one.
    pub fn untrack_pull_request(&self, workspace_id: &str, id_or_url: &str) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "DELETE FROM tracked_pull_requests WHERE workspace_id = ? AND (id = ? OR url = ?)",
            params![workspace_id, id_or_url, id_or_url],
        )? > 0)
    }

    /// The forgotten row for a path, if there is one: importing that worktree again revives it.
    pub fn forgotten_workspace_at(&self, path: &str) -> StoreResult<Option<WorkspaceRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!(
                    "SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE path = ? AND forgotten = 1"
                ),
                [path],
                workspace_from_row,
            )
            .optional()?)
    }

    pub fn project(&self, id: &str) -> StoreResult<Option<ProjectRow>> {
        Ok(self
            .projects()?
            .into_iter()
            .find(|project| project.id == id))
    }

    /// Record a worktree workspace, after `local` and any earlier ones.
    pub fn add_worktree(
        &self,
        project_id: &str,
        name: &str,
        path: &str,
        branch: Option<&str>,
        base_branch: Option<&str>,
    ) -> StoreResult<WorkspaceRow> {
        let conn = self.conn();
        let id = new_id();
        conn.execute(
            "INSERT INTO workspaces (id, project_id, kind, name, path, branch, base_branch, sort_order, created_at)
             VALUES (?1, ?2, 'worktree', ?3, ?4, ?5, ?6,
                     (SELECT COALESCE(MAX(sort_order), 0) + 1 FROM workspaces WHERE project_id = ?2), ?7)",
            params![id, project_id, name, path, branch, base_branch, now_ms()],
        )?;
        Ok(WorkspaceRow {
            id,
            project_id: project_id.to_owned(),
            kind: "worktree".to_owned(),
            name: name.to_owned(),
            path: path.to_owned(),
            branch: branch.map(str::to_owned),
            base_branch: base_branch.map(str::to_owned),
            archived: false,
            forgotten: false,
        })
    }

    /// Like [`Self::add_worktree`], but a path that is already a workspace is left alone and
    /// `None` comes back. Adoption uses this: finding a worktree twice must not list it twice.
    pub fn add_worktree_if_new(
        &self,
        project_id: &str,
        name: &str,
        path: &str,
        branch: Option<&str>,
    ) -> StoreResult<Option<WorkspaceRow>> {
        let known: bool = self.conn().query_row(
            "SELECT EXISTS (SELECT 1 FROM workspaces WHERE path = ?)",
            [path],
            |row| row.get(0),
        )?;
        if known {
            return Ok(None);
        }
        match self.add_worktree(project_id, name, path, branch, None) {
            Ok(row) => Ok(Some(row)),
            // Someone else recorded it between our look and our insert: same outcome.
            Err(StoreError::Sqlite(rusqlite::Error::SqliteFailure(error, _)))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Ok(None)
            }
            Err(other) => Err(other),
        }
    }

    /// Forget a worktree workspace. `local` rows cannot be removed this way.
    pub fn remove_worktree(&self, id: &str) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "DELETE FROM workspaces WHERE id = ? AND kind = 'worktree'",
            [id],
        )? > 0)
    }

    /// Give a workspace a new display name. Its folder and branch keep theirs.
    pub fn rename_workspace(&self, id: &str, name: &str) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE workspaces SET name = ? WHERE id = ? AND kind = 'worktree'",
            [name, id],
        )? > 0)
    }

    pub fn set_workspace_archived(&self, id: &str, archived: bool) -> StoreResult<bool> {
        let status = if archived { "archived" } else { "active" };
        Ok(self.conn().execute(
            "UPDATE workspaces SET status = ? WHERE id = ? AND kind = 'worktree'",
            [status, id],
        )? > 0)
    }

    /// Stop showing a worktree workspace. With `keep_history` the row stays, hidden, so its
    /// conversations are still there when the worktree is imported again; without it the row
    /// and its sessions go. `local` cannot be forgotten either way.
    pub fn forget_workspace(&self, id: &str, keep_history: bool) -> StoreResult<bool> {
        if !keep_history {
            return self.remove_worktree(id);
        }
        Ok(self.conn().execute(
            "UPDATE workspaces SET forgotten = 1 WHERE id = ? AND kind = 'worktree'",
            [id],
        )? > 0)
    }

    /// Show a forgotten workspace again, active and on whatever branch its worktree has now.
    pub fn revive_workspace(&self, id: &str, branch: Option<&str>) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE workspaces SET forgotten = 0, status = 'active', branch = ?
             WHERE id = ? AND kind = 'worktree' AND forgotten = 1",
            params![branch, id],
        )? > 0)
    }

    // --- sessions -----------------------------------------------------------------------------

    pub fn add_session(&self, new: &NewSession<'_>) -> StoreResult<()> {
        let now = now_ms();
        self.conn().execute(
            "INSERT INTO sessions (id, workspace_id, harness_id, model, effort, harness_session_id,
                                   title, forked_from, state, pty_session_id, started_at, began_at,
                                   prompt)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'running', ?, ?, ?, ?)",
            params![
                new.id,
                new.workspace_id,
                new.harness_id,
                new.model,
                new.effort,
                new.harness_session_id,
                new.title,
                new.forked_from,
                new.pty_session_id,
                now,
                now,
                new.prompt
            ],
        )?;
        Ok(())
    }

    pub fn session(&self, id: &str) -> StoreResult<Option<SessionRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE id = ?"),
                [id],
                session_from_row,
            )
            .optional()?)
    }

    /// A workspace's sessions, most recently started first.
    pub fn sessions(&self, workspace_id: &str) -> StoreResult<Vec<SessionRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {SESSION_COLUMNS} FROM sessions WHERE workspace_id = ?
             ORDER BY started_at DESC, rowid DESC"
        ))?;
        let rows = stmt.query_map([workspace_id], session_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Every workspace's sessions at once, most recently started first.
    ///
    /// For asking "which record does this running process belong to?" without a workspace in
    /// hand — which is where `ys attach` starts from, since it is given a process.
    pub fn all_sessions(&self) -> StoreResult<Vec<SessionRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {SESSION_COLUMNS} FROM sessions ORDER BY started_at DESC, rowid DESC"
        ))?;
        let rows = stmt.query_map([], session_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// What the user asked for in a workspace: the first message of each of its conversations
    /// that had one, oldest first.
    pub fn session_prompts(&self, workspace_id: &str) -> StoreResult<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT prompt FROM sessions WHERE workspace_id = ? AND prompt IS NOT NULL
             ORDER BY rowid",
        )?;
        let rows = stmt.query_map([workspace_id], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The conversations in a workspace that were started with a first message, by record id.
    pub fn prompted_sessions(&self, workspace_id: &str) -> StoreResult<HashSet<String>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT id FROM sessions WHERE workspace_id = ? AND prompt IS NOT NULL")?;
        let rows = stmt.query_map([workspace_id], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The conversation the workspace's task was given to: the oldest one with a first message.
    pub fn task_session(&self, workspace_id: &str) -> StoreResult<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id FROM sessions WHERE workspace_id = ? AND prompt IS NOT NULL
                 ORDER BY rowid LIMIT 1",
                [workspace_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// What a conversation is called in lists.
    pub fn set_session_title(&self, id: &str, title: &str) -> StoreResult<()> {
        self.conn().execute(
            "UPDATE sessions SET title = ? WHERE id = ?",
            params![title, id],
        )?;
        Ok(())
    }

    /// A resumed session runs again, in a new PTY.
    pub fn mark_session_running(&self, id: &str, pty_session_id: &str) -> StoreResult<()> {
        self.conn().execute(
            "UPDATE sessions SET state = 'running', pty_session_id = ?, exit_code = NULL,
                                 ended_at = NULL, started_at = ?
             WHERE id = ?",
            params![pty_session_id, now_ms(), id],
        )?;
        Ok(())
    }

    /// The process behind a PTY session ended. Returns the record it belonged to, if any.
    pub fn end_session_by_pty(
        &self,
        pty_session_id: &str,
        exit_code: Option<i64>,
    ) -> StoreResult<Option<String>> {
        self.end_session_by_pty_at(pty_session_id, exit_code, now_ms())
    }

    /// [`Self::end_session_by_pty`], with the moment the process ended as whoever saw it
    /// reports it — the daemon's clock for an exit kept in its spool overnight.
    pub fn end_session_by_pty_at(
        &self,
        pty_session_id: &str,
        exit_code: Option<i64>,
        ended_at: i64,
    ) -> StoreResult<Option<String>> {
        let conn = self.conn();
        let id: Option<String> = conn
            .query_row(
                "SELECT id FROM sessions WHERE pty_session_id = ? AND state = 'running'",
                [pty_session_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = &id {
            conn.execute(
                "UPDATE sessions SET state = 'ended', exit_code = ?, pty_session_id = NULL, ended_at = ?
                 WHERE id = ?",
                params![exit_code, ended_at, id],
            )?;
        }
        Ok(id)
    }

    /// Settle the rows that only *claim* to be running.
    ///
    /// `alive` is the PTY sessions the daemon is actually running — before the daemon, that was
    /// always empty and every row was cut short at startup. Now a conversation whose process is
    /// still there stays `running`, and comes back with its terminal when the app reopens.
    /// Returns how many rows were ended.
    pub fn end_interrupted_sessions(&self, alive: &[String]) -> StoreResult<usize> {
        let conn = self.conn();
        // Built rather than bound as a list: rusqlite has no array binding, and these ids are
        // ours (UUIDs from the host), never user input.
        let keep = alive
            .iter()
            .map(|id| format!("'{}'", id.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",");
        Ok(conn.execute(
            &format!(
                "UPDATE sessions SET state = 'ended', exit_code = NULL, pty_session_id = NULL,
                                     ended_at = COALESCE(ended_at, ?)
                 WHERE state = 'running'
                   AND (pty_session_id IS NULL OR pty_session_id NOT IN ({keep}))"
            ),
            [now_ms()],
        )?)
    }

    pub fn remove_session(&self, id: &str) -> StoreResult<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM sessions WHERE id = ?", [id])?
            > 0)
    }

    /// Forget a project and its workspaces. Returns whether it existed.
    /// Take a project off the list. With `keep_history` it is only hidden, workspaces and
    /// sessions intact, until the same folder is opened again; without it the row goes, and
    /// its workspaces and sessions with it. Files on disk are never touched either way.
    pub fn remove_project(&self, id: &str, keep_history: bool) -> StoreResult<bool> {
        let conn = self.conn();
        let changed = if keep_history {
            conn.execute(
                "UPDATE projects SET removed = 1 WHERE id = ? AND removed = 0",
                [id],
            )?
        } else {
            conn.execute("DELETE FROM projects WHERE id = ?", [id])?
        };
        Ok(changed > 0)
    }

    /// Put projects in the given order. Ids not mentioned keep their relative order, after.
    pub fn reorder_projects(&self, ordered_ids: &[String]) -> StoreResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let count = i64::try_from(ordered_ids.len()).unwrap_or(i64::MAX);
        tx.execute("UPDATE projects SET sort_order = sort_order + ?", [count])?;
        for (index, id) in ordered_ids.iter().enumerate() {
            tx.execute(
                "UPDATE projects SET sort_order = ? WHERE id = ?",
                params![i64::try_from(index).unwrap_or(i64::MAX), id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn ui_state(&self) -> StoreResult<BTreeMap<String, String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT key, value FROM ui_state")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn set_ui_state(&self, key: &str, value: &str) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO ui_state (key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }

    // --- outcomes --------------------------------------------------------------------------

    /// Write or refresh what an outcome describes, keeping everything already known about it: its
    /// label, its evidence, how it ended. The first snapshot creates the row.
    pub fn snapshot_outcome(&self, snapshot: &OutcomeSnapshot<'_>) -> StoreResult<()> {
        let OutcomeSnapshot {
            id,
            project_id,
            workspace_name,
            branch,
            base_branch,
            task,
            harnesses,
        } = *snapshot;
        let at = now_ms();
        self.conn().execute(
            "INSERT INTO workspace_outcomes (id, project_id, workspace_id, workspace_name, branch,
                                             base_branch, task, harnesses, began_at, created_at,
                                             updated_at)
             VALUES (?1, ?2, ?1, ?3, ?4, ?5, ?6, ?7,
                     (SELECT created_at FROM workspaces WHERE id = ?1), ?8, ?8)
             ON CONFLICT(id) DO UPDATE SET
                 workspace_name = excluded.workspace_name,
                 began_at = COALESCE(began_at, excluded.began_at),
                 branch = COALESCE(excluded.branch, branch),
                 base_branch = COALESCE(excluded.base_branch, base_branch),
                 task = COALESCE(excluded.task, task),
                 harnesses = CASE WHEN excluded.harnesses = '' THEN harnesses
                                  ELSE excluded.harnesses END,
                 updated_at = excluded.updated_at",
            params![
                id,
                project_id,
                workspace_name,
                branch,
                base_branch,
                task,
                harnesses,
                at
            ],
        )?;
        Ok(())
    }

    pub fn outcome(&self, id: &str) -> StoreResult<Option<OutcomeRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {OUTCOME_COLUMNS} FROM workspace_outcomes WHERE id = ?"
        ))?;
        Ok(stmt.query_row([id], outcome_from_row).optional()?)
    }

    /// A project's outcomes, or every project's with `None`; newest first.
    pub fn outcomes(&self, project_id: Option<&str>) -> StoreResult<Vec<OutcomeRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {OUTCOME_COLUMNS} FROM workspace_outcomes
             WHERE ?1 IS NULL OR project_id = ?1
             ORDER BY COALESCE(began_at, created_at) DESC, rowid DESC"
        ))?;
        let rows = stmt.query_map([project_id], outcome_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The user's word on an attempt, or `None` to take it back.
    pub fn label_outcome(&self, id: &str, label: Option<&str>) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE workspace_outcomes SET label = ?, labeled_at = CASE WHEN ? IS NULL THEN NULL
                                                                   ELSE ? END, updated_at = ?
             WHERE id = ?",
            params![label, label, now_ms(), now_ms(), id],
        )? > 0)
    }

    /// Git's evidence: the branch seen ahead of its base with this tip (its first time kept,
    /// its tip the latest), and — once — that tip found reachable from the base.
    pub fn outcome_git(&self, id: &str, ahead_tip: Option<&str>, merged: bool) -> StoreResult<()> {
        let at = now_ms();
        let conn = self.conn();
        if let Some(tip) = ahead_tip {
            conn.execute(
                "UPDATE workspace_outcomes SET ahead_at = COALESCE(ahead_at, ?), ahead_tip = ?
                 WHERE id = ?",
                params![at, tip, id],
            )?;
        }
        if merged {
            conn.execute(
                "UPDATE workspace_outcomes SET merged_at = COALESCE(merged_at, ?)
                 WHERE id = ? AND ahead_tip IS NOT NULL",
                params![at, id],
            )?;
        }
        Ok(())
    }

    /// The forge's evidence: the pull request whose head is the attempt's branch.
    pub fn outcome_pull_request(&self, id: &str, number: i64, state: &str) -> StoreResult<()> {
        self.conn().execute(
            "UPDATE workspace_outcomes SET pr_number = ?, pr_state = ?, updated_at = ? WHERE id = ?",
            params![number, state, now_ms(), id],
        )?;
        Ok(())
    }

    /// How the workspace ended, once: archived or deleted. Restoring clears it.
    pub fn outcome_ended(&self, id: &str, ended: Option<&str>) -> StoreResult<()> {
        self.conn().execute(
            "UPDATE workspace_outcomes SET ended = ?, ended_at = CASE WHEN ? IS NULL THEN NULL
                                                                 ELSE ? END WHERE id = ?",
            params![ended, ended, now_ms(), id],
        )?;
        Ok(())
    }

    // --- memory ----------------------------------------------------------------------------

    /// Write a new entry and its first history line, in one transaction. Returns its id.
    pub fn add_memory(&self, new: &NewMemory<'_>, action: &str) -> StoreResult<String> {
        let id = new_id();
        let at = now_ms();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO memory_entries (id, project_id, text, state, author, harness_id,
                                         workspace_id, workspace_name, run_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                id,
                new.project_id,
                new.text,
                new.state,
                new.author,
                new.harness_id,
                new.workspace_id,
                new.workspace_name,
                new.run_id,
                at,
                at
            ],
        )?;
        tx.execute(
            "INSERT INTO memory_history (entry_id, at, action, by, previous_text)
             VALUES (?, ?, ?, ?, NULL)",
            params![id, at, action, new.author],
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// Add a candidate unless the same text is already in the project or the queue is full —
    /// all three decided in one immediate transaction, so proposals racing from separate
    /// processes cannot both pass the checks and then both insert.
    pub fn propose_memory(
        &self,
        new: &NewMemory<'_>,
        max_waiting: usize,
    ) -> StoreResult<MemoryInsert> {
        let at = now_ms();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let known: Option<(String, String)> = tx
            .query_row(
                "SELECT id, state FROM memory_entries
                 WHERE project_id = ? AND text = ? COLLATE NOCASE LIMIT 1",
                params![new.project_id, new.text],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((id, state)) = known {
            return Ok(MemoryInsert::Known { id, state });
        }
        let waiting: i64 = tx.query_row(
            "SELECT COUNT(*) FROM memory_entries WHERE project_id = ? AND state = 'candidate'",
            [new.project_id],
            |r| r.get(0),
        )?;
        if usize::try_from(waiting).unwrap_or(usize::MAX) >= max_waiting {
            return Ok(MemoryInsert::Full);
        }
        let id = new_id();
        tx.execute(
            "INSERT INTO memory_entries (id, project_id, text, state, author, harness_id,
                                         workspace_id, workspace_name, run_id, created_at, updated_at)
             VALUES (?, ?, ?, 'candidate', ?, ?, ?, ?, ?, ?, ?)",
            params![
                id,
                new.project_id,
                new.text,
                new.author,
                new.harness_id,
                new.workspace_id,
                new.workspace_name,
                new.run_id,
                at,
                at
            ],
        )?;
        tx.execute(
            "INSERT INTO memory_history (entry_id, at, action, by, previous_text)
             VALUES (?, ?, 'proposed', ?, NULL)",
            params![id, at, new.author],
        )?;
        tx.commit()?;
        Ok(MemoryInsert::Added(id))
    }

    pub fn memory_entry(&self, id: &str) -> StoreResult<Option<MemoryRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {MEMORY_COLUMNS} FROM memory_entries WHERE id = ?"
        ))?;
        Ok(stmt.query_row([id], memory_from_row).optional()?)
    }

    /// A project's entries, oldest first.
    pub fn memory_entries(&self, project_id: &str) -> StoreResult<Vec<MemoryRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {MEMORY_COLUMNS} FROM memory_entries WHERE project_id = ?
             ORDER BY created_at, rowid"
        ))?;
        let rows = stmt.query_map([project_id], memory_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Change an entry's state and, optionally, its text, and write the history line for it.
    /// An edit keeps the text it replaced.
    pub fn change_memory(
        &self,
        id: &str,
        state: &str,
        text: Option<&str>,
        action: &str,
    ) -> StoreResult<bool> {
        let at = now_ms();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let previous: Option<String> = tx
            .query_row("SELECT text FROM memory_entries WHERE id = ?", [id], |r| {
                r.get(0)
            })
            .optional()?;
        let Some(previous) = previous else {
            return Ok(false);
        };
        tx.execute(
            "UPDATE memory_entries SET state = ?, text = COALESCE(?, text), updated_at = ?
             WHERE id = ?",
            params![state, text, at, id],
        )?;
        let replaced = text.filter(|t| *t != previous).map(|_| previous);
        tx.execute(
            "INSERT INTO memory_history (entry_id, at, action, by, previous_text)
             VALUES (?, ?, ?, 'user', ?)",
            params![id, at, action, replaced],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// An entry's history, oldest first.
    pub fn memory_history(&self, id: &str) -> StoreResult<Vec<MemoryHistoryRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT at, action, by, previous_text FROM memory_history WHERE entry_id = ?
             ORDER BY seq",
        )?;
        let rows = stmt.query_map([id], |r| {
            Ok(MemoryHistoryRow {
                at: r.get(0)?,
                action: r.get(1)?,
                by: r.get(2)?,
                previous_text: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Whether a project's approved entries go into its agents' first messages. They do until
    /// the user says otherwise: a project with no row here has never been asked, and shares.
    pub fn memory_shared(&self, project_id: &str) -> StoreResult<bool> {
        let conn = self.conn();
        Ok(conn
            .query_row(
                "SELECT share FROM project_memory WHERE project_id = ?",
                [project_id],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .is_none_or(|share| share != 0))
    }

    pub fn set_memory_shared(&self, project_id: &str, share: bool) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO project_memory (project_id, share) VALUES (?, ?)
             ON CONFLICT(project_id) DO UPDATE SET share = excluded.share",
            params![project_id, i64::from(share)],
        )?;
        Ok(())
    }

    /// Whether a project's agents may read each other's conversations: yes unless the user has
    /// turned it off. A project with no row here shares — the default since 2026-10-26; migration
    /// 0016 describes the first one, off.
    pub fn conversations_shared(&self, project_id: &str) -> StoreResult<bool> {
        let conn = self.conn();
        Ok(conn
            .query_row(
                "SELECT share FROM project_conversations WHERE project_id = ?",
                [project_id],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .is_none_or(|share| share != 0))
    }

    pub fn set_conversations_shared(&self, project_id: &str, share: bool) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO project_conversations (project_id, share) VALUES (?, ?)
             ON CONFLICT(project_id) DO UPDATE SET share = excluded.share",
            params![project_id, i64::from(share)],
        )?;
        Ok(())
    }

    /// How many candidates wait in each project, for the counts on the project rows.
    pub fn memory_waiting(&self) -> StoreResult<BTreeMap<String, u32>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT project_id, COUNT(*) FROM memory_entries WHERE state = 'candidate'
             GROUP BY project_id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    // --- activity: runs, events, diagnostics --------------------------------------------------

    /// Record a run that is about to be spawned. Its PTY id arrives with [`Self::run_spawned`].
    pub fn add_run(&self, new: &NewRun<'_>) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO agent_runs (id, workspace_id, session_id, kind, harness_id,
                                     harness_session_id, launched_by, started_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                new.id,
                new.workspace_id,
                new.session_id,
                new.kind,
                new.harness_id,
                new.harness_session_id,
                new.launched_by,
                now_ms()
            ],
        )?;
        Ok(())
    }

    pub fn run(&self, id: &str) -> StoreResult<Option<RunRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {RUN_COLUMNS} FROM agent_runs WHERE id = ?"),
                [id],
                run_from_row,
            )
            .optional()?)
    }

    pub fn run_by_pty(&self, pty_session_id: &str) -> StoreResult<Option<RunRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {RUN_COLUMNS} FROM agent_runs WHERE pty_session_id = ?"),
                [pty_session_id],
                run_from_row,
            )
            .optional()?)
    }

    /// The newest run recorded under a harness's own session id, an open one first: the
    /// fallback key for a reported event whose environment did not name its run.
    pub fn run_by_harness_session(&self, harness_session_id: &str) -> StoreResult<Option<RunRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!(
                    "SELECT {RUN_COLUMNS} FROM agent_runs WHERE harness_session_id = ?
                     ORDER BY (ended_at IS NULL) DESC, started_at DESC, rowid DESC LIMIT 1"
                ),
                [harness_session_id],
                run_from_row,
            )
            .optional()?)
    }

    /// A harness's runs that are still going, or ended after `ended_after`: the ones whose
    /// own session files may still gain lines worth reading.
    pub fn live_runs(&self, harness_id: &str, ended_after: i64) -> StoreResult<Vec<RunRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {RUN_COLUMNS} FROM agent_runs
             WHERE harness_id = ? AND pty_session_id IS NOT NULL
               AND (ended_at IS NULL OR ended_at > ?)
             ORDER BY started_at DESC, rowid DESC"
        ))?;
        let rows = stmt.query_map(params![harness_id, ended_after], run_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// A workspace's runs, most recently started first.
    pub fn runs(&self, workspace_id: &str) -> StoreResult<Vec<RunRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {RUN_COLUMNS} FROM agent_runs WHERE workspace_id = ?
             ORDER BY started_at DESC, rowid DESC"
        ))?;
        let rows = stmt.query_map([workspace_id], run_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The ids an agent reported for a conversation in its events' `field` — Codex's
    /// `threadId`, the `sessionId` of Cursor and OpenCode — first reported first: what names its
    /// transcript. Only a conversation that was reporting has any.
    pub fn reported_ids(&self, session_id: &str, field: &str) -> StoreResult<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT json_extract(payload, ?2) AS reported FROM agent_events
             WHERE (session_id = ?1
                    OR run_id IN (SELECT id FROM agent_runs WHERE session_id = ?1))
               AND reported IS NOT NULL
             GROUP BY reported ORDER BY MIN(seq)",
        )?;
        let rows = stmt.query_map(params![session_id, format!("$.{field}")], |r| {
            r.get::<_, String>(0)
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The host has spawned the run's process.
    pub fn run_spawned(&self, id: &str, pty_session_id: &str) -> StoreResult<()> {
        self.conn().execute(
            "UPDATE agent_runs SET pty_session_id = ? WHERE id = ?",
            params![pty_session_id, id],
        )?;
        Ok(())
    }

    /// The run now has a session record: a harness conversation's row is written after its
    /// process is up, so the link is made afterwards. Events already written for the run are
    /// linked too.
    pub fn link_run_session(&self, run_id: &str, session_id: &str) -> StoreResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE agent_runs SET session_id = ? WHERE id = ?",
            params![session_id, run_id],
        )?;
        tx.execute(
            "UPDATE agent_events SET session_id = ? WHERE run_id = ? AND session_id IS NULL",
            params![session_id, run_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// End a run, once. Returns false when it had already ended (or does not exist).
    pub fn end_run(&self, id: &str, exit_code: Option<i64>, end_reason: &str) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE agent_runs SET ended_at = ?, exit_code = ?, end_reason = ?
             WHERE id = ? AND ended_at IS NULL",
            params![now_ms(), exit_code, end_reason, id],
        )? > 0)
    }

    /// End the run behind a PTY session, once, at `ended_at` — the reporter's clock, not this
    /// one's. Returns the run it was, if it was still open.
    pub fn end_run_by_pty(
        &self,
        pty_session_id: &str,
        exit_code: Option<i64>,
        end_reason: &str,
        ended_at: i64,
    ) -> StoreResult<Option<RunRow>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let open = tx
            .query_row(
                &format!(
                    "SELECT {RUN_COLUMNS} FROM agent_runs
                     WHERE pty_session_id = ? AND ended_at IS NULL"
                ),
                [pty_session_id],
                run_from_row,
            )
            .optional()?;
        if let Some(run) = &open {
            tx.execute(
                "UPDATE agent_runs SET ended_at = ?, exit_code = ?, end_reason = ? WHERE id = ?",
                params![ended_at, exit_code, end_reason, run.id],
            )?;
        }
        tx.commit()?;
        Ok(open)
    }

    /// End the runs whose process is not among `alive` — the PTY sessions the daemon is
    /// actually running — as interrupted, and return them so their exits can be recorded.
    ///
    /// A run that never got a PTY id is only counted once it is older than `pending_grace_ms`:
    /// another client may be between writing the row and spawning right now.
    pub fn end_interrupted_runs(
        &self,
        alive: &[String],
        pending_grace_ms: i64,
    ) -> StoreResult<Vec<RunRow>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let keep = alive
            .iter()
            .map(|id| format!("'{}'", id.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",");
        let now = now_ms();
        let gone: Vec<RunRow> = {
            let mut stmt = tx.prepare(&format!(
                "SELECT {RUN_COLUMNS} FROM agent_runs
                 WHERE ended_at IS NULL
                   AND ((pty_session_id IS NULL AND started_at < ?)
                        OR (pty_session_id IS NOT NULL AND pty_session_id NOT IN ({keep})))"
            ))?;
            let rows = stmt.query_map([now - pending_grace_ms], run_from_row)?;
            rows.collect::<Result<_, _>>()?
        };
        for run in &gone {
            tx.execute(
                "UPDATE agent_runs SET ended_at = ?, exit_code = NULL, end_reason = 'interrupted'
                 WHERE id = ?",
                params![now, run.id],
            )?;
        }
        tx.commit()?;
        Ok(gone)
    }

    /// Record an event. Returns false when its source key had been seen before: the same fact
    /// delivered twice is one row.
    pub fn add_event(&self, new: &NewEvent<'_>) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "INSERT OR IGNORE INTO agent_events
                 (id, schema_version, workspace_id, session_id, run_id, occurred_at, received_at,
                  kind, producer, method, fidelity, source_key, privacy_class, payload)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                new.id,
                i64::from(new.schema_version),
                new.workspace_id,
                new.session_id,
                new.run_id,
                new.occurred_at,
                now_ms(),
                new.kind,
                new.producer,
                new.method,
                new.fidelity,
                new.source_key,
                new.privacy_class,
                new.payload
            ],
        )? > 0)
    }

    /// A page of a workspace's events, newest first: those with a sequence number below
    /// `before`, at most `limit` of them. `None` starts from the newest.
    pub fn events(
        &self,
        workspace_id: &str,
        before: Option<i64>,
        limit: usize,
    ) -> StoreResult<Vec<EventRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {EVENT_COLUMNS} FROM agent_events
             WHERE workspace_id = ?1 AND seq < ?2 ORDER BY seq DESC LIMIT ?3"
        ))?;
        let rows = stmt.query_map(
            params![
                workspace_id,
                before.unwrap_or(i64::MAX),
                i64::try_from(limit).unwrap_or(i64::MAX)
            ],
            event_from_row,
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Up to `limit` of a workspace's events of one privacy class, optionally one session's, on
    /// one side of a sequence number — oldest first whichever side. For readers that page in
    /// both directions; the timeline uses [`Store::events`].
    pub fn events_window(
        &self,
        filter: &EventFilter<'_>,
        bound: EventBound,
        limit: usize,
    ) -> StoreResult<Vec<EventRow>> {
        let (condition, order, seq) = match bound {
            EventBound::Newest => ("seq < ?4", "DESC", i64::MAX),
            EventBound::Before(seq) => ("seq < ?4", "DESC", seq),
            EventBound::After(seq) => ("seq > ?4", "ASC", seq),
        };
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {EVENT_COLUMNS} FROM agent_events
             WHERE workspace_id IN (SELECT value FROM json_each(?1)) AND privacy_class = ?2
               AND (?3 IS NULL OR session_id = ?3) AND {condition}
             ORDER BY seq {order} LIMIT ?5"
        ))?;
        let rows = stmt.query_map(
            params![
                id_list(filter.workspace_ids),
                filter.privacy_class,
                filter.session_id,
                seq,
                i64::try_from(limit).unwrap_or(i64::MAX)
            ],
            event_from_row,
        )?;
        let mut rows: Vec<EventRow> = rows.collect::<Result<_, _>>()?;
        if order == "DESC" {
            rows.reverse();
        }
        Ok(rows)
    }

    /// The sequence number of the newest event kept, of any workspace or class.
    pub fn newest_event_seq(&self) -> StoreResult<Option<i64>> {
        Ok(self
            .conn()
            .query_row("SELECT MAX(seq) FROM agent_events", [], |row| row.get(0))?)
    }

    /// Whether the event at `seq` is still kept and matches `filter`.
    pub fn event_matches(&self, filter: &EventFilter<'_>, seq: i64) -> StoreResult<bool> {
        Ok(self
            .conn()
            .query_row(
                "SELECT 1 FROM agent_events
                 WHERE seq = ?1 AND workspace_id IN (SELECT value FROM json_each(?2))
                   AND privacy_class = ?3 AND (?4 IS NULL OR session_id = ?4)",
                params![
                    seq,
                    id_list(filter.workspace_ids),
                    filter.privacy_class,
                    filter.session_id
                ],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// A workspace's events of the given kinds, oldest first. For a join that reads one or
    /// two kinds out of everything a busy workspace recorded, without paging through the rest.
    pub fn events_of_kinds(
        &self,
        workspace_id: &str,
        kinds: &[&str],
    ) -> StoreResult<Vec<EventRow>> {
        if kinds.is_empty() {
            return Ok(vec![]);
        }
        let conn = self.conn();
        let marks = kinds.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let mut stmt = conn.prepare(&format!(
            "SELECT {EVENT_COLUMNS} FROM agent_events
             WHERE workspace_id = ? AND kind IN ({marks}) ORDER BY seq"
        ))?;
        let params = std::iter::once(workspace_id).chain(kinds.iter().copied());
        let rows = stmt.query_map(rusqlite::params_from_iter(params), event_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The newest of one session's events of the given kinds. Scoped by workspace too, so the
    /// timeline index does the narrowing.
    pub fn latest_session_event_of_kinds(
        &self,
        workspace_id: &str,
        session_id: &str,
        kinds: &[&str],
    ) -> StoreResult<Option<EventRow>> {
        if kinds.is_empty() {
            return Ok(None);
        }
        let conn = self.conn();
        let marks = kinds.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let mut stmt = conn.prepare(&format!(
            "SELECT {EVENT_COLUMNS} FROM agent_events
             WHERE workspace_id = ? AND session_id = ? AND kind IN ({marks})
             ORDER BY occurred_at DESC, seq DESC LIMIT 1"
        ))?;
        let params = [workspace_id, session_id]
            .into_iter()
            .chain(kinds.iter().copied());
        let mut rows = stmt.query_map(rusqlite::params_from_iter(params), event_from_row)?;
        Ok(rows.next().transpose()?)
    }

    /// Which of a workspace's runs have events from somewhere other than Yardsort's own
    /// lifecycle, and through which method: one `(run id, method)` per pair, in first-seen
    /// order. What a run was asked to report survives here after its start event is gone.
    pub fn native_methods_by_run(&self, workspace_id: &str) -> StoreResult<Vec<(String, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT run_id, method, MIN(seq) AS first FROM agent_events
             WHERE workspace_id = ? AND run_id IS NOT NULL AND producer <> 'yardsort'
             GROUP BY run_id, method ORDER BY first",
        )?;
        let rows = stmt.query_map([workspace_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Every event, oldest first — one workspace's, or all of them. For export.
    pub fn all_events(&self, workspace_id: Option<&str>) -> StoreResult<Vec<EventRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {EVENT_COLUMNS} FROM agent_events
             WHERE ?1 IS NULL OR workspace_id = ?1 ORDER BY seq"
        ))?;
        let rows = stmt.query_map([workspace_id], event_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// How many events and runs there are.
    pub fn activity_counts(&self) -> StoreResult<(i64, i64)> {
        let conn = self.conn();
        let events = conn.query_row("SELECT COUNT(*) FROM agent_events", [], |r| r.get(0))?;
        let runs = conn.query_row("SELECT COUNT(*) FROM agent_runs", [], |r| r.get(0))?;
        Ok((events, runs))
    }

    /// Keep activity bounded: drop events older than `older_than_ms`, then the oldest beyond
    /// `max_rows`. Runs left without events go too, unless they are still open. Returns how
    /// many events went.
    pub fn prune_activity(&self, max_rows: usize, older_than_ms: i64) -> StoreResult<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut removed = tx.execute(
            "DELETE FROM agent_events WHERE received_at < ?",
            [now_ms() - older_than_ms],
        )?;
        removed += tx.execute(
            "DELETE FROM agent_events WHERE seq <= (
                 SELECT seq FROM agent_events ORDER BY seq DESC LIMIT 1 OFFSET ?)",
            [i64::try_from(max_rows).unwrap_or(i64::MAX)],
        )?;
        tx.execute(
            "DELETE FROM agent_runs WHERE ended_at IS NOT NULL
               AND NOT EXISTS (SELECT 1 FROM agent_events WHERE run_id = agent_runs.id)",
            [],
        )?;
        tx.commit()?;
        Ok(removed)
    }

    /// Forget recorded activity — one workspace's, or everything. Runs still open keep their
    /// row so their exit can still be matched; everything else goes.
    pub fn clear_activity(&self, workspace_id: Option<&str>) -> StoreResult<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let removed = tx.execute(
            "DELETE FROM agent_events WHERE ?1 IS NULL OR workspace_id = ?1",
            [workspace_id],
        )?;
        tx.execute(
            "DELETE FROM agent_runs WHERE ended_at IS NOT NULL
               AND (?1 IS NULL OR workspace_id = ?1)",
            [workspace_id],
        )?;
        tx.commit()?;
        Ok(removed)
    }

    /// Count one more occurrence of `name`, keeping the latest detail.
    pub fn bump_diagnostic(&self, name: &str, detail: Option<&str>) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO agent_event_diagnostics (name, count, last_at, last_detail)
             VALUES (?1, 1, ?2, ?3)
             ON CONFLICT(name) DO UPDATE SET count = count + 1, last_at = excluded.last_at,
                 last_detail = COALESCE(excluded.last_detail, last_detail)",
            params![name, now_ms(), detail],
        )?;
        Ok(())
    }

    pub fn diagnostics(&self) -> StoreResult<Vec<DiagnosticRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT name, count, last_at, last_detail FROM agent_event_diagnostics ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(DiagnosticRow {
                name: row.get(0)?,
                count: row.get(1)?,
                last_at: row.get(2)?,
                last_detail: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Move a session's start, first and latest, for tests of what is read by time.
    #[cfg(test)]
    pub fn set_session_started(&self, id: &str, at: i64) {
        self.conn()
            .execute(
                "UPDATE sessions SET started_at = ?1, began_at = ?1 WHERE id = ?2",
                params![at, id],
            )
            .unwrap();
    }

    /// Make every activity write fail from here on, for tests of what happens then. The
    /// diagnostics table stays, so the failure can still be counted.
    #[cfg(any(test, feature = "testing"))]
    pub fn break_activity_tables(&self) {
        self.conn()
            .execute_batch("DROP TABLE agent_events; DROP TABLE agent_runs;")
            .unwrap();
    }

    // --- workflow runs --------------------------------------------------------------------

    /// Queue a run and its steps, unless this workflow is already queued or running in this
    /// workspace. One transaction, and a unique index behind it, so two processes cannot both.
    pub fn queue_workflow_run(&self, new: &NewWorkflowRun<'_>) -> StoreResult<Queued> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let running: Option<String> = tx
            .query_row(
                "SELECT id FROM workflow_runs WHERE workflow_id = ? AND workspace_id = ?
                   AND status IN ('queued', 'running')",
                params![new.workflow_id, new.workspace_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = running {
            return Ok(Queued::AlreadyRunning(id));
        }
        let now = now_ms();
        tx.execute(
            "INSERT INTO workflow_runs (id, workflow_id, workflow_name, definition, project_id,
                                        workspace_id, workspace_name, inputs, status,
                                        requested_by, created_at, context)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'queued', ?, ?, ?)",
            params![
                new.id,
                new.workflow_id,
                new.workflow_name,
                new.definition,
                new.project_id,
                new.workspace_id,
                new.workspace_name,
                new.inputs,
                new.requested_by,
                now,
                new.context
            ],
        )?;
        for (position, (step_id, action)) in new.steps.iter().enumerate() {
            tx.execute(
                "INSERT INTO workflow_step_runs (run_id, step_id, position, action, status)
                 VALUES (?, ?, ?, ?, 'pending')",
                params![new.id, step_id, position as i64, action],
            )?;
        }
        tx.commit()?;
        Ok(Queued::Queued)
    }

    pub fn workflow_run(&self, id: &str) -> StoreResult<Option<WorkflowRunRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {WORKFLOW_RUN_COLUMNS} FROM workflow_runs WHERE id = ?"),
                [id],
                workflow_run_from_row,
            )
            .optional()?)
    }

    /// Runs, newest first: all of them, or one workspace's.
    pub fn workflow_runs(
        &self,
        workspace_id: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<WorkflowRunRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {WORKFLOW_RUN_COLUMNS} FROM workflow_runs
             WHERE ?1 IS NULL OR workspace_id = ?1
             ORDER BY created_at DESC, rowid DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(
            params![workspace_id, i64::try_from(limit).unwrap_or(i64::MAX)],
            workflow_run_from_row,
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// One workflow's runs, newest first, at most `limit` of them: the workflow's own history,
    /// however many runs other workflows have had since.
    pub fn workflow_runs_of(
        &self,
        workflow_id: &str,
        limit: usize,
    ) -> StoreResult<Vec<WorkflowRunRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {WORKFLOW_RUN_COLUMNS} FROM workflow_runs
             WHERE workflow_id = ?1
             ORDER BY created_at DESC, rowid DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(
            params![workflow_id, i64::try_from(limit).unwrap_or(i64::MAX)],
            workflow_run_from_row,
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Queued and running runs, oldest first: what the driver has to move on.
    pub fn active_workflow_runs(&self) -> StoreResult<Vec<WorkflowRunRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {WORKFLOW_RUN_COLUMNS} FROM workflow_runs
             WHERE status IN ('queued', 'running') ORDER BY created_at, rowid"
        ))?;
        let rows = stmt.query_map([], workflow_run_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// A run's steps in file order.
    pub fn workflow_steps(&self, run_id: &str) -> StoreResult<Vec<WorkflowStepRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT run_id, step_id, position, action, status, started_at, ended_at, outputs,
                    note, outcome
             FROM workflow_step_runs WHERE run_id = ? ORDER BY position",
        )?;
        let rows = stmt.query_map([run_id], |row| {
            Ok(WorkflowStepRow {
                run_id: row.get(0)?,
                step_id: row.get(1)?,
                position: row.get(2)?,
                action: row.get(3)?,
                status: row.get(4)?,
                started_at: row.get(5)?,
                ended_at: row.get(6)?,
                outputs: row.get(7)?,
                note: row.get(8)?,
                outcome: row.get(9)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The named documents a run has handed on, by name.
    pub fn workflow_artifacts(&self, run_id: &str) -> StoreResult<Vec<WorkflowArtifactRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT run_id, name, step_id, source, detail, body, truncated, updated_at
             FROM workflow_artifacts WHERE run_id = ? ORDER BY name",
        )?;
        let rows = stmt.query_map([run_id], |row| {
            Ok(WorkflowArtifactRow {
                run_id: row.get(0)?,
                name: row.get(1)?,
                step_id: row.get(2)?,
                source: row.get(3)?,
                detail: row.get(4)?,
                body: row.get(5)?,
                truncated: row.get::<_, i64>(6)? != 0,
                updated_at: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Replace every artifact `step_id` left on this run. One transaction, so a failure keeps
    /// the previous bodies. A name this step no longer leaves is removed. A later read sees
    /// these bodies and no older one from the same step.
    pub fn replace_step_artifacts(
        &self,
        run_id: &str,
        step_id: &str,
        artifacts: &[NewWorkflowArtifact],
    ) -> StoreResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let now = now_ms();
        tx.execute(
            "DELETE FROM workflow_artifacts WHERE run_id = ? AND step_id = ?",
            params![run_id, step_id],
        )?;
        for artifact in artifacts {
            tx.execute(
                "INSERT INTO workflow_artifacts
                 (run_id, name, step_id, source, detail, body, truncated, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    run_id,
                    artifact.name,
                    step_id,
                    artifact.source,
                    artifact.detail,
                    artifact.body,
                    i64::from(artifact.truncated),
                    now,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Insert or replace one artifact. Used when a step fails and still leaves a decision, which
    /// is not the success path that replaces every artifact the step produced.
    pub fn upsert_workflow_artifact(
        &self,
        run_id: &str,
        step_id: &str,
        artifact: &NewWorkflowArtifact,
    ) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO workflow_artifacts
             (run_id, name, step_id, source, detail, body, truncated, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(run_id, name) DO UPDATE SET
               step_id = excluded.step_id,
               source = excluded.source,
               detail = excluded.detail,
               body = excluded.body,
               truncated = excluded.truncated,
               updated_at = excluded.updated_at",
            params![
                run_id,
                artifact.name,
                step_id,
                artifact.source,
                artifact.detail,
                artifact.body,
                i64::from(artifact.truncated),
                now_ms(),
            ],
        )?;
        Ok(())
    }

    /// The decision stored for one step, if a person has recorded one.
    pub fn workflow_approval(
        &self,
        run_id: &str,
        step_id: &str,
    ) -> StoreResult<Option<WorkflowApproval>> {
        let conn = self.conn();
        let found = conn
            .query_row(
                "SELECT decision, feedback, actor, decided_at
                 FROM workflow_approval_decisions
                 WHERE run_id = ?1 AND step_id = ?2",
                params![run_id, step_id],
                |row| {
                    Ok(WorkflowApproval {
                        decision: row.get(0)?,
                        feedback: row.get(1)?,
                        actor: row.get(2)?,
                        decided_at: row.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(found)
    }

    /// Drop a step's decision. A step that starts again calls this, so a later revision can be
    /// decided once more. A missing row is not an error.
    pub fn clear_workflow_approval(&self, run_id: &str, step_id: &str) -> StoreResult<()> {
        self.conn().execute(
            "DELETE FROM workflow_approval_decisions WHERE run_id = ?1 AND step_id = ?2",
            params![run_id, step_id],
        )?;
        Ok(())
    }

    /// Record a person's decision. The row is written only while the run is running, the step
    /// is an approval, and it is waiting. A second decision does not replace the first. The
    /// engine reads the row on a later tick. Nothing here moves the step.
    pub fn record_workflow_approval(
        &self,
        run_id: &str,
        step_id: &str,
        decision: &str,
        feedback: Option<&str>,
        actor: &str,
    ) -> StoreResult<ApprovalWrite> {
        if !matches!(decision, "approved" | "rejected" | "changes_requested") {
            return Ok(ApprovalWrite::NotADecision);
        }
        let (body, _) = crate::workflow::artifact::cap(feedback.unwrap_or(""));
        let feedback = {
            let trimmed = body.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        };
        if decision == "changes_requested" && feedback.is_none() {
            return Ok(ApprovalWrite::FeedbackRequired);
        }
        let actor = crate::workflow::artifact::actor_name(actor);
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let run: Option<(String,)> = tx
            .query_row(
                "SELECT status FROM workflow_runs WHERE id = ?1",
                [run_id],
                |row| Ok((row.get(0)?,)),
            )
            .optional()?;
        let Some((status,)) = run else {
            return Ok(ApprovalWrite::MissingRun);
        };
        if status != "running" {
            return Ok(ApprovalWrite::RunEnded { status });
        }
        let step: Option<(String, String)> = tx
            .query_row(
                "SELECT action, status FROM workflow_step_runs WHERE run_id = ?1 AND step_id = ?2",
                params![run_id, step_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((action, step_status)) = step else {
            return Ok(ApprovalWrite::MissingStep);
        };
        let existing: Option<String> = tx
            .query_row(
                "SELECT decision FROM workflow_approval_decisions
                 WHERE run_id = ?1 AND step_id = ?2",
                params![run_id, step_id],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Ok(ApprovalWrite::AlreadyDecided);
        }
        if action != "approve" {
            return Ok(ApprovalWrite::NotAnApproval);
        }
        if step_status != "waiting" {
            return Ok(ApprovalWrite::NotWaiting);
        }
        let inserted = tx.execute(
            "INSERT INTO workflow_approval_decisions
             (run_id, step_id, decision, feedback, actor, decided_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(run_id, step_id) DO NOTHING",
            params![run_id, step_id, decision, feedback, actor, now_ms()],
        )?;
        if inserted == 0 {
            return Ok(ApprovalWrite::AlreadyDecided);
        }
        tx.commit()?;
        Ok(ApprovalWrite::Recorded)
    }

    /// Whether a run that has just started waiting for approval should show a notification.
    ///
    /// A person who started the run is told. So is Run now on a schedule. A schedule or a
    /// trigger whose `notify` is `never` is not told. `failure` and `always` are told, the
    /// same way a smoke test waiting for approval is.
    pub fn tell_workflow_approval(&self, run_id: &str) -> StoreResult<bool> {
        let conn = self.conn();
        let schedule: Option<(String, i64)> = conn
            .query_row(
                "SELECT s.notify, r.manual FROM schedule_runs r
                 JOIN schedules s ON s.id = r.schedule_id
                 WHERE r.run_id = ?1
                 LIMIT 1",
                [run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((notify, manual)) = schedule {
            if manual != 0 {
                return Ok(true);
            }
            return Ok(notify != "never");
        }
        let trigger: Option<String> = conn
            .query_row(
                "SELECT t.notify FROM trigger_fires f
                 JOIN triggers t ON t.id = f.trigger_id
                 WHERE f.run_id = ?1
                 LIMIT 1",
                [run_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(notify) = trigger {
            return Ok(notify != "never");
        }
        Ok(true)
    }

    /// Store `artifacts` and move the step, or change neither. The move is taken only while the
    /// run is still running and the step is in one of `from`. False, and the artifacts are not
    /// written, when that move does not apply: the run stopped, or the step moved first.
    #[allow(clippy::too_many_arguments)]
    pub fn finish_step_with_artifacts(
        &self,
        run_id: &str,
        step_id: &str,
        from: &[StepStatus],
        to: StepStatus,
        artifacts: &[NewWorkflowArtifact],
        outputs: Option<&str>,
        note: Option<&str>,
        outcome: Option<&str>,
        clear_outcome: bool,
    ) -> StoreResult<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let now = now_ms();
        tx.execute(
            "DELETE FROM workflow_artifacts WHERE run_id = ? AND step_id = ?",
            params![run_id, step_id],
        )?;
        for artifact in artifacts {
            tx.execute(
                "INSERT INTO workflow_artifacts
                 (run_id, name, step_id, source, detail, body, truncated, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    run_id,
                    artifact.name,
                    step_id,
                    artifact.source,
                    artifact.detail,
                    artifact.body,
                    i64::from(artifact.truncated),
                    now,
                ],
            )?;
        }
        let marks = from.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "UPDATE workflow_step_runs
             SET status = ?1,
                 started_at = CASE WHEN ?2 THEN started_at ELSE COALESCE(started_at, ?3) END,
                 ended_at = CASE WHEN ?2 THEN ?3 ELSE ended_at END,
                 outputs = COALESCE(?4, outputs),
                 note = COALESCE(?5, note),
                 outcome = CASE WHEN ?6 THEN NULL ELSE COALESCE(?7, outcome) END
             WHERE run_id = ?8 AND step_id = ?9 AND status IN ({marks})
               AND EXISTS (SELECT 1 FROM workflow_runs WHERE id = ?8 AND status = 'running')"
        );
        let mut values: Vec<rusqlite::types::Value> = vec![
            to.as_str().to_owned().into(),
            i64::from(to.is_final()).into(),
            now.into(),
            outputs.map(str::to_owned).into(),
            note.map(str::to_owned).into(),
            i64::from(clear_outcome).into(),
            outcome.map(str::to_owned).into(),
            run_id.to_owned().into(),
            step_id.to_owned().into(),
        ];
        values.extend(from.iter().map(|status| status.as_str().to_owned().into()));
        let moved = tx.execute(&sql, rusqlite::params_from_iter(values))? > 0;
        if !moved {
            return Ok(false);
        }
        tx.commit()?;
        Ok(true)
    }

    /// A queued run starts, with `context` replacing what it knew, when given. False when it was
    /// not queued any more — cancelled in between.
    pub fn start_workflow_run(&self, id: &str, context: Option<&str>) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE workflow_runs SET status = 'running', started_at = ?,
                                      context = COALESCE(?, context)
             WHERE id = ? AND status = 'queued'",
            params![now_ms(), context, id],
        )? > 0)
    }

    /// Move a step on, only if its run is still running and the step is in one of `from`. False
    /// otherwise, and nothing changes: the run was cancelled, or someone else moved it first.
    /// `outputs` and `note`, when given, replace those columns. `clear_outcome` sets the
    /// outcome back to nothing, which a step that starts again needs. Otherwise `outcome`,
    /// when given, replaces it, and when absent leaves it.
    #[allow(clippy::too_many_arguments)]
    pub fn move_workflow_step(
        &self,
        run_id: &str,
        step_id: &str,
        from: &[StepStatus],
        to: StepStatus,
        outputs: Option<&str>,
        note: Option<&str>,
        outcome: Option<&str>,
        clear_outcome: bool,
    ) -> StoreResult<bool> {
        let now = now_ms();
        let marks = from.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "UPDATE workflow_step_runs
             SET status = ?1,
                 started_at = CASE WHEN ?2 THEN started_at ELSE COALESCE(started_at, ?3) END,
                 ended_at = CASE WHEN ?2 THEN ?3 ELSE ended_at END,
                 outputs = COALESCE(?4, outputs),
                 note = COALESCE(?5, note),
                 outcome = CASE WHEN ?6 THEN NULL ELSE COALESCE(?7, outcome) END
             WHERE run_id = ?8 AND step_id = ?9 AND status IN ({marks})
               AND EXISTS (SELECT 1 FROM workflow_runs WHERE id = ?8 AND status = 'running')"
        );
        // A step that goes straight from pending to an end, skipped say, never started, and
        // keeps no start time.
        let mut values: Vec<rusqlite::types::Value> = vec![
            to.as_str().to_owned().into(),
            i64::from(to.is_final()).into(),
            now.into(),
            outputs.map(str::to_owned).into(),
            note.map(str::to_owned).into(),
            i64::from(clear_outcome).into(),
            outcome.map(str::to_owned).into(),
            run_id.to_owned().into(),
            step_id.to_owned().into(),
        ];
        values.extend(from.iter().map(|s| s.as_str().to_owned().into()));
        Ok(self
            .conn()
            .execute(&sql, rusqlite::params_from_iter(values))?
            > 0)
    }

    /// A running run ends as `succeeded` or `failed`. False when it was not running any more.
    pub fn finish_workflow_run(
        &self,
        id: &str,
        status: &str,
        error: Option<&str>,
    ) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE workflow_runs SET status = ?, error = ?, ended_at = ?
             WHERE id = ? AND status IN ('queued', 'running')",
            params![status, error, now_ms(), id],
        )? > 0)
    }

    /// Cancel a queued or running run: the run and every step not yet finished. Sessions it
    /// started are not touched. False when the run had already ended.
    pub fn cancel_workflow_run(&self, id: &str, reason: &str) -> StoreResult<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let now = now_ms();
        let changed = tx.execute(
            "UPDATE workflow_runs SET status = 'cancelled', error = ?, ended_at = ?
             WHERE id = ? AND status IN ('queued', 'running')",
            params![reason, now, id],
        )? > 0;
        if changed {
            tx.execute(
                "UPDATE workflow_step_runs SET status = 'cancelled', ended_at = ?
                 WHERE run_id = ? AND status IN ('pending', 'running', 'waiting')",
                params![now, id],
            )?;
        }
        tx.commit()?;
        Ok(changed)
    }

    /// This machine as a host, when Remote is on.
    pub fn remote_host(&self) -> StoreResult<Option<RemoteHostRow>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT server_url, server_id, host_id, device_id, name, enabled_at
                 FROM remote_host",
                [],
                |r| {
                    Ok(RemoteHostRow {
                        server_url: r.get(0)?,
                        server_id: r.get(1)?,
                        host_id: r.get(2)?,
                        device_id: r.get(3)?,
                        name: r.get(4)?,
                        enabled_at: r.get(5)?,
                    })
                },
            )
            .optional()?)
    }

    /// Turn Remote on, or move it to another server. Grants from an earlier server are dropped
    /// in the same step: they were that server's devices.
    pub fn set_remote_host(&self, host: &RemoteHostRow) -> StoreResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let same_server = tx
            .query_row(
                "SELECT server_id = ? AND host_id = ? FROM remote_host",
                params![host.server_id, host.host_id],
                |r| r.get::<_, bool>(0),
            )
            .optional()?
            .unwrap_or(false);
        if !same_server {
            tx.execute("DELETE FROM remote_grants", [])?;
        }
        tx.execute(
            "INSERT INTO remote_host
                 (singleton, server_url, server_id, host_id, device_id, name, enabled_at)
             VALUES (1, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(singleton) DO UPDATE SET
                 server_url = excluded.server_url, server_id = excluded.server_id,
                 host_id = excluded.host_id, device_id = excluded.device_id,
                 name = excluded.name, enabled_at = excluded.enabled_at",
            params![
                host.server_url,
                host.server_id,
                host.host_id,
                host.device_id,
                host.name,
                host.enabled_at
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Turn Remote off: the host and every grant go, in one step.
    pub fn forget_remote_host(&self) -> StoreResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM remote_grants", [])?;
        tx.execute("DELETE FROM remote_host", [])?;
        tx.commit()?;
        Ok(())
    }

    /// This machine as a client of other hosts, when it is signed in as one.
    pub fn remote_computer(&self) -> StoreResult<Option<RemoteComputerRow>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT server_url, server_id, device_id, name, signed_in_at FROM remote_computer",
                [],
                |r| {
                    Ok(RemoteComputerRow {
                        server_url: r.get(0)?,
                        server_id: r.get(1)?,
                        device_id: r.get(2)?,
                        name: r.get(3)?,
                        signed_in_at: r.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn set_remote_computer(&self, computer: &RemoteComputerRow) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO remote_computer
                 (singleton, server_url, server_id, device_id, name, signed_in_at)
             VALUES (1, ?, ?, ?, ?, ?)
             ON CONFLICT(singleton) DO UPDATE SET
                 server_url = excluded.server_url, server_id = excluded.server_id,
                 device_id = excluded.device_id, name = excluded.name,
                 signed_in_at = excluded.signed_in_at",
            params![
                computer.server_url,
                computer.server_id,
                computer.device_id,
                computer.name,
                computer.signed_in_at
            ],
        )?;
        Ok(())
    }

    /// Sign out as a client: the pairings forgotten go too, in one step.
    pub fn forget_remote_computer(&self) -> StoreResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM remote_computer_forgotten", [])?;
        tx.execute("DELETE FROM remote_computer", [])?;
        tx.commit()?;
        Ok(())
    }

    /// The grants this machine, as a client, was told to forget: not shown, whatever the server
    /// lists.
    pub fn forgotten_remote_pairings(&self) -> StoreResult<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT grant_id FROM remote_computer_forgotten")?;
        let ids = stmt
            .query_map([], |r| r.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(ids)
    }

    pub fn forget_remote_pairing(&self, grant_id: &str) -> StoreResult<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO remote_computer_forgotten (grant_id) VALUES (?)",
            [grant_id],
        )?;
        Ok(())
    }

    /// The devices this host's person let in, oldest first.
    pub fn remote_grants(&self) -> StoreResult<Vec<RemoteGrantRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, device_id, device_name, scopes, projects, approved_at
             FROM remote_grants ORDER BY approved_at, id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(RemoteGrantRow {
                    id: r.get(0)?,
                    device_id: r.get(1)?,
                    device_name: r.get(2)?,
                    scopes: r.get(3)?,
                    projects: r.get(4)?,
                    approved_at: r.get(5)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// Keep a grant this host's person just approved, in place of any the device had.
    pub fn keep_remote_grant(&self, grant: &RemoteGrantRow) -> StoreResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM remote_grants WHERE device_id = ? OR id = ?",
            params![grant.device_id, grant.id],
        )?;
        tx.execute(
            "INSERT INTO remote_grants
                 (id, device_id, device_name, scopes, projects, approved_at)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![
                grant.id,
                grant.device_id,
                grant.device_name,
                grant.scopes,
                grant.projects,
                grant.approved_at
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Drop every grant but these: what the server still has in force. Only ever drops, so the
    /// server can take a grant away but never add one. Returns how many went.
    pub fn retain_remote_grants(&self, in_force: &[String]) -> StoreResult<usize> {
        let kept: std::collections::HashSet<&str> = in_force.iter().map(String::as_str).collect();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let ids: Vec<String> = tx
            .prepare("SELECT id FROM remote_grants")?
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let mut dropped = 0;
        for id in ids.iter().filter(|id| !kept.contains(id.as_str())) {
            dropped += tx.execute("DELETE FROM remote_grants WHERE id = ?", [id])?;
        }
        tx.commit()?;
        Ok(dropped)
    }

    /// Drop one grant, revoked here.
    pub fn drop_remote_grant(&self, id: &str) -> StoreResult<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM remote_grants WHERE id = ?", [id])?
            > 0)
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

const WORKFLOW_RUN_COLUMNS: &str = "id, workflow_id, workflow_name, definition, project_id, \
     workspace_id, workspace_name, inputs, status, requested_by, error, created_at, started_at, \
     ended_at, context";

fn workflow_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowRunRow> {
    Ok(WorkflowRunRow {
        id: row.get(0)?,
        workflow_id: row.get(1)?,
        workflow_name: row.get(2)?,
        definition: row.get(3)?,
        project_id: row.get(4)?,
        workspace_id: row.get(5)?,
        workspace_name: row.get(6)?,
        inputs: row.get(7)?,
        status: row.get(8)?,
        requested_by: row.get(9)?,
        error: row.get(10)?,
        created_at: row.get(11)?,
        started_at: row.get(12)?,
        ended_at: row.get(13)?,
        context: row.get(14)?,
    })
}

const RUN_COLUMNS: &str = "id, workspace_id, session_id, kind, harness_id, harness_session_id, \
     pty_session_id, launched_by, started_at, ended_at, exit_code, end_reason, collection";

fn run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        session_id: row.get(2)?,
        kind: row.get(3)?,
        harness_id: row.get(4)?,
        harness_session_id: row.get(5)?,
        pty_session_id: row.get(6)?,
        launched_by: row.get(7)?,
        started_at: row.get(8)?,
        ended_at: row.get(9)?,
        exit_code: row.get(10)?,
        end_reason: row.get(11)?,
        collection: row.get(12)?,
    })
}

const OUTCOME_COLUMNS: &str = "id, project_id, workspace_id, workspace_name, branch, base_branch, \
     task, harnesses, label, labeled_at, ahead_at, ahead_tip, merged_at, pr_number, pr_state, ended, \
     ended_at, began_at, created_at, updated_at";

fn outcome_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<OutcomeRow> {
    Ok(OutcomeRow {
        id: row.get(0)?,
        project_id: row.get(1)?,
        workspace_id: row.get(2)?,
        workspace_name: row.get(3)?,
        branch: row.get(4)?,
        base_branch: row.get(5)?,
        task: row.get(6)?,
        harnesses: row.get(7)?,
        label: row.get(8)?,
        labeled_at: row.get(9)?,
        ahead_at: row.get(10)?,
        ahead_tip: row.get(11)?,
        merged_at: row.get(12)?,
        pr_number: row.get(13)?,
        pr_state: row.get(14)?,
        ended: row.get(15)?,
        ended_at: row.get(16)?,
        began_at: row.get(17)?,
        created_at: row.get(18)?,
        updated_at: row.get(19)?,
    })
}

const MEMORY_COLUMNS: &str = "id, project_id, text, state, author, harness_id, workspace_id, \
     workspace_name, run_id, created_at, updated_at";

fn memory_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryRow> {
    Ok(MemoryRow {
        id: row.get(0)?,
        project_id: row.get(1)?,
        text: row.get(2)?,
        state: row.get(3)?,
        author: row.get(4)?,
        harness_id: row.get(5)?,
        workspace_id: row.get(6)?,
        workspace_name: row.get(7)?,
        run_id: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

const EVENT_COLUMNS: &str = "seq, id, schema_version, workspace_id, session_id, run_id, \
     occurred_at, received_at, kind, producer, method, fidelity, source_key, privacy_class, payload";

/// Ids as one JSON array, for `IN (SELECT value FROM json_each(?))`: one parameter however many
/// there are.
fn id_list(ids: &[String]) -> String {
    serde_json::to_string(ids).expect("strings serialise")
}

fn event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventRow> {
    Ok(EventRow {
        seq: row.get(0)?,
        id: row.get(1)?,
        schema_version: row.get(2)?,
        workspace_id: row.get(3)?,
        session_id: row.get(4)?,
        run_id: row.get(5)?,
        occurred_at: row.get(6)?,
        received_at: row.get(7)?,
        kind: row.get(8)?,
        producer: row.get(9)?,
        method: row.get(10)?,
        fidelity: row.get(11)?,
        source_key: row.get(12)?,
        privacy_class: row.get(13)?,
        payload: row.get(14)?,
    })
}

fn project_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectRow> {
    Ok(ProjectRow {
        id: row.get(0)?,
        name: row.get(1)?,
        root_path: row.get(2)?,
    })
}

const WORKSPACE_COLUMNS: &str =
    "id, project_id, kind, name, path, branch, base_branch, status, forgotten";

fn workspace_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceRow> {
    Ok(WorkspaceRow {
        id: row.get(0)?,
        project_id: row.get(1)?,
        kind: row.get(2)?,
        name: row.get(3)?,
        path: row.get(4)?,
        branch: row.get(5)?,
        base_branch: row.get(6)?,
        archived: row.get::<_, String>(7)? == "archived",
        forgotten: row.get::<_, i64>(8)? != 0,
    })
}

const SESSION_COLUMNS: &str = "id, workspace_id, harness_id, model, effort, harness_session_id, \
     title, forked_from, state, exit_code, pty_session_id, started_at, ended_at, \
     COALESCE(began_at, started_at)";

fn session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        harness_id: row.get(2)?,
        model: row.get(3)?,
        effort: row.get(4)?,
        harness_session_id: row.get(5)?,
        title: row.get(6)?,
        forked_from: row.get(7)?,
        running: row.get::<_, String>(8)? == "running",
        exit_code: row.get(9)?,
        pty_session_id: row.get(10)?,
        started_at: row.get(11)?,
        ended_at: row.get(12)?,
        began_at: row.get(13)?,
    })
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(store: &Store) -> Vec<String> {
        store
            .projects()
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect()
    }

    /// This computer as a host and as a client of other hosts are two devices: turning Remote
    /// off as a host leaves the client signed in, and the other way round.
    #[test]
    fn signing_in_as_a_client_and_turning_the_host_off_leave_each_other_be() {
        let store = Store::in_memory();
        let host = RemoteHostRow {
            server_url: "https://remote.example.com".into(),
            server_id: "server-1".into(),
            host_id: "host-1".into(),
            device_id: "device-host".into(),
            name: "Workstation".into(),
            enabled_at: 1,
        };
        let computer = RemoteComputerRow {
            server_url: "https://remote.example.com".into(),
            server_id: "server-1".into(),
            device_id: "device-client".into(),
            name: "Workstation".into(),
            signed_in_at: 2,
        };
        store.set_remote_host(&host).unwrap();
        store.set_remote_computer(&computer).unwrap();
        store.forget_remote_host().unwrap();
        assert_eq!(store.remote_computer().unwrap(), Some(computer.clone()));

        store.forget_remote_pairing("grant-1").unwrap();
        store.forget_remote_pairing("grant-1").unwrap();
        assert_eq!(store.forgotten_remote_pairings().unwrap(), ["grant-1"]);

        store.set_remote_host(&host).unwrap();
        store.forget_remote_computer().unwrap();
        assert_eq!(store.remote_computer().unwrap(), None);
        assert_eq!(
            store.forgotten_remote_pairings().unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(store.remote_host().unwrap(), Some(host));
    }

    #[test]
    fn a_project_arrives_with_its_local_workspace() {
        let store = Store::in_memory();
        let project = store.add_project("app", "/code/app").unwrap();

        let workspaces = store.workspaces().unwrap();
        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].project_id, project.id);
        assert_eq!(workspaces[0].kind, "local");
        assert_eq!(workspaces[0].path, "/code/app");
        assert_eq!(
            store.workspace(&workspaces[0].id).unwrap(),
            Some(workspaces[0].clone())
        );
    }

    #[test]
    fn a_workspace_remembers_the_task_it_was_started_from_until_it_is_deleted() {
        let store = Store::in_memory();
        let project = store.add_project("app", "/code/app").unwrap();
        let made = store
            .add_worktree(
                &project.id,
                "fix",
                "/trees/fix",
                Some("ys/fix"),
                Some("main"),
            )
            .unwrap();
        let other = store
            .add_worktree(
                &project.id,
                "other",
                "/trees/other",
                Some("ys/other"),
                Some("main"),
            )
            .unwrap();
        let task = |key: &str| WorkspaceTaskRow {
            source: "github".into(),
            repo: "github.com/o/r".into(),
            key: key.into(),
            url: format!("https://github.com/o/r/issues/{}", &key[1..]),
            title: format!("Task {key}"),
        };
        assert!(store.workspace_tasks(&made.id).unwrap().is_empty());

        store.link_task(&made.id, &task("#91")).unwrap();
        store.link_task(&made.id, &task("#91")).unwrap();
        store.link_task(&made.id, &task("#92")).unwrap();
        assert_eq!(
            store.workspace_tasks(&made.id).unwrap(),
            [task("#91"), task("#92")],
            "each once, oldest first"
        );
        assert!(
            store.workspace_tasks(&other.id).unwrap().is_empty(),
            "nothing is written for a workspace made any other way"
        );

        assert!(store.remove_worktree(&made.id).unwrap());
        assert!(store.workspace_tasks(&made.id).unwrap().is_empty());
    }

    #[test]
    fn the_same_root_cannot_be_added_twice() {
        let store = Store::in_memory();
        store.add_project("app", "/code/app").unwrap();
        assert!(store.add_project("again", "/code/app").is_err());
        assert_eq!(
            store.project_by_root("/code/app").unwrap().unwrap().name,
            "app"
        );
        assert_eq!(store.project_by_root("/code/other").unwrap(), None);
    }

    #[test]
    fn projects_keep_insertion_order_until_reordered() {
        let store = Store::in_memory();
        let ids: Vec<_> = ["a", "b", "c"]
            .iter()
            .map(|n| store.add_project(n, &format!("/{n}")).unwrap().id)
            .collect();
        assert_eq!(names(&store), ["a", "b", "c"]);

        store
            .reorder_projects(&[ids[2].clone(), ids[0].clone()])
            .unwrap();
        assert_eq!(
            names(&store),
            ["c", "a", "b"],
            "unmentioned projects go last"
        );

        store.add_project("d", "/d").unwrap();
        assert_eq!(names(&store), ["c", "a", "b", "d"]);
    }

    #[test]
    fn removing_a_project_takes_its_workspaces_along() {
        let store = Store::in_memory();
        let project = store.add_project("app", "/code/app").unwrap();
        assert!(store.remove_project(&project.id, false).unwrap());
        assert!(store.workspaces().unwrap().is_empty());
        assert!(!store.remove_project(&project.id, false).unwrap());
    }

    #[test]
    fn a_project_removed_with_its_history_kept_is_hidden_and_comes_back_whole() {
        let store = Store::in_memory();
        let project = store.add_project("app", "/code/app").unwrap();
        let ws = store
            .add_worktree(
                &project.id,
                "fix",
                "/wt/app/fix",
                Some("ys/fix"),
                Some("main"),
            )
            .unwrap();
        store
            .add_session(&new_session("s1", &ws.id, "pty-1"))
            .unwrap();
        store.add_project("other", "/code/other").unwrap();

        assert!(store.remove_project(&project.id, true).unwrap());
        assert_eq!(names(&store), ["other"]);
        assert!(store
            .workspaces()
            .unwrap()
            .iter()
            .all(|w| w.project_id != project.id));
        assert!(
            store
                .workspace_paths()
                .unwrap()
                .contains(&"/wt/app/fix".to_owned()),
            "still ours"
        );
        assert!(store.project_by_root("/code/app").unwrap().is_none());
        assert_eq!(
            store
                .removed_project_by_root("/code/app")
                .unwrap()
                .unwrap()
                .id,
            project.id
        );
        assert!(
            !store.remove_project(&project.id, true).unwrap(),
            "already hidden"
        );

        assert!(store.revive_project(&project.id).unwrap());
        assert_eq!(
            names(&store),
            ["other", "app"],
            "back at the end, like a new one"
        );
        assert_eq!(
            store
                .workspaces()
                .unwrap()
                .iter()
                .filter(|w| w.project_id == project.id)
                .count(),
            2
        );
        assert_eq!(store.sessions(&ws.id).unwrap().len(), 1, "history intact");
        assert!(!store.revive_project(&project.id).unwrap());
    }

    #[test]
    fn worktrees_list_after_local_in_creation_order_and_local_is_permanent() {
        let store = Store::in_memory();
        let project = store.add_project("app", "/code/app").unwrap();
        let first = store
            .add_worktree(&project.id, "one", "/wt/one", Some("ys/one"), Some("main"))
            .unwrap();
        store
            .add_worktree(&project.id, "two", "/wt/two", Some("ys/two"), Some("main"))
            .unwrap();

        let names: Vec<_> = store
            .workspaces()
            .unwrap()
            .into_iter()
            .map(|w| w.name)
            .collect();
        assert_eq!(names, ["local", "one", "two"]);

        let local = store.workspaces().unwrap().remove(0);
        assert!(
            !store.remove_worktree(&local.id).unwrap(),
            "local cannot be deleted"
        );
        assert!(store.remove_worktree(&first.id).unwrap());
        assert_eq!(store.workspaces().unwrap().len(), 2);
        assert_eq!(store.project(&project.id).unwrap().unwrap().name, "app");
    }

    #[test]
    fn a_worktree_remembers_whether_yardsort_created_its_branch() {
        let store = Store::in_memory();
        let project = store.add_project("app", "/code/app").unwrap();
        let ours = store
            .add_worktree(&project.id, "a", "/wt/a", Some("ys/a"), Some("main"))
            .unwrap();
        let theirs = store
            .add_worktree(&project.id, "b", "/wt/b", Some("feature"), None)
            .unwrap();
        assert_eq!(
            store
                .workspace(&ours.id)
                .unwrap()
                .unwrap()
                .base_branch
                .as_deref(),
            Some("main")
        );
        assert_eq!(
            store.workspace(&theirs.id).unwrap().unwrap().base_branch,
            None
        );
    }

    #[test]
    fn a_folder_can_only_be_one_workspace() {
        let store = Store::in_memory();
        let project = store.add_project("app", "/code/app").unwrap();
        let first = store
            .add_worktree_if_new(&project.id, "wt", "/wt/x", Some("b"))
            .unwrap();
        assert!(first.is_some());
        assert_eq!(
            store
                .add_worktree_if_new(&project.id, "wt", "/wt/x", Some("b"))
                .unwrap(),
            None
        );
        assert!(store
            .add_worktree(&project.id, "again", "/wt/x", Some("b"), None)
            .is_err());
        assert_eq!(store.workspaces().unwrap().len(), 2, "local + one worktree");
    }

    #[test]
    fn upgrading_removes_duplicate_workspaces_left_by_the_adoption_race() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("yardsort.db");
        {
            // A database as version 1 left it: the same worktree recorded twice.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(MIGRATIONS[0]).unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
            conn.execute_batch(
                "INSERT INTO projects VALUES ('p', 'app', '/code/app', 0, 0);
                 INSERT INTO workspaces (id, project_id, kind, name, path, created_at) VALUES
                   ('l', 'p', 'local', 'local', '/code/app', 0),
                   ('a', 'p', 'worktree', 'by-hand', '/wt/by-hand', 1),
                   ('b', 'p', 'worktree', 'by-hand', '/wt/by-hand', 2),
                   ('c', 'p', 'worktree', 'other', '/wt/other', 3);",
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        let ids: Vec<_> = store
            .workspaces()
            .unwrap()
            .into_iter()
            .map(|w| w.id)
            .collect();
        assert_eq!(
            ids,
            ["l", "a", "c"],
            "the older of the two duplicates is kept"
        );
    }

    fn worktree(store: &Store) -> WorkspaceRow {
        let project = store.add_project("app", "/code/app").unwrap();
        store
            .add_worktree(&project.id, "fix", "/wt/fix", Some("ys/fix"), Some("main"))
            .unwrap()
    }

    fn new_session<'a>(id: &'a str, workspace_id: &'a str, pty: &'a str) -> NewSession<'a> {
        NewSession {
            id,
            workspace_id,
            harness_id: "claude",
            harness_session_id: Some("h-1"),
            title: "fix the login bug",
            pty_session_id: pty,
            ..Default::default()
        }
    }

    #[test]
    fn a_tracked_pull_request_is_kept_once_per_workspace_and_goes_with_it() {
        let store = Store::in_memory();
        let ws = worktree(&store);
        let other = store
            .add_worktree(
                &ws.project_id,
                "other",
                "/wt/other",
                Some("ys/o"),
                Some("main"),
            )
            .unwrap();
        store.add_session(&new_session("s1", &ws.id, "p1")).unwrap();
        let link = |workspace_id, session_id, number: u32| NewTrackedPullRequest {
            workspace_id,
            session_id,
            repo: "github.com/o/lib",
            number,
            url: if number == 7 {
                "https://github.com/o/lib/pull/7"
            } else {
                "https://github.com/o/lib/pull/8"
            },
            tracked_by: "cli",
        };

        let (first, added) = store.track_pull_request(&link(&ws.id, None, 7)).unwrap();
        assert!(added);
        assert_eq!(first.session_id, None);
        let (again, added) = store
            .track_pull_request(&link(&ws.id, Some("s1"), 7))
            .unwrap();
        assert!(!added, "the same link twice is one row");
        assert_eq!(again.id, first.id);
        assert_eq!(
            again.session_id.as_deref(),
            Some("s1"),
            "a later look knows who"
        );
        store.track_pull_request(&link(&ws.id, None, 8)).unwrap();
        store.track_pull_request(&link(&other.id, None, 7)).unwrap();

        let mine = store.tracked_pull_requests(&ws.id).unwrap();
        assert_eq!(
            mine.iter().map(|row| row.number).collect::<Vec<_>>(),
            [8, 7],
            "newest tracked first"
        );
        assert_eq!(
            store
                .tracked_pull_requests_in_project(&ws.project_id)
                .unwrap()
                .len(),
            3,
            "both workspaces' rows, the shared link once each"
        );

        // The conversation going leaves the row, without it.
        assert!(store.remove_session("s1").unwrap());
        let row = store.tracked_pull_requests(&ws.id).unwrap().pop().unwrap();
        assert_eq!((row.number, row.session_id), (7, None));

        assert!(store
            .untrack_pull_request(&ws.id, "https://github.com/o/lib/pull/8")
            .unwrap());
        assert!(!store.untrack_pull_request(&ws.id, "nothing").unwrap());
        assert!(
            !store.untrack_pull_request(&other.id, &row.id).unwrap(),
            "another workspace's id is not this one's to remove"
        );

        // Forgotten with its history: kept, but not listed.
        store.forget_workspace(&other.id, true).unwrap();
        assert_eq!(
            store
                .tracked_pull_requests_in_project(&ws.project_id)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(store.tracked_pull_requests(&other.id).unwrap().len(), 1);

        assert!(store.remove_worktree(&ws.id).unwrap());
        assert!(store.tracked_pull_requests(&ws.id).unwrap().is_empty());
    }

    #[test]
    fn a_workspace_remembers_what_was_asked_in_order() {
        let store = Store::in_memory();
        let ws = worktree(&store);
        let with_prompt = |id, pty, prompt| NewSession {
            prompt,
            ..new_session(id, &ws.id, pty)
        };
        store
            .add_session(&with_prompt(
                "a",
                "p1",
                Some("Fix the login bug\nand add a test"),
            ))
            .unwrap();
        store.add_session(&with_prompt("b", "p2", None)).unwrap();
        store
            .add_session(&with_prompt("c", "p3", Some("Also update the docs")))
            .unwrap();
        // Resuming the first conversation must not reorder what was asked.
        store.mark_session_running("a", "p4").unwrap();
        assert_eq!(
            store.session_prompts(&ws.id).unwrap(),
            ["Fix the login bug\nand add a test", "Also update the docs"]
        );
    }

    #[test]
    fn a_session_runs_ends_and_runs_again() {
        let store = Store::in_memory();
        let ws = worktree(&store);
        store
            .add_session(&new_session("s1", &ws.id, "pty-1"))
            .unwrap();
        assert!(store.session("s1").unwrap().unwrap().running);

        assert_eq!(
            store
                .end_session_by_pty("pty-1", Some(0))
                .unwrap()
                .as_deref(),
            Some("s1")
        );
        let ended = store.session("s1").unwrap().unwrap();
        assert!(!ended.running);
        assert_eq!(
            (ended.exit_code, ended.pty_session_id.as_deref()),
            (Some(0), None)
        );
        assert!(ended.ended_at.is_some());
        assert_eq!(
            store.end_session_by_pty("pty-1", Some(0)).unwrap(),
            None,
            "only once"
        );

        store.mark_session_running("s1", "pty-2").unwrap();
        let resumed = store.session("s1").unwrap().unwrap();
        assert!(resumed.running && resumed.ended_at.is_none() && resumed.exit_code.is_none());
        assert_eq!(
            resumed.harness_session_id.as_deref(),
            Some("h-1"),
            "same conversation"
        );
    }

    #[test]
    fn sessions_left_running_by_a_dead_app_are_ended_without_an_exit_code() {
        let store = Store::in_memory();
        let ws = worktree(&store);
        store
            .add_session(&new_session("s1", &ws.id, "pty-1"))
            .unwrap();
        store
            .add_session(&new_session("s2", &ws.id, "pty-2"))
            .unwrap();
        store.end_session_by_pty("pty-2", Some(1)).unwrap();

        assert_eq!(store.end_interrupted_sessions(&[]).unwrap(), 1);
        let s1 = store.session("s1").unwrap().unwrap();
        assert!(!s1.running && s1.exit_code.is_none());
        assert_eq!(
            store.session("s2").unwrap().unwrap().exit_code,
            Some(1),
            "left alone"
        );
    }

    /// With the daemon, "the app restarted" no longer means "the conversation is over": a row
    /// whose process is still running must be left alone, or the app would offer to resume a
    /// conversation that never stopped.
    #[test]
    fn a_session_whose_process_outlived_the_app_is_left_running() {
        let store = Store::in_memory();
        let ws = worktree(&store);
        store
            .add_session(&new_session("still-going", &ws.id, "pty-1"))
            .unwrap();
        store
            .add_session(&new_session("really-gone", &ws.id, "pty-2"))
            .unwrap();

        let alive = vec!["pty-1".to_owned()];
        assert_eq!(store.end_interrupted_sessions(&alive).unwrap(), 1);

        let kept = store.session("still-going").unwrap().unwrap();
        assert!(kept.running, "its agent is still working");
        assert_eq!(kept.pty_session_id.as_deref(), Some("pty-1"));
        assert!(!store.session("really-gone").unwrap().unwrap().running);
    }

    #[test]
    fn sessions_are_listed_newest_first_and_go_with_their_workspace() {
        let store = Store::in_memory();
        let ws = worktree(&store);
        store
            .add_session(&new_session("old", &ws.id, "p1"))
            .unwrap();
        store
            .add_session(&NewSession {
                forked_from: Some("old"),
                ..new_session("new", &ws.id, "p2")
            })
            .unwrap();
        let ids: Vec<_> = store
            .sessions(&ws.id)
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, ["new", "old"]);

        assert!(store.remove_session("old").unwrap());
        assert_eq!(
            store.session("new").unwrap().unwrap().forked_from,
            None,
            "the link is cleared"
        );

        store.remove_worktree(&ws.id).unwrap();
        assert!(store.sessions(&ws.id).unwrap().is_empty());
    }

    #[test]
    fn workspaces_can_be_renamed_and_archived_but_local_cannot() {
        let store = Store::in_memory();
        let ws = worktree(&store);
        assert!(store.rename_workspace(&ws.id, "login fix").unwrap());
        assert!(store.set_workspace_archived(&ws.id, true).unwrap());
        let row = store.workspace(&ws.id).unwrap().unwrap();
        assert_eq!((row.name.as_str(), row.archived), ("login fix", true));

        let all = store.workspaces().unwrap();
        assert_eq!(all.last().unwrap().id, ws.id, "archived ones sort last");
        let local = all.iter().find(|w| w.kind == "local").unwrap();
        assert!(!store.rename_workspace(&local.id, "x").unwrap());
        assert!(!store.set_workspace_archived(&local.id, true).unwrap());
    }

    #[test]
    fn a_forgotten_workspace_is_hidden_and_its_history_waits_for_an_import() {
        let store = Store::in_memory();
        let ws = worktree(&store);
        store
            .add_session(&new_session("s1", &ws.id, "pty-1"))
            .unwrap();

        assert!(store.forget_workspace(&ws.id, true).unwrap());
        assert!(store.workspaces().unwrap().iter().all(|w| w.id != ws.id));
        assert!(
            store.workspace_paths().unwrap().contains(&ws.path),
            "still ours"
        );
        assert!(store.workspace(&ws.id).unwrap().unwrap().forgotten);
        assert_eq!(store.sessions(&ws.id).unwrap().len(), 1, "history kept");
        assert!(!store
            .add_worktree_if_new("p", "x", &ws.path, None)
            .unwrap()
            .is_some());

        let found = store.forgotten_workspace_at(&ws.path).unwrap().unwrap();
        assert_eq!(found.id, ws.id);
        assert!(store.revive_workspace(&ws.id, Some("other")).unwrap());
        let back = store.workspace(&ws.id).unwrap().unwrap();
        assert!(!back.forgotten && !back.archived);
        assert_eq!(back.branch.as_deref(), Some("other"));
        assert!(store.forgotten_workspace_at(&ws.path).unwrap().is_none());

        assert!(store.forget_workspace(&ws.id, false).unwrap());
        assert!(store.workspace(&ws.id).unwrap().is_none());
        assert!(
            store.sessions(&ws.id).unwrap().is_empty(),
            "history dropped with it"
        );

        let local = store.workspaces().unwrap().remove(0);
        assert_eq!(local.kind, "local");
        assert!(!store.forget_workspace(&local.id, true).unwrap());
        assert!(!store.forget_workspace(&local.id, false).unwrap());
    }

    #[test]
    fn ui_state_upserts() {
        let store = Store::in_memory();
        store.set_ui_state("selected", "a").unwrap();
        store.set_ui_state("selected", "b").unwrap();
        store.set_ui_state("expanded", "[]").unwrap();
        let state = store.ui_state().unwrap();
        assert_eq!(state["selected"], "b");
        assert_eq!(state.len(), 2);
    }

    /// A database from the version before step outcomes has no `outcome` column. Opening it
    /// adds the column, and a step that was already there still reads as having none.
    #[test]
    fn a_run_saved_before_step_outcomes_still_opens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("yardsort.db");
        {
            let mut conn = Connection::open(&path).unwrap();
            // The schema as it was before 0029_workflow_step_outcome. 0001 is index 0,
            // so that migration is index 28. A later migration must not become part of
            // the old database this test opens.
            assert!(MIGRATIONS[28].contains("workflow_step_runs ADD COLUMN outcome"));
            upgrade(&mut conn, &MIGRATIONS[..28], None).unwrap();
            conn.execute(
                "INSERT INTO workflow_runs
                 (id, workflow_id, workflow_name, definition, workspace_name, inputs, status,
                  requested_by, created_at)
                 VALUES ('run-old', 'demo', 'Demo',
                         'id: demo
name: Demo
version: 1
trigger:
  kind: manual
steps:
  - id: ping
    action: notify
    title: Hi
', 'fix', '{}', 'succeeded', 'cli', 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO workflow_step_runs (run_id, step_id, position, action, status)
                 VALUES ('run-old', 'ping', 0, 'notify', 'succeeded')",
                [],
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        let run = store.workflow_run("run-old").unwrap().unwrap();
        assert_eq!(run.status, "succeeded");
        let steps = store.workflow_steps("run-old").unwrap();
        assert_eq!(steps[0].step_id, "ping");
        assert_eq!(steps[0].outcome, None);
    }

    /// 0030 is index 29. A database that has every migration before it still opens, and a run
    /// saved then has no artifacts.
    #[test]
    fn a_run_saved_before_artifacts_still_opens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("yardsort.db");
        {
            let mut conn = Connection::open(&path).unwrap();
            assert!(MIGRATIONS[29].contains("CREATE TABLE workflow_artifacts"));
            upgrade(&mut conn, &MIGRATIONS[..29], None).unwrap();
            conn.execute(
                "INSERT INTO workflow_runs
                 (id, workflow_id, workflow_name, definition, workspace_name, inputs, status,
                  requested_by, created_at)
                 VALUES ('run-old', 'demo', 'Demo', 'id: demo\n', 'fix', '{}', 'succeeded',
                         'cli', 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO workflow_step_runs (run_id, step_id, position, action, status)
                 VALUES ('run-old', 'ping', 0, 'notify', 'succeeded')",
                [],
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.workflow_run("run-old").unwrap().unwrap().status,
            "succeeded"
        );
        assert_eq!(store.workflow_steps("run-old").unwrap()[0].step_id, "ping");
        assert!(store.workflow_artifacts("run-old").unwrap().is_empty());
    }

    /// 0031 is index 30. A database that has every migration before it still opens, and it has
    /// no approval decisions until someone records one.
    #[test]
    fn a_run_saved_before_approval_decisions_still_opens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("yardsort.db");
        {
            let mut conn = Connection::open(&path).unwrap();
            assert!(MIGRATIONS[30].contains("CREATE TABLE workflow_approval_decisions"));
            assert_eq!(MIGRATIONS.len(), 31);
            upgrade(&mut conn, &MIGRATIONS[..30], None).unwrap();
            conn.execute(
                "INSERT INTO workflow_runs
                 (id, workflow_id, workflow_name, definition, workspace_name, inputs, status,
                  requested_by, created_at)
                 VALUES ('run-old', 'demo', 'Demo', 'id: demo\n', 'fix', '{}', 'running', 'cli', 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO workflow_step_runs
                 (run_id, step_id, position, action, status)
                 VALUES ('run-old', 'sign-off', 0, 'approve', 'waiting')",
                [],
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.workflow_run("run-old").unwrap().unwrap().status,
            "running"
        );
        assert!(store
            .workflow_approval("run-old", "sign-off")
            .unwrap()
            .is_none());
        assert_eq!(
            store
                .record_workflow_approval("run-old", "sign-off", "approved", None, "ada")
                .unwrap(),
            ApprovalWrite::Recorded
        );
        assert_eq!(
            store
                .record_workflow_approval("run-old", "sign-off", "rejected", Some("no"), "ada")
                .unwrap(),
            ApprovalWrite::AlreadyDecided
        );
        store
            .finish_workflow_run("run-old", "failed", Some("stopped"))
            .unwrap();
        assert!(matches!(
            store
                .record_workflow_approval("run-old", "sign-off", "approved", None, "ada")
                .unwrap(),
            ApprovalWrite::RunEnded { .. }
        ));
    }

    #[test]
    fn a_second_record_replaces_the_step_artifacts() {
        let store = Store::in_memory();
        store
            .conn()
            .execute(
                "INSERT INTO workflow_runs
                 (id, workflow_id, workflow_name, definition, workspace_name, inputs, status,
                  requested_by, created_at)
                 VALUES ('run-1', 'demo', 'Demo', 'id: demo\n', 'fix', '{}', 'running', 'cli', 1)",
                [],
            )
            .unwrap();
        let spec = |body: &str| NewWorkflowArtifact {
            name: "spec".into(),
            source: "file".into(),
            detail: Some("SPEC.md".into()),
            body: body.into(),
            truncated: false,
        };
        let notes = NewWorkflowArtifact {
            name: "notes".into(),
            source: "text".into(),
            detail: None,
            body: "first notes".into(),
            truncated: false,
        };
        store
            .replace_step_artifacts("run-1", "draft", &[spec("v1"), notes])
            .unwrap();
        let mut again = spec("v2");
        again.truncated = true;
        store
            .replace_step_artifacts("run-1", "draft", &[again])
            .unwrap();
        let rows = store.workflow_artifacts("run-1").unwrap();
        assert_eq!(rows.len(), 1, "the step's previous names are gone");
        assert_eq!(rows[0].body, "v2");
        assert!(rows[0].truncated);
        assert_eq!(rows[0].step_id, "draft");
    }

    #[test]
    fn artifacts_and_the_success_are_one_change() {
        let store = Store::in_memory();
        store
            .conn()
            .execute(
                "INSERT INTO workflow_runs
                 (id, workflow_id, workflow_name, definition, workspace_name, inputs, status,
                  requested_by, created_at)
                 VALUES ('run-1', 'demo', 'Demo', 'id: demo\n', 'fix', '{}', 'running', 'cli', 1)",
                [],
            )
            .unwrap();
        store
            .conn()
            .execute(
                "INSERT INTO workflow_step_runs (run_id, step_id, position, action, status)
                 VALUES ('run-1', 'draft', 0, 'notify', 'running')",
                [],
            )
            .unwrap();
        let spec = |body: &str| NewWorkflowArtifact {
            name: "spec".into(),
            source: "text".into(),
            detail: None,
            body: body.into(),
            truncated: false,
        };
        assert!(store
            .finish_step_with_artifacts(
                "run-1",
                "draft",
                &[StepStatus::Running],
                StepStatus::Succeeded,
                &[spec("v1")],
                Some(r#"{"outcome":"settled"}"#),
                Some("done"),
                None,
                false,
            )
            .unwrap());
        assert_eq!(
            store.workflow_steps("run-1").unwrap()[0].status,
            "succeeded"
        );
        assert_eq!(store.workflow_artifacts("run-1").unwrap()[0].body, "v1");

        store
            .conn()
            .execute(
                "UPDATE workflow_runs SET status = 'cancelled' WHERE id = 'run-1'",
                [],
            )
            .unwrap();
        store
            .conn()
            .execute(
                "UPDATE workflow_step_runs SET status = 'running' WHERE run_id = 'run-1'",
                [],
            )
            .unwrap();
        assert!(!store
            .finish_step_with_artifacts(
                "run-1",
                "draft",
                &[StepStatus::Running],
                StepStatus::Succeeded,
                &[spec("v2")],
                Some("{}"),
                None,
                None,
                false,
            )
            .unwrap());
        assert_eq!(
            store.workflow_artifacts("run-1").unwrap()[0].body,
            "v1",
            "a stopped run keeps the previous body"
        );
        assert_eq!(store.workflow_steps("run-1").unwrap()[0].status, "running");
    }

    #[test]
    fn data_survives_reopening_and_migrations_run_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("yardsort.db");
        Store::open(&path)
            .unwrap()
            .add_project("app", "/code/app")
            .unwrap();
        let reopened = Store::open(&path).unwrap();
        assert_eq!(names(&reopened), ["app"]);
    }

    #[test]
    fn a_database_from_the_future_is_refused_not_mangled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("yardsort.db");
        drop(Store::open(&path).unwrap());
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", 999)
            .unwrap();
        assert!(matches!(
            Store::open(&path),
            Err(StoreError::TooNew { found: 999, .. })
        ));
    }

    /// Going back to an older version needs the database as it was before the newer one
    /// upgraded it. On 2026-09-29 there was none, and the only way back was forward.
    #[test]
    fn an_upgrade_keeps_a_copy_the_version_before_it_can_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("yardsort.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(MIGRATIONS[0]).unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
            conn.execute(
                "INSERT INTO projects VALUES ('p', 'app', '/code/app', 0, 0)",
                [],
            )
            .unwrap();
        }
        assert_eq!(readable_copy_before_upgrade(&path), None);

        let store = Store::open(&path).unwrap();
        store.add_project("after", "/code/after").unwrap();

        let copy = readable_copy_before_upgrade(&path).expect("a copy from before the upgrade");
        assert_eq!(copy, dir.path().join("yardsort.db.before-upgrade"));
        let old = Connection::open(&copy).unwrap();
        let schema: u32 = old
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(schema, 1, "the version before the upgrade can open it");
        let names: Vec<String> = old
            .prepare("SELECT name FROM projects")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            names,
            ["app"],
            "as it was before the upgrade, and nothing since"
        );
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.contains("partial"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    /// An upgrade over several migrations that stopped partway, then was tried again. It used to
    /// commit each migration on its own, so the database was left between the two schemas; the
    /// retry then copied that, over the only copy the version before could open.
    #[test]
    fn an_interrupted_upgrade_keeps_the_copy_of_where_it_started() {
        let schema = |path: &Path| -> u32 {
            Connection::open(path)
                .unwrap()
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap()
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("yardsort.db");
        let copy = copy_before_upgrade(&path);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE first (x)").unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
        }

        // The third migration fails, after the second would have committed on its own.
        let broken = [
            "CREATE TABLE first (x)",
            "CREATE TABLE second (x)",
            "NOT SQL",
        ];
        let mut conn = Connection::open(&path).unwrap();
        assert!(upgrade(&mut conn, &broken, Some(&path)).is_err());
        drop(conn);
        assert_eq!(schema(&path), 1, "nothing of the upgrade is left");
        assert_eq!(schema(&copy), 1);

        let fixed = [
            "CREATE TABLE first (x)",
            "CREATE TABLE second (x)",
            "CREATE TABLE third (x)",
        ];
        let mut conn = Connection::open(&path).unwrap();
        upgrade(&mut conn, &fixed, Some(&path)).unwrap();
        drop(conn);
        assert_eq!(schema(&path), 3);
        assert_eq!(
            schema(&copy),
            1,
            "still the schema the version before can open"
        );
    }

    #[test]
    fn a_new_or_current_database_needs_no_copy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("yardsort.db");
        drop(Store::open(&path).unwrap());
        drop(Store::open(&path).unwrap());
        assert!(!copy_before_upgrade(&path).exists());
    }

    /// The CLI and the app are two processes on one database. Two `Store`s writing at once must
    /// both get their rows in — which is what [`BUSY_TIMEOUT`] buys: without it the second
    /// writer is told `SQLITE_BUSY` straight away instead of waiting for the first to finish.
    #[test]
    fn two_stores_on_one_file_can_both_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("yardsort.db");
        let (app, cli) = (Store::open(&path).unwrap(), Store::open(&path).unwrap());

        const EACH: usize = 25;
        std::thread::scope(|scope| {
            for (store, whose) in [(&app, "app"), (&cli, "cli")] {
                scope.spawn(move || {
                    for n in 0..EACH {
                        store
                            .add_project(&format!("{whose}-{n}"), &format!("/code/{whose}/{n}"))
                            .unwrap_or_else(|e| panic!("{whose} could not write row {n}: {e}"));
                    }
                });
            }
        });

        assert_eq!(app.projects().unwrap().len(), EACH * 2);
    }

    #[test]
    fn a_project_shares_its_agents_conversations_until_turned_off() {
        let store = Store::in_memory();
        let project = store.add_project("app", "/code/app").unwrap();
        assert!(
            store.conversations_shared(&project.id).unwrap(),
            "on by default"
        );
        store.set_conversations_shared(&project.id, false).unwrap();
        assert!(!store.conversations_shared(&project.id).unwrap());
        store.set_conversations_shared(&project.id, true).unwrap();
        assert!(store.conversations_shared(&project.id).unwrap());
    }
}
