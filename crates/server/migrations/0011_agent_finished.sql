-- Whether an agent that finished, and was not looked at, reaches the phone: the `agent`
-- category's `finished` kind, with a switch of its own. On unless turned off.

ALTER TABLE notification_preferences ADD COLUMN agent_finished BOOLEAN NOT NULL DEFAULT TRUE;
