-- Track worktrees created here, including those opened on existing branches. Imported and
-- adopted worktrees must not acquire project setup when restored.
CREATE TABLE workspace_preparation (
    workspace_id TEXT PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE
);
-- Older rows with a base branch were created by Yardsort. Unknown provenance stays opted out.
INSERT INTO workspace_preparation (workspace_id)
SELECT id FROM workspaces WHERE kind = 'worktree' AND base_branch IS NOT NULL;
