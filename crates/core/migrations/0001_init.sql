-- Projects are git repositories Yardsort knows about. Removing one never touches the disk.
CREATE TABLE projects (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    root_path   TEXT NOT NULL UNIQUE,
    sort_order  INTEGER NOT NULL,
    created_at  INTEGER NOT NULL
);

-- A workspace is one place work happens inside a project. Every project has exactly one
-- `local` workspace, whose path is the project's own checkout; `worktree` rows arrive in M3.
CREATE TABLE workspaces (
    id           TEXT PRIMARY KEY,
    project_id   TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL CHECK (kind IN ('local', 'worktree')),
    name         TEXT NOT NULL,
    path         TEXT NOT NULL,
    branch       TEXT,
    base_branch  TEXT,
    status       TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived')),
    sort_order   INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL
);

CREATE INDEX workspaces_by_project ON workspaces(project_id, sort_order);
CREATE UNIQUE INDEX one_local_per_project ON workspaces(project_id) WHERE kind = 'local';

-- Small pieces of UI state that should survive a restart: selection, expansion, last-used picks.
CREATE TABLE ui_state (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);
