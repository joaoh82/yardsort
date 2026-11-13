-- The pull requests of a run's own repository that were opened on its workspace's branch while
-- the run was going, as the app saw them each time it read the project's pull requests. Kept
-- here so `ys` and the phone can say what a run made without asking GitHub. The ones a
-- workspace tracks in other repositories are not copied here: they are read from
-- `tracked_pull_requests` by when they were tracked. See `workflow::made`.
CREATE TABLE workflow_run_pull_requests (
    run_id    TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    number    INTEGER NOT NULL,
    url       TEXT NOT NULL,
    title     TEXT NOT NULL,
    -- As the forge said at the last look.
    state     TEXT NOT NULL CHECK (state IN ('open', 'merged', 'closed')),
    draft     INTEGER NOT NULL DEFAULT 0,
    -- When the forge says it was opened.
    opened_at INTEGER NOT NULL,
    PRIMARY KEY (run_id, number)
);
