-- Pull requests a workspace follows in repositories other than its project's. See
-- docs/design/38-pull-requests-elsewhere.md.
--
-- A project's own pull requests are found by asking its repository and matching branches and
-- commits; one in another repository leaves no trace git or the forge can tie to a workspace, so
-- it is written down here when an agent says so (`ys pr track`), the user does (Track pull
-- request…), or an adapter sees one being created. The pull request itself is the forge's and is
-- read from there; a row only says which workspace follows it.
CREATE TABLE tracked_pull_requests (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    -- The conversation that opened it, when that is known.
    session_id   TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    -- host/owner/name.
    repo         TEXT NOT NULL,
    number       INTEGER NOT NULL,
    -- https://host/owner/name/pull/number, rebuilt from the three above.
    url          TEXT NOT NULL,
    tracked_by   TEXT NOT NULL CHECK (tracked_by IN ('app', 'cli', 'hook')),
    created_at   INTEGER NOT NULL,
    UNIQUE (workspace_id, url)
);
CREATE INDEX tracked_pull_requests_workspace ON tracked_pull_requests (workspace_id);
