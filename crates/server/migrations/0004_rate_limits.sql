-- What is left of each rate limit for each address, shared by every instance of the server. The
-- address is kept as a hash of it (or of its IPv6 /64), and a row untouched for a minute is full
-- again and is swept, so an address stays here a minute or two after its last request.
CREATE TABLE rate_limits (
    gate    TEXT NOT NULL,
    key     BYTEA NOT NULL,
    tokens  DOUBLE PRECISION NOT NULL,
    at      BIGINT NOT NULL,
    -- Whether the latest request was let through.
    taken   BOOLEAN NOT NULL,
    PRIMARY KEY (gate, key)
);
CREATE INDEX rate_limits_by_at ON rate_limits (at);
