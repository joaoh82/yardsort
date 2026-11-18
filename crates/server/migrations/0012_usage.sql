-- What each account uses, a day at a time (docs/design/46-remote-r7.md §11). A day is a UTC
-- date, as days since 1970-01-01. Each instance adds what it counted every minute.
CREATE TABLE usage_days (
    account TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    day BIGINT NOT NULL,
    relay_bytes BIGINT NOT NULL DEFAULT 0,
    peak_streams BIGINT NOT NULL DEFAULT 0,
    sync_bytes BIGINT NOT NULL DEFAULT 0,
    active_devices BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (account, day)
);
CREATE INDEX usage_days_by_day ON usage_days (day);

-- Which devices made a request on a day, so each is counted once. Kept for that day and the next
-- only: the day's row keeps the count.
CREATE TABLE usage_devices (
    account TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    day BIGINT NOT NULL,
    device TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    PRIMARY KEY (account, day, device)
);
CREATE INDEX usage_devices_by_day ON usage_devices (day);
