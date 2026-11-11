-- Push tokens, per-account preferences, and the notices waiting for a phone.
-- A token is the secret the provider needs, so it is stored as itself and is
-- never returned by an API. A one-time approval or answer code is never stored.

CREATE TABLE push_tokens (
    device TEXT PRIMARY KEY REFERENCES devices (id) ON DELETE CASCADE,
    account TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    platform TEXT NOT NULL,
    token TEXT NOT NULL,
    updated_at BIGINT NOT NULL
);

CREATE INDEX push_tokens_by_account ON push_tokens (account);

CREATE TABLE notification_preferences (
    account TEXT PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    muted BOOLEAN NOT NULL,
    task BOOLEAN NOT NULL,
    review BOOLEAN NOT NULL,
    failure BOOLEAN NOT NULL,
    desktop_update BOOLEAN NOT NULL,
    workflow BOOLEAN NOT NULL
);

CREATE TABLE mobile_notices (
    id TEXT PRIMARY KEY,
    account TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    event_key TEXT NOT NULL,
    category TEXT NOT NULL,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    suppressed BOOLEAN NOT NULL,
    UNIQUE (account, event_key)
);

CREATE INDEX mobile_notices_by_account ON mobile_notices (account, created_at);

CREATE TABLE mobile_notice_deliveries (
    notice TEXT NOT NULL REFERENCES mobile_notices (id) ON DELETE CASCADE,
    device TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    state TEXT NOT NULL,
    queued_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    sent_at BIGINT,
    PRIMARY KEY (notice, device)
);

CREATE INDEX mobile_notice_deliveries_by_device ON mobile_notice_deliveries (device, state);
