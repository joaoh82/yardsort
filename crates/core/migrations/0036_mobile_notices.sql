-- Notices waiting to be handed to the Yardsort server for the phone.
-- The words are chosen on the server. A one-time code is never stored here.

CREATE TABLE mobile_notice_outbox (
    event_key TEXT PRIMARY KEY,
    category TEXT NOT NULL,
    kind TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    state TEXT NOT NULL
);

-- What this computer has already seen, so the first look does not notify
-- about every open task and every review.
CREATE TABLE mobile_notice_seen (
    item_key TEXT PRIMARY KEY,
    stamp TEXT NOT NULL,
    seen_at INTEGER NOT NULL
);
