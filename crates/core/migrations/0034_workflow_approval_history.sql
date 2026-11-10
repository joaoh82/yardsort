-- A decision a rewind removed, so the next pass can be decided again without forgetting
-- who decided the earlier one. `attempt` is the step's attempt count when the rewind
-- happened. See docs/design/21-workflows.md.
CREATE TABLE workflow_approval_history (
    run_id TEXT NOT NULL,
    step_id TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    decision TEXT NOT NULL,
    feedback TEXT,
    actor TEXT,
    decided_at INTEGER,
    archived_at INTEGER NOT NULL,
    PRIMARY KEY (run_id, step_id, attempt)
);
