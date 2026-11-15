//! This installation's side of sync: which local records it syncs and as what, the ops waiting to
//! go up, and how far it has pulled. See `migrations/0043_sync.sql` and `crate::sync`, which owns
//! the rules; the loop that talks to the server is `remote_host::sync`.
//!
//! **Notes** go up from the memory methods' own transactions (`queue_note`), so a crash cannot
//! keep the change and lose the op. Each op is made against the revision the server will have
//! once the ops ahead of it are applied. When one meets a newer revision instead, what goes up
//! next is made from the note as it is now: a revoke there wins; a settle someone else made first
//! stands; and two texts both stay, the server's standing and this host's beside it as the
//! note's conflict, on every host, until the person settles it.

use remote_proto::sync::{
    Action, Cursor, OpId, OpResult, RecordKind, Revision, SyncId, SyncOp, SyncRecord, SYNC_SCHEMA,
};
use rusqlite::{params, OptionalExtension, Transaction};

use super::{memory_from_row, new_id, MemoryRow, Store, StoreError, StoreResult, MEMORY_COLUMNS};
use crate::sync::{NoteBody, NoteConflict, NoteHistory, WorkspaceBody, WorkspaceTask};

/// A local record this installation syncs, and its logical id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncIdRow {
    pub kind: RecordKind,
    pub local_id: String,
    pub sync_id: SyncId,
    /// The logical id of its project; a project's own.
    pub project: SyncId,
    /// The server's revision, as last heard; `None` until the server has it.
    pub revision: Option<Revision>,
    /// Linked here and not yet read from a snapshot.
    pub fresh: bool,
}

/// An op the server refused, held for the person to see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusedOp {
    pub op: OpId,
    pub action: Action,
    pub kind: RecordKind,
    pub project: SyncId,
    pub why: String,
}

/// A workspace of another host, as its record says it. Nothing of it is on this disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncedWorkspaceRow {
    /// Its logical id.
    pub id: String,
    /// The host it lives on, by name.
    pub host: String,
    pub name: String,
    pub branch: Option<String>,
    pub base_branch: Option<String>,
    pub archived: bool,
    pub tasks: Vec<WorkspaceTask>,
}

/// How far this host has pulled, and how its last pass went.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncStateRow {
    pub cursor: Cursor,
    pub synced_at: Option<i64>,
    pub problem: Option<String>,
}

pub(crate) fn kind_spelled(kind: RecordKind) -> &'static str {
    match kind {
        RecordKind::Project => "project",
        RecordKind::Note => "note",
        RecordKind::Workspace => "workspace",
        RecordKind::Unknown => "unknown",
    }
}

fn kind_of(text: &str) -> RecordKind {
    match text {
        "project" => RecordKind::Project,
        "note" => RecordKind::Note,
        "workspace" => RecordKind::Workspace,
        _ => RecordKind::Unknown,
    }
}

fn action_spelled(action: Action) -> &'static str {
    match action {
        Action::Put => "put",
        Action::Revoke => "revoke",
        Action::Delete => "delete",
        Action::Unknown => "unknown",
    }
}

fn action_of(text: &str) -> Action {
    match text {
        "put" => Action::Put,
        "revoke" => Action::Revoke,
        "delete" => Action::Delete,
        _ => Action::Unknown,
    }
}

