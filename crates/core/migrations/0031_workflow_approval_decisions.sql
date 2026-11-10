-- A person's decision on a workflow approval step, and the one-time code a terminal must
-- present. Written before the engine applies the decision. One row per step of a run.
-- The code itself is not stored: only a salt and a slow hash. `decision` stays null until
-- someone decides. The engine reads a row whose decision is set and records the same
-- decision as the step's outcome. A step that starts again deletes its row, so a later
-- revision gets a new code and can be decided once more. See docs/design/21-workflows.md.
CREATE TABLE workflow_approval_decisions (
    run_id TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    step_id TEXT NOT NULL,
    -- `approved`, `rejected` or `changes_requested`, once someone has decided. Null while
    -- the step is still waiting.
    decision TEXT CHECK (
        decision IS NULL
        OR decision IN ('approved', 'rejected', 'changes_requested')
    ),
    feedback TEXT,
    actor TEXT,
    decided_at INTEGER,
    -- Salt and PBKDF2-HMAC-SHA256 of the one-time code. The code is never written here.
    code_salt BLOB NOT NULL,
    code_hash BLOB NOT NULL,
    -- Wrong codes tried from a terminal. At 5 the terminal stops until the app shows a new code.
    attempts INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (run_id, step_id)
);
