-- Approved notes sync between an account's hosts (docs/design/42-remote-r6.md, slice 3). A note
-- that came from another host says which, and a note two hosts edited at once keeps the text
-- that lost beside the one that stands, until the person settles it.

-- The host the note was written or approved on, its id and its name; NULL for this
-- installation's own.
ALTER TABLE memory_entries ADD COLUMN from_host_id TEXT;
ALTER TABLE memory_entries ADD COLUMN from_host TEXT;

-- A conflict waiting for the person: the other text, which host it came from and when. The
-- note's own text is the one that stands, and the one agents get, until it is settled. The id is
-- the same on every host, so two hosts settling it at once settle it once.
ALTER TABLE memory_entries ADD COLUMN conflict_id TEXT;
ALTER TABLE memory_entries ADD COLUMN conflict_text TEXT;
ALTER TABLE memory_entries ADD COLUMN conflict_from TEXT;
ALTER TABLE memory_entries ADD COLUMN conflict_at INTEGER;
