-- Remote access: this machine as a host on one Yardsort server, and the grants its person
-- approved here. Nothing is written until Remote is turned on, and turning it off deletes both.
-- The host's credentials are not here: they are in the OS credential store, or a credentials
-- directory for a headless host. See docs/design/27-remote-r2.md.
CREATE TABLE remote_host (
    singleton  INTEGER PRIMARY KEY CHECK (singleton = 1),
    -- The server's address, as the person typed it, and the id it gave itself.
    server_url TEXT NOT NULL,
    server_id  TEXT NOT NULL,
    -- What the server calls this host, and the device it signed in as.
    host_id    TEXT NOT NULL,
    device_id  TEXT NOT NULL,
    name       TEXT NOT NULL,
    enabled_at INTEGER NOT NULL
);

-- Written only when this host's person approves a device, never from the server's word alone: a
-- device the server says may come in, and that is not here, is refused. One per device.
CREATE TABLE remote_grants (
    id          TEXT PRIMARY KEY,
    device_id   TEXT NOT NULL UNIQUE,
    device_name TEXT NOT NULL,
    -- JSON, as on the wire: an array of scopes, and a `ProjectGrant`.
    scopes      TEXT NOT NULL,
    projects    TEXT NOT NULL,
    approved_at INTEGER NOT NULL
);
