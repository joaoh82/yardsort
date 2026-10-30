-- The server's identity, accounts, devices, their credentials' hashes, and sign-ins in progress.
-- Times are milliseconds since the Unix epoch, as everywhere in the remote protocol. Credentials
-- and one-time values are SHA-256 hashes (`tokens::TokenHash`); nothing here opens anything.

-- One row: the id this server was given the first time it started.
CREATE TABLE server_identity (
    singleton  BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    id         TEXT NOT NULL,
    created_at BIGINT NOT NULL
);

-- An identity provider subject. Never an email address.
CREATE TABLE accounts (
    id         TEXT PRIMARY KEY,
    issuer     TEXT NOT NULL,
    subject    TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    UNIQUE (issuer, subject)
);

-- Between a device starting a sign-in and the provider answering: minutes at most.
CREATE TABLE pending_logins (
    state             BYTEA PRIMARY KEY,
    nonce             BYTEA NOT NULL,
    -- Sent to the provider with its code; the only value here that is not a hash.
    provider_verifier TEXT NOT NULL,
    device_challenge  TEXT NOT NULL,
    device_redirect   TEXT NOT NULL,
    expires_at        BIGINT NOT NULL
);

-- A signed-in person's one-time code, before the device redeems it: a minute at most.
CREATE TABLE handoffs (
    code             BYTEA PRIMARY KEY,
    account          TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    device_challenge TEXT NOT NULL,
    expires_at       BIGINT NOT NULL
);

CREATE TABLE devices (
    id             TEXT PRIMARY KEY,
    account        TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    name           TEXT NOT NULL,
    -- `DeviceKind` and `Platform`, as they are spelled on the wire.
    kind           TEXT NOT NULL,
    platform       TEXT NOT NULL,
    created_at     BIGINT NOT NULL,
    last_seen_at   BIGINT,
    revoked_at     BIGINT,
    -- `RevokeReason`: 'byOwner' or 'refreshReused'.
    revoked_reason TEXT,
    CHECK ((revoked_at IS NULL) = (revoked_reason IS NULL))
);
CREATE INDEX devices_by_account ON devices (account);

CREATE TABLE access_credentials (
    hash       BYTEA PRIMARY KEY,
    device     TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    expires_at BIGINT NOT NULL
);
CREATE INDEX access_credentials_by_device ON access_credentials (device);

-- A refresh credential is kept after it is traded in (`spent_at`), until it would have lapsed,
-- so that presenting it again is recognised as a copy.
CREATE TABLE refresh_credentials (
    hash       BYTEA PRIMARY KEY,
    device     TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    expires_at BIGINT NOT NULL,
    spent_at   BIGINT
);
CREATE INDEX refresh_credentials_by_device ON refresh_credentials (device);
