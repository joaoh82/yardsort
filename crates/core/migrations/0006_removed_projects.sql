-- A project the user removed from Yardsort while keeping its history: hidden, with its workspaces
-- and their sessions still attached, so adding the same folder again brings everything back.
-- Removing *without* keeping the history deletes the row (and, by cascade, the rest) as before.
ALTER TABLE projects ADD COLUMN removed INTEGER NOT NULL DEFAULT 0;
