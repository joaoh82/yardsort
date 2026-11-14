-- A label the user gives a conversation, so a row of tabs in one workspace says what each agent
-- is doing. Empty means none: the tab shows the agent's name, as it always did. Kept apart from
-- `title`, which is the start of the first message and is never edited.
ALTER TABLE sessions ADD COLUMN label TEXT NOT NULL DEFAULT '';
