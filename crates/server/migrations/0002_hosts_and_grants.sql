-- Hosts, the offers they show to let a device in, and the grants their people give. Times are
-- milliseconds since the Unix epoch. Offers are kept as hashes, as credentials are.

-- A machine that runs agents, on one account. `device` is the device it signs in as now; signing
-- in again moves it. A removed host keeps its row, so its id is never handed out again.
CREATE TABLE hosts (
    id         TEXT PRIMARY KEY,
    account    TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    device     TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    removed_at BIGINT
);
CREATE INDEX hosts_by_account ON hosts (account);

-- Five minutes at most, one open per host. `misses` counts the account's wrong codes while it
-- was open.
CREATE TABLE host_offers (
    secret     BYTEA PRIMARY KEY,
    code       BYTEA NOT NULL,
    host       TEXT NOT NULL REFERENCES hosts (id) ON DELETE CASCADE,
    account    TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    expires_at BIGINT NOT NULL,
    misses     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX host_offers_by_account ON host_offers (account);
CREATE INDEX host_offers_by_host ON host_offers (host);

-- One device's grant on one host, from when it was asked for. `status` is 'pending',
-- 'approved', 'denied' or 'revoked'; a pending one past `expires_at` has expired. Scopes are
-- spelled as on the wire; `projects` is the wire's `ProjectGrant` as JSON. A revoked grant keeps
-- what it gave.
CREATE TABLE grants (
    id         TEXT PRIMARY KEY,
    account    TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    host       TEXT NOT NULL REFERENCES hosts (id) ON DELETE CASCADE,
    device     TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    requested  TEXT[] NOT NULL,
    scopes     TEXT[] NOT NULL DEFAULT '{}',
    projects   TEXT,
    status     TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    decided_at BIGINT,
    CHECK (status <> 'approved' OR projects IS NOT NULL)
);
CREATE INDEX grants_by_host ON grants (host);
CREATE INDEX grants_by_device ON grants (device);
