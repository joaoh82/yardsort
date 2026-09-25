//! Persistent app state in SQLite. Pure storage: no git, no filesystem, no policy.

use std::collections::BTreeMap;
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
    /// Milliseconds since the Unix epoch.
    pub started_at: i64,
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

pub struct Store {
    conn: Mutex<Connection>,
}

/// How long to wait for another process to finish writing before giving up. Generous: the writes
/// here are single statements or small transactions, so anything approaching this means something
/// is wrong rather than merely busy.
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

impl Store {
    pub fn open(path: &Path) -> StoreResult<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        Self::prepare(Connection::open(path)?)
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::prepare(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn prepare(mut conn: Connection) -> StoreResult<Self> {
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

        let applied: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let applied = applied as usize;
        if applied > MIGRATIONS.len() {
            return Err(StoreError::TooNew {
                found: applied,
                known: MIGRATIONS.len(),
            });
        }
        for (index, sql) in MIGRATIONS.iter().enumerate().skip(applied) {
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(
                None,
                "user_version",
                u32::try_from(index + 1).unwrap_or(u32::MAX),
            )?;
            tx.commit()?;
        }
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
        self.conn().execute(
            "INSERT INTO sessions (id, workspace_id, harness_id, model, effort, harness_session_id,
                                   title, forked_from, state, pty_session_id, started_at, prompt)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'running', ?, ?, ?)",
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
                now_ms(),
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

    /// Make every activity write fail from here on, for tests of what happens then. The
    /// diagnostics table stays, so the failure can still be counted.
    #[cfg(any(test, feature = "testing"))]
    pub fn break_activity_tables(&self) {
        self.conn()
            .execute_batch("DROP TABLE agent_events; DROP TABLE agent_runs;")
            .unwrap();
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }
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

const EVENT_COLUMNS: &str = "seq, id, schema_version, workspace_id, session_id, run_id, \
     occurred_at, received_at, kind, producer, method, fidelity, source_key, privacy_class, payload";

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
     title, forked_from, state, exit_code, pty_session_id, started_at, ended_at";

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
}
