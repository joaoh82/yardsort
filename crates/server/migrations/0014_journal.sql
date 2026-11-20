-- The journal (docs/design/46-remote-r7.md §12, src/journal.rs): every revocation and deletion,
-- by a key that sorts by time, with its entry, which holds ids and a time and nothing else. A row
-- is made in the same transaction as what it records, sent to the journal outside the database
-- and marked written, and never deleted: the keys held are how a database restored from a backup
-- is told apart from a current one, and `yardsort-server reconcile` replays what it lacks. It
-- names no account by a column, and nothing cascades into it: a deleted account's id stays here
-- so that a restore cannot bring the account back.
CREATE TABLE journal (
    key     TEXT PRIMARY KEY,
    body    TEXT NOT NULL,
    written BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX journal_unwritten ON journal (key) WHERE NOT written;
