-- The whole first message of a conversation (`title` keeps only its start). Assist compares a
-- workspace's changes with what was asked; NULL for resumed, forked and adopted conversations,
-- and for every conversation started before this column existed.
ALTER TABLE sessions ADD COLUMN prompt TEXT;
