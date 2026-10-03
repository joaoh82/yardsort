-- Tasks, slice 2: which task a workspace was started from. See docs/design/23-tasks.md.
--
-- Written when a workspace is created from a task — by the composer, or by `ys task start` —
-- and never guessed from a branch name or a first message. Two workspaces started from one
-- issue are two rows: two attempts at one task, known and not inferred.
--
-- A row carries its own copy of the task's link and title as they were then, so the workspace
-- can say where it came from without asking the forge. It goes when its workspace does; the
-- task itself is the forge's, and nothing here ever changes it.
CREATE TABLE workspace_tasks (
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    -- Where the task is kept: 'github'.
    source       TEXT NOT NULL,
    -- Which of that source's projects: host/owner/name.
    repo         TEXT NOT NULL,
    -- What a person calls it there: '#91'.
    key          TEXT NOT NULL,
    url          TEXT NOT NULL,
    title        TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    PRIMARY KEY (workspace_id, source, repo, key)
);
