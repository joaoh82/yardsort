-- Smoke tests, slice 2: runs, their steps, approvals and the per-project test environment. See
-- docs/design/32-smoke-tests.md.
--
-- A run is rows, carried out by a thread in the app: `ys test run` writes the row and nothing
-- else. The file is copied into the run as it was when the run was asked for, with its hash, so
-- what ran is what was approved, whatever the branch does to the file afterwards. The workspace's
-- name is copied too, so history reads the same after the workspace is deleted.
CREATE TABLE test_runs (
    id                  TEXT PRIMARY KEY,
    project_id          TEXT REFERENCES projects(id) ON DELETE SET NULL,
    workspace_id        TEXT REFERENCES workspaces(id) ON DELETE SET NULL,
    workspace_name      TEXT NOT NULL,
    test_id             TEXT NOT NULL,
    test_name           TEXT NOT NULL,
    -- The test file's text, exactly as it was when the run was asked for, and its SHA-256.
    definition          TEXT NOT NULL,
    content_hash        TEXT NOT NULL,
    -- What was given for each input, as a JSON object of strings.
    inputs              TEXT NOT NULL DEFAULT '{}',
    -- `awaiting_approval`: the file, at this hash, has not been approved in the app yet.
    -- `errored`: the run could not be carried out, as distinct from steps that failed.
    status              TEXT NOT NULL
                        CHECK (status IN ('awaiting_approval', 'queued', 'running', 'passed',
                                          'failed', 'refused', 'cancelled', 'errored')),
    requested_by        TEXT NOT NULL CHECK (requested_by IN ('app', 'cli', 'workflow')),
    -- The agent's conversation record when `ys test run` was run from an agent's terminal.
    session_record_id   TEXT,
    -- The free port the run was given, once it started.
    port                INTEGER,
    -- Why the run ended the way it did, when it is not one step's own note.
    error               TEXT,
    -- Where the steps' output files went.
    output_dir          TEXT,
    -- A cancel asked for while the run was running: the runner stops the step it is on, runs
    -- the teardown, and ends the run as cancelled.
    cancel_requested_at INTEGER,
    created_at          INTEGER NOT NULL,
    started_at          INTEGER,
    ended_at            INTEGER
);
CREATE INDEX test_runs_by_created ON test_runs (created_at DESC);
CREATE INDEX test_runs_active ON test_runs (status)
    WHERE status IN ('awaiting_approval', 'queued', 'running');
-- A test runs at most once at a time in a workspace. Enforced here so the app and `ys`, two
-- processes, cannot both start one.
CREATE UNIQUE INDEX test_runs_one_at_a_time ON test_runs (test_id, workspace_id)
    WHERE status IN ('awaiting_approval', 'queued', 'running');

CREATE TABLE test_step_runs (
    run_id      TEXT NOT NULL REFERENCES test_runs(id) ON DELETE CASCADE,
    step_id     TEXT NOT NULL,
    phase       TEXT NOT NULL CHECK (phase IN ('setup', 'steps', 'teardown')),
    -- The step's place in the file, over all three phases, for listing in the order it ran.
    position    INTEGER NOT NULL,
    action      TEXT NOT NULL,
    -- `waiting`: a `manual` step, until the person answers it in the app.
    status      TEXT NOT NULL
                CHECK (status IN ('pending', 'running', 'waiting', 'passed', 'failed',
                                  'skipped', 'cancelled')),
    started_at  INTEGER,
    ended_at    INTEGER,
    -- The process's exit code, for `run`, `shell` and a `wait_for` on a command.
    exit        INTEGER,
    -- What the step left for later steps, as a JSON object of strings: the fields its action
    -- produces. `stdout` and `stderr` are paths to files, not the text.
    outputs     TEXT NOT NULL DEFAULT '{}',
    -- Every expectation that failed, in words, or why the step was skipped.
    note        TEXT,
    -- Paths, as a JSON array of strings. Written by browser steps.
    screenshots TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY (run_id, step_id)
);

-- What enables a run: the user, in the app, having seen this exact file. A file in a repository
-- never enables a command by itself, and `ys` has no way to write here.
CREATE TABLE test_approvals (
    project_id   TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    test_id      TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    approved_at  INTEGER NOT NULL,
    PRIMARY KEY (project_id, test_id, content_hash)
);

-- A project's test environment: what `{{ env.NAME }}` means in its tests. Set by the user in
-- the app, never read from the repository.
CREATE TABLE project_test_env (
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    value      TEXT NOT NULL,
    PRIMARY KEY (project_id, name)
);
