-- A refresh credential traded in a moment ago may be presented again by a device whose answer was
-- lost on the way. That is told apart from a copy by what was issued in its place: the retry is
-- accepted, once, only while neither of those has been used (27 §21).
ALTER TABLE refresh_credentials ADD COLUMN successor_refresh BYTEA;
ALTER TABLE refresh_credentials ADD COLUMN successor_access BYTEA;
ALTER TABLE refresh_credentials ADD COLUMN retried BOOLEAN NOT NULL DEFAULT FALSE;
-- Whether an access credential was ever presented.
ALTER TABLE access_credentials ADD COLUMN used BOOLEAN NOT NULL DEFAULT FALSE;
