-- Clarification delivery and the one-time code an answer from a terminal needs.
--
-- typed_at is set only after the answer was typed into the session. applied_at is
-- the claim, taken before the keystrokes. A restart that finds applied_at set and
-- typed_at null does not type again: the step fails, because the keystrokes may
-- already have landed. A session that was busy rolls the claim back, and the next
-- tick tries again.
--
-- The code columns on a question wait are the same scheme as an approval: salt,
-- hash, round count and a wrong-try count. The plain code is not stored. A rewind
-- copies rounds and the summary into the history tables, then removes the live
-- rows, the way an approval decision is archived.
-- See docs/design/21-workflows.md.
ALTER TABLE workflow_clarification_rounds ADD COLUMN typed_at INTEGER;

ALTER TABLE workflow_waits ADD COLUMN code_salt BLOB;
ALTER TABLE workflow_waits ADD COLUMN code_hash BLOB;
ALTER TABLE workflow_waits ADD COLUMN code_rounds INTEGER NOT NULL DEFAULT 100000;
ALTER TABLE workflow_waits ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0;

CREATE TABLE workflow_clarification_history (
    run_id TEXT NOT NULL,
    step_id TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    round INTEGER NOT NULL,
    questions TEXT NOT NULL,
    answer TEXT,
    actor TEXT,
    asked_at INTEGER NOT NULL,
    answered_at INTEGER,
    applied_at INTEGER,
    typed_at INTEGER,
    archived_at INTEGER NOT NULL,
    PRIMARY KEY (run_id, step_id, attempt, round)
);

CREATE TABLE workflow_clarification_summary_history (
    run_id TEXT NOT NULL,
    step_id TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    summary TEXT NOT NULL,
    recorded_at INTEGER NOT NULL,
    archived_at INTEGER NOT NULL,
    PRIMARY KEY (run_id, step_id, attempt)
);
