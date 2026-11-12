-- An agent notice: the workspace and the terminal it opens on the phone, as the
-- computer's own ids, never a title or a path. The push payload does not include them.
-- And whether agents that need a person reach the phone, on unless turned off.

ALTER TABLE mobile_notices ADD COLUMN workspace_id TEXT;
ALTER TABLE mobile_notices ADD COLUMN terminal_id TEXT;
ALTER TABLE notification_preferences ADD COLUMN agent BOOLEAN NOT NULL DEFAULT TRUE;
