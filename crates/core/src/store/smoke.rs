//! Smoke test runs, their steps, approvals and the per-project test environment. See
//! `migrations/0019_smoke_tests.sql` and `crate::smoke`.
//!
//! The rules are the runner's; this is storage. The one rule that lives here is the one two
//! processes must agree on: a step moves, and a run starts, only through a conditional update,
//! so a cancel from `ys` and the runner in the app never both win.

use rusqlite::{params, OptionalExtension};

use super::{now_ms, Store, StoreResult};

/// One time a smoke test was asked to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestRunRow {
    pub id: String,
    pub project_id: Option<String>,
    /// `None` once the workspace is deleted; `workspace_name` keeps what it was called.
    pub workspace_id: Option<String>,
    pub workspace_name: String,
    pub test_id: String,
    pub test_name: String,
    /// The test file as it was when the run was asked for.
    pub definition: String,
    /// SHA-256 of `definition`, hex.
    pub content_hash: String,
    /// A JSON object of strings.
    pub inputs: String,
    /// `awaiting_approval`, `queued`, `running`, `passed`, `failed`, `refused`, `cancelled` or
    /// `errored`.
    pub status: String,
    /// `app`, `cli` or `workflow`.
    pub requested_by: String,
    /// The agent's conversation record, when `ys test run` came from an agent's terminal.
    pub session_record_id: Option<String>,
    pub port: Option<u16>,
    pub error: Option<String>,
    pub output_dir: Option<String>,
    pub cancel_requested_at: Option<i64>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
}

impl TestRunRow {
    /// Not ended yet: waiting for approval, queued or running.
    pub fn active(&self) -> bool {
        matches!(
            self.status.as_str(),
            "awaiting_approval" | "queued" | "running"
        )
    }

    pub fn ended(&self) -> bool {
        !self.active()
    }
}

/// One step of a smoke test run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestStepRow {
    pub run_id: String,
    pub step_id: String,
    /// `setup`, `steps` or `teardown`.
    pub phase: String,
    pub position: i64,
    pub action: String,
    /// `pending`, `running`, `waiting`, `passed`, `failed`, `skipped` or `cancelled`.
    pub status: String,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub exit: Option<i64>,
    /// A JSON object of strings. `stdout` and `stderr` are paths.
    pub outputs: String,
    pub note: Option<String>,
    /// A JSON array of paths.
    pub screenshots: String,
}

impl TestStepRow {
    pub fn outputs(&self) -> std::collections::BTreeMap<String, String> {
        serde_json::from_str(&self.outputs).unwrap_or_default()
    }

    pub fn screenshots(&self) -> Vec<String> {
        serde_json::from_str(&self.screenshots).unwrap_or_default()
    }

    pub fn ended(&self) -> bool {
        matches!(
            self.status.as_str(),
            "passed" | "failed" | "skipped" | "cancelled"
        )
    }
}

/// A run about to be queued, with its steps in file order as `(step id, phase, action)`.
#[derive(Debug, Clone)]
pub struct NewTestRun<'a> {
    pub id: &'a str,
    pub project_id: &'a str,
    pub workspace_id: &'a str,
    pub workspace_name: &'a str,
    pub test_id: &'a str,
    pub test_name: &'a str,
    pub definition: &'a str,
    pub content_hash: &'a str,
    pub inputs: &'a str,
    /// `awaiting_approval` or `queued`: whether the file at this hash is approved.
    pub status: &'a str,
    pub requested_by: &'a str,
    pub session_record_id: Option<&'a str>,
    pub steps: &'a [(&'a str, &'a str, &'a str)],
}

/// Whether a run was queued, or refused because the same test is already going there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestQueued {
    Queued,
    AlreadyRunning(String),
}

/// What one of a run's steps becomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepChange<'a> {
    pub to: &'a str,
    pub exit: Option<i64>,
    /// Replaces the step's outputs when given.
    pub outputs: Option<&'a str>,
    pub note: Option<&'a str>,
    pub screenshots: Option<&'a str>,
}

