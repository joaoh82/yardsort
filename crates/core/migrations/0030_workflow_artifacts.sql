-- Named documents a step hands to later steps of the same run. One row per name.
-- A step that succeeds again replaces its rows, so a consumer reads the latest body.
-- The body is capped. A transcript is stored only when the workflow file asks for one.
-- See docs/design/21-workflows.md.
CREATE TABLE workflow_artifacts (
    run_id TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    step_id TEXT NOT NULL,
    source TEXT NOT NULL,
    detail TEXT,
    body TEXT NOT NULL,
    truncated INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (run_id, name)
);

CREATE INDEX workflow_artifacts_step ON workflow_artifacts(run_id, step_id);
