-- Remote: this machine as a client of other hosts, a device of kind `computer` on one Yardsort
-- server, apart from this machine as a host (0018): the two are separate devices, each with its
-- own sign-in. Nothing is written until the person signs in under Settings → Remote → Other
-- computers, and signing out deletes it. The hosts it reaches are the server's grants, asked for
-- each time, though the server keeps those it has taken back or refused: Forget hides one here. Its credential is not here, as for the host.
-- See docs/design/35-remote-r4-r5.md.
CREATE TABLE remote_computer (
    singleton    INTEGER PRIMARY KEY CHECK (singleton = 1),
    -- The server's address, as the person typed it, and the id it gave itself.
    server_url   TEXT NOT NULL,
    server_id    TEXT NOT NULL,
    -- The device it signed in as, and what hosts call it.
    device_id    TEXT NOT NULL,
    name         TEXT NOT NULL,
    signed_in_at INTEGER NOT NULL
);

-- The pairings forgotten here, by their grant id: not shown again, though the server lists them.
-- A new pairing with the same host is a new grant, and shows. Signing out deletes them.
CREATE TABLE remote_computer_forgotten (
    grant_id TEXT PRIMARY KEY
);
