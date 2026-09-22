-- A folder is one workspace. Two overlapping adoption runs could record the same worktree twice;
-- drop such duplicates (keeping the oldest row) and make it impossible from here on.
DELETE FROM workspaces
WHERE rowid NOT IN (SELECT MIN(rowid) FROM workspaces GROUP BY path);

CREATE UNIQUE INDEX one_workspace_per_path ON workspaces(path);
