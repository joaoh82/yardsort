-- One row per harness conversation in a workspace. The PTY process comes and goes — it dies with
-- the app — but the harness keeps the conversation on disk, so a record is what lets us resume
-- or fork it later. Shells are not recorded: there is nothing to come back to.
CREATE TABLE sessions (
    id                  TEXT PRIMARY KEY,
    workspace_id        TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    harness_id          TEXT NOT NULL,
    model               TEXT,
    effort              TEXT,
    -- The harness's own id for the conversation, when we chose it (`assigned` mode). NULL means
    -- the harness picked, and resuming is "the latest one in this folder".
    harness_session_id  TEXT,
    -- Shown in lists: the start of the first message, or empty.
    title               TEXT NOT NULL DEFAULT '',
    forked_from         TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    -- 'running' while its process lives; 'ended' after. A row still 'running' when the app
    -- starts belongs to a process that died with the previous run.
    state               TEXT NOT NULL CHECK (state IN ('running', 'ended')),
    -- NULL when ended without an exit status: the app quit or crashed underneath it.
    exit_code           INTEGER,
    -- The live PTY session, while 'running'.
    pty_session_id      TEXT,
    started_at          INTEGER NOT NULL,
    ended_at            INTEGER
);

CREATE INDEX sessions_by_workspace ON sessions(workspace_id, started_at DESC);
CREATE INDEX sessions_by_pty ON sessions(pty_session_id) WHERE pty_session_id IS NOT NULL;