/// A logical id read back. Only this installation and the server write them.
fn sync_id(text: String) -> rusqlite::Result<SyncId> {
    SyncId::new(text).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

fn op_id(text: String) -> rusqlite::Result<OpId> {
    OpId::new(text).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

const ID_COLUMNS: &str = "kind, local_id, sync_id, project, revision, fresh";

fn id_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SyncIdRow> {
    Ok(SyncIdRow {
        kind: kind_of(&row.get::<_, String>(0)?),
        local_id: row.get(1)?,
        sync_id: sync_id(row.get(2)?)?,
        project: sync_id(row.get(3)?)?,
        revision: row.get::<_, Option<i64>>(4)?.map(|r| r as Revision),
        fresh: row.get(5)?,
    })
}

const OP_COLUMNS: &str = "op, action, kind, sync_id, project, base, schema, body";

fn op_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SyncOp> {
    let body: Option<String> = row.get(7)?;
    Ok(SyncOp {
        op: op_id(row.get(0)?)?,
        action: action_of(&row.get::<_, String>(1)?),
        kind: kind_of(&row.get::<_, String>(2)?),
        id: sync_id(row.get(3)?)?,
        project: sync_id(row.get(4)?)?,
        base: row.get::<_, Option<i64>>(5)?.map(|r| r as Revision),
        schema: row.get(6)?,
        body: body.and_then(|text| serde_json::from_str(&text).ok()),
    })
}

/// Queue `op` behind every op already waiting.
fn queue(tx: &Transaction<'_>, op: &SyncOp, now: i64) -> StoreResult<()> {
    tx.execute(
        "INSERT INTO sync_outbox
             (op, action, kind, sync_id, project, base, schema, body, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            op.op.as_str(),
            action_spelled(op.action),
            kind_spelled(op.kind),
            op.id.as_str(),
            op.project.as_str(),
            op.base.map(|b| b as i64),
            op.schema,
            op.body.as_ref().map(|b| b.to_string()),
            now
        ],
    )?;
    Ok(())
}

/// Drop everything this installation keeps for syncing one project: its records' ids and the
/// ops waiting for it. The records themselves stay, as they were.
fn forget_project(tx: &Transaction<'_>, project: &str) -> StoreResult<()> {
    tx.execute("DELETE FROM sync_ids WHERE project = ?", [project])?;
    tx.execute("DELETE FROM sync_outbox WHERE project = ?", [project])?;
    tx.execute("DELETE FROM synced_workspaces WHERE project = ?", [project])?;
    Ok(())
}

/// Clear every sync table: Remote is off, or on as another host.
pub(super) fn forget_sync(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(
        "DELETE FROM sync_ids; DELETE FROM sync_outbox; DELETE FROM sync_state;
         DELETE FROM synced_workspaces;",
    )
}

impl Store {
    /// A local project's logical id, when it syncs here.
    pub fn synced_project(&self, project_id: &str) -> StoreResult<Option<SyncIdRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!(
                    "SELECT {ID_COLUMNS} FROM sync_ids WHERE kind = 'project' AND local_id = ?"
                ),
                [project_id],
                id_row,
            )
            .optional()?)
    }

    /// Every project that syncs here, removed or not.
    pub fn synced_projects(&self) -> StoreResult<Vec<SyncIdRow>> {
        let conn = self.conn();
        let mut statement = conn.prepare(&format!(
            "SELECT {ID_COLUMNS} FROM sync_ids WHERE kind = 'project' ORDER BY local_id"
        ))?;
        let rows = statement.query_map([], id_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Turn sync on for a project: give it a logical id and queue its record, in one step.
    /// `body` is what goes up (`crate::sync::project_body`).
    pub fn turn_on_sync(
        &self,
        project_id: &str,
        body: serde_json::Value,
        now: i64,
    ) -> StoreResult<SyncIdRow> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let present: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM projects WHERE id = ? AND removed = 0)",
            [project_id],
            |r| r.get(0),
        )?;
        if !present {
            return Err(StoreError::Refused("There is no such project.".into()));
        }
        let already: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM sync_ids WHERE kind = 'project' AND local_id = ?)",
            [project_id],
            |r| r.get(0),
        )?;
        if already {
            return Err(StoreError::Refused("That project already syncs.".into()));
        }
        let id = SyncId::new(new_id()).expect("a v4 uuid");
        tx.execute(
            "INSERT INTO sync_ids (kind, local_id, sync_id, project) VALUES ('project', ?, ?, ?)",
            params![project_id, id.as_str(), id.as_str()],
        )?;
        let op = SyncOp {
            op: OpId::new(new_id()).expect("a v4 uuid"),
            action: Action::Put,
            kind: RecordKind::Project,
            id: id.clone(),
            project: id.clone(),
            base: None,
            schema: Some(SYNC_SCHEMA),
            body: Some(body),
        };
        queue(&tx, &op, now)?;
        queue_unsynced_notes(&tx, project_id, now)?;
        queue_unsynced_workspaces(&tx, project_id, now)?;
        tx.commit()?;
        Ok(SyncIdRow {
            kind: RecordKind::Project,
            local_id: project_id.to_owned(),
            sync_id: id.clone(),
            project: id,
            revision: None,
            fresh: false,
        })
    }

    /// Link a local project to a synced one the server has already linked this host to. Its
    /// records are read from a snapshot on the next pass.
    pub fn link_synced_project(&self, project_id: &str, record: &SyncRecord) -> StoreResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let present: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM projects WHERE id = ? AND removed = 0)",
            [project_id],
            |r| r.get(0),
        )?;
        if !present {
            return Err(StoreError::Refused("There is no such project.".into()));
        }
        let taken: bool = tx.query_row(
            "SELECT EXISTS (
                 SELECT 1 FROM sync_ids
                 WHERE (kind = 'project' AND local_id = ?) OR sync_id = ?
             )",
            params![project_id, record.id.as_str()],
            |r| r.get(0),
        )?;
        if taken {
            return Err(StoreError::Refused(
                "That project, or the synced one, is linked here already.".into(),
            ));
        }
        tx.execute(
            "INSERT INTO sync_ids (kind, local_id, sync_id, project, revision, fresh)
             VALUES ('project', ?, ?, ?, ?, 1)",
            params![
                project_id,
                record.id.as_str(),
                record.id.as_str(),
                record.revision as i64
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Stop syncing a project here. True when it was syncing.
    pub fn forget_synced_project(&self, project_id: &str) -> StoreResult<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let Some(project) = tx
            .query_row(
                "SELECT sync_id FROM sync_ids WHERE kind = 'project' AND local_id = ?",
                [project_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        else {
            return Ok(false);
        };
        forget_project(&tx, &project)?;
        tx.commit()?;
        Ok(true)
    }

    /// Queue the deletion of a project's records from the server, for every host. It stops
    /// syncing here once the server has done it.
    pub fn queue_sync_removal(&self, project_id: &str, now: i64) -> StoreResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let row = tx
            .query_row(
                &format!(
                    "SELECT {ID_COLUMNS} FROM sync_ids WHERE kind = 'project' AND local_id = ?"
                ),
                [project_id],
                id_row,
            )
            .optional()?
            .ok_or_else(|| StoreError::Refused("That project does not sync.".into()))?;
        let Some(base) = row.revision else {
            return Err(StoreError::Refused(
                "It has not reached the server yet. Try again once it has synced.".into(),
            ));
        };
        let waiting: bool = tx.query_row(
            "SELECT EXISTS (
                 SELECT 1 FROM sync_outbox
                 WHERE project = ? AND kind = 'project' AND action = 'delete'
             )",
            [row.project.as_str()],
            |r| r.get(0),
        )?;
        if !waiting {
            let op = SyncOp {
                op: OpId::new(new_id()).expect("a v4 uuid"),
                action: Action::Delete,
                kind: RecordKind::Project,
                id: row.sync_id.clone(),
                project: row.project.clone(),
                base: Some(base),
                schema: None,
                body: None,
            };
            queue(&tx, &op, now)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Up to `limit` ops waiting to go up, oldest first. Refused ones are held back.
    pub fn sync_outbox(&self, limit: usize) -> StoreResult<Vec<SyncOp>> {
        let conn = self.conn();
        let mut statement = conn.prepare(&format!(
            "SELECT {OP_COLUMNS} FROM sync_outbox WHERE refused IS NULL ORDER BY seq LIMIT ?"
        ))?;
        let rows = statement.query_map([limit as i64], op_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Up to `limit` ops to push now, oldest first: the oldest waiting for each record, and none
    /// after it. The next op on a record is made against the revision the one before it makes, so
    /// it goes only once that one is answered; sent together, a later one could apply against a
    /// revision another host made, a revoke included, after the one before it was refused.
    pub fn sync_batch(&self, limit: usize) -> StoreResult<Vec<SyncOp>> {
        let conn = self.conn();
        let mut statement = conn.prepare(&format!(
            "SELECT {OP_COLUMNS} FROM sync_outbox o
             WHERE refused IS NULL
               AND seq = (SELECT MIN(seq) FROM sync_outbox
                          WHERE sync_id = o.sync_id AND refused IS NULL)
             ORDER BY seq LIMIT ?"
        ))?;
        let rows = statement.query_map([limit as i64], op_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Ops waiting to go up, refused ones included.
    pub fn sync_waiting(&self) -> StoreResult<usize> {
        Ok(self
            .conn()
            .query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| {
                r.get::<_, i64>(0)
            })? as usize)
    }

    /// The ops the server refused, oldest first.
    pub fn sync_refused(&self) -> StoreResult<Vec<RefusedOp>> {
        let conn = self.conn();
        let mut statement = conn.prepare(
            "SELECT op, action, kind, project, refused FROM sync_outbox
             WHERE refused IS NOT NULL ORDER BY seq",
        )?;
        let rows = statement.query_map([], |r| {
            Ok(RefusedOp {
                op: op_id(r.get(0)?)?,
                action: action_of(&r.get::<_, String>(1)?),
                kind: kind_of(&r.get::<_, String>(2)?),
                project: sync_id(r.get(3)?)?,
                why: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Settle the outbox with the server's answers to a push, in one step. True when anything a
    /// person sees changed: a project stopped syncing, or an op was refused.
    pub fn settle_sync(&self, results: &[OpResult], now: i64) -> StoreResult<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut changed = false;
        for result in results {
            let op = match result {
                OpResult::Applied { op, .. }
                | OpResult::Conflict { op, .. }
                | OpResult::Refused { op, .. } => op,
                // An answer this version cannot read: the op stays queued for a later one.
                OpResult::Unknown => continue,
            };
            let Some(queued) = tx
                .query_row(
                    &format!("SELECT {OP_COLUMNS} FROM sync_outbox WHERE op = ?"),
                    [op.as_str()],
                    op_row,
                )
                .optional()?
            else {
                continue;
            };
            changed |= match queued.kind {
                RecordKind::Note => settle_note(&tx, &queued, result, now)?,
                RecordKind::Workspace => settle_workspace(&tx, &queued, result, now)?,
                _ => settle_one(&tx, &queued, result)?,
            };
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Take in records the server sent, and the cursor they bring this host to, in one step.
    /// Nothing is queued for what comes in, so nothing goes back up. True when anything a
    /// person sees changed.
    pub fn take_sync_records(
        &self,
        records: &[SyncRecord],
        cursor: Option<Cursor>,
    ) -> StoreResult<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut changed = false;
        for record in records {
            changed |= take_one(&tx, record)?;
        }
        if let Some(cursor) = cursor {
            set_cursor(&tx, cursor)?;
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Take in a snapshot of the projects named, or of every linked project when `projects` is
    /// `None`, with the head it was read at. A project of those missing from it is gone from the
    /// server, and stops syncing here. A full snapshot also moves the cursor to its head.
    pub fn take_sync_snapshot(
        &self,
        projects: Option<&[SyncId]>,
        records: &[SyncRecord],
        head: Cursor,
        now: i64,
    ) -> StoreResult<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut changed = false;
        for record in records {
            changed |= take_one(&tx, record)?;
        }
        let covered: Vec<String> = match projects {
            Some(projects) => projects.iter().map(|p| p.as_str().to_owned()).collect(),
            None => {
                let mut statement =
                    tx.prepare("SELECT sync_id FROM sync_ids WHERE kind = 'project'")?;
                let rows = statement.query_map([], |r| r.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<_>>()?
            }
        };
        for project in &covered {
            let seen = records
                .iter()
                .any(|r| r.kind == RecordKind::Project && r.id.as_str() == project);
            // Not yet on the server: its create is still waiting to go up.
            let waiting: bool = tx
                .query_row(
                    "SELECT revision IS NULL FROM sync_ids WHERE kind = 'project' AND sync_id = ?",
                    [project],
                    |r| r.get::<_, bool>(0),
                )
                .optional()?
                .unwrap_or(true);
            if !seen && !waiting {
                forget_project(&tx, project)?;
                changed = true;
            } else {
                // Another host's workspace the snapshot does not have is gone, its tombstone
                // pruned while this host was away.
                let kept: Vec<&str> = records
                    .iter()
                    .filter(|r| r.kind == RecordKind::Workspace && r.project.as_str() == project)
                    .map(|r| r.id.as_str())
                    .collect();
                changed |= tx.execute(
                    "DELETE FROM synced_workspaces
                     WHERE project = ? AND sync_id NOT IN (SELECT value FROM json_each(?))",
                    params![project, serde_json::to_string(&kept).expect("ids are JSON")],
                )? > 0;
                let linked: Option<(String, bool)> = tx
                    .query_row(
                        "SELECT local_id, fresh FROM sync_ids WHERE kind = 'project' AND sync_id = ?",
                        [project],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                tx.execute(
                    "UPDATE sync_ids SET fresh = 0 WHERE kind = 'project' AND sync_id = ?",
                    [project],
                )?;
                // Just linked: this host's own notes for it go up, those the server had already
                // taken as the same note above.
                if let Some((local, true)) = linked {
                    queue_unsynced_notes(&tx, &local, now)?;
                    queue_unsynced_workspaces(&tx, &local, now)?;
                }
            }
        }
        if projects.is_none() {
            set_cursor(&tx, head)?;
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Queue what an earlier version left unsent for the projects that sync here: approved notes
    /// from before notes synced, workspaces from before workspaces did. Each pass asks; a record
    /// already queued or sent is not queued again, so a pass after the first queues nothing. A
    /// project just linked waits for its snapshot, which queues its own.
    pub fn backfill_sync(&self, now: i64) -> StoreResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let projects: Vec<String> = {
            let mut statement =
                tx.prepare("SELECT local_id FROM sync_ids WHERE kind = 'project' AND fresh = 0")?;
            let rows = statement.query_map([], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for project in projects {
            queue_unsynced_notes(&tx, &project, now)?;
            queue_unsynced_workspaces(&tx, &project, now)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The projects linked here and not yet read from a snapshot.
    pub fn fresh_synced_projects(&self) -> StoreResult<Vec<SyncId>> {
        let conn = self.conn();
        let mut statement = conn.prepare(
            "SELECT sync_id FROM sync_ids WHERE kind = 'project' AND fresh = 1 ORDER BY sync_id",
        )?;
        let rows = statement.query_map([], |r| sync_id(r.get(0)?))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The other hosts' workspaces of a local project that syncs here, by host then name.
    pub fn synced_workspaces(&self, project_id: &str) -> StoreResult<Vec<SyncedWorkspaceRow>> {
        let conn = self.conn();
        let mut statement = conn.prepare(
            "SELECT w.sync_id, w.host, w.name, w.branch, w.base_branch, w.archived, w.tasks
             FROM synced_workspaces w
             JOIN sync_ids p ON p.kind = 'project' AND p.sync_id = w.project
             WHERE p.local_id = ?
             ORDER BY w.host, w.name, w.sync_id",
        )?;
        let rows = statement.query_map([project_id], |r| {
            let tasks: String = r.get(6)?;
            Ok(SyncedWorkspaceRow {
                id: r.get(0)?,
                host: r.get(1)?,
                name: r.get(2)?,
                branch: r.get(3)?,
                base_branch: r.get(4)?,
                archived: r.get(5)?,
                tasks: serde_json::from_str(&tasks).unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn sync_state(&self) -> StoreResult<SyncStateRow> {
        Ok(self
            .conn()
            .query_row(
                "SELECT cursor, synced_at, problem FROM sync_state",
                [],
                |r| {
                    Ok(SyncStateRow {
                        cursor: r.get::<_, i64>(0)? as Cursor,
                        synced_at: r.get(1)?,
                        problem: r.get(2)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    /// How a pass ended: at `now` without a problem, or with one, keeping the last good time.
    pub fn note_sync_pass(&self, problem: Option<&str>, now: i64) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO sync_state (singleton, synced_at, problem)
             VALUES (1, CASE WHEN ?1 IS NULL THEN ?2 END, ?1)
             ON CONFLICT(singleton) DO UPDATE SET
                 synced_at = CASE WHEN ?1 IS NULL THEN ?2 ELSE synced_at END,
                 problem = ?1",
            params![problem, now],
        )?;
        Ok(())
    }
}

fn set_cursor(tx: &Transaction<'_>, cursor: Cursor) -> StoreResult<()> {
    tx.execute(
        "INSERT INTO sync_state (singleton, cursor) VALUES (1, ?)
         ON CONFLICT(singleton) DO UPDATE SET cursor = excluded.cursor",
        [cursor as i64],
    )?;
    Ok(())
}

/// Note the server's revision of a record. True when the server had not been heard to have it
/// before, which a person sees: the project has reached the server.
fn set_revision(tx: &Transaction<'_>, id: &SyncId, revision: Revision) -> StoreResult<bool> {
    let reached = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM sync_ids WHERE sync_id = ? AND revision IS NULL)",
        [id.as_str()],
        |r| r.get::<_, bool>(0),
    )?;
    tx.execute(
        "UPDATE sync_ids SET revision = MAX(COALESCE(revision, 0), ?) WHERE sync_id = ?",
        params![revision as i64, id.as_str()],
    )?;
    Ok(reached)
}

fn settle_one(tx: &Transaction<'_>, op: &SyncOp, result: &OpResult) -> StoreResult<bool> {
    let removal = op.kind == RecordKind::Project && op.action == Action::Delete;
    let done = |tx: &Transaction<'_>| -> StoreResult<()> {
        tx.execute("DELETE FROM sync_outbox WHERE op = ?", [op.op.as_str()])?;
        Ok(())
    };
    match result {
        OpResult::Applied { revision, .. } if removal => {
            let _ = revision;
            forget_project(tx, op.project.as_str())?;
            Ok(true)
        }
        OpResult::Applied { revision, .. } => {
            let reached = set_revision(tx, &op.id, *revision)?;
            done(tx)?;
            Ok(reached)
        }
        // Removed already, by another host.
        OpResult::Conflict { current, .. }
            if current.kind == RecordKind::Project && current.deleted =>
        {
            forget_project(tx, op.project.as_str())?;
            Ok(true)
        }
        // Changed since: the person's removal stands, against the revision as it is now.
        OpResult::Conflict { current, .. } if removal => {
            let _ = set_revision(tx, &op.id, current.revision)?;
            tx.execute(
                "UPDATE sync_outbox SET base = ? WHERE op = ?",
                params![current.revision as i64, op.op.as_str()],
            )?;
            Ok(false)
        }
        // A project's own record is only ever created here. A conflict on that create is the
        // record there already, as when the answer to an earlier push was lost.
        OpResult::Conflict { current, .. } => {
            let reached = set_revision(tx, &op.id, current.revision)?;
            done(tx)?;
            Ok(reached)
        }
        OpResult::Refused { error, .. } => {
            tx.execute(
                "UPDATE sync_outbox SET refused = ? WHERE op = ?",
                params![error.message, op.op.as_str()],
            )?;
            Ok(true)
        }
        OpResult::Unknown => Ok(false),
    }
}

/// One record from the server.
fn take_one(tx: &Transaction<'_>, record: &SyncRecord) -> StoreResult<bool> {
    match record.kind {
        RecordKind::Note => return take_note(tx, record),
        RecordKind::Workspace => return take_workspace(tx, record),
        RecordKind::Project => {}
        RecordKind::Unknown => return Ok(false),
    }
    let known: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM sync_ids WHERE kind = 'project' AND sync_id = ?)",
        [record.id.as_str()],
        |r| r.get(0),
    )?;
    if !known {
        return Ok(false);
    }
    if record.deleted {
        // Removed from the server, by this host or another: it stops syncing here, and the
        // project stays as it is.
        forget_project(tx, record.id.as_str())?;
        return Ok(true);
    }
    set_revision(tx, &record.id, record.revision)
}

// --- notes ------------------------------------------------------------------------------------

/// This host as a note names it: its host id and its name. `None` while Remote is off.
fn this_host(tx: &Transaction<'_>) -> StoreResult<Option<(String, String)>> {
    Ok(tx
        .query_row("SELECT host_id, name FROM remote_host", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?)
}

fn note_row(tx: &Transaction<'_>, entry_id: &str) -> StoreResult<Option<SyncIdRow>> {
    Ok(tx
        .query_row(
            &format!("SELECT {ID_COLUMNS} FROM sync_ids WHERE kind = 'note' AND local_id = ?"),
            [entry_id],
            id_row,
        )
        .optional()?)
}

fn entry(tx: &Transaction<'_>, entry_id: &str) -> StoreResult<Option<MemoryRow>> {
    Ok(tx
        .query_row(
            &format!("SELECT {MEMORY_COLUMNS} FROM memory_entries WHERE id = ?"),
            [entry_id],
            memory_from_row,
        )
        .optional()?)
}

/// Ops for a record still to be answered, refused ones aside.
fn pending(tx: &Transaction<'_>, id: &SyncId) -> StoreResult<u64> {
    Ok(tx.query_row(
        "SELECT COUNT(*) FROM sync_outbox WHERE sync_id = ? AND refused IS NULL",
        [id.as_str()],
        |r| r.get::<_, i64>(0),
    )? as u64)
}

/// The revision the next op on a record is made against: the server's as last heard, and one
/// more for each op ahead of it.
fn next_base(tx: &Transaction<'_>, row: &SyncIdRow) -> StoreResult<Option<Revision>> {
    let base = row.revision.unwrap_or(0) + pending(tx, &row.sync_id)?;
    Ok((base > 0).then_some(base))
}

/// A local note as its record says it, or `None` while Remote is off.
fn note_body(
    tx: &Transaction<'_>,
    entry_id: &str,
    settles: Option<String>,
) -> StoreResult<Option<NoteBody>> {
    let (Some((host_id, host_name)), Some(entry)) = (this_host(tx)?, entry(tx, entry_id)?) else {
        return Ok(None);
    };
    let origin: Option<String> = tx.query_row(
        "SELECT from_host_id FROM memory_entries WHERE id = ?",
        [entry_id],
        |r| r.get(0),
    )?;
    let mut statement = tx.prepare(
        "SELECT at, action, by, previous_text FROM memory_history WHERE entry_id = ? ORDER BY seq",
    )?;
    let history = statement
        .query_map([entry_id], |r| {
            Ok(NoteHistory {
                at: r.get(0)?,
                action: r.get(1)?,
                by: r.get(2)?,
                previous_text: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let (origin, from) = match origin {
        Some(origin) => (origin, entry.from_host.clone().unwrap_or_default()),
        None => (host_id, host_name),
    };
    Ok(Some(NoteBody {
        text: entry.text,
        author: entry.author,
        harness: entry.harness_id,
        workspace: entry.workspace_name,
        origin,
        from,
        created_at: entry.created_at,
        history,
        conflict: entry.conflict.map(|c| NoteConflict {
            id: c.id,
            text: c.text,
            from: c.from,
            at: c.at,
        }),
        settles,
    }))
}

fn queue_put(
    tx: &Transaction<'_>,
    row: &SyncIdRow,
    base: Option<Revision>,
    body: NoteBody,
    now: i64,
) -> StoreResult<()> {
    let op = SyncOp {
        op: OpId::new(new_id()).expect("a v4 uuid"),
        action: Action::Put,
        kind: RecordKind::Note,
        id: row.sync_id.clone(),
        project: row.project.clone(),
        base,
        schema: Some(SYNC_SCHEMA),
        body: Some(body.to_value()),
    };
    queue(tx, &op, now)
}

fn queue_revoke(
    tx: &Transaction<'_>,
    row: &SyncIdRow,
    base: Option<Revision>,
    now: i64,
) -> StoreResult<()> {
    let op = SyncOp {
        op: OpId::new(new_id()).expect("a v4 uuid"),
        action: Action::Revoke,
        kind: RecordKind::Note,
        id: row.sync_id.clone(),
        project: row.project.clone(),
        base,
        schema: None,
        body: None,
    };
    queue(tx, &op, now)
}

/// Queue what a note's change means for the server, in the change's own transaction. Only an
/// approved or revoked note of a project that syncs here goes up: a candidate or a rejected note
/// never does, and a note revoked before it ever went up stays here. `settles` is the conflict
/// the change settled.
pub(super) fn queue_note(
    tx: &Transaction<'_>,
    entry_id: &str,
    settles: Option<String>,
    now: i64,
) -> StoreResult<()> {
    let Some((project_id, state)) = tx
        .query_row(
            "SELECT project_id, state FROM memory_entries WHERE id = ?",
            [entry_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?
    else {
        return Ok(());
    };
    let Some(project) = tx
        .query_row(
            "SELECT sync_id FROM sync_ids WHERE kind = 'project' AND local_id = ?",
            [&project_id],
            |r| sync_id(r.get(0)?),
        )
        .optional()?
    else {
        return Ok(());
    };
    match (state.as_str(), note_row(tx, entry_id)?) {
        ("approved", row) => {
            let Some(body) = note_body(tx, entry_id, settles)? else {
                return Ok(());
            };
            let row = match row {
                Some(row) => row,
                None => {
                    let id = SyncId::new(new_id()).expect("a v4 uuid");
                    tx.execute(
                        "INSERT INTO sync_ids (kind, local_id, sync_id, project)
                         VALUES ('note', ?, ?, ?)",
                        params![entry_id, id.as_str(), project.as_str()],
                    )?;
                    SyncIdRow {
                        kind: RecordKind::Note,
                        local_id: entry_id.to_owned(),
                        sync_id: id,
                        project,
                        revision: None,
                        fresh: false,
                    }
                }
            };
            let base = next_base(tx, &row)?;
            queue_put(tx, &row, base, body, now)
        }
        ("revoked", Some(row)) => {
            let base = next_base(tx, &row)?;
            queue_revoke(tx, &row, base, now)
        }
        _ => Ok(()),
    }
}

/// Queue a project's approved notes that do not sync yet: when it is turned on, and once it is
/// linked and its snapshot is in.
fn queue_unsynced_notes(tx: &Transaction<'_>, project_id: &str, now: i64) -> StoreResult<()> {
    let ids: Vec<String> = {
        let mut statement = tx.prepare(
            "SELECT id FROM memory_entries e
             WHERE project_id = ? AND state = 'approved'
               AND NOT EXISTS (SELECT 1 FROM sync_ids WHERE kind = 'note' AND local_id = e.id)
             ORDER BY created_at, rowid",
        )?;
        let rows = statement.query_map([project_id], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for id in ids {
        queue_note(tx, &id, None, now)?;
    }
    Ok(())
}

/// Make a local note say what its record says: its text, whether it is revoked, where it is
/// from, any conflict and its history. Nothing is queued for it.
fn apply_note(
    tx: &Transaction<'_>,
    entry_id: &str,
    record: &SyncRecord,
    body: &NoteBody,
) -> StoreResult<()> {
    let mine = this_host(tx)?.is_some_and(|(id, _)| id == body.origin);
    let (origin, from) = if mine {
        (None, None)
    } else {
        (Some(body.origin.as_str()), Some(body.from.as_str()))
    };
    // A revoked note has nothing left to settle.
    let conflict = body.conflict.as_ref().filter(|_| !record.revoked);
    tx.execute(
        "UPDATE memory_entries SET text = ?, state = ?, from_host_id = ?, from_host = ?,
             conflict_id = ?, conflict_text = ?, conflict_from = ?, conflict_at = ?, updated_at = ?
         WHERE id = ?",
        params![
            body.text,
            if record.revoked {
                "revoked"
            } else {
                "approved"
            },
            origin,
            from,
            conflict.map(|c| &c.id),
            conflict.map(|c| &c.text),
            conflict.map(|c| &c.from),
            conflict.map(|c| c.at),
            record.changed_at,
            entry_id
        ],
    )?;
    tx.execute("DELETE FROM memory_history WHERE entry_id = ?", [entry_id])?;
    for line in body.history.iter().filter(|line| line.fits_here()) {
        tx.execute(
            "INSERT INTO memory_history (entry_id, at, action, by, previous_text)
             VALUES (?, ?, ?, ?, ?)",
            params![entry_id, line.at, line.action, line.by, line.previous_text],
        )?;
    }
    // A revoke carries no body, so the history it kept stops before it.
    if record.revoked
        && body
            .history
            .last()
            .is_none_or(|line| line.action != "revoked")
    {
        tx.execute(
            "INSERT INTO memory_history (entry_id, at, action, by, previous_text)
             VALUES (?, ?, 'revoked', 'user', NULL)",
            params![entry_id, record.changed_at],
        )?;
    }
    Ok(())
}

/// Keep a text this host had and the record replaced, in the note's history.
fn keep_replaced(tx: &Transaction<'_>, entry_id: &str, text: &str, now: i64) -> StoreResult<()> {
    tx.execute(
        "INSERT INTO memory_history (entry_id, at, action, by, previous_text)
         VALUES (?, ?, 'edited', 'user', ?)",
        params![entry_id, now, text],
    )?;
    Ok(())
}

/// A note from the server. One this host does not have yet becomes a note here, approved or
/// revoked as it is there, or is taken as the same note as one here with the same text that
/// does not sync yet. One with an op of this host's still to be answered is left until it is:
/// the answer says what to do.
fn take_note(tx: &Transaction<'_>, record: &SyncRecord) -> StoreResult<bool> {
    let Some(project_id) = tx
        .query_row(
            "SELECT local_id FROM sync_ids WHERE kind = 'project' AND sync_id = ?",
            [record.project.as_str()],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    else {
        return Ok(false);
    };
    let row = tx
        .query_row(
            &format!("SELECT {ID_COLUMNS} FROM sync_ids WHERE kind = 'note' AND sync_id = ?"),
            [record.id.as_str()],
            id_row,
        )
        .optional()?;
    if record.deleted {
        // Its project is being removed from the server; the note stays here as it is.
        if let Some(row) = row {
            tx.execute(
                "DELETE FROM sync_ids WHERE kind = 'note' AND local_id = ?",
                [&row.local_id],
            )?;
        }
        return Ok(false);
    }
    // A schema from a newer version: left on the server as it is.
    let Some(body) = NoteBody::of(record) else {
        return Ok(false);
    };
    match row {
        Some(row) => {
            if pending(tx, &row.sync_id)? > 0
                || row.revision.is_some_and(|known| known >= record.revision)
            {
                return Ok(false);
            }
            apply_note(tx, &row.local_id, record, &body)?;
            set_revision(tx, &record.id, record.revision)?;
            Ok(true)
        }
        None => {
            let same: Option<String> = tx
                .query_row(
                    "SELECT id FROM memory_entries e
                     WHERE project_id = ? AND text = ? COLLATE NOCASE
                       AND NOT EXISTS (SELECT 1 FROM sync_ids WHERE kind = 'note' AND local_id = e.id)
                     ORDER BY created_at, rowid LIMIT 1",
                    params![project_id, body.text],
                    |r| r.get(0),
                )
                .optional()?;
            let entry_id = match same {
                Some(id) => id,
                None => {
                    let id = new_id();
                    tx.execute(
                        "INSERT INTO memory_entries (id, project_id, text, state, author, harness_id,
                                                     workspace_name, created_at, updated_at)
                         VALUES (?, ?, ?, 'approved', ?, ?, ?, ?, ?)",
                        params![
                            id,
                            project_id,
                            body.text,
                            body.author,
                            body.harness,
                            body.workspace,
                            body.created_at,
                            record.changed_at
                        ],
                    )?;
                    id
                }
            };
            tx.execute(
                "INSERT INTO sync_ids (kind, local_id, sync_id, project, revision)
                 VALUES ('note', ?, ?, ?, ?)",
                params![
                    entry_id,
                    record.id.as_str(),
                    record.project.as_str(),
                    record.revision as i64
                ],
            )?;
            apply_note(tx, &entry_id, record, &body)?;
            Ok(true)
        }
    }
}

/// The server's answer to one of this host's ops on a note.
fn settle_note(
    tx: &Transaction<'_>,
    op: &SyncOp,
    result: &OpResult,
    now: i64,
) -> StoreResult<bool> {
    match result {
        OpResult::Applied { revision, .. } => {
            set_revision(tx, &op.id, *revision)?;
            tx.execute("DELETE FROM sync_outbox WHERE op = ?", [op.op.as_str()])?;
            Ok(false)
        }
        OpResult::Conflict { current, .. } => note_conflict(tx, op, current, now),
        OpResult::Refused { .. } | OpResult::Unknown => settle_one(tx, op, result),
    }
}

/// One of this host's ops on a note met a newer revision. Every op still waiting for the note was
/// made against a revision that is gone, so they are dropped, and what goes up instead is made
/// from the note as it is here now, against the revision as it is there.
fn note_conflict(
    tx: &Transaction<'_>,
    op: &SyncOp,
    current: &SyncRecord,
    now: i64,
) -> StoreResult<bool> {
    let Some(row) = tx
        .query_row(
            &format!("SELECT {ID_COLUMNS} FROM sync_ids WHERE kind = 'note' AND sync_id = ?"),
            [op.id.as_str()],
            id_row,
        )
        .optional()?
    else {
        tx.execute("DELETE FROM sync_outbox WHERE op = ?", [op.op.as_str()])?;
        return Ok(false);
    };
    let dropped: Vec<SyncOp> = {
        let mut statement = tx.prepare(&format!(
            "SELECT {OP_COLUMNS} FROM sync_outbox WHERE sync_id = ? AND refused IS NULL ORDER BY seq"
        ))?;
        let rows = statement.query_map([op.id.as_str()], op_row)?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let bodies: Vec<NoteBody> = dropped
        .iter()
        .filter_map(|op| serde_json::from_value(op.body.clone()?).ok())
        .collect();
    let settles = bodies.iter().rev().find_map(|body| body.settles.clone());
    // The conflicts this host published, its own text beside another's: a body with one that does
    // not settle anything is only ever sent by the host whose edit lost.
    let published: Vec<String> = bodies
        .iter()
        .filter(|body| body.settles.is_none())
        .filter_map(|body| body.conflict.as_ref().map(|c| c.id.clone()))
        .collect();
    tx.execute(
        "DELETE FROM sync_outbox WHERE sync_id = ? AND refused IS NULL",
        [op.id.as_str()],
    )?;
    if current.deleted {
        tx.execute(
            "DELETE FROM sync_ids WHERE kind = 'note' AND local_id = ?",
            [&row.local_id],
        )?;
        return Ok(false);
    }
    tx.execute(
        "UPDATE sync_ids SET revision = ? WHERE kind = 'note' AND local_id = ?",
        params![current.revision as i64, row.local_id],
    )?;
    let row = SyncIdRow {
        revision: Some(current.revision),
        ..row
    };
    let (Some(theirs), Some(local)) = (NoteBody::of(current), entry(tx, &row.local_id)?) else {
        return Ok(false);
    };
    let base = Some(current.revision);

    // Revoked there: the revoke wins over whatever this host did since.
    if current.revoked {
        apply_note(tx, &row.local_id, current, &theirs)?;
        if local.text != theirs.text {
            keep_replaced(tx, &row.local_id, &local.text, now)?;
        }
        return Ok(true);
    }
    // Revoked here since: the revoke goes up against the revision as it is.
    if local.state == "revoked" {
        queue_revoke(tx, &row, base, now)?;
        return Ok(false);
    }
    if let Some(settled) = settles {
        if theirs
            .conflict
            .as_ref()
            .is_none_or(|open| open.id != settled)
        {
            // Settled on another host first: that stands.
            apply_note(tx, &row.local_id, current, &theirs)?;
            if local.text != theirs.text {
                keep_replaced(tx, &row.local_id, &local.text, now)?;
            }
            return Ok(true);
        }
        // Still open there: this settle goes up again.
        if let Some(body) = note_body(tx, &row.local_id, Some(settled))? {
            queue_put(tx, &row, base, body, now)?;
        }
        return Ok(false);
    }
    // This host's text: its own edit, or, while the conflict it published is on its way, the text
    // it set beside another's, which is not the note's own here.
    let mine = match local.conflict.as_ref() {
        Some(open) if published.contains(&open.id) => open.text.clone(),
        _ => local.text.clone(),
    };
    if theirs.text == mine {
        // The same text both ways: nothing to settle.
        apply_note(tx, &row.local_id, current, &theirs)?;
        return Ok(local.conflict.is_some() || theirs.conflict.is_some());
    }
    if theirs.conflict.as_ref().is_some_and(|c| c.text == mine) {
        // Already beside it there.
        apply_note(tx, &row.local_id, current, &theirs)?;
        return Ok(true);
    }
    // Two texts: theirs stands, and this host's goes beside it, here and on every host.
    let Some((_, here)) = this_host(tx)? else {
        return Ok(false);
    };
    apply_note(tx, &row.local_id, current, &theirs)?;
    if let Some(waiting) = theirs.conflict.as_ref() {
        // A third text while one waits: the one waiting is kept in the history.
        keep_replaced(tx, &row.local_id, &waiting.text, now)?;
    }
    if local.text != mine && local.text != theirs.text {
        // The text that stood here before is replaced too: kept in the history.
        keep_replaced(tx, &row.local_id, &local.text, now)?;
    }
    let conflict = NoteConflict {
        id: new_id(),
        text: mine,
        from: here,
        at: now,
    };
    tx.execute(
        "UPDATE memory_entries SET conflict_id = ?, conflict_text = ?, conflict_from = ?,
             conflict_at = ?
         WHERE id = ?",
        params![
            conflict.id,
            conflict.text,
            conflict.from,
            conflict.at,
            row.local_id
        ],
    )?;
    if let Some(body) = note_body(tx, &row.local_id, None)? {
        queue_put(tx, &row, base, body, now)?;
    }
    Ok(true)
}

// --- workspaces -------------------------------------------------------------------------------

fn workspace_row(tx: &Transaction<'_>, workspace_id: &str) -> StoreResult<Option<SyncIdRow>> {
    Ok(tx
        .query_row(
            &format!("SELECT {ID_COLUMNS} FROM sync_ids WHERE kind = 'workspace' AND local_id = ?"),
            [workspace_id],
            id_row,
        )
        .optional()?)
}

/// A local workspace as its record says it, or `None` when it is not one that syncs: gone,
/// forgotten, the project's own checkout, or Remote off.
fn workspace_body(tx: &Transaction<'_>, workspace_id: &str) -> StoreResult<Option<WorkspaceBody>> {
    let Some((origin, host)) = this_host(tx)? else {
        return Ok(None);
    };
    let Some((name, branch, base_branch, archived)) = tx
        .query_row(
            "SELECT name, branch, base_branch, status = 'archived' FROM workspaces
             WHERE id = ? AND kind = 'worktree' AND forgotten = 0",
            [workspace_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let mut statement = tx.prepare(
        "SELECT source, repo, key, url, title FROM workspace_tasks WHERE workspace_id = ?
         ORDER BY created_at, rowid",
    )?;
    let tasks = statement
        .query_map([workspace_id], |r| {
            Ok(WorkspaceTask {
                source: r.get(0)?,
                repo: r.get(1)?,
                key: r.get(2)?,
                url: r.get(3)?,
                title: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(WorkspaceBody {
        name,
        branch,
        base_branch,
        archived,
        tasks,
        origin,
        host,
    }))
}

fn queue_workspace_op(
    tx: &Transaction<'_>,
    row: &SyncIdRow,
    action: Action,
    base: Option<Revision>,
    body: Option<WorkspaceBody>,
    now: i64,
) -> StoreResult<()> {
    let op = SyncOp {
        op: OpId::new(new_id()).expect("a v4 uuid"),
        action,
        kind: RecordKind::Workspace,
        id: row.sync_id.clone(),
        project: row.project.clone(),
        base,
        schema: body.is_some().then_some(SYNC_SCHEMA),
        body: body.map(|b| b.to_value()),
    };
    queue(tx, &op, now)
}

/// Whether a delete of the record waits to go up.
fn deleting(tx: &Transaction<'_>, id: &SyncId) -> StoreResult<bool> {
    Ok(tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM sync_outbox
                        WHERE sync_id = ? AND action = 'delete' AND refused IS NULL)",
        [id.as_str()],
        |r| r.get(0),
    )?)
}

/// Queue what a workspace's change means for the server, in the change's own transaction. Only
/// this host's worktree workspaces of a project that syncs here go up, and only this host writes
/// them. One that is gone or forgotten is deleted there; one that never reached the server is
/// only forgotten here.
pub(super) fn queue_workspace(
    tx: &Transaction<'_>,
    workspace_id: &str,
    now: i64,
) -> StoreResult<()> {
    let row = workspace_row(tx, workspace_id)?;
    let body = workspace_body(tx, workspace_id)?;
    match (row, body) {
        (Some(row), None) => {
            if deleting(tx, &row.sync_id)? {
                return Ok(());
            }
            match next_base(tx, &row)? {
                Some(base) => queue_workspace_op(tx, &row, Action::Delete, Some(base), None, now),
                // Never reached the server: nothing there to delete.
                None => {
                    tx.execute(
                        "DELETE FROM sync_ids WHERE kind = 'workspace' AND local_id = ?",
                        [workspace_id],
                    )?;
                    Ok(())
                }
            }
        }
        (row, Some(body)) => {
            let project_id: String = tx.query_row(
                "SELECT project_id FROM workspaces WHERE id = ?",
                [workspace_id],
                |r| r.get(0),
            )?;
            let Some(project) = tx
                .query_row(
                    "SELECT sync_id FROM sync_ids WHERE kind = 'project' AND local_id = ?",
                    [&project_id],
                    |r| sync_id(r.get(0)?),
                )
                .optional()?
            else {
                return Ok(());
            };
            // Brought back while its delete waits to go up: that record goes, and this is a new one.
            let row = match row {
                Some(row) if !deleting(tx, &row.sync_id)? => Some(row),
                Some(_) => {
                    tx.execute(
                        "DELETE FROM sync_ids WHERE kind = 'workspace' AND local_id = ?",
                        [workspace_id],
                    )?;
                    None
                }
                None => None,
            };
            let row = match row {
                Some(row) => row,
                None => {
                    let id = SyncId::new(new_id()).expect("a v4 uuid");
                    tx.execute(
                        "INSERT INTO sync_ids (kind, local_id, sync_id, project)
                         VALUES ('workspace', ?, ?, ?)",
                        params![workspace_id, id.as_str(), project.as_str()],
                    )?;
                    SyncIdRow {
                        kind: RecordKind::Workspace,
                        local_id: workspace_id.to_owned(),
                        sync_id: id,
                        project,
                        revision: None,
                        fresh: false,
                    }
                }
            };
            let base = next_base(tx, &row)?;
            queue_workspace_op(tx, &row, Action::Put, base, Some(body), now)
        }
        (None, None) => Ok(()),
    }
}

/// Queue a project's worktree workspaces that do not sync yet: when it is turned on, and once it
/// is linked and its snapshot is in.
fn queue_unsynced_workspaces(tx: &Transaction<'_>, project_id: &str, now: i64) -> StoreResult<()> {
    let ids: Vec<String> = {
        let mut statement = tx.prepare(
            "SELECT id FROM workspaces w
             WHERE project_id = ? AND kind = 'worktree' AND forgotten = 0
               AND NOT EXISTS (SELECT 1 FROM sync_ids WHERE kind = 'workspace' AND local_id = w.id)
             ORDER BY sort_order, rowid",
        )?;
        let rows = statement.query_map([project_id], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for id in ids {
        queue_workspace(tx, &id, now)?;
    }
    Ok(())
}

/// The server's answer to one of this host's ops on a workspace. The host it lives on is the only
/// one that writes it, so a conflict is never another host's edit: it is a record that went on
/// without this host hearing, an answer lost, or a project removed. What it says here stands.
fn settle_workspace(
    tx: &Transaction<'_>,
    op: &SyncOp,
    result: &OpResult,
    now: i64,
) -> StoreResult<bool> {
    let forget = |tx: &Transaction<'_>| -> StoreResult<()> {
        tx.execute(
            "DELETE FROM sync_outbox WHERE sync_id = ? AND refused IS NULL",
            [op.id.as_str()],
        )?;
        tx.execute(
            "DELETE FROM sync_ids WHERE kind = 'workspace' AND sync_id = ?",
            [op.id.as_str()],
        )?;
        Ok(())
    };
    match result {
        OpResult::Applied { .. } if op.action == Action::Delete => {
            forget(tx)?;
            Ok(false)
        }
        OpResult::Applied { revision, .. } => {
            set_revision(tx, &op.id, *revision)?;
            tx.execute("DELETE FROM sync_outbox WHERE op = ?", [op.op.as_str()])?;
            Ok(false)
        }
        OpResult::Conflict { current, .. } if current.deleted => {
            forget(tx)?;
            Ok(false)
        }
        OpResult::Conflict { current, .. } => {
            tx.execute(
                "DELETE FROM sync_outbox WHERE sync_id = ? AND refused IS NULL",
                [op.id.as_str()],
            )?;
            tx.execute(
                "UPDATE sync_ids SET revision = ? WHERE kind = 'workspace' AND sync_id = ?",
                params![current.revision as i64, op.id.as_str()],
            )?;
            let local: Option<String> = tx
                .query_row(
                    "SELECT local_id FROM sync_ids WHERE kind = 'workspace' AND sync_id = ?",
                    [op.id.as_str()],
                    |r| r.get(0),
                )
                .optional()?;
            match local {
                Some(local) => queue_workspace(tx, &local, now)?,
                // A record of this host's it no longer knows: deleted against what is there.
                None => queue_stray_delete(tx, current, now)?,
            }
            Ok(false)
        }
        OpResult::Refused { .. } | OpResult::Unknown => settle_one(tx, op, result),
    }
}

/// Delete a workspace record of this host's that no local workspace is linked to any more, as
/// after Remote was turned off and on, or a project linked again: the workspaces here go up anew.
fn queue_stray_delete(tx: &Transaction<'_>, record: &SyncRecord, now: i64) -> StoreResult<()> {
    if pending(tx, &record.id)? > 0 {
        return Ok(());
    }
    let row = SyncIdRow {
        kind: RecordKind::Workspace,
        local_id: String::new(),
        sync_id: record.id.clone(),
        project: record.project.clone(),
        revision: Some(record.revision),
        fresh: false,
    };
    queue_workspace_op(tx, &row, Action::Delete, Some(record.revision), None, now)
}

/// A workspace from the server: another host's is shown here, and nothing is made for it; one of
/// this host's own only tells its revision.
fn take_workspace(tx: &Transaction<'_>, record: &SyncRecord) -> StoreResult<bool> {
    let linked: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM sync_ids WHERE kind = 'project' AND sync_id = ?)",
        [record.project.as_str()],
        |r| r.get(0),
    )?;
    if !linked {
        return Ok(false);
    }
    let mine: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM sync_ids WHERE kind = 'workspace' AND sync_id = ?)",
        [record.id.as_str()],
        |r| r.get(0),
    )?;
    if mine {
        if pending(tx, &record.id)? == 0 && !record.deleted {
            set_revision(tx, &record.id, record.revision)?;
        }
        return Ok(false);
    }
    if record.deleted {
        return Ok(tx.execute(
            "DELETE FROM synced_workspaces WHERE sync_id = ?",
            [record.id.as_str()],
        )? > 0);
    }
    // A schema from a newer version: left as it is.
    let Some(body) = WorkspaceBody::of(record) else {
        return Ok(false);
    };
    if this_host(tx)?.is_some_and(|(id, _)| id == body.origin) {
        queue_stray_delete(tx, record, record.changed_at)?;
        return Ok(false);
    }
    let tasks = serde_json::to_string(&body.tasks).expect("tasks are JSON");
    Ok(tx.execute(
        "INSERT INTO synced_workspaces (sync_id, project, host_id, host, name, branch, base_branch,
                                        archived, tasks, revision, changed_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(sync_id) DO UPDATE SET
             host_id = excluded.host_id, host = excluded.host, name = excluded.name,
             branch = excluded.branch, base_branch = excluded.base_branch,
             archived = excluded.archived, tasks = excluded.tasks, revision = excluded.revision,
             changed_at = excluded.changed_at
         WHERE excluded.revision > synced_workspaces.revision",
        params![
            record.id.as_str(),
            record.project.as_str(),
            body.origin,
            body.host,
            body.name,
            body.branch,
            body.base_branch,
            body.archived,
            tasks,
            record.revision as i64,
            record.changed_at
        ],
    )? > 0)
}
