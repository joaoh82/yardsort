-- What a trigger last saw of a pull request a workspace tracks in another repository, as
-- `pr_watches` keeps it for the workspace's own pull request: so that the `pr.*` events of one
-- elsewhere are changes rather than states. See docs/design/38-pull-requests-elsewhere.md and
-- `crate::trigger::watch`.
--
-- One row per tracked pull request whose workspace has an enabled trigger that runs a workflow
-- about one elsewhere (`context: tracked_pull_request`). It goes with the tracked row: a pull
-- request no longer tracked is no longer watched.
CREATE TABLE tracked_pr_watches (
    tracked_id      TEXT PRIMARY KEY REFERENCES tracked_pull_requests(id) ON DELETE CASCADE,
    -- As in `pr_watches`. A tracked pull request is asked about by its link, so there is no
    -- branch to follow: its number is the tracked row's, once a look has seen it.
    seeded          INTEGER NOT NULL DEFAULT 0,
    number          INTEGER,
    closed          INTEGER NOT NULL DEFAULT 0,
    draft           INTEGER NOT NULL DEFAULT 0,
    head            TEXT,
    checks          TEXT CHECK (checks IN ('failed', 'passed')),
    checks_head     TEXT,
    conflict_head   TEXT,
    seen            TEXT NOT NULL DEFAULT '[]',
    looked_at       INTEGER NOT NULL DEFAULT 0,
    problem         TEXT
);
