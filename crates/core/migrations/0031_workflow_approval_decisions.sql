-- A person's decision on a workflow approval step, written before the engine applies it.
-- One row per step of a run. The engine reads it on a later tick and records the same
-- decision as the step's outcome. A step that starts again deletes its row, so a later
-- revision can be decided once more. A second insert for the same step does not replace
-- the first. See docs/design/21-workflows.md.
CREATE TABLE workflow_approval_decisions (
    run_id TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    step_id TEXT NOT NULL,
    -- `approved`, `rejected` or `changes_requested`.
    decision TEXT NOT NULL CHECK (decision IN ('approved', 'rejected', 'changes_requested')),
    feedback TEXT,
    actor TEXT NOT NULL,
    decided_at INTEGER NOT NULL,
    PRIMARY KEY (run_id, step_id)
);
