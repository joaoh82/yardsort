-- Sync between this installation and the account's other hosts (docs/design/42-remote-r6.md). Only
-- this installation's own bookkeeping: what each synced record is called on the server, what is
-- waiting to go up, and how far it has pulled. The records themselves stay in their own tables.

-- A local record's logical id on the server, beside its own id, never replacing it. A row is a
-- record this installation syncs: for a project, that it is turned on or linked here.
CREATE TABLE sync_ids (
    kind TEXT NOT NULL,
    local_id TEXT NOT NULL,
    sync_id TEXT NOT NULL UNIQUE,
    -- The logical id of the project the record belongs to; a project's own.
    project TEXT NOT NULL,
    -- The server's revision of it, as last heard; NULL until the server has it.
    revision INTEGER,
    -- Linked here and not yet read from a snapshot.
    fresh INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (kind, local_id)
);

CREATE INDEX sync_ids_by_project ON sync_ids (project);

-- Ops waiting to go up, in the order they were made. A refused op is held, with the server's
-- reason, for the person to see; it is never retried by itself.
CREATE TABLE sync_outbox (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    op TEXT NOT NULL UNIQUE,
    action TEXT NOT NULL,
    kind TEXT NOT NULL,
    sync_id TEXT NOT NULL,
    project TEXT NOT NULL,
    base INTEGER,
    schema INTEGER,
    body TEXT,
    created_at INTEGER NOT NULL,
    refused TEXT
);

-- How far this host has pulled, and how its last pass went. One row, for the host Remote is on
-- as; turning Remote off, or moving to another server, clears every sync table.
CREATE TABLE sync_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    cursor INTEGER NOT NULL DEFAULT 0,
    synced_at INTEGER,
    problem TEXT
);
