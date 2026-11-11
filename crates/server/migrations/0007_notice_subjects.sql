-- The run and step a workflow notice is about, and the computer that submitted it.
-- A one-time code is never stored. The phone reads these when it opens the notice.
-- The push payload does not include them.

ALTER TABLE mobile_notices ADD COLUMN run_id TEXT;
ALTER TABLE mobile_notices ADD COLUMN step_id TEXT;
ALTER TABLE mobile_notices ADD COLUMN host_id TEXT;
