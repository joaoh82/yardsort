-- Where each connected host's relay link is held: which instance of the server, and which link,
-- so that a device reaching another instance is sent to that one. The instance notes its links
-- again every few seconds; a row it stopped noting is from an instance that went away. Nothing
-- the relay carries is here.
CREATE TABLE relay_links (
    host     TEXT PRIMARY KEY REFERENCES hosts (id) ON DELETE CASCADE,
    instance TEXT NOT NULL,
    link     TEXT NOT NULL,
    seen_at  BIGINT NOT NULL
);
CREATE INDEX relay_links_by_instance ON relay_links (instance);
