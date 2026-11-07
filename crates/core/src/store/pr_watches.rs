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

const COLUMNS: &str = "workspace_id, seeded, number, branch, draft, head, checks, checks_head, \
                       conflict_head, seen, looked_at, problem";

fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PrWatchRow> {
    let seen: String = row.get(9)?;
    Ok(PrWatchRow {
        workspace_id: row.get(0)?,
        seeded: row.get(1)?,
        number: row.get(2)?,
        branch: row.get(3)?,
        draft: row.get(4)?,
        head: row.get(5)?,
        checks: row.get(6)?,
        checks_head: row.get(7)?,
        conflict_head: row.get(8)?,
        seen: serde_json::from_str(&seen).unwrap_or_default(),
        looked_at: row.get(10)?,
        problem: row.get(11)?,
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
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
            ),
            params![
                watch.workspace_id,
                watch.seeded,
                watch.number,
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

    /// A look could not be made: say why, and when, keeping what was seen before.
    pub fn pr_watch_failed(&self, workspace_id: &str, at: i64, problem: &str) -> StoreResult<()> {
        self.conn().execute(
            "UPDATE pr_watches SET looked_at = ?, problem = ? WHERE workspace_id = ?",
            params![at, problem, workspace_id],
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
