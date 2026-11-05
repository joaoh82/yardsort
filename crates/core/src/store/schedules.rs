//! Schedules and the times they came due. See `migrations/0022_schedules.sql` and
//! `crate::schedule`.
//!
//! The rules are the scheduler's; this is storage. The one rule that lives here is the one two
//! writers must agree on: a time is claimed by moving `next_at` on in a conditional update, so
//! it is acted on once however many are looking.

use rusqlite::{params, OptionalExtension};

use super::{new_id, now_ms, Store, StoreResult};

/// How many of a schedule's times are kept. Older ones go when a new one is recorded.
pub const SCHEDULE_RUNS_KEPT: usize = 200;

/// A standing instruction to start a workflow or a smoke test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleRow {
    pub id: String,
    pub name: String,
    /// `workflow` or `test`.
    pub target_kind: String,
    pub target_id: String,
    pub project_id: String,
    /// `None` once the workspace is deleted; `workspace_name` keeps what it was called.
    pub workspace_id: Option<String>,
    pub workspace_name: String,
    /// A JSON object of strings.
    pub inputs: String,
    /// `schedule::Spec` as JSON.
    pub spec: String,
    /// `run_once` or `skip`.
    pub missed: String,
    /// `failure`, `always` or `never`.
    pub notify: String,
    pub enabled: bool,
    /// The next time it fires; `None` when there is none.
    pub next_at: Option<i64>,
    /// `app` or `cli`.
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// What a schedule is made of, for adding one and for changing one.
#[derive(Debug, Clone)]
pub struct ScheduleFields<'a> {
    pub name: &'a str,
    pub target_kind: &'a str,
    pub target_id: &'a str,
    pub project_id: &'a str,
    pub workspace_id: &'a str,
    pub workspace_name: &'a str,
    pub inputs: &'a str,
    pub spec: &'a str,
    pub missed: &'a str,
    pub notify: &'a str,
    pub next_at: Option<i64>,
}

/// One time a schedule came due.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleRunRow {
    pub id: String,
    pub schedule_id: String,
    pub due_at: i64,
    pub fired_at: i64,
    /// `started`, `skipped`, `missed` or `refused`.
    pub outcome: String,
    /// Asked for with Run now, not by the clock.
    pub manual: bool,
    pub run_id: Option<String>,
    pub note: Option<String>,
    /// The run's status once it ended.
    pub result: Option<String>,
    pub notified_at: Option<i64>,
    pub told_waiting: bool,
}

#[derive(Debug, Clone)]
pub struct NewScheduleRun<'a> {
    pub schedule_id: &'a str,
    pub due_at: i64,
    pub fired_at: i64,
    pub outcome: &'a str,
    pub manual: bool,
    pub run_id: Option<&'a str>,
    pub note: Option<&'a str>,
}

const SCHEDULE_COLUMNS: &str = "id, name, target_kind, target_id, project_id, workspace_id, \
     workspace_name, inputs, spec, missed, notify, enabled, next_at, created_by, created_at, \
     updated_at";

