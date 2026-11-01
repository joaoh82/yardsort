-- Smoke tests, slice 3: what a project asks of its runs, set in the app under the project's
-- settings. One row per project that changed anything; a project with no row has the defaults.
CREATE TABLE project_test_settings (
    project_id     TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    -- Show every run's steps and ask, whether or not the file was approved before.
    ask_every_time INTEGER NOT NULL DEFAULT 0,
    -- Ended runs older than this many days are removed, with their output.
    keep_days      INTEGER NOT NULL DEFAULT 30
);
