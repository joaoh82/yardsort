-- Sync between an account's hosts (docs/design/42-remote-r6.md). Records are kept at their
-- latest revision, a deleted one as a tombstone without its body. A pruned tombstone stays as a
-- marker that pulls and snapshots no longer return, so an old create cannot bring it back. Every
-- change takes the account's next cursor. Bodies are the hosts' own JSON, kept as text and never
-- logged.

-- One row per account that has synced: the last change's cursor, and the floor below which a
-- cursor may have missed a tombstone that has since been pruned.
CREATE TABLE sync_heads (
    account TEXT PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    head BIGINT NOT NULL,
    floor BIGINT NOT NULL
);

CREATE TABLE sync_records (
    account TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    id TEXT NOT NULL,
    kind TEXT NOT NULL,
    project TEXT NOT NULL,
    revision BIGINT NOT NULL,
    schema INTEGER NOT NULL,
    body TEXT,
    revoked BOOLEAN NOT NULL,
    deleted BOOLEAN NOT NULL,
    -- The host whose op made this revision. Not a reference: the record outlives the host.
    host TEXT NOT NULL,
    changed_at BIGINT NOT NULL,
    cursor BIGINT NOT NULL,
    pruned BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (account, id)
);

CREATE INDEX sync_records_by_cursor ON sync_records (account, cursor);
CREATE INDEX sync_records_by_project ON sync_records (account, project, id);
CREATE INDEX sync_tombstones ON sync_records (changed_at) WHERE deleted AND NOT pruned;

-- The record of every change applied: what, by which host, and when. Never a body.
CREATE TABLE sync_changes (
    account TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    cursor BIGINT NOT NULL,
    kind TEXT NOT NULL,
    id TEXT NOT NULL,
    project TEXT NOT NULL,
    revision BIGINT NOT NULL,
    host TEXT NOT NULL,
    outcome TEXT NOT NULL,
    at BIGINT NOT NULL,
    PRIMARY KEY (account, cursor)
);

CREATE INDEX sync_changes_by_time ON sync_changes (at);

-- The ops applied, so that a retried push is answered as the first was.
CREATE TABLE sync_ops (
    account TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    op TEXT NOT NULL,
    revision BIGINT NOT NULL,
    cursor BIGINT NOT NULL,
    at BIGINT NOT NULL,
    PRIMARY KEY (account, op)
);

CREATE INDEX sync_ops_by_time ON sync_ops (at);

-- Which hosts sync which project. A link to a deleted project stays until its tombstone is
-- pruned, so each host pulls the tombstone.
CREATE TABLE sync_links (
    account TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    project TEXT NOT NULL,
    host TEXT NOT NULL REFERENCES hosts (id) ON DELETE CASCADE,
    linked_at BIGINT NOT NULL,
    PRIMARY KEY (account, project, host)
);

CREATE INDEX sync_links_by_host ON sync_links (host);
