-- Agent activity, stage 1: what Yardsort itself knows about the processes it starts in a
-- workspace. See docs/design/09-agent-events-and-memory.md and 10-agent-events-stage-1.md.
--
-- A *run* is one process in a PTY: a harness conversation being started, resumed or forked, a
-- shell, or the project's run command. It is a fourth id beside the session record, the PTY id
-- and the harness's own conversation id: the row is written *before* the spawn, so a process
-- that exits before anyone hears of it still has something to be matched back to.
CREATE TABLE agent_runs (
    id                  TEXT PRIMARY KEY,
    workspace_id        TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    -- The conversation this process belongs to, once its record exists. Forgetting the record
    -- keeps the run; a run with no record is a shell, a program, or a launch that failed.
    session_id          TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    kind                TEXT NOT NULL CHECK (kind IN ('harness', 'shell', 'program')),
    harness_id          TEXT,
    -- The harness's own conversation id when Yardsort chose it (see sessions.harness_session_id).
    harness_session_id  TEXT,
    -- NULL until the host has spawned the process, and afterwards if it never did.
    pty_session_id      TEXT UNIQUE,
    launched_by         TEXT NOT NULL CHECK (launched_by IN ('app', 'cli')),
    started_at          INTEGER NOT NULL,
    ended_at            INTEGER,
    exit_code           INTEGER,
    -- 'exited' with a status, 'interrupted' when nothing was there to hear the exit, or
    -- 'spawn_failed' when the host could not start the program at all.
    end_reason          TEXT CHECK (end_reason IN ('exited', 'interrupted', 'spawn_failed')),
    -- What this run's events cover. 'lifecycle' means only what Yardsort saw from outside;
    -- later stages add native adapters and say so here.
    collection          TEXT NOT NULL DEFAULT 'lifecycle'
);

CREATE INDEX agent_runs_by_workspace ON agent_runs(workspace_id, started_at DESC);
CREATE INDEX agent_runs_by_session ON agent_runs(session_id, started_at);
CREATE INDEX agent_runs_open ON agent_runs(pty_session_id) WHERE ended_at IS NULL;

-- One event per fact, in one table for every producer to come. `seq` is the ingestion order and
-- the tie-breaker for display; `occurred_at` is the producer's clock. `source_key` is what makes
-- delivery idempotent: the same exit arriving live, from the daemon's spool and from a startup
-- reconciliation is one row.
CREATE TABLE agent_events (
    seq             INTEGER PRIMARY KEY AUTOINCREMENT,
    id              TEXT NOT NULL UNIQUE,
    schema_version  INTEGER NOT NULL,
    workspace_id    TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    session_id      TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    run_id          TEXT REFERENCES agent_runs(id) ON DELETE CASCADE,
    occurred_at     INTEGER NOT NULL,
    received_at     INTEGER NOT NULL,
    kind            TEXT NOT NULL,
    -- Who said so and how sure to be: 'yardsort'/'lifecycle'/'observed' for everything in
    -- stage 1; native adapters will report themselves as 'claude'/'hook'/'reported' and so on.
    producer        TEXT NOT NULL,
    method          TEXT NOT NULL,
    fidelity        TEXT NOT NULL,
    source_key      TEXT,
    -- 'metadata' holds no prompt, command, path or output. Content classes arrive with an
    -- explicit setting in a later stage; nothing writes them today.
    privacy_class   TEXT NOT NULL,
    payload         TEXT NOT NULL
);

CREATE INDEX agent_events_timeline ON agent_events(workspace_id, seq);
CREATE INDEX agent_events_by_run ON agent_events(run_id, seq);
CREATE UNIQUE INDEX agent_events_source ON agent_events(producer, source_key)
    WHERE source_key IS NOT NULL;

-- Counters for what went wrong while recording: writes that failed, spool files dropped or
-- unreadable, duplicates seen. Small, and the only place a telemetry failure is allowed to go.
CREATE TABLE agent_event_diagnostics (
    name        TEXT PRIMARY KEY,
    count       INTEGER NOT NULL DEFAULT 0,
    last_at     INTEGER NOT NULL,
    last_detail TEXT
);