fn schedule_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduleRow> {
    Ok(ScheduleRow {
        id: row.get(0)?,
        name: row.get(1)?,
        target_kind: row.get(2)?,
        target_id: row.get(3)?,
        project_id: row.get(4)?,
        workspace_id: row.get(5)?,
        workspace_name: row.get(6)?,
        inputs: row.get(7)?,
        spec: row.get(8)?,
        missed: row.get(9)?,
        notify: row.get(10)?,
        enabled: row.get(11)?,
        next_at: row.get(12)?,
        created_by: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

const SCHEDULE_RUN_COLUMNS: &str = "id, schedule_id, due_at, fired_at, outcome, manual, run_id, \
     note, result, notified_at, told_waiting";

fn schedule_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduleRunRow> {
    Ok(ScheduleRunRow {
        id: row.get(0)?,
        schedule_id: row.get(1)?,
        due_at: row.get(2)?,
        fired_at: row.get(3)?,
        outcome: row.get(4)?,
        manual: row.get(5)?,
        run_id: row.get(6)?,
        note: row.get(7)?,
        result: row.get(8)?,
        notified_at: row.get(9)?,
        told_waiting: row.get(10)?,
    })
}

/// Schedules of projects that are still in the app.
const OF_LIVE_PROJECTS: &str = "project_id IN (SELECT id FROM projects WHERE removed = 0)";

impl Store {
    // --- schedules ------------------------------------------------------------------------

    pub fn add_schedule(
        &self,
        fields: &ScheduleFields<'_>,
        created_by: &str,
    ) -> StoreResult<ScheduleRow> {
        let id = new_id();
        let now = now_ms();
        self.conn().execute(
            "INSERT INTO schedules (id, name, target_kind, target_id, project_id, workspace_id,
                                    workspace_name, inputs, spec, missed, notify, enabled, next_at,
                                    created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?, ?)",
            params![
                id,
                fields.name,
                fields.target_kind,
                fields.target_id,
                fields.project_id,
                fields.workspace_id,
                fields.workspace_name,
                fields.inputs,
                fields.spec,
                fields.missed,
                fields.notify,
                fields.next_at,
                created_by,
                now,
                now,
            ],
        )?;
        Ok(self.schedule(&id)?.expect("the schedule just added"))
    }

    /// Replace what a schedule is made of. Whether it is on is left as it was.
    pub fn update_schedule(&self, id: &str, fields: &ScheduleFields<'_>) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE schedules
             SET name = ?, target_kind = ?, target_id = ?, project_id = ?, workspace_id = ?,
                 workspace_name = ?, inputs = ?, spec = ?, missed = ?, notify = ?, next_at = ?,
                 updated_at = ?
             WHERE id = ?",
            params![
                fields.name,
                fields.target_kind,
                fields.target_id,
                fields.project_id,
                fields.workspace_id,
                fields.workspace_name,
                fields.inputs,
                fields.spec,
                fields.missed,
                fields.notify,
                fields.next_at,
                now_ms(),
                id,
            ],
        )? > 0)
    }

    pub fn schedule(&self, id: &str) -> StoreResult<Option<ScheduleRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {SCHEDULE_COLUMNS} FROM schedules WHERE id = ?"),
                [id],
                schedule_from_row,
            )
            .optional()?)
    }

    /// Every schedule of every project still in the app, by name.
    pub fn schedules(&self) -> StoreResult<Vec<ScheduleRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {SCHEDULE_COLUMNS} FROM schedules WHERE {OF_LIVE_PROJECTS}
             ORDER BY name COLLATE NOCASE, created_at"
        ))?;
        let rows = stmt.query_map([], schedule_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Schedules that are on and whose time has come, the longest overdue first.
    pub fn due_schedules(&self, now: i64) -> StoreResult<Vec<ScheduleRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {SCHEDULE_COLUMNS} FROM schedules
             WHERE enabled = 1 AND next_at IS NOT NULL AND next_at <= ? AND {OF_LIVE_PROJECTS}
             ORDER BY next_at, created_at"
        ))?;
        let rows = stmt.query_map([now], schedule_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Schedules that are on and have a time to come: what is recomputed when the machine's
    /// zone changes.
    pub fn waiting_schedules(&self) -> StoreResult<Vec<ScheduleRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {SCHEDULE_COLUMNS} FROM schedules
             WHERE enabled = 1 AND next_at IS NOT NULL AND {OF_LIVE_PROJECTS}"
        ))?;
        let rows = stmt.query_map([], schedule_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Take the time `due_at` for this schedule by moving it on to `next_at`. `false` when it
    /// had already been taken, or the schedule was switched off or changed meanwhile: whoever
    /// gets `true` is the one to act. A schedule with no next time is switched off.
    pub fn claim_schedule(&self, id: &str, due_at: i64, next_at: Option<i64>) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE schedules
             SET next_at = ?1, enabled = CASE WHEN ?1 IS NULL THEN 0 ELSE enabled END
             WHERE id = ?2 AND enabled = 1 AND next_at = ?3",
            params![next_at, id, due_at],
        )? > 0)
    }

    /// Take the time `due_at` as [`claim_schedule`](Self::claim_schedule) does, for a schedule
    /// whose next time could not be worked out. It stays switched on, with no next time: it
    /// fires nothing until it is edited, and is not mistaken for a `once` that is done.
    pub fn stall_schedule(&self, id: &str, due_at: i64) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE schedules SET next_at = NULL WHERE id = ? AND enabled = 1 AND next_at = ?",
            params![id, due_at],
        )? > 0)
    }

    /// Move a waiting schedule's next time, when the zone it was worked out in changed. Only
    /// while it still has the time it was read with.
    pub fn move_schedule(&self, id: &str, from: i64, to: Option<i64>) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE schedules SET next_at = ?1 WHERE id = ?2 AND enabled = 1 AND next_at = ?3",
            params![to, id, from],
        )? > 0)
    }

    /// Switch a schedule on, with its next time worked out from now, or off, keeping its place.
    pub fn set_schedule_enabled(
        &self,
        id: &str,
        enabled: bool,
        next_at: Option<i64>,
    ) -> StoreResult<bool> {
        let conn = self.conn();
        let changed = if enabled {
            conn.execute(
                "UPDATE schedules SET enabled = 1, next_at = ?, updated_at = ? WHERE id = ?",
                params![next_at, now_ms(), id],
            )?
        } else {
            conn.execute(
                "UPDATE schedules SET enabled = 0, updated_at = ? WHERE id = ?",
                params![now_ms(), id],
            )?
        };
        Ok(changed > 0)
    }

    /// Remove a schedule and its history. The runs it started are not its to remove.
    pub fn remove_schedule(&self, id: &str) -> StoreResult<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM schedules WHERE id = ?", [id])?
            > 0)
    }

    /// Something that is different whenever a schedule was added, changed, switched or removed,
    /// by any process: how the app notices what `ys` did to the list it is showing.
    pub fn schedules_stamp(&self) -> StoreResult<(i64, i64)> {
        Ok(self.conn().query_row(
            "SELECT count(*), COALESCE(max(updated_at), 0) FROM schedules",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?)
    }

    // --- the times a schedule came due ----------------------------------------------------

    /// Record what became of a time, and drop the schedule's oldest beyond what is kept.
    pub fn record_schedule_run(&self, new: &NewScheduleRun<'_>) -> StoreResult<ScheduleRunRow> {
        let id = new_id();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO schedule_runs (id, schedule_id, due_at, fired_at, outcome, manual,
                                        run_id, note)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                id,
                new.schedule_id,
                new.due_at,
                new.fired_at,
                new.outcome,
                new.manual,
                new.run_id,
                new.note,
            ],
        )?;
        tx.execute(
            "DELETE FROM schedule_runs
             WHERE schedule_id = ?1 AND id NOT IN (
                 SELECT id FROM schedule_runs WHERE schedule_id = ?1
                 ORDER BY fired_at DESC, rowid DESC LIMIT ?2)",
            params![new.schedule_id, SCHEDULE_RUNS_KEPT as i64],
        )?;
        let row = tx.query_row(
            &format!("SELECT {SCHEDULE_RUN_COLUMNS} FROM schedule_runs WHERE id = ?"),
            [&id],
            schedule_run_from_row,
        )?;
        tx.commit()?;
        Ok(row)
    }

    /// A schedule's times, newest first.
    pub fn schedule_runs(
        &self,
        schedule_id: &str,
        limit: usize,
    ) -> StoreResult<Vec<ScheduleRunRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {SCHEDULE_RUN_COLUMNS} FROM schedule_runs WHERE schedule_id = ?
             ORDER BY fired_at DESC, rowid DESC LIMIT ?"
        ))?;
        let rows = stmt.query_map(params![schedule_id, limit as i64], schedule_run_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The newest time of every schedule that has one.
    pub fn latest_schedule_runs(&self) -> StoreResult<Vec<ScheduleRunRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {SCHEDULE_RUN_COLUMNS} FROM schedule_runs AS run
             WHERE rowid = (SELECT rowid FROM schedule_runs
                            WHERE schedule_id = run.schedule_id
                            ORDER BY fired_at DESC, rowid DESC LIMIT 1)"
        ))?;
        let rows = stmt.query_map([], schedule_run_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Times that started a run whose ending has not been copied here yet.
    pub fn unsettled_schedule_runs(&self) -> StoreResult<Vec<ScheduleRunRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {SCHEDULE_RUN_COLUMNS} FROM schedule_runs
             WHERE outcome = 'started' AND result IS NULL ORDER BY fired_at"
        ))?;
        let rows = stmt.query_map([], schedule_run_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Copy the run's ending onto the time that started it. Once: `false` when it already has
    /// one.
    pub fn settle_schedule_run(&self, id: &str, result: &str, notified: bool) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE schedule_runs SET result = ?, notified_at = ?
             WHERE id = ? AND result IS NULL",
            params![result, notified.then(now_ms), id],
        )? > 0)
    }

    /// Note that the user was told this time's run waits for approval. Once: `false` when they
    /// already were.
    pub fn mark_schedule_run_told_waiting(&self, id: &str) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE schedule_runs SET told_waiting = 1 WHERE id = ? AND told_waiting = 0",
            [id],
        )? > 0)
    }

    /// Every run a schedule started and still has a line for, as `(run id, schedule id,
    /// schedule name)`: what a run list marks as scheduled.
    pub fn scheduled_runs(&self) -> StoreResult<Vec<(String, String, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT run.run_id, schedule.id, schedule.name
             FROM schedule_runs AS run JOIN schedules AS schedule ON schedule.id = run.schedule_id
             WHERE run.run_id IS NOT NULL ORDER BY run.fired_at DESC, run.rowid DESC",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
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

    fn fields<'a>(
        project: &'a str,
        workspace: &'a str,
        next_at: Option<i64>,
    ) -> ScheduleFields<'a> {
        ScheduleFields {
            name: "Nightly",
            target_kind: "test",
            target_id: "health",
            project_id: project,
            workspace_id: workspace,
            workspace_name: "fix",
            inputs: "{}",
            spec: r#"{"kind":"every","minutes":60}"#,
            missed: "run_once",
            notify: "failure",
            next_at,
        }
    }

    fn came_due<'a>(schedule: &'a str, fired_at: i64, outcome: &'a str) -> NewScheduleRun<'a> {
        NewScheduleRun {
            schedule_id: schedule,
            due_at: fired_at,
            fired_at,
            outcome,
            manual: false,
            run_id: None,
            note: None,
        }
    }

    #[test]
    fn a_schedule_is_due_once_its_time_has_come_and_only_while_it_is_on() {
        let (_dir, store, project, workspace) = store_with_workspace();
        let schedule = store
            .add_schedule(&fields(&project, &workspace, Some(1_000)), "app")
            .unwrap();
        assert!(schedule.enabled);
        assert_eq!(schedule.created_by, "app");

        assert!(store.due_schedules(999).unwrap().is_empty());
        assert_eq!(store.due_schedules(1_000).unwrap(), vec![schedule.clone()]);

        assert!(store
            .set_schedule_enabled(&schedule.id, false, None)
            .unwrap());
        assert!(store.due_schedules(5_000).unwrap().is_empty());
        // Switched off, it keeps the time it had; switched on, it gets the one it is given.
        assert_eq!(
            store.schedule(&schedule.id).unwrap().unwrap().next_at,
            Some(1_000)
        );
        assert!(store
            .set_schedule_enabled(&schedule.id, true, Some(9_000))
            .unwrap());
        assert!(store.due_schedules(5_000).unwrap().is_empty());
        assert_eq!(store.due_schedules(9_000).unwrap().len(), 1);
    }

    #[test]
    fn a_time_is_claimed_once_even_by_two_stores_on_one_file() {
        let (dir, store, project, workspace) = store_with_workspace();
        let schedule = store
            .add_schedule(&fields(&project, &workspace, Some(1_000)), "cli")
            .unwrap();
        let other = Store::open(&dir.path().join("yardsort.db")).unwrap();

        assert!(store
            .claim_schedule(&schedule.id, 1_000, Some(2_000))
            .unwrap());
        assert!(!other
            .claim_schedule(&schedule.id, 1_000, Some(2_000))
            .unwrap());
        assert_eq!(
            other.schedule(&schedule.id).unwrap().unwrap().next_at,
            Some(2_000)
        );
    }

    #[test]
    fn claiming_the_last_time_switches_the_schedule_off() {
        let (_dir, store, project, workspace) = store_with_workspace();
        let schedule = store
            .add_schedule(&fields(&project, &workspace, Some(1_000)), "app")
            .unwrap();
        assert!(store.claim_schedule(&schedule.id, 1_000, None).unwrap());
        let done = store.schedule(&schedule.id).unwrap().unwrap();
        assert!(!done.enabled);
        assert_eq!(done.next_at, None);
        assert!(store.due_schedules(i64::MAX).unwrap().is_empty());
    }

    #[test]
    fn a_schedule_switched_off_meanwhile_is_not_claimed() {
        let (_dir, store, project, workspace) = store_with_workspace();
        let schedule = store
            .add_schedule(&fields(&project, &workspace, Some(1_000)), "app")
            .unwrap();
        store
            .set_schedule_enabled(&schedule.id, false, None)
            .unwrap();
        assert!(!store
            .claim_schedule(&schedule.id, 1_000, Some(2_000))
            .unwrap());
    }

    #[test]
    fn changing_a_schedule_keeps_whether_it_is_on() {
        let (_dir, store, project, workspace) = store_with_workspace();
        let schedule = store
            .add_schedule(&fields(&project, &workspace, Some(1_000)), "app")
            .unwrap();
        store
            .set_schedule_enabled(&schedule.id, false, None)
            .unwrap();
        let mut changed = fields(&project, &workspace, Some(7_000));
        changed.name = "Hourly";
        changed.notify = "always";
        assert!(store.update_schedule(&schedule.id, &changed).unwrap());
        let row = store.schedule(&schedule.id).unwrap().unwrap();
        assert_eq!(
            (
                row.name.as_str(),
                row.notify.as_str(),
                row.next_at,
                row.enabled
            ),
            ("Hourly", "always", Some(7_000), false)
        );
    }

    #[test]
    fn a_deleted_workspace_leaves_the_schedule_with_its_name() {
        let (_dir, store, project, workspace) = store_with_workspace();
        let schedule = store
            .add_schedule(&fields(&project, &workspace, Some(1_000)), "app")
            .unwrap();
        assert!(store.remove_worktree(&workspace).unwrap());
        let row = store.schedule(&schedule.id).unwrap().unwrap();
        assert_eq!(row.workspace_id, None);
        assert_eq!(row.workspace_name, "fix");
    }

    #[test]
    fn a_removed_projects_schedules_are_neither_listed_nor_due() {
        let (_dir, store, project, workspace) = store_with_workspace();
        store
            .add_schedule(&fields(&project, &workspace, Some(1_000)), "app")
            .unwrap();
        store.remove_project(&project, true).unwrap();
        assert!(store.schedules().unwrap().is_empty());
        assert!(store.due_schedules(i64::MAX).unwrap().is_empty());
        assert!(store.waiting_schedules().unwrap().is_empty());
    }

    #[test]
    fn only_the_newest_times_are_kept() {
        let (_dir, store, project, workspace) = store_with_workspace();
        let schedule = store
            .add_schedule(&fields(&project, &workspace, Some(1_000)), "app")
            .unwrap();
        let other = store
            .add_schedule(&fields(&project, &workspace, Some(1_000)), "app")
            .unwrap();
        store
            .record_schedule_run(&came_due(&other.id, 1, "missed"))
            .unwrap();
        for at in 0..(SCHEDULE_RUNS_KEPT as i64 + 5) {
            store
                .record_schedule_run(&came_due(&schedule.id, at, "skipped"))
                .unwrap();
        }
        let kept = store.schedule_runs(&schedule.id, 1_000).unwrap();
        assert_eq!(kept.len(), SCHEDULE_RUNS_KEPT);
        assert_eq!(kept[0].fired_at, SCHEDULE_RUNS_KEPT as i64 + 4);
        assert_eq!(kept.last().unwrap().fired_at, 5);
        // Another schedule's history is not this one's to prune.
        assert_eq!(store.schedule_runs(&other.id, 10).unwrap().len(), 1);

        let latest = store.latest_schedule_runs().unwrap();
        assert_eq!(latest.len(), 2);
        assert!(latest
            .iter()
            .any(|run| run.schedule_id == schedule.id && run.fired_at == kept[0].fired_at));
    }

    #[test]
    fn a_started_time_is_settled_once_and_names_its_schedule() {
        let (_dir, store, project, workspace) = store_with_workspace();
        let schedule = store
            .add_schedule(&fields(&project, &workspace, Some(1_000)), "app")
            .unwrap();
        let mut started = came_due(&schedule.id, 10, "started");
        started.run_id = Some("run-1");
        let row = store.record_schedule_run(&started).unwrap();
        store
            .record_schedule_run(&came_due(&schedule.id, 11, "skipped"))
            .unwrap();

        assert_eq!(store.unsettled_schedule_runs().unwrap(), vec![row.clone()]);
        assert!(store.mark_schedule_run_told_waiting(&row.id).unwrap());
        assert!(!store.mark_schedule_run_told_waiting(&row.id).unwrap());
        assert!(store.settle_schedule_run(&row.id, "failed", true).unwrap());
        assert!(!store.settle_schedule_run(&row.id, "passed", false).unwrap());
        assert!(store.unsettled_schedule_runs().unwrap().is_empty());
        let settled = &store.schedule_runs(&schedule.id, 10).unwrap()[1];
        assert_eq!(settled.result.as_deref(), Some("failed"));
        assert!(settled.notified_at.is_some());

        assert_eq!(
            store.scheduled_runs().unwrap(),
            vec![(
                "run-1".to_owned(),
                schedule.id.clone(),
                "Nightly".to_owned()
            )]
        );
    }

    #[test]
    fn the_stamp_changes_when_another_process_adds_or_removes_a_schedule() {
        let (dir, store, project, workspace) = store_with_workspace();
        let empty = store.schedules_stamp().unwrap();
        let other = Store::open(&dir.path().join("yardsort.db")).unwrap();
        let schedule = other
            .add_schedule(&fields(&project, &workspace, Some(1_000)), "cli")
            .unwrap();
        let added = store.schedules_stamp().unwrap();
        assert_ne!(added, empty);
        assert_eq!(store.schedules_stamp().unwrap(), added, "and only then");
        assert!(other.remove_schedule(&schedule.id).unwrap());
        assert_ne!(store.schedules_stamp().unwrap(), added);
    }

    #[test]
    fn removing_a_schedule_removes_its_history() {
        let (_dir, store, project, workspace) = store_with_workspace();
        let schedule = store
            .add_schedule(&fields(&project, &workspace, Some(1_000)), "app")
            .unwrap();
        store
            .record_schedule_run(&came_due(&schedule.id, 1, "missed"))
            .unwrap();
        assert!(store.remove_schedule(&schedule.id).unwrap());
        assert!(store.schedule(&schedule.id).unwrap().is_none());
        assert!(store.schedule_runs(&schedule.id, 10).unwrap().is_empty());
        assert!(!store.remove_schedule(&schedule.id).unwrap());
    }
}
