-- One round of questions an agent asked during a clarifying wait_session, and the
-- person's answer. The open round is also the workflow_waits row (kind question).
-- Rounds stay after that row is reused for the next question, so the outcome brief
-- can list every round. applied_at is set in the same change as sending the answer
-- back into the session, and a restart does not send it twice.
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
    -- Set when the answer has been handed to the driver to type into the session.
    -- Null until then. A restart types an answer whose applied_at is null once.
    applied_at INTEGER,
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
