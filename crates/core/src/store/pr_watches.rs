//! What triggers last saw of each watched workspace's pull request. See
//! `migrations/0024_pr_watches.sql` and `crate::trigger::watch`, which owns the rules.

use std::collections::BTreeSet;

use rusqlite::{params, OptionalExtension};

use super::{Store, StoreResult};

/// One workspace's pull request as a watch last saw it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrWatchRow {
    pub workspace_id: String,
    /// The states were read from the forge; until then a look only records them.
    pub seeded: bool,
    pub number: Option<u32>,
    /// It was merged or closed; `number` and what was seen of it are kept for a reopening.
    pub closed: bool,
    pub branch: Option<String>,
    pub draft: bool,
    pub head: Option<String>,
    /// `failed` or `passed`: the last finished verdict told, for `checks_head`.
    pub checks: Option<String>,
    pub checks_head: Option<String>,
    pub conflict_head: Option<String>,
    /// Ids of the reviews, comments and replies seen.
    pub seen: BTreeSet<String>,
    pub looked_at: i64,
    pub problem: Option<String>,
}

const COLUMNS: &str = "workspace_id, seeded, number, closed, branch, draft, head, checks, \
                       checks_head, conflict_head, seen, looked_at, problem";

fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PrWatchRow> {
    let seen: String = row.get(10)?;
    Ok(PrWatchRow {
        workspace_id: row.get(0)?,
        seeded: row.get(1)?,
        number: row.get(2)?,
        closed: row.get(3)?,
        branch: row.get(4)?,
        draft: row.get(5)?,
        head: row.get(6)?,
        checks: row.get(7)?,
        checks_head: row.get(8)?,
        conflict_head: row.get(9)?,
        seen: serde_json::from_str(&seen).unwrap_or_default(),
        looked_at: row.get(11)?,
        problem: row.get(12)?,
    })
}

impl Store {
    pub fn pr_watch(&self, workspace_id: &str) -> StoreResult<Option<PrWatchRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {COLUMNS} FROM pr_watches WHERE workspace_id = ?"),
                [workspace_id],
                from_row,
            )
            .optional()?)
    }

    pub fn pr_watches(&self) -> StoreResult<Vec<PrWatchRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM pr_watches ORDER BY workspace_id"
        ))?;
        let rows = stmt.query_map([], from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Write a watch whole, adding it or replacing what was there.
    pub fn put_pr_watch(&self, watch: &PrWatchRow) -> StoreResult<()> {
        let seen = serde_json::to_string(&watch.seen).unwrap_or_else(|_| "[]".to_owned());
        self.conn().execute(
            &format!(
                "INSERT OR REPLACE INTO pr_watches ({COLUMNS})
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
            ),
            params![
                watch.workspace_id,
                watch.seeded,
                watch.number,
                watch.closed,
                watch.branch,
                watch.draft,
                watch.head,
                watch.checks,
                watch.checks_head,
                watch.conflict_head,
                seen,
                watch.looked_at,
                watch.problem,
            ],
        )?;
        Ok(())
    }

    pub fn remove_pr_watch(&self, workspace_id: &str) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "DELETE FROM pr_watches WHERE workspace_id = ?",
            [workspace_id],
        )? > 0)
    }
}

const TRACKED_COLUMNS: &str = "t.workspace_id, w.seeded, w.number, w.closed, w.draft, w.head, \
                               w.checks, w.checks_head, w.conflict_head, w.seen, w.looked_at, \
                               w.problem, w.tracked_id";

/// A watch of a pull request tracked elsewhere, with the tracked row's id. Its `branch` is
/// always `None`: one elsewhere is asked about by its link.
fn tracked_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(String, PrWatchRow)> {
    let seen: String = row.get(9)?;
    Ok((
        row.get(12)?,
        PrWatchRow {
            workspace_id: row.get(0)?,
            seeded: row.get(1)?,
            number: row.get(2)?,
            closed: row.get(3)?,
            branch: None,
            draft: row.get(4)?,
            head: row.get(5)?,
            checks: row.get(6)?,
            checks_head: row.get(7)?,
            conflict_head: row.get(8)?,
            seen: serde_json::from_str(&seen).unwrap_or_default(),
            looked_at: row.get(10)?,
            problem: row.get(11)?,
        },
    ))
}

/// What triggers last saw of each watched pull request a workspace tracks elsewhere. See
/// `migrations/0028_tracked_pr_watches.sql`.
impl Store {
    /// The watch of the tracked pull request `tracked_id`, with its workspace's id.
    pub fn tracked_pr_watch(&self, tracked_id: &str) -> StoreResult<Option<PrWatchRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!(
                    "SELECT {TRACKED_COLUMNS} FROM tracked_pr_watches w
                     JOIN tracked_pull_requests t ON t.id = w.tracked_id
                     WHERE w.tracked_id = ?"
                ),
                [tracked_id],
                tracked_from_row,
            )
            .optional()?
            .map(|(_, watch)| watch))
    }

    /// Every watch of a tracked pull request, by the tracked row's id.
    pub fn tracked_pr_watches(&self) -> StoreResult<Vec<(String, PrWatchRow)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {TRACKED_COLUMNS} FROM tracked_pr_watches w
             JOIN tracked_pull_requests t ON t.id = w.tracked_id
             ORDER BY w.tracked_id"
        ))?;
        let rows = stmt.query_map([], tracked_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Write the watch of the tracked pull request `tracked_id` whole. Its `workspace_id` and
    /// `branch` are not kept: the tracked row says whose it is.
    pub fn put_tracked_pr_watch(&self, tracked_id: &str, watch: &PrWatchRow) -> StoreResult<()> {
        let seen = serde_json::to_string(&watch.seen).unwrap_or_else(|_| "[]".to_owned());
        self.conn().execute(
            "INSERT OR REPLACE INTO tracked_pr_watches (tracked_id, seeded, number, closed, draft,
                 head, checks, checks_head, conflict_head, seen, looked_at, problem)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                tracked_id,
                watch.seeded,
                watch.number,
                watch.closed,
                watch.draft,
                watch.head,
                watch.checks,
                watch.checks_head,
                watch.conflict_head,
                seen,
                watch.looked_at,
                watch.problem,
            ],
        )?;
        Ok(())
    }

    pub fn remove_tracked_pr_watch(&self, tracked_id: &str) -> StoreResult<bool> {
        Ok(self.conn().execute(
            "DELETE FROM tracked_pr_watches WHERE tracked_id = ?",
            [tracked_id],
        )? > 0)
    }
}
