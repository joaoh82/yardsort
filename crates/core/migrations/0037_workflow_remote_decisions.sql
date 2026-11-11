-- A phone's decision on a workflow, and which device made it.
-- The one-time code is still not stored. `remote_key` is the decision id from the
-- phone, so a retry returns the same outcome and does not apply again.
-- A rewind copies `device_id` into the history row. See docs/design/41-mobile-workflow-controls.md.

ALTER TABLE workflow_approval_decisions ADD COLUMN device_id TEXT;
ALTER TABLE workflow_approval_decisions ADD COLUMN remote_key TEXT;

ALTER TABLE workflow_approval_history ADD COLUMN device_id TEXT;

ALTER TABLE workflow_waits ADD COLUMN device_id TEXT;
ALTER TABLE workflow_waits ADD COLUMN remote_key TEXT;

ALTER TABLE workflow_clarification_rounds ADD COLUMN device_id TEXT;
ALTER TABLE workflow_clarification_history ADD COLUMN device_id TEXT;

-- One remote decision, including a refusal. The key is the phone and its decision id,
-- so two phones do not share a retry. `fingerprint` hashes the run, the device, the
-- action, the step and the note. It does not include the one-time code. A retry of
-- the same key returns `outcome` and does not write again, including after a rewind.
-- Rows older than seven days are removed when a new one is stored.
CREATE TABLE workflow_remote_actions (
    device_id TEXT NOT NULL,
    decision_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    step_id TEXT NOT NULL,
    outcome TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (device_id, decision_id)
);

-- The run and step a workflow notice is about, so the phone can open that decision.
-- A one-time code is never stored here.
ALTER TABLE mobile_notice_outbox ADD COLUMN run_id TEXT;
ALTER TABLE mobile_notice_outbox ADD COLUMN step_id TEXT;
