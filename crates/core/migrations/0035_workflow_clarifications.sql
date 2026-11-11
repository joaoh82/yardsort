-- One round of questions an agent asked during a clarifying wait_session, and the
-- person's answer. The open round is also the workflow_waits row (kind question).
-- Rounds stay after that row is reused for the next question, so the outcome brief
-- can list every round.
--
-- applied_at is the claim, taken before the keystrokes. typed_at is set only after
-- the answer was typed into the session. A restart that finds applied_at set and
-- typed_at null does not type again: the step fails, because the keystrokes may
-- already have landed. A session that was busy rolls the claim back, and the next
-- tick tries again.
--
-- The code columns on a question wait are the same scheme as an approval: salt,
-- hash, round count and a wrong-try count. The plain code is not stored. A rewind
-- copies rounds and the summary into the history tables, then removes the live
-- rows, the way an approval decision is archived.
-- See docs/design/21-workflows.md.
CREATE TABLE workflow_clarification_rounds (
    run_id TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    step_id TEXT NOT NULL,
    round INTEGER NOT NULL,
    -- JSON array of { "id", "prompt" }. The agent names the questions. The person
    -- answers the round in one reply.
    questions TEXT NOT NULL,
    answer TEXT,
    actor TEXT,
    asked_at INTEGER NOT NULL,
    answered_at INTEGER,
    -- Set when the answer has been claimed for typing into the session. Null until
    -- then. A restart types an answer whose applied_at is null once.
    applied_at INTEGER,
    -- Set only after typing returns done. A claim with this still null is not typed
    -- again; the step fails.
    typed_at INTEGER,
    PRIMARY KEY (run_id, step_id, round)
);

-- The agent's written outcome for that step, recorded with `ys workflow summary`.
-- One per step. The brief is this plus the rounds, not text scraped from the terminal.
CREATE TABLE workflow_clarification_summaries (
    run_id TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    step_id TEXT NOT NULL,
    summary TEXT NOT NULL,
    recorded_at INTEGER NOT NULL,
    PRIMARY KEY (run_id, step_id)
);

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
