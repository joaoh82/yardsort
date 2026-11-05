-- Schedules: a standing instruction to start a workflow or a smoke test in a workspace, once at
-- a set time or again and again. See docs/design/34-schedules.md.
--
-- A schedule only ever queues a run through the doors a person's click uses, so the run tables
-- know nothing of it: `schedule_runs.run_id` is where "which schedule started this" is kept.
CREATE TABLE schedules (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    -- What it starts: a workflow by its id, or a smoke test by its id in the workspace's folder.
    target_kind     TEXT NOT NULL CHECK (target_kind IN ('workflow', 'test')),
    target_id       TEXT NOT NULL,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    -- NULL once the workspace is deleted; `workspace_name` keeps what it was called. A schedule
    -- without a workspace fires nothing.
    workspace_id    TEXT REFERENCES workspaces(id) ON DELETE SET NULL,
    workspace_name  TEXT NOT NULL,
    -- What to give for each input, as a JSON object of strings.
    inputs          TEXT NOT NULL DEFAULT '{}',
    -- When, as JSON: `{"kind":"daily","time":{"hour":9,"minute":0}}`. See `schedule::Spec`.
    spec            TEXT NOT NULL,
    -- What to do with a time that passed while the app was not running.
    missed          TEXT NOT NULL DEFAULT 'run_once' CHECK (missed IN ('run_once', 'skip')),
    notify          TEXT NOT NULL DEFAULT 'failure' CHECK (notify IN ('failure', 'always', 'never')),
    enabled         INTEGER NOT NULL DEFAULT 1,
    -- The next time it fires. NULL when there is none: a `once` that has fired.
    next_at         INTEGER,
    created_by      TEXT NOT NULL CHECK (created_by IN ('app', 'cli')),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE INDEX schedules_due ON schedules (next_at) WHERE enabled = 1 AND next_at IS NOT NULL;

-- One row per time a schedule came due, whatever became of it.
CREATE TABLE schedule_runs (
    id              TEXT PRIMARY KEY,
    schedule_id     TEXT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    -- When it was due, and when the app acted on it. Far apart when the app was closed.
    due_at          INTEGER NOT NULL,
    fired_at        INTEGER NOT NULL,
    -- `started`: a run was queued. `skipped`: the previous run was still going. `missed`: the
    -- time passed while the app was not running and the schedule skips those. `refused`: the
    -- run could not be queued, and `note` says why.
    outcome         TEXT NOT NULL CHECK (outcome IN ('started', 'skipped', 'missed', 'refused')),
    -- Asked for with Run now, not by the clock.
    manual          INTEGER NOT NULL DEFAULT 0,
    -- The workflow run's or the test run's id, by the schedule's `target_kind`.
    run_id          TEXT,
    note            TEXT,
    -- The run's status once it ended, copied here so the history reads the same after the run
    -- is pruned. NULL while it is going.
    result          TEXT,
    -- When the user was told the result, and whether they were told it waits for approval.
    notified_at     INTEGER,
    told_waiting    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX schedule_runs_by_schedule ON schedule_runs (schedule_id, fired_at DESC);
CREATE INDEX schedule_runs_by_run ON schedule_runs (run_id) WHERE run_id IS NOT NULL;
CREATE INDEX schedule_runs_unsettled ON schedule_runs (outcome)
    WHERE outcome = 'started' AND result IS NULL;
