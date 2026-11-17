-- Deleting an account (docs/design/46-remote-r7.md §10). A sign-in that confirms a deletion, and
-- the one-time code it brings back, are marked as such: that code enrolls no device.
ALTER TABLE pending_logins ADD COLUMN confirm BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE handoffs ADD COLUMN confirm BOOLEAN NOT NULL DEFAULT FALSE;

-- The hashes of a deleted account's devices' credentials, each with when it would have lapsed,
-- and nothing else: linked to no account and no device. A device that asks with one is told its
-- account was deleted, rather than only that it is signed out. Swept once lapsed.
CREATE TABLE deleted_credentials (
    hash       BYTEA PRIMARY KEY,
    expires_at BIGINT NOT NULL
);
CREATE INDEX deleted_credentials_by_expiry ON deleted_credentials (expires_at);
