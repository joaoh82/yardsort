-- A workspace the user told Yardsort to stop showing. Its folder and branch are never touched,
-- and the row stays so its session history is still there if the worktree is imported again.
-- Forgetting *without* keeping the history deletes the row outright instead.
ALTER TABLE workspaces ADD COLUMN forgotten INTEGER NOT NULL DEFAULT 0;
