-- How many times a workflow step has been entered, and one kept body per attempt of a
-- named artifact. A loop reads `attempts` and does not reset it. A restart does not add
-- one. An older body stays in `workflow_artifact_versions`, so a later attempt does not
-- overwrite it. The row in `workflow_artifacts` is the latest body a consumer reads.
-- See docs/design/21-workflows.md.
ALTER TABLE workflow_step_runs ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0;

ALTER TABLE workflow_artifacts ADD COLUMN attempt INTEGER NOT NULL DEFAULT 1;

CREATE TABLE workflow_artifact_versions (
    run_id TEXT NOT NULL,
    name TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    step_id TEXT NOT NULL,
    source TEXT NOT NULL,
    detail TEXT,
    body TEXT NOT NULL,
    truncated INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (run_id, name, attempt)
);
