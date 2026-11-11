-- A human wait on a workflow step. One row per step of a run, opened when the step
-- starts waiting on a person and kept after the engine applies it, so a restart can
-- see that the wait was already taken. `kind` is `approval` today. `question` is the
-- same row for a clarification (YARD-7): the question is `prompt` and the reply is
-- `answer`, with no second table. A phone or a remote crate lists these rows and
-- records a decision through the core. This migration adds no remote route.
-- See docs/design/21-workflows.md.
--
-- `code_rounds` is how many PBKDF2 rounds `code_hash` was made with. Rows from before
-- this were hashed at 100000. New codes use 600000. The code itself is still not stored.
CREATE TABLE workflow_waits (
    run_id TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    step_id TEXT NOT NULL,
    -- `approval` or `question`. A question is a clarification, not a second mechanism.
    kind TEXT NOT NULL CHECK (kind IN ('approval', 'question')),
    -- What the run view and `ys` say the run is waiting for: "your approval", "your answer".
    blocked_on TEXT NOT NULL,
    -- What the person should do, in one sentence.
    action_needed TEXT NOT NULL,
    prompt TEXT,
    -- JSON array of artifact names. The bodies stay in workflow_artifacts.
    artifact_refs TEXT NOT NULL DEFAULT '[]',
    -- JSON: workflow, workspace, step and action. Enough to say where the run is
    -- without parsing the workflow file.
    phase TEXT NOT NULL DEFAULT '{}',
    -- A clarification's answer. An approval's decision stays on
    -- workflow_approval_decisions, with the code, the lockout and the hash.
    answer TEXT,
    actor TEXT,
    answered_at INTEGER,
    -- Set in the same change as the engine applying the wait. Null until then.
    -- A restart applies a wait whose applied_at is null exactly once.
    applied_at INTEGER,
    -- Set when the waiting notification was sent. A restart does not send it again.
    -- A replaced code sends its own notice and does not clear this.
    notified_at INTEGER,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (run_id, step_id)
);

ALTER TABLE workflow_approval_decisions
    ADD COLUMN code_rounds INTEGER NOT NULL DEFAULT 100000;
