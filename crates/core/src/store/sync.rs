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
use crate::sync::{NoteBody, NoteConflict, NoteHistory};

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
    Ok(())
}

/// Clear every sync table: Remote is off, or on as another host.
pub(super) fn forget_sync(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch("DELETE FROM sync_ids; DELETE FROM sync_outbox; DELETE FROM sync_state;")
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
            changed |= if queued.kind == RecordKind::Note {
                settle_note(&tx, &queued, result, now)?
            } else {
                settle_one(&tx, &queued, result)?
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
                }
            }
        }
        if projects.is_none() {
            set_cursor(&tx, head)?;
        }
        tx.commit()?;
        Ok(changed)
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

/// One record from the server. Workspaces are taken in by their own slice.
fn take_one(tx: &Transaction<'_>, record: &SyncRecord) -> StoreResult<bool> {
    if record.kind == RecordKind::Note {
        return take_note(tx, record);
    }
    if record.kind != RecordKind::Project {
        return Ok(false);
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
    let settles = dropped.iter().rev().find_map(|op| {
        let body: NoteBody = serde_json::from_value(op.body.clone()?).ok()?;
        body.settles
    });
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
    if theirs.text == local.text && theirs.conflict.is_none() && local.conflict.is_none() {
        // The same text both ways: nothing to settle.
        apply_note(tx, &row.local_id, current, &theirs)?;
        return Ok(false);
    }
    // Two texts: theirs stands, and this host's goes beside it, here and on every host.
    let Some((_, here)) = this_host(tx)? else {
        return Ok(false);
    };
    apply_note(tx, &row.local_id, current, &theirs)?;
    if let Some(waiting) = theirs.conflict.as_ref().filter(|c| c.text != local.text) {
        // A third text while one waits: the one waiting is kept in the history.
        keep_replaced(tx, &row.local_id, &waiting.text, now)?;
    }
    let conflict = NoteConflict {
        id: new_id(),
        text: local.text,
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
