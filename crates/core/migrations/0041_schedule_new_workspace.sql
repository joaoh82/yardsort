-- A schedule may make a new workspace for each run instead of naming one that is there. See
-- docs/design/34-schedules.md, "A new workspace per run".
--
-- Such a schedule has no `workspace_id`; `workspace_name` holds what its workspaces are called
-- (empty for "after the workflow"). `base_branch` is the branch each one starts from, NULL for
-- the repository's default. `keep` is the most of its workspaces that may be there at once:
-- at that many, a time is refused until one is archived or deleted. Nothing is ever removed.
ALTER TABLE schedules ADD COLUMN new_workspace INTEGER NOT NULL DEFAULT 0;
ALTER TABLE schedules ADD COLUMN base_branch TEXT;
ALTER TABLE schedules ADD COLUMN keep INTEGER NOT NULL DEFAULT 5;

-- The workspaces a schedule made, and the ones it is making. A table of its own, not a column
-- of `schedule_runs`: that history is pruned, and a workspace kept after its setup failed has
-- no run to hang on.
--
-- A row is written before the workspace is made, with no `workspace_id`: a slot taken, in one
-- statement that also counts, so two runs asked for at once cannot both find the last slot
-- free. The workspace's id is filled in the moment it has a row of its own, and the slot is
-- given back when nothing was made.
CREATE TABLE schedule_workspaces (
    id            TEXT PRIMARY KEY,
    schedule_id   TEXT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    workspace_id  TEXT REFERENCES workspaces(id) ON DELETE CASCADE,
    created_at    INTEGER NOT NULL
);
CREATE UNIQUE INDEX schedule_workspaces_once ON schedule_workspaces (schedule_id, workspace_id)
    WHERE workspace_id IS NOT NULL;
CREATE INDEX schedule_workspaces_by_schedule ON schedule_workspaces (schedule_id);
