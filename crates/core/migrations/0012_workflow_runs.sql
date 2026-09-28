-- Workflow runs: one row per time a workflow was asked to run, one row per step of it. See
-- docs/design/21-workflows.md.
--
-- A run is rows, not a process: a step waiting for an agent to settle is a row marked
-- `waiting`, and the app's driver moves it on when something changes. The file is copied into
-- the run as it was when the run was asked for, so editing a workflow never changes a run that
-- is already going. The workspace's name is copied too, so a run's history reads the same after
-- the workspace is deleted.
CREATE TABLE workflow_runs (
    id              TEXT PRIMARY KEY,
    workflow_id     TEXT NOT NULL,
    workflow_name   TEXT NOT NULL,
    -- The workflow file's text, exactly as it was when the run was asked for.
    definition      TEXT NOT NULL,
    project_id      TEXT REFERENCES projects(id) ON DELETE SET NULL,
    workspace_id    TEXT REFERENCES workspaces(id) ON DELETE SET NULL,
    workspace_name  TEXT NOT NULL,
    -- What was given for each input, as a JSON object of strings.
    inputs          TEXT NOT NULL DEFAULT '{}',
    status          TEXT NOT NULL
                    CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'cancelled')),
    requested_by    TEXT NOT NULL CHECK (requested_by IN ('app', 'cli')),
    -- Why the run failed or was cancelled, when it is not one step's own error.
    error           TEXT,
    created_at      INTEGER NOT NULL,
    started_at      INTEGER,
    ended_at        INTEGER
);
CREATE INDEX workflow_runs_by_created ON workflow_runs (created_at DESC);
CREATE INDEX workflow_runs_active ON workflow_runs (status) WHERE status IN ('queued', 'running');
-- A workflow runs at most once at a time in a workspace. Enforced here so the app and `ys`,
-- two processes, cannot both start one.
CREATE UNIQUE INDEX workflow_runs_one_at_a_time ON workflow_runs (workflow_id, workspace_id)
    WHERE status IN ('queued', 'running');

CREATE TABLE workflow_step_runs (
    run_id      TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    step_id     TEXT NOT NULL,
    -- The step's place in the file, for listing in the order it was written.
    position    INTEGER NOT NULL,
    action      TEXT NOT NULL,
    -- `running`: the driver is doing the step's one action (starting, pasting, notifying).
    -- `waiting`: it is waiting for something outside to happen.
    status      TEXT NOT NULL
                CHECK (status IN ('pending', 'running', 'waiting', 'succeeded', 'failed',
                                  'skipped', 'cancelled')),
    started_at  INTEGER,
    ended_at    INTEGER,
    -- What the step left for later steps, as a JSON object of strings: `session`, `run`, ….
    outputs     TEXT NOT NULL DEFAULT '{}',
    -- Why it failed or was skipped, in words.
    note        TEXT,
    PRIMARY KEY (run_id, step_id)
);
