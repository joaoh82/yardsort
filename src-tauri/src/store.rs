//! Persistent app state in SQLite. Pure storage: no git, no filesystem, no policy.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

/// Applied in order; a database's `user_version` is how many it has had. Never edit a shipped
/// migration — add a new one.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_unique_workspace_path.sql"),
    include_str!("../migrations/0003_sessions.sql"),
    include_str!("../migrations/0004_session_prompt.sql"),
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

pub struct Store {
    conn: Mutex<Connection>,
}

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

    pub fn projects(&self) -> StoreResult<Vec<ProjectRow>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT id, name, root_path FROM projects ORDER BY sort_order, created_at")?;
        let rows = stmt.query_map([], |row| {
            Ok(ProjectRow {
                id: row.get(0)?,
                name: row.get(1)?,
                root_path: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn project_by_root(&self, root_path: &str) -> StoreResult<Option<ProjectRow>> {
        Ok(self
            .projects()?
            .into_iter()
            .find(|project| project.root_path == root_path))
    }

    /// Workspaces of every project: `local` first, then active ones, then archived ones.
    pub fn workspaces(&self) -> StoreResult<Vec<WorkspaceRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, project_id, kind, name, path, branch, base_branch, status FROM workspaces
             ORDER BY project_id, kind = 'local' DESC, status = 'active' DESC, sort_order, created_at",
        )?;
        let rows = stmt.query_map([], workspace_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn workspace(&self, id: &str) -> StoreResult<Option<WorkspaceRow>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, project_id, kind, name, path, branch, base_branch, status FROM workspaces WHERE id = ?",
                [id],
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
                params![exit_code, now_ms(), id],
            )?;
        }
        Ok(id)
    }

    /// At startup nothing is running yet, so every row that claims to be was cut short when the
    /// previous run ended. Returns how many there were.
    pub fn end_interrupted_sessions(&self) -> StoreResult<usize> {
        Ok(self.conn().execute(
            "UPDATE sessions SET state = 'ended', exit_code = NULL, pty_session_id = NULL,
                                 ended_at = COALESCE(ended_at, ?)
             WHERE state = 'running'",
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
    pub fn remove_project(&self, id: &str) -> StoreResult<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM projects WHERE id = ?", [id])?
            > 0)
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

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

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

fn now_ms() -> i64 {
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
        assert!(store.remove_project(&project.id).unwrap());
        assert!(store.workspaces().unwrap().is_empty());
        assert!(!store.remove_project(&project.id).unwrap());
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

        assert_eq!(store.end_interrupted_sessions().unwrap(), 1);
        let s1 = store.session("s1").unwrap().unwrap();
        assert!(!s1.running && s1.exit_code.is_none());
        assert_eq!(
            store.session("s2").unwrap().unwrap().exit_code,
            Some(1),
            "left alone"
        );
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
}
