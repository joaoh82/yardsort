-- Outcomes, stage 6: what became of each attempt. A workspace is an attempt at its task; the
-- user labels how it went, and a merge backs it up. See docs/design/09-agent-events-and-memory.md
-- and 20-agent-events-stage-6-outcomes.md.
--
-- A row carries its own copy of what it describes — the workspace's name and branch, the task,
-- the agents that worked in it — because deleting a workspace removes its row and its sessions,
-- and that is exactly when an attempt is most often judged. It is keyed by the workspace's id,
-- which is never reused; `workspace_id` goes NULL when the workspace does.
CREATE TABLE workspace_outcomes (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    workspace_id    TEXT REFERENCES workspaces(id) ON DELETE SET NULL,
    workspace_name  TEXT NOT NULL,
    branch          TEXT,
    base_branch     TEXT,
    -- The workspace's first message, cut short; NULL when it had none.
    task            TEXT,
    -- The harnesses that held a conversation here, comma-separated, in first-seen order.
    harnesses       TEXT NOT NULL DEFAULT '',
    -- What the user said: kept, partly, or discarded. NULL until they say.
    label           TEXT CHECK (label IN ('kept', 'partly', 'discarded')),
    labeled_at      INTEGER,
    -- Git's evidence. `ahead_at`: the branch was seen with commits the base did not have.
    -- `merged_at`: after that, all of them were seen reachable from the base — a merge or a
    -- fast-forward. A squash merge never sets it; the pull request's state covers that.
    ahead_at        INTEGER,
    merged_at       INTEGER,
    -- The forge's evidence, from the pull request whose head is this branch.
    pr_number       INTEGER,
    pr_state        TEXT CHECK (pr_state IN ('open', 'merged', 'closed')),
    -- When the workspace ended: archived or deleted, and when.
    ended           TEXT CHECK (ended IN ('archived', 'deleted')),
    ended_at        INTEGER,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE INDEX workspace_outcomes_by_project ON workspace_outcomes (project_id);
