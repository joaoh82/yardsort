-- When the attempt began: its workspace's own creation time, copied with the rest of the
-- snapshot. `created_at` is only when the outcome row was first written — often the moment the
-- workspace was deleted — so it cannot say whether a pull request was opened during the
-- attempt. NULL for rows written before this, whose workspace is gone and took the time with it.
-- See docs/design/20-agent-events-stage-6-outcomes.md.
ALTER TABLE workspace_outcomes ADD COLUMN began_at INTEGER;
UPDATE workspace_outcomes
   SET began_at = (SELECT created_at FROM workspaces WHERE workspaces.id = workspace_outcomes.id);
