//! Triggers, the fires they gathered, the done signal, and where the trigger thread has read
//! up to. See `migrations/0023_triggers.sql` and `crate::trigger`.
//!
//! The rules are the trigger engine's; this is storage. The rule that lives here is the one two
//! writers must agree on: a fire is decided by a conditional update from the outcome it was
//! read with, so it is acted on once however many are looking.

use rusqlite::{params, OptionalExtension};

use super::{new_id, now_ms, Store, StoreResult};

/// How many of a trigger's decided fires are kept. Older ones go when a new one is decided; a
/// fire still open, or whose run has not ended, is kept whatever its age.
pub const TRIGGER_FIRES_KEPT: usize = 200;

/// A standing instruction to start a workflow or a smoke test when something happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerRow {
    pub id: String,
    pub name: String,
    pub preset: Option<String>,
    pub project_id: String,
    /// `None` once the workspace is deleted; `workspace_name` keeps what it was called.
    pub workspace_id: Option<String>,
    pub workspace_name: String,
    /// The agent session it belongs to, for a session trigger.
    pub session_id: Option<String>,
    /// A JSON array of event names.
    pub events: String,
    /// `trigger::Filters` as JSON.
    pub filters: String,
    /// `workflow` or `test`.
    pub target_kind: String,
    pub target_id: String,
    /// A JSON object of strings.
    pub inputs: String,
    pub quiet_ms: i64,
    pub cap: i64,
    /// `failure`, `always` or `never`.
    pub notify: String,
    pub enabled: bool,
    /// Only what happened from this time on counts.
    pub since: i64,
    /// `app` or `cli`.
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// What a trigger is made of, for adding one and for changing one.
#[derive(Debug, Clone)]
pub struct TriggerFields<'a> {
    pub name: &'a str,
    pub preset: Option<&'a str>,
    pub project_id: &'a str,
    pub workspace_id: &'a str,
    pub workspace_name: &'a str,
    pub session_id: Option<&'a str>,
    pub events: &'a str,
    pub filters: &'a str,
    pub target_kind: &'a str,
    pub target_id: &'a str,
    pub inputs: &'a str,
    pub quiet_ms: i64,
    pub cap: i64,
    pub notify: &'a str,
}

/// The events one trigger gathered for one subject, and what became of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerFireRow {
    pub id: String,
    pub trigger_id: String,
    /// `session:<record>` or `workspace:<id>`.
    pub subject: String,
    /// The newest event's kind.
    pub event: String,
    pub events_count: i64,
    pub first_at: i64,
    pub due_at: i64,
    pub decided_at: Option<i64>,
    /// `gathering`, `waiting`, `started`, `skipped`, `capped` or `refused`.
    pub outcome: String,
    /// A JSON object of strings: what the events leave for `{{ event.* }}`.
    pub facts: String,
    pub depth: i64,
    pub run_id: Option<String>,
    pub note: Option<String>,
    pub counted: bool,
    /// The run's status once it ended.
    pub result: Option<String>,
    pub notified_at: Option<i64>,
    pub told_waiting: bool,
}

#[derive(Debug, Clone)]
pub struct NewTriggerFire<'a> {
    pub trigger_id: &'a str,
    pub subject: &'a str,
    pub event: &'a str,
    pub at: i64,
    pub due_at: i64,
    pub facts: &'a str,
    pub depth: i64,
}

/// What became of a fire that was due.
#[derive(Debug, Clone)]
pub struct FireDecision<'a> {
    /// `waiting`, `started`, `skipped`, `capped` or `refused`.
    pub outcome: &'a str,
    pub run_id: Option<&'a str>,
    pub note: Option<&'a str>,
    /// Counted against the cap: a run was started.
    pub counted: bool,
    pub at: i64,
}

/// One `ys workspace done`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoneSignalRow {
    pub seq: i64,
    pub id: String,
    pub workspace_id: String,
    pub session_id: Option<String>,
    pub head: Option<String>,
    pub note: Option<String>,
    pub at: i64,
}

/// One `ys event emit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomEventRow {
    pub seq: i64,
    pub id: String,
    pub workspace_id: String,
    pub session_id: Option<String>,
    /// The smoke test run whose `run` step raised it.
    pub run_id: Option<String>,
    /// Without `custom.`.
    pub name: String,
    pub head: Option<String>,
    pub note: Option<String>,
    pub at: i64,
}

