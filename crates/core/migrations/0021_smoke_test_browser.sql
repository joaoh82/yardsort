-- A project may name the browser its `browser` steps use, for when the finder's search of PATH
-- and the usual install folders does not turn one up, or turns up the wrong one. Empty means
-- "find one".
ALTER TABLE project_test_settings ADD COLUMN browser TEXT NOT NULL DEFAULT '';
