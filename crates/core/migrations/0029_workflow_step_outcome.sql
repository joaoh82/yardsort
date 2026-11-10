-- A step's structured outcome, as JSON: {"verdict":"passed"|"failed","reason":"..."}.
-- Null when the step produced none. Every row from before this column is null, and so is a
-- step that did not judge anything. A gate reads a failed verdict and fails the step; the
-- run stops with the reason. See docs/design/21-workflows.md.
ALTER TABLE workflow_step_runs ADD COLUMN outcome TEXT;
