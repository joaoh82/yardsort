-- Custom events: one row per `ys event emit <name>`, which a trigger on `custom.<name>` in the
-- workspace hears. See docs/design/36-triggers.md and `crate::trigger::sources::read_custom`.
--
-- Like `done_signals`, and for the same reason: the note is someone's own words, which no
-- `agent_events` row may hold. Read by sequence number from the trigger thread's cursor, so an
-- event raised while the app was closed is heard when it opens.
CREATE TABLE custom_events (
    seq             INTEGER PRIMARY KEY AUTOINCREMENT,
    id              TEXT NOT NULL UNIQUE,
    workspace_id    TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    -- The conversation it was raised in, from an agent's terminal.
    session_id      TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    -- The smoke test run whose `run` step raised it: what makes a run it starts one deeper.
    run_id          TEXT,
    -- Without `custom.`.
    name            TEXT NOT NULL,
    -- The worktree's HEAD when it was raised, when there is one.
    head            TEXT,
    note            TEXT,
    at              INTEGER NOT NULL
);
