-- The Notifications list: what waits for a person, and what finished while they were not
-- looking. One row is one thing that happened: to an agent's conversation, to a workflow's run,
-- to a smoke test's run, to a schedule or to a trigger. The core writes it when the thing
-- happens and settles it when it is answered or seen; the window, and later a phone, read it.
-- See docs/design/49-notifications.md.
--
-- The words a row is shown with are not kept here. A command an agent wants to run is read from
-- its open question, what it said last from its own session file, and a workflow's question
-- from its wait, when the list is asked for.
CREATE TABLE notifications (
    id TEXT PRIMARY KEY,
    -- Stable for one event, so the same event told twice is one row.
    key TEXT NOT NULL UNIQUE,
    -- What happened: `agent.asks`, `agent.finished`, `workflow.failed`… The core's
    -- `NotificationKind` is the list of them. A kind this Yardsort does not know is not shown.
    kind TEXT NOT NULL,
    -- `needs`: it leaves when it is answered. `finished`: it leaves when it is seen.
    part TEXT NOT NULL CHECK (part IN ('needs', 'finished')),
    -- The workspace it happened in. Only a schedule or a trigger can be without one: its
    -- workspace is gone, or it makes its own.
    workspace_id TEXT REFERENCES workspaces(id) ON DELETE CASCADE,
    -- The project, for a row with no workspace to say it.
    project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
    -- The agent's conversation, for a row about an agent.
    session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
    -- The agent's run, when its hooks named one: what tells whether a question was answered.
    run_id TEXT,
    -- What else the row is about, by its id: a workflow's run, a smoke test's run, a schedule
    -- or a trigger. No foreign key: each of those is in a table of its own.
    subject_id TEXT,
    -- The step that waits, for a workflow's question or approval.
    step_id TEXT,
    -- How long the agent worked before it finished, when that is known.
    worked_ms INTEGER,
    -- What its process exited with, for one that stopped.
    exit_code INTEGER,
    -- When it happened. A conversation that finishes again before it was seen keeps its one
    -- row, moved to the later time.
    created_at INTEGER NOT NULL,
    -- When it was answered or seen. Null while it is still in its part of the list.
    settled_at INTEGER
);

CREATE INDEX notifications_by_settled ON notifications (settled_at, created_at);
CREATE INDEX notifications_by_session ON notifications (session_id, part, settled_at);
CREATE INDEX notifications_by_subject ON notifications (subject_id, kind, settled_at);