const TEST_RUN_COLUMNS: &str = "id, project_id, workspace_id, workspace_name, test_id, test_name, \
     definition, content_hash, inputs, status, requested_by, session_record_id, port, error, \
     output_dir, cancel_requested_at, created_at, started_at, ended_at";

fn test_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TestRunRow> {
    let port: Option<i64> = row.get(12)?;
    Ok(TestRunRow {
        id: row.get(0)?,
        project_id: row.get(1)?,
        workspace_id: row.get(2)?,
        workspace_name: row.get(3)?,
        test_id: row.get(4)?,
        test_name: row.get(5)?,
        definition: row.get(6)?,
        content_hash: row.get(7)?,
        inputs: row.get(8)?,
        status: row.get(9)?,
        requested_by: row.get(10)?,
        session_record_id: row.get(11)?,
        port: port.and_then(|p| u16::try_from(p).ok()),
        error: row.get(13)?,
        output_dir: row.get(14)?,
        cancel_requested_at: row.get(15)?,
        created_at: row.get(16)?,
        started_at: row.get(17)?,
        ended_at: row.get(18)?,
    })
}

const TEST_STEP_COLUMNS: &str = "run_id, step_id, phase, position, action, status, started_at, \
     ended_at, exit, outputs, note, screenshots";

fn test_step_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TestStepRow> {
    Ok(TestStepRow {
        run_id: row.get(0)?,
        step_id: row.get(1)?,
        phase: row.get(2)?,
        position: row.get(3)?,
        action: row.get(4)?,
        status: row.get(5)?,
        started_at: row.get(6)?,
        ended_at: row.get(7)?,
        exit: row.get(8)?,
        outputs: row.get(9)?,
        note: row.get(10)?,
        screenshots: row.get(11)?,
    })
}

impl Store {
    // --- runs -----------------------------------------------------------------------------

