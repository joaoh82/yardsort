-- What a trigger last saw of a workspace's pull request, so that the `pr.*` events are changes
-- rather than states: checks that failed are an event once, not at every look. See
-- docs/design/36-triggers.md and `crate::trigger::watch`.
--
-- One row per workspace an enabled trigger watches. It is written when such a trigger is saved
-- or switched on, from what the forge says then, and after every look.
CREATE TABLE pr_watches (
    workspace_id    TEXT PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
    -- Whether the states below were read from the forge. Until they were, the first look only
    -- records them: a pull request already open is not news, nor checks already red.
    seeded          INTEGER NOT NULL DEFAULT 0,
    -- The pull request, once the workspace has one: NULL is a state too, so that one opening
    -- is a change.
    number          INTEGER,
    -- The branch it was found for. Another branch checked out is another pull request.
    branch          TEXT,
    draft           INTEGER NOT NULL DEFAULT 0,
    head            TEXT,
    -- The last finished verdict of its checks that was told, and on which commit: `failed` or
    -- `passed`. A new head, or a different verdict on it, is news.
    checks          TEXT CHECK (checks IN ('failed', 'passed')),
    checks_head     TEXT,
    -- The commit it was last told to conflict on. Cleared when it merges cleanly again.
    conflict_head   TEXT,
    -- The reviews, comments and replies seen, as a JSON array of ids such as `review:123`.
    seen            TEXT NOT NULL DEFAULT '[]',
    looked_at       INTEGER NOT NULL DEFAULT 0,
    -- Why the last look could not be made, if it could not.
    problem         TEXT
);
