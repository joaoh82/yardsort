-- The workspace and the terminal an agent notice opens on the phone: this computer's
-- own ids, never a title, a path or anything the terminal showed.

ALTER TABLE mobile_notice_outbox ADD COLUMN workspace_id TEXT;
ALTER TABLE mobile_notice_outbox ADD COLUMN terminal_id TEXT;