    /// Queue a run and its steps, unless this test is already waiting, queued or running in
    /// this workspace. One transaction, and a unique index behind it, so two processes cannot
    /// both.
    pub fn queue_test_run(&self, new: &NewTestRun<'_>) -> StoreResult<TestQueued> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let going: Option<String> = tx
            .query_row(
                "SELECT id FROM test_runs WHERE test_id = ? AND workspace_id = ?
                   AND status IN ('awaiting_approval', 'queued', 'running')",
                params![new.test_id, new.workspace_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = going {
            return Ok(TestQueued::AlreadyRunning(id));
        }
        tx.execute(
            "INSERT INTO test_runs (id, project_id, workspace_id, workspace_name, test_id,
                                    test_name, definition, content_hash, inputs, status,
                                    requested_by, session_record_id, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                new.id,
                new.project_id,
                new.workspace_id,
                new.workspace_name,
                new.test_id,
                new.test_name,
                new.definition,
                new.content_hash,
                new.inputs,
                new.status,
                new.requested_by,
                new.session_record_id,
                now_ms(),
            ],
        )?;
        for (position, (step_id, phase, action)) in new.steps.iter().enumerate() {
            tx.execute(
                "INSERT INTO test_step_runs (run_id, step_id, phase, position, action, status)
                 VALUES (?, ?, ?, ?, ?, 'pending')",
                params![new.id, step_id, phase, position as i64, action],
            )?;
        }
        tx.commit()?;
        Ok(TestQueued::Queued)
    }

    pub fn test_run(&self, id: &str) -> StoreResult<Option<TestRunRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {TEST_RUN_COLUMNS} FROM test_runs WHERE id = ?"),
                [id],
                test_run_from_row,
            )
            .optional()?)
    }

    /// Runs, newest first: all of them, or one workspace's.
    pub fn test_runs(
        &self,
        workspace_id: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<TestRunRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {TEST_RUN_COLUMNS} FROM test_runs
             WHERE ?1 IS NULL OR workspace_id = ?1
             ORDER BY created_at DESC, rowid DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(
            params![workspace_id, i64::try_from(limit).unwrap_or(i64::MAX)],
            test_run_from_row,
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Runs not ended yet, oldest first: what the runner has to look at.
    pub fn active_test_runs(&self) -> StoreResult<Vec<TestRunRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {TEST_RUN_COLUMNS} FROM test_runs
             WHERE status IN ('awaiting_approval', 'queued', 'running')
             ORDER BY created_at, rowid"
        ))?;
        let rows = stmt.query_map([], test_run_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// A run's steps in the order they run.
    pub fn test_steps(&self, run_id: &str) -> StoreResult<Vec<TestStepRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {TEST_STEP_COLUMNS} FROM test_step_runs WHERE run_id = ? ORDER BY position"
        ))?;
        let rows = stmt.query_map([run_id], test_step_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn test_step(&self, run_id: &str, step_id: &str) -> StoreResult<Option<TestStepRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!(
                    "SELECT {TEST_STEP_COLUMNS} FROM test_step_runs
                     WHERE run_id = ? AND step_id = ?"
                ),
                [run_id, step_id],
                test_step_from_row,
            )
            .optional()?)
    }

    /// A queued run starts, with the port and the output folder it was given. False when it was
    /// not queued any more: cancelled in between.
    pub fn start_test_run(&self, id: &str, port: u16, output_dir: &str) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE test_runs SET status = 'running', started_at = ?, port = ?, output_dir = ?
             WHERE id = ? AND status = 'queued'",
            params![now_ms(), i64::from(port), output_dir, id],
        )? > 0)
    }

    /// Move a step on, only while its run is running and the step is in one of `from`. False
    /// otherwise, and nothing changes.
    pub fn move_test_step(
        &self,
        run_id: &str,
        step_id: &str,
        from: &[&str],
        change: &StepChange<'_>,
    ) -> StoreResult<bool> {
        let now = now_ms();
        let is_final = matches!(change.to, "passed" | "failed" | "skipped" | "cancelled");
        let marks = from.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "UPDATE test_step_runs
             SET status = ?1,
                 started_at = CASE WHEN ?2 THEN started_at ELSE COALESCE(started_at, ?3) END,
                 ended_at = CASE WHEN ?2 THEN ?3 ELSE ended_at END,
                 exit = COALESCE(?4, exit),
                 outputs = COALESCE(?5, outputs),
                 note = COALESCE(?6, note),
                 screenshots = COALESCE(?7, screenshots)
             WHERE run_id = ?8 AND step_id = ?9 AND status IN ({marks})
               AND EXISTS (SELECT 1 FROM test_runs WHERE id = ?8 AND status = 'running')"
        );
        // A step skipped from pending never started, and keeps no start time.
        let mut values: Vec<rusqlite::types::Value> = vec![
            change.to.to_owned().into(),
            i64::from(is_final).into(),
            now.into(),
            change.exit.into(),
            change.outputs.map(str::to_owned).into(),
            change.note.map(str::to_owned).into(),
            change.screenshots.map(str::to_owned).into(),
            run_id.to_owned().into(),
            step_id.to_owned().into(),
        ];
        values.extend(from.iter().map(|s| (*s).to_owned().into()));
        Ok(self
            .conn()
            .execute(&sql, rusqlite::params_from_iter(values))?
            > 0)
    }

    /// A running run ends: `passed`, `failed`, `cancelled` or `errored`. False when it was not
    /// running any more.
    pub fn finish_test_run(
        &self,
        id: &str,
        status: &str,
        error: Option<&str>,
    ) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE test_runs SET status = ?, error = ?, ended_at = ?
             WHERE id = ? AND status = 'running'",
            params![status, error, now_ms(), id],
        )? > 0)
    }

    /// Cancel a run. One waiting for approval or queued ends at once, its steps with it. One
    /// running is asked to stop: the runner ends the step it is on, runs the teardown, and
    /// ends the run as cancelled. False when the run had already ended, or was already asked.
    pub fn cancel_test_run(&self, id: &str, reason: &str) -> StoreResult<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let now = now_ms();
        let ended = tx.execute(
            "UPDATE test_runs SET status = 'cancelled', error = ?, ended_at = ?
             WHERE id = ? AND status IN ('awaiting_approval', 'queued')",
            params![reason, now, id],
        )? > 0;
        if ended {
            tx.execute(
                "UPDATE test_step_runs SET status = 'cancelled', ended_at = ?
                 WHERE run_id = ? AND status IN ('pending', 'running', 'waiting')",
                params![now, id],
            )?;
            tx.commit()?;
            return Ok(true);
        }
        let asked = tx.execute(
            "UPDATE test_runs SET cancel_requested_at = ?, error = ?
             WHERE id = ? AND status = 'running' AND cancel_requested_at IS NULL",
            params![now, reason, id],
        )? > 0;
        tx.commit()?;
        Ok(asked)
    }

    /// Whether a cancel was asked for on a running run.
    pub fn test_run_cancel_requested(&self, id: &str) -> StoreResult<bool> {
        Ok(self
            .conn()
            .query_row(
                "SELECT cancel_requested_at IS NOT NULL FROM test_runs WHERE id = ?",
                [id],
                |row| row.get::<_, bool>(0),
            )
            .optional()?
            .unwrap_or(false))
    }

    /// A run found `running` when the app starts was interrupted by the app stopping: ended
    /// as `errored`, its unfinished steps with it. Returns the rows that were ended.
    pub fn end_interrupted_test_runs(&self, error: &str) -> StoreResult<Vec<TestRunRow>> {
        let interrupted: Vec<TestRunRow> = self
            .active_test_runs()?
            .into_iter()
            .filter(|run| run.status == "running")
            .collect();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let now = now_ms();
        for run in &interrupted {
            tx.execute(
                "UPDATE test_step_runs SET status = 'failed', ended_at = ?, note = ?
                 WHERE run_id = ? AND status IN ('running', 'waiting')",
                params![now, error, run.id],
            )?;
            tx.execute(
                "UPDATE test_step_runs SET status = 'skipped', ended_at = ?
                 WHERE run_id = ? AND status = 'pending'",
                params![now, run.id],
            )?;
            tx.execute(
                "UPDATE test_runs SET status = 'errored', error = ?, ended_at = ?
                 WHERE id = ? AND status = 'running'",
                params![error, now, run.id],
            )?;
        }
        tx.commit()?;
        Ok(interrupted)
    }

    /// A `manual` step is answered: `waiting` becomes `passed` or `failed`. False when it was
    /// not waiting, or its run is not running.
    pub fn answer_test_step(
        &self,
        run_id: &str,
        step_id: &str,
        passed: bool,
        note: Option<&str>,
    ) -> StoreResult<bool> {
        let outputs = serde_json::json!({ "note": note.unwrap_or_default() }).to_string();
        self.move_test_step(
            run_id,
            step_id,
            &["waiting"],
            &StepChange {
                to: if passed { "passed" } else { "failed" },
                exit: None,
                outputs: Some(&outputs),
                note,
                screenshots: None,
            },
        )
    }

    /// Ended runs older than `before`, removed with their steps. Returns `(id, output_dir)` of
    /// each, so the caller can remove the output folders too.
    pub fn prune_test_runs(&self, before: i64) -> StoreResult<Vec<(String, Option<String>)>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let gone: Vec<(String, Option<String>)> = {
            let mut stmt = tx.prepare(
                "SELECT id, output_dir FROM test_runs
                 WHERE status NOT IN ('awaiting_approval', 'queued', 'running')
                   AND COALESCE(ended_at, created_at) < ?",
            )?;
            let rows = stmt.query_map([before], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<Result<_, _>>()?
        };
        for (id, _) in &gone {
            tx.execute("DELETE FROM test_runs WHERE id = ?", [id])?;
        }
        tx.commit()?;
        Ok(gone)
    }

    // --- approvals ------------------------------------------------------------------------

    /// Whether this exact file has been approved for this project.
    pub fn test_approved(
        &self,
        project_id: &str,
        test_id: &str,
        content_hash: &str,
    ) -> StoreResult<bool> {
        Ok(self
            .conn()
            .query_row(
                "SELECT 1 FROM test_approvals
                 WHERE project_id = ? AND test_id = ? AND content_hash = ?",
                [project_id, test_id, content_hash],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// The user approved a run's file: the approval is recorded for the project, and the run,
    /// if it was waiting, is queued. False when the run was not waiting for approval.
    pub fn approve_test_run(&self, id: &str) -> StoreResult<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let Some(run) = tx
            .query_row(
                &format!(
                    "SELECT {TEST_RUN_COLUMNS} FROM test_runs
                     WHERE id = ? AND status = 'awaiting_approval'"
                ),
                [id],
                test_run_from_row,
            )
            .optional()?
        else {
            return Ok(false);
        };
        if let Some(project_id) = &run.project_id {
            tx.execute(
                "INSERT OR IGNORE INTO test_approvals (project_id, test_id, content_hash,
                                                      approved_at)
                 VALUES (?, ?, ?, ?)",
                params![project_id, run.test_id, run.content_hash, now_ms()],
            )?;
        }
        tx.execute(
            "UPDATE test_runs SET status = 'queued' WHERE id = ? AND status = 'awaiting_approval'",
            [id],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// The user refused a run's file. Nothing is recorded against the file: asked again, the
    /// same file waits again. False when the run was not waiting for approval.
    pub fn refuse_test_run(&self, id: &str) -> StoreResult<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let now = now_ms();
        let refused = tx.execute(
            "UPDATE test_runs SET status = 'refused', ended_at = ?
             WHERE id = ? AND status = 'awaiting_approval'",
            params![now, id],
        )? > 0;
        if refused {
            tx.execute(
                "UPDATE test_step_runs SET status = 'skipped', ended_at = ?
                 WHERE run_id = ? AND status = 'pending'",
                params![now, id],
            )?;
        }
        tx.commit()?;
        Ok(refused)
    }

    // --- the test environment -------------------------------------------------------------

    /// A project's test environment, by name.
    pub fn project_test_env(&self, project_id: &str) -> StoreResult<Vec<(String, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT name, value FROM project_test_env WHERE project_id = ? ORDER BY name",
        )?;
        let rows = stmt.query_map([project_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Replace a project's test environment with `entries`.
    pub fn set_project_test_env(
        &self,
        project_id: &str,
        entries: &[(String, String)],
    ) -> StoreResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM project_test_env WHERE project_id = ?",
            [project_id],
        )?;
        for (name, value) in entries {
            tx.execute(
                "INSERT INTO project_test_env (project_id, name, value) VALUES (?, ?, ?)",
                params![project_id, name, value],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_workspace() -> (tempfile::TempDir, Store, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("yardsort.db")).unwrap();
        let project = store.add_project("app", "/code/app").unwrap();
        let workspace = store
            .add_worktree(&project.id, "fix", "/code/app-wt/fix", None, None)
            .unwrap();
        (dir, store, project.id, workspace.id)
    }

    fn queue(store: &Store, project: &str, workspace: &str, id: &str, status: &str) -> TestQueued {
        store
            .queue_test_run(&NewTestRun {
                id,
                project_id: project,
                workspace_id: workspace,
                workspace_name: "fix",
                test_id: "health",
                test_name: "Health",
                definition: "id: health\n",
                content_hash: "abc",
                inputs: "{}",
                status,
                requested_by: "cli",
                session_record_id: None,
                steps: &[
                    ("server", "setup", "start"),
                    ("ping", "steps", "http"),
                    ("ask", "steps", "manual"),
                    ("reset", "teardown", "run"),
                ],
            })
            .unwrap()
    }

    #[test]
    fn a_test_runs_once_at_a_time_in_a_workspace_whatever_its_state() {
        let (_dir, store, project, workspace) = store_with_workspace();
        assert_eq!(
            queue(&store, &project, &workspace, "r1", "awaiting_approval"),
            TestQueued::Queued
        );
        assert_eq!(
            queue(&store, &project, &workspace, "r2", "queued"),
            TestQueued::AlreadyRunning("r1".into())
        );
        assert!(store.refuse_test_run("r1").unwrap());
        let refused = store.test_run("r1").unwrap().unwrap();
        assert_eq!(refused.status, "refused");
        assert!(store
            .test_steps("r1")
            .unwrap()
            .iter()
            .all(|s| s.status == "skipped"));
        assert_eq!(
            queue(&store, &project, &workspace, "r2", "queued"),
            TestQueued::Queued,
            "a refused run holds nothing"
        );
    }

    #[test]
    fn approving_records_the_hash_and_queues_the_waiting_run() {
        let (_dir, store, project, workspace) = store_with_workspace();
        assert!(!store.test_approved(&project, "health", "abc").unwrap());
        queue(&store, &project, &workspace, "r1", "awaiting_approval");
        assert!(store.approve_test_run("r1").unwrap());
        assert!(!store.approve_test_run("r1").unwrap(), "only once");
        assert_eq!(store.test_run("r1").unwrap().unwrap().status, "queued");
        assert!(store.test_approved(&project, "health", "abc").unwrap());
        assert!(
            !store.test_approved(&project, "health", "abd").unwrap(),
            "another hash is another file"
        );
    }

    #[test]
    fn steps_move_only_while_the_run_runs_and_a_cancel_asks_a_running_run_to_stop() {
        let (_dir, store, project, workspace) = store_with_workspace();
        queue(&store, &project, &workspace, "r1", "queued");
        let running = StepChange {
            to: "running",
            exit: None,
            outputs: None,
            note: None,
            screenshots: None,
        };
        assert!(
            !store
                .move_test_step("r1", "server", &["pending"], &running)
                .unwrap(),
            "the run has not started"
        );
        assert!(store.start_test_run("r1", 4321, "/out/r1").unwrap());
        assert!(!store.start_test_run("r1", 4321, "/out/r1").unwrap());
        let row = store.test_run("r1").unwrap().unwrap();
        assert_eq!(
            (row.port, row.output_dir.as_deref()),
            (Some(4321), Some("/out/r1"))
        );
        assert!(store
            .move_test_step("r1", "server", &["pending"], &running)
            .unwrap());
        assert!(
            !store
                .move_test_step("r1", "server", &["pending"], &running)
                .unwrap(),
            "not pending any more"
        );
        let passed = StepChange {
            to: "passed",
            exit: Some(0),
            outputs: Some(r#"{"pid":"12"}"#),
            note: None,
            screenshots: None,
        };
        assert!(store
            .move_test_step("r1", "server", &["running"], &passed)
            .unwrap());
        let server = store.test_step("r1", "server").unwrap().unwrap();
        assert_eq!(server.exit, Some(0));
        assert_eq!(server.outputs().get("pid").map(String::as_str), Some("12"));
        assert!(server.started_at.is_some() && server.ended_at.is_some());

        assert!(store.cancel_test_run("r1", "Cancelled from ys.").unwrap());
        assert!(!store.cancel_test_run("r1", "again").unwrap(), "asked once");
        let row = store.test_run("r1").unwrap().unwrap();
        assert_eq!(
            row.status, "running",
            "the runner ends it, after the teardown"
        );
        assert!(store.test_run_cancel_requested("r1").unwrap());
        assert!(store
            .finish_test_run("r1", "cancelled", Some("Cancelled from ys."))
            .unwrap());
        assert!(!store.finish_test_run("r1", "passed", None).unwrap());
        assert!(
            !store
                .move_test_step("r1", "reset", &["pending"], &running)
                .unwrap(),
            "nothing moves after the end"
        );
    }

    #[test]
    fn a_queued_run_cancels_at_once_and_a_manual_step_is_answered() {
        let (_dir, store, project, workspace) = store_with_workspace();
        queue(&store, &project, &workspace, "r1", "queued");
        assert!(store.cancel_test_run("r1", "Cancelled from ys.").unwrap());
        let row = store.test_run("r1").unwrap().unwrap();
        assert_eq!(row.status, "cancelled");
        assert!(store
            .test_steps("r1")
            .unwrap()
            .iter()
            .all(|s| s.status == "cancelled"));

        queue(&store, &project, &workspace, "r2", "queued");
        store.start_test_run("r2", 1, "/out").unwrap();
        let waiting = StepChange {
            to: "waiting",
            exit: None,
            outputs: None,
            note: None,
            screenshots: None,
        };
        assert!(!store.answer_test_step("r2", "ask", true, None).unwrap());
        store
            .move_test_step("r2", "ask", &["pending"], &waiting)
            .unwrap();
        assert!(store
            .answer_test_step("r2", "ask", false, Some("the chart flashed"))
            .unwrap());
        let ask = store.test_step("r2", "ask").unwrap().unwrap();
        assert_eq!(ask.status, "failed");
        assert_eq!(ask.note.as_deref(), Some("the chart flashed"));
        assert_eq!(
            ask.outputs().get("note").map(String::as_str),
            Some("the chart flashed")
        );
    }

    #[test]
    fn an_interrupted_run_is_ended_as_errored_and_old_runs_are_pruned() {
        let (_dir, store, project, workspace) = store_with_workspace();
        queue(&store, &project, &workspace, "r1", "queued");
        store.start_test_run("r1", 1, "/out/r1").unwrap();
        let running = StepChange {
            to: "running",
            exit: None,
            outputs: None,
            note: None,
            screenshots: None,
        };
        store
            .move_test_step("r1", "server", &["pending"], &running)
            .unwrap();
        let ended = store
            .end_interrupted_test_runs("Yardsort stopped during the run.")
            .unwrap();
        assert_eq!(ended.len(), 1);
        let row = store.test_run("r1").unwrap().unwrap();
        assert_eq!(row.status, "errored");
        let steps = store.test_steps("r1").unwrap();
        assert_eq!(steps[0].status, "failed");
        assert_eq!(steps[1].status, "skipped");
        assert!(store.end_interrupted_test_runs("again").unwrap().is_empty());

        let pruned = store.prune_test_runs(now_ms() + 1).unwrap();
        assert_eq!(pruned, vec![("r1".to_owned(), Some("/out/r1".to_owned()))]);
        assert!(store.test_run("r1").unwrap().is_none());
        assert!(store.test_steps("r1").unwrap().is_empty());
    }

    #[test]
    fn the_test_environment_is_replaced_whole() {
        let (_dir, store, project, _) = store_with_workspace();
        assert!(store.project_test_env(&project).unwrap().is_empty());
        store
            .set_project_test_env(
                &project,
                &[
                    ("TEST_PASSWORD".into(), "hunter2".into()),
                    ("DATABASE_URL".into(), "postgres://".into()),
                ],
            )
            .unwrap();
        assert_eq!(
            store.project_test_env(&project).unwrap(),
            vec![
                ("DATABASE_URL".to_owned(), "postgres://".to_owned()),
                ("TEST_PASSWORD".to_owned(), "hunter2".to_owned()),
            ]
        );
        store
            .set_project_test_env(&project, &[("TEST_PASSWORD".into(), "x".into())])
            .unwrap();
        assert_eq!(
            store.project_test_env(&project).unwrap(),
            vec![("TEST_PASSWORD".to_owned(), "x".to_owned())]
        );
    }
}
