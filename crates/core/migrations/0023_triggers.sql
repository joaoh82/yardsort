-- Triggers: a standing instruction to start a workflow or a smoke test in a workspace when
-- something happens there. See docs/design/36-triggers.md.
--
-- Like a schedule, a trigger only ever queues a run through the doors a person's click uses,
-- so the run tables know nothing of it: `trigger_fires.run_id` is where "which trigger started
-- this" is kept.
CREATE TABLE triggers (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    -- The preset it was made from, when it was made from one.
    preset          TEXT,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    -- NULL once the workspace is deleted; `workspace_name` keeps what it was called. A trigger
    -- without a workspace fires nothing.
    workspace_id    TEXT REFERENCES workspaces(id) ON DELETE SET NULL,
    workspace_name  TEXT NOT NULL,
    -- Set for a trigger that belongs to one agent session: it goes with the session's record.
    session_id      TEXT REFERENCES sessions(id) ON DELETE CASCADE,
    -- What it listens for, as a JSON array of event names: `["workspace.done"]`.
    events          TEXT NOT NULL,
    -- What each event must also be, as a JSON object. See `trigger::Filters`.
    filters         TEXT NOT NULL DEFAULT '{}',
    -- What it starts: a workflow by its id, or a smoke test by its id in the workspace's folder.
    target_kind     TEXT NOT NULL CHECK (target_kind IN ('workflow', 'test')),
    target_id       TEXT NOT NULL,
    -- What to give for each input, as a JSON object of strings.
    inputs          TEXT NOT NULL DEFAULT '{}',
    -- How long after the last event it waits before firing.
    quiet_ms        INTEGER NOT NULL DEFAULT 0,
    -- Runs per subject before it stops and asks.
    cap             INTEGER NOT NULL DEFAULT 3,
    notify          TEXT NOT NULL DEFAULT 'failure' CHECK (notify IN ('failure', 'always', 'never')),
    enabled         INTEGER NOT NULL DEFAULT 1,
    -- Only what happens from here on counts: set when it is saved and when it is switched on.
    since           INTEGER NOT NULL,
    created_by      TEXT NOT NULL CHECK (created_by IN ('app', 'cli')),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE INDEX triggers_by_workspace ON triggers (workspace_id) WHERE workspace_id IS NOT NULL;

-- One row per fire: the events a trigger gathered for one subject, and what became of them.
CREATE TABLE trigger_fires (
    id              TEXT PRIMARY KEY,
    trigger_id      TEXT NOT NULL REFERENCES triggers(id) ON DELETE CASCADE,
    -- What the events were about: `session:<record>` or `workspace:<id>`. The cap counts runs
    -- per subject.
    subject         TEXT NOT NULL,
    -- The kind of the newest event gathered.
    event           TEXT NOT NULL,
    events_count    INTEGER NOT NULL DEFAULT 1,
    first_at        INTEGER NOT NULL,
    -- When it fires, unless another event moves it later.
    due_at          INTEGER NOT NULL,
    decided_at      INTEGER,
    -- `gathering`: waiting for its quiet time. `waiting`: a run of the same thing is active in
    -- the workspace. `started`: a run was queued. `skipped`: there was no one to run it for.
    -- `capped`: the subject had its runs. `refused`: it could not be queued, and `note` says why.
    outcome         TEXT NOT NULL CHECK (outcome IN
                        ('gathering', 'waiting', 'started', 'skipped', 'capped', 'refused')),
    -- What the events leave for `{{ event.* }}`, as a JSON object of strings.
    facts           TEXT NOT NULL DEFAULT '{}',
    -- How many triggered runs led to it: a person's run is 0.
    depth           INTEGER NOT NULL DEFAULT 0,
    run_id          TEXT,
    note            TEXT,
    -- Counted against the cap. Reset lifts the cap by clearing it.
    counted         INTEGER NOT NULL DEFAULT 0,
    -- The run's status once it ended, copied here so the history reads the same after the run
    -- is pruned. NULL while it is going.
    result          TEXT,
    notified_at     INTEGER,
    told_waiting    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX trigger_fires_by_trigger ON trigger_fires (trigger_id, first_at DESC);
CREATE INDEX trigger_fires_by_run ON trigger_fires (run_id) WHERE run_id IS NOT NULL;
CREATE INDEX trigger_fires_open ON trigger_fires (outcome)
    WHERE outcome IN ('gathering', 'waiting');
CREATE INDEX trigger_fires_unsettled ON trigger_fires (outcome)
    WHERE outcome = 'started' AND result IS NULL;

-- `ys workspace done`: an agent saying the work in its workspace is committed and ready to be
-- looked at. A table of its own because `note` is the agent's words, which `agent_events`, all
-- metadata, does not hold.
CREATE TABLE done_signals (
    seq             INTEGER PRIMARY KEY AUTOINCREMENT,
    id              TEXT NOT NULL UNIQUE,
    workspace_id    TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    session_id      TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    -- The worktree's HEAD when it was said, when there is one.
    head            TEXT,
    note            TEXT,
    at              INTEGER NOT NULL
);

-- Where the app's trigger thread has read up to in each source it follows, so that a restart
-- starts where it stopped and nothing is seen twice: `agent_events` and `done_signals` by
-- sequence number, run endings by time.
CREATE TABLE trigger_cursors (
    source          TEXT PRIMARY KEY,
    position        INTEGER NOT NULL
);
