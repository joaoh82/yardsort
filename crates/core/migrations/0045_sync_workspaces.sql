-- Workspaces seen from another host (docs/design/42-remote-r6.md, slice 4): the other hosts'
-- workspaces of a project synced here, as their records say them. Never a workspace of this
-- installation, and never anything on this disk: no path, no worktree, no branch is made here for
-- any of them. Cleared with the rest of sync's tables.
CREATE TABLE synced_workspaces (
    sync_id TEXT PRIMARY KEY,
    -- The logical id of the project it belongs to.
    project TEXT NOT NULL,
    -- The host it lives on: its id, and its name as shown.
    host_id TEXT NOT NULL,
    host TEXT NOT NULL,
    name TEXT NOT NULL,
    branch TEXT,
    base_branch TEXT,
    archived INTEGER NOT NULL DEFAULT 0,
    -- The tasks it was started from, as JSON: [{source, repo, key, url, title}].
    tasks TEXT NOT NULL DEFAULT '[]',
    revision INTEGER NOT NULL,
    changed_at INTEGER NOT NULL
);

CREATE INDEX synced_workspaces_by_project ON synced_workspaces (project);