/// What a new custom event holds.
#[derive(Debug, Clone, Copy)]
pub struct NewCustomEvent<'a> {
    pub workspace_id: &'a str,
    pub session_id: Option<&'a str>,
    pub run_id: Option<&'a str>,
    pub name: &'a str,
    pub head: Option<&'a str>,
    pub note: Option<&'a str>,
}

/// A workflow or test run that ended, as the trigger thread reads them: `(kind, id, workspace,
/// status, ended_at)`, `kind` being `workflow` or `test`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndedRun {
    pub kind: &'static str,
    pub id: String,
    /// The workflow's or the test's own id.
    pub target_id: String,
    pub workspace_id: String,
    pub status: String,
    pub ended_at: i64,
}

const TRIGGER_COLUMNS: &str = "id, name, preset, project_id, workspace_id, workspace_name, \
     session_id, events, filters, target_kind, target_id, inputs, quiet_ms, cap, notify, \
     enabled, since, created_by, created_at, updated_at";

fn trigger_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TriggerRow> {
    Ok(TriggerRow {
        id: row.get(0)?,
        name: row.get(1)?,
        preset: row.get(2)?,
        project_id: row.get(3)?,
        workspace_id: row.get(4)?,
        workspace_name: row.get(5)?,
        session_id: row.get(6)?,
        events: row.get(7)?,
        filters: row.get(8)?,
        target_kind: row.get(9)?,
        target_id: row.get(10)?,
        inputs: row.get(11)?,
        quiet_ms: row.get(12)?,
        cap: row.get(13)?,
        notify: row.get(14)?,
        enabled: row.get(15)?,
        since: row.get(16)?,
        created_by: row.get(17)?,
        created_at: row.get(18)?,
        updated_at: row.get(19)?,
    })
}

const FIRE_COLUMNS: &str = "id, trigger_id, subject, event, events_count, first_at, due_at, \
     decided_at, outcome, facts, depth, run_id, note, counted, result, notified_at, told_waiting";

fn fire_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TriggerFireRow> {
    Ok(TriggerFireRow {
        id: row.get(0)?,
        trigger_id: row.get(1)?,
        subject: row.get(2)?,
        event: row.get(3)?,
        events_count: row.get(4)?,
        first_at: row.get(5)?,
        due_at: row.get(6)?,
        decided_at: row.get(7)?,
        outcome: row.get(8)?,
        facts: row.get(9)?,
        depth: row.get(10)?,
        run_id: row.get(11)?,
        note: row.get(12)?,
        counted: row.get(13)?,
        result: row.get(14)?,
        notified_at: row.get(15)?,
        told_waiting: row.get(16)?,
    })
}

const DONE_COLUMNS: &str = "seq, id, workspace_id, session_id, head, note, at";

fn done_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DoneSignalRow> {
    Ok(DoneSignalRow {
        seq: row.get(0)?,
        id: row.get(1)?,
        workspace_id: row.get(2)?,
        session_id: row.get(3)?,
        head: row.get(4)?,
        note: row.get(5)?,
        at: row.get(6)?,
    })
}

const CUSTOM_COLUMNS: &str = "seq, id, workspace_id, session_id, run_id, name, head, note, at";

fn custom_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CustomEventRow> {
    Ok(CustomEventRow {
        seq: row.get(0)?,
        id: row.get(1)?,
        workspace_id: row.get(2)?,
        session_id: row.get(3)?,
        run_id: row.get(4)?,
        name: row.get(5)?,
        head: row.get(6)?,
        note: row.get(7)?,
        at: row.get(8)?,
    })
}

/// Triggers of projects that are still in the app.
const OF_LIVE_PROJECTS: &str = "project_id IN (SELECT id FROM projects WHERE removed = 0)";

impl Store {
    // --- triggers -------------------------------------------------------------------------

