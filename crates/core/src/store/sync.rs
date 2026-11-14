//! This installation's side of sync: which local records it syncs and as what, the ops waiting to
//! go up, and how far it has pulled. See `migrations/0043_sync.sql` and `crate::sync`, which owns
//! the rules; the loop that talks to the server is `remote_host::sync`.

use remote_proto::sync::{
    Action, Cursor, OpId, OpResult, RecordKind, Revision, SyncId, SyncOp, SyncRecord, SYNC_SCHEMA,
};
use rusqlite::{params, OptionalExtension, Transaction};

use super::{new_id, Store, StoreError, StoreResult};

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
    pub fn settle_sync(&self, results: &[OpResult]) -> StoreResult<bool> {
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
            changed |= settle_one(&tx, &queued, result)?;
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
                tx.execute(
                    "UPDATE sync_ids SET fresh = 0 WHERE kind = 'project' AND sync_id = ?",
                    [project],
                )?;
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

/// One record from the server. Only projects are taken in so far; other kinds' slices take in
/// theirs.
fn take_one(tx: &Transaction<'_>, record: &SyncRecord) -> StoreResult<bool> {
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
