-- When a conversation first began. `started_at` moves to the latest start each time the
-- conversation is resumed, which is what the sessions list wants; reading the conversation back
-- needs the first: a fork's own turns are the ones after it, and a Codex session file is found by
-- when it began. Rows written before this get the earliest start known: their first capture run,
-- if any began sooner. See docs/design/29-conversation-context.md.
ALTER TABLE sessions ADD COLUMN began_at INTEGER;
UPDATE sessions
   SET began_at = MIN(started_at,
                      COALESCE((SELECT MIN(started_at) FROM agent_runs
                                 WHERE agent_runs.session_id = sessions.id), started_at));