    /// Add a trigger, switched on, counting what happens from `since`.
    pub fn add_trigger(
        &self,
        fields: &TriggerFields<'_>,
        created_by: &str,
        since: i64,
    ) -> StoreResult<TriggerRow> {
        let id = new_id();
        let now = now_ms();
        self.conn().execute(
            "INSERT INTO triggers (id, name, preset, project_id, workspace_id, workspace_name,
                                   session_id, events, filters, target_kind, target_id, inputs,
                                   quiet_ms, cap, notify, enabled, since, created_by, created_at,
                                   updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?, ?)",
            params![
                id,
                fields.name,
                fields.preset,
                fields.project_id,
                fields.workspace_id,
                fields.workspace_name,
                fields.session_id,
                fields.events,
                fields.filters,
                fields.target_kind,
                fields.target_id,
                fields.inputs,
                fields.quiet_ms,
                fields.cap,
                fields.notify,
                since,
                created_by,
                now,
                now,
            ],
        )?;
        Ok(self.trigger(&id)?.expect("the trigger just added"))
    }

    /// Replace what a trigger is made of. Whether it is on, and what it has seen, stay.
    pub fn update_trigger(&self, id: &str, fields: &TriggerFields<'_>) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE triggers
             SET name = ?, preset = ?, project_id = ?, workspace_id = ?, workspace_name = ?,
                 session_id = ?, events = ?, filters = ?, target_kind = ?, target_id = ?,
                 inputs = ?, quiet_ms = ?, cap = ?, notify = ?, updated_at = ?
             WHERE id = ?",
            params![
                fields.name,
                fields.preset,
                fields.project_id,
                fields.workspace_id,
                fields.workspace_name,
                fields.session_id,
                fields.events,
                fields.filters,
                fields.target_kind,
                fields.target_id,
                fields.inputs,
                fields.quiet_ms,
                fields.cap,
                fields.notify,
                now_ms(),
                id,
            ],
        )? > 0)
    }

    pub fn trigger(&self, id: &str) -> StoreResult<Option<TriggerRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {TRIGGER_COLUMNS} FROM triggers WHERE id = ?"),
                [id],
                trigger_from_row,
            )
            .optional()?)
    }

    /// Every trigger of every project still in the app, by name.
    pub fn triggers(&self) -> StoreResult<Vec<TriggerRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {TRIGGER_COLUMNS} FROM triggers WHERE {OF_LIVE_PROJECTS}
             ORDER BY name COLLATE NOCASE, created_at"
        ))?;
        let rows = stmt.query_map([], trigger_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The triggers that are on in a workspace, oldest first: the ones an event there can
    /// fire.
    pub fn workspace_triggers(&self, workspace_id: &str) -> StoreResult<Vec<TriggerRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {TRIGGER_COLUMNS} FROM triggers
             WHERE enabled = 1 AND workspace_id = ? AND {OF_LIVE_PROJECTS}
             ORDER BY created_at, id"
        ))?;
        let rows = stmt.query_map([workspace_id], trigger_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Switch a trigger on, counting what happens from `since`, or off.
    pub fn set_trigger_enabled(&self, id: &str, enabled: bool, since: i64) -> StoreResult<bool> {
        let conn = self.conn();
        let changed = if enabled {
            conn.execute(
                "UPDATE triggers SET enabled = 1, since = ?, updated_at = ? WHERE id = ?",
                params![since, now_ms(), id],
            )?
        } else {
            conn.execute(
                "UPDATE triggers SET enabled = 0, updated_at = ? WHERE id = ?",
                params![now_ms(), id],
            )?
        };
        Ok(changed > 0)
    }

    /// Remove a trigger and its history. The runs it started are not its to remove.
    pub fn remove_trigger(&self, id: &str) -> StoreResult<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM triggers WHERE id = ?", [id])?
            > 0)
    }

    /// Something that is different whenever a trigger was added, changed, switched or removed,
    /// by any process.
    pub fn triggers_stamp(&self) -> StoreResult<(i64, i64)> {
        Ok(self.conn().query_row(
            "SELECT count(*), COALESCE(max(updated_at), 0) FROM triggers",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?)
    }

    // --- fires ----------------------------------------------------------------------------

    /// The fire of this trigger for this subject that has not been decided yet: gathering, or
    /// waiting for a run in its way.
    pub fn open_trigger_fire(
        &self,
        trigger_id: &str,
        subject: &str,
    ) -> StoreResult<Option<TriggerFireRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!(
                    "SELECT {FIRE_COLUMNS} FROM trigger_fires
                     WHERE trigger_id = ? AND subject = ? AND outcome IN ('gathering', 'waiting')
                     ORDER BY first_at, rowid LIMIT 1"
                ),
                params![trigger_id, subject],
                fire_from_row,
            )
            .optional()?)
    }

    /// Open a fire, gathering.
    pub fn open_new_trigger_fire(&self, new: &NewTriggerFire<'_>) -> StoreResult<TriggerFireRow> {
        let id = new_id();
        self.conn().execute(
            "INSERT INTO trigger_fires (id, trigger_id, subject, event, first_at, due_at,
                                        outcome, facts, depth)
             VALUES (?, ?, ?, ?, ?, ?, 'gathering', ?, ?)",
            params![
                id,
                new.trigger_id,
                new.subject,
                new.event,
                new.at,
                new.due_at,
                new.facts,
                new.depth,
            ],
        )?;
        Ok(self.trigger_fire(&id)?.expect("the fire just opened"))
    }

    /// Add one more event to an open fire: its kind and facts are the newest's, its depth the
    /// deepest, and it is due at `due_at`.
    pub fn gather_into_trigger_fire(
        &self,
        id: &str,
        event: &str,
        due_at: i64,
        facts: &str,
        depth: i64,
    ) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE trigger_fires
             SET event = ?, events_count = events_count + 1, due_at = ?, facts = ?,
                 depth = max(depth, ?)
             WHERE id = ? AND outcome IN ('gathering', 'waiting')",
            params![event, due_at, facts, depth, id],
        )? > 0)
    }

    pub fn trigger_fire(&self, id: &str) -> StoreResult<Option<TriggerFireRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {FIRE_COLUMNS} FROM trigger_fires WHERE id = ?"),
                [id],
                fire_from_row,
            )
            .optional()?)
    }

    /// Fires to decide now: gathering ones whose time has come, and every waiting one, oldest
    /// first.
    pub fn due_trigger_fires(&self, now: i64) -> StoreResult<Vec<TriggerFireRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {FIRE_COLUMNS} FROM trigger_fires
             WHERE (outcome = 'gathering' AND due_at <= ?) OR outcome = 'waiting'
             ORDER BY first_at, rowid"
        ))?;
        let rows = stmt.query_map([now], fire_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Decide an open fire. Only from the outcome it was read with: `false` when another
    /// writer decided it first.
    pub fn decide_trigger_fire(
        &self,
        id: &str,
        from: &str,
        decision: &FireDecision<'_>,
    ) -> StoreResult<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE trigger_fires
             SET outcome = ?, run_id = ?, note = ?, counted = ?, decided_at = ?
             WHERE id = ? AND outcome = ?",
            params![
                decision.outcome,
                decision.run_id,
                decision.note,
                decision.counted,
                decision.at,
                id,
                from,
            ],
        )?;
        if changed > 0 && decision.outcome != "waiting" {
            tx.execute(
                "DELETE FROM trigger_fires
                 WHERE trigger_id = (SELECT trigger_id FROM trigger_fires WHERE id = ?1)
                   AND outcome NOT IN ('gathering', 'waiting')
                   AND NOT (outcome = 'started' AND result IS NULL)
                   AND id NOT IN (
                     SELECT id FROM trigger_fires
                     WHERE trigger_id = (SELECT trigger_id FROM trigger_fires WHERE id = ?1)
                     ORDER BY first_at DESC, rowid DESC LIMIT ?2)",
                params![id, TRIGGER_FIRES_KEPT as i64],
            )?;
        }
        tx.commit()?;
        Ok(changed > 0)
    }

    /// Replace what an open fire leaves for `{{ event.* }}`: what was learnt when it was decided.
    pub fn set_trigger_fire_facts(&self, id: &str, facts: &str) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE trigger_fires SET facts = ?
             WHERE id = ? AND outcome IN ('gathering', 'waiting')",
            params![facts, id],
        )? > 0)
    }

    /// Count `more` events onto a decided fire: a capped line takes the events that came after
    /// it, instead of a line for each.
    pub fn add_to_trigger_fire(&self, id: &str, more: i64) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE trigger_fires SET events_count = events_count + ? WHERE id = ?",
            params![more, id],
        )? > 0)
    }

    /// Let a gathering fire go without a line: what it gathered turned out not to count, as a
    /// settled agent that went back to work before its quiet time was up.
    pub fn drop_trigger_fire(&self, id: &str) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "DELETE FROM trigger_fires WHERE id = ? AND outcome = 'gathering'",
            [id],
        )? > 0)
    }

    /// Fires still gathering for this subject on events of this kind: what an agent going back
    /// to work cancels.
    pub fn gathering_trigger_fires(
        &self,
        subject: &str,
        event: &str,
    ) -> StoreResult<Vec<TriggerFireRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {FIRE_COLUMNS} FROM trigger_fires
             WHERE subject = ? AND event = ? AND outcome = 'gathering'"
        ))?;
        let rows = stmt.query_map(params![subject, event], fire_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// How many runs this trigger started for this subject that count against its cap.
    pub fn counted_trigger_runs(&self, trigger_id: &str, subject: &str) -> StoreResult<i64> {
        Ok(self.conn().query_row(
            "SELECT count(*) FROM trigger_fires
             WHERE trigger_id = ? AND subject = ? AND counted = 1",
            params![trigger_id, subject],
            |row| row.get(0),
        )?)
    }

    /// How many runs this trigger started since `since`, whatever their subject.
    pub fn trigger_runs_since(&self, trigger_id: &str, since: i64) -> StoreResult<i64> {
        Ok(self.conn().query_row(
            "SELECT count(*) FROM trigger_fires
             WHERE trigger_id = ? AND outcome = 'started' AND decided_at >= ?",
            params![trigger_id, since],
            |row| row.get(0),
        )?)
    }

    /// The newest decided fire of this trigger for this subject.
    pub fn last_trigger_fire(
        &self,
        trigger_id: &str,
        subject: &str,
    ) -> StoreResult<Option<TriggerFireRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!(
                    "SELECT {FIRE_COLUMNS} FROM trigger_fires
                     WHERE trigger_id = ? AND subject = ?
                       AND outcome NOT IN ('gathering', 'waiting')
                     ORDER BY decided_at DESC, rowid DESC LIMIT 1"
                ),
                params![trigger_id, subject],
                fire_from_row,
            )
            .optional()?)
    }

    /// Lift a trigger's cap: the runs it started stop counting against it. How many did.
    pub fn reset_trigger(&self, trigger_id: &str) -> StoreResult<usize> {
        Ok(self.conn().execute(
            "UPDATE trigger_fires SET counted = 0 WHERE trigger_id = ? AND counted = 1",
            [trigger_id],
        )?)
    }

    /// The fire that started a run, by the run's id.
    pub fn trigger_fire_of_run(&self, run_id: &str) -> StoreResult<Option<TriggerFireRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {FIRE_COLUMNS} FROM trigger_fires WHERE run_id = ? LIMIT 1"),
                [run_id],
                fire_from_row,
            )
            .optional()?)
    }

    /// A trigger's fires, newest first.
    pub fn trigger_fires(
        &self,
        trigger_id: &str,
        limit: usize,
    ) -> StoreResult<Vec<TriggerFireRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {FIRE_COLUMNS} FROM trigger_fires WHERE trigger_id = ?
             ORDER BY first_at DESC, rowid DESC LIMIT ?"
        ))?;
        let rows = stmt.query_map(params![trigger_id, limit as i64], fire_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The newest fire of every trigger that has one.
    pub fn latest_trigger_fires(&self) -> StoreResult<Vec<TriggerFireRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {FIRE_COLUMNS} FROM trigger_fires AS fire
             WHERE rowid = (SELECT rowid FROM trigger_fires
                            WHERE trigger_id = fire.trigger_id
                            ORDER BY first_at DESC, rowid DESC LIMIT 1)"
        ))?;
        let rows = stmt.query_map([], fire_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Every run a trigger started, newest first, as `(run, trigger, trigger's name)`: what run
    /// lists mark.
    pub fn triggered_runs(&self) -> StoreResult<Vec<(String, String, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT fire.run_id, trigger.id, trigger.name
             FROM trigger_fires AS fire JOIN triggers AS trigger ON trigger.id = fire.trigger_id
             WHERE fire.run_id IS NOT NULL ORDER BY fire.first_at DESC, fire.rowid DESC",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Fires that started a run whose ending has not been copied here yet.
    pub fn unsettled_trigger_fires(&self) -> StoreResult<Vec<TriggerFireRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {FIRE_COLUMNS} FROM trigger_fires
             WHERE outcome = 'started' AND result IS NULL ORDER BY first_at, rowid"
        ))?;
        let rows = stmt.query_map([], fire_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Copy the run's ending onto the fire that started it. Once.
    pub fn settle_trigger_fire(&self, id: &str, result: &str, notified: bool) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE trigger_fires SET result = ?, notified_at = ?
             WHERE id = ? AND result IS NULL",
            params![result, notified.then(now_ms), id],
        )? > 0)
    }

    /// Note that the user was told this fire's run waits for approval. Once.
    pub fn mark_trigger_fire_told_waiting(&self, id: &str) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "UPDATE trigger_fires SET told_waiting = 1 WHERE id = ? AND told_waiting = 0",
            [id],
        )? > 0)
    }

    // --- the done signal ------------------------------------------------------------------

    /// Record that the work in a workspace is done, as an agent said with `ys workspace done`.
    pub fn add_done_signal(
        &self,
        workspace_id: &str,
        session_id: Option<&str>,
        head: Option<&str>,
        note: Option<&str>,
    ) -> StoreResult<DoneSignalRow> {
        let id = new_id();
        self.conn().execute(
            "INSERT INTO done_signals (id, workspace_id, session_id, head, note, at)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![id, workspace_id, session_id, head, note, now_ms()],
        )?;
        Ok(self.conn().query_row(
            &format!("SELECT {DONE_COLUMNS} FROM done_signals WHERE id = ?"),
            [&id],
            done_from_row,
        )?)
    }

    /// Done signals after `seq`, oldest first.
    pub fn done_signals_after(&self, seq: i64, limit: usize) -> StoreResult<Vec<DoneSignalRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {DONE_COLUMNS} FROM done_signals WHERE seq > ? ORDER BY seq LIMIT ?"
        ))?;
        let rows = stmt.query_map(params![seq, limit as i64], done_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The newest done signal's sequence number, or 0.
    pub fn newest_done_signal(&self) -> StoreResult<i64> {
        Ok(self.conn().query_row(
            "SELECT COALESCE(max(seq), 0) FROM done_signals",
            [],
            |row| row.get(0),
        )?)
    }

    // --- custom events -------------------------------------------------------------------

    /// Record a custom event, as `ys event emit` raised it.
    pub fn add_custom_event(&self, event: &NewCustomEvent<'_>) -> StoreResult<CustomEventRow> {
        let id = new_id();
        self.conn().execute(
            "INSERT INTO custom_events (id, workspace_id, session_id, run_id, name, head, note, at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                id,
                event.workspace_id,
                event.session_id,
                event.run_id,
                event.name,
                event.head,
                event.note,
                now_ms()
            ],
        )?;
        Ok(self.conn().query_row(
            &format!("SELECT {CUSTOM_COLUMNS} FROM custom_events WHERE id = ?"),
            [&id],
            custom_from_row,
        )?)
    }

    /// Custom events after `seq`, oldest first.
    pub fn custom_events_after(&self, seq: i64, limit: usize) -> StoreResult<Vec<CustomEventRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {CUSTOM_COLUMNS} FROM custom_events WHERE seq > ? ORDER BY seq LIMIT ?"
        ))?;
        let rows = stmt.query_map(params![seq, limit as i64], custom_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The newest custom event's sequence number, or 0.
    pub fn newest_custom_event(&self) -> StoreResult<i64> {
        Ok(self.conn().query_row(
            "SELECT COALESCE(max(seq), 0) FROM custom_events",
            [],
            |row| row.get(0),
        )?)
    }

    // --- what the trigger thread follows --------------------------------------------------

    /// Where the trigger thread has read up to in `source`; `None` before it ever has.
    pub fn trigger_cursor(&self, source: &str) -> StoreResult<Option<i64>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT position FROM trigger_cursors WHERE source = ?",
                [source],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn set_trigger_cursor(&self, source: &str, position: i64) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO trigger_cursors (source, position) VALUES (?1, ?2)
             ON CONFLICT (source) DO UPDATE SET position = ?2",
            params![source, position],
        )?;
        Ok(())
    }

    /// Events of these kinds after `seq`, oldest first.
    pub fn events_after(
        &self,
        seq: i64,
        kinds: &[&str],
        limit: usize,
    ) -> StoreResult<Vec<super::EventRow>> {
        if kinds.is_empty() {
            return Ok(Vec::new());
        }
        let marks = vec!["?"; kinds.len()].join(", ");
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM agent_events WHERE seq > ? AND kind IN ({marks})
             ORDER BY seq LIMIT ?",
            super::EVENT_COLUMNS
        ))?;
        let mut values: Vec<rusqlite::types::Value> = vec![seq.into()];
        values.extend(kinds.iter().map(|kind| (*kind).to_owned().into()));
        values.push((limit as i64).into());
        let rows = stmt.query_map(rusqlite::params_from_iter(values), super::event_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Workflow and test runs that ended after `after` and no later than `until`, oldest
    /// first. Only runs with a workspace.
    pub fn runs_ended_between(&self, after: i64, until: i64) -> StoreResult<Vec<EndedRun>> {
        let conn = self.conn();
        let mut ended = Vec::new();
        for (kind, sql) in [
            (
                "workflow",
                "SELECT id, workflow_id, workspace_id, status, ended_at FROM workflow_runs
                 WHERE ended_at > ? AND ended_at <= ? AND workspace_id IS NOT NULL",
            ),
            (
                "test",
                "SELECT id, test_id, workspace_id, status, ended_at FROM test_runs
                 WHERE ended_at > ? AND ended_at <= ? AND workspace_id IS NOT NULL",
            ),
        ] {
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map(params![after, until], |row| {
                Ok(EndedRun {
                    kind,
                    id: row.get(0)?,
                    target_id: row.get(1)?,
                    workspace_id: row.get(2)?,
                    status: row.get(3)?,
                    ended_at: row.get(4)?,
                })
            })?;
            for row in rows {
                ended.push(row?);
            }
        }
        ended.sort_by(|a, b| a.ended_at.cmp(&b.ended_at).then_with(|| a.id.cmp(&b.id)));
        Ok(ended)
    }

    /// Whether a workflow run started this agent: a `start_session` step left its terminal's id
    /// as `session`, or its agent run's id as `run`. Nothing on the session itself says so.
    pub fn started_by_workflow(
        &self,
        terminal: Option<&str>,
        agent_run: Option<&str>,
    ) -> StoreResult<bool> {
        if terminal.is_none() && agent_run.is_none() {
            return Ok(false);
        }
        Ok(self.conn().query_row(
            "SELECT EXISTS (
               SELECT 1 FROM workflow_step_runs
               WHERE action = 'start_session'
                 AND (json_extract(outputs, '$.session') = ?1
                      OR json_extract(outputs, '$.run') = ?2))",
            params![terminal, agent_run],
            |row| row.get(0),
        )?)
    }

    /// The workflow run whose `run_test` step queued this test run: its step's one output,
    /// `run`, is the test run's id.
    pub fn workflow_run_of_test(&self, test_run_id: &str) -> StoreResult<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT run_id FROM workflow_step_runs
                 WHERE action = 'run_test' AND json_extract(outputs, '$.run') = ? LIMIT 1",
                [test_run_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Whether a run of this workflow is active in this workspace, whoever started it: the run
    /// that would make the door refuse a second.
    pub fn workflow_active_in(&self, workflow_id: &str, workspace_id: &str) -> StoreResult<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS (SELECT 1 FROM workflow_runs
                            WHERE workflow_id = ? AND workspace_id = ?
                              AND status IN ('queued', 'running'))",
            params![workflow_id, workspace_id],
            |row| row.get(0),
        )?)
    }

    /// Whether a run of this test is active in this workspace, a run waiting for approval
    /// included.
    pub fn test_active_in(&self, test_id: &str, workspace_id: &str) -> StoreResult<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS (SELECT 1 FROM test_runs
                            WHERE test_id = ? AND workspace_id = ?
                              AND status IN ('awaiting_approval', 'queued', 'running'))",
            params![test_id, workspace_id],
            |row| row.get(0),
        )?)
    }
}
