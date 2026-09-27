-- Project memory, stage 5: short lessons about a project that its agents should know, each one
-- written by the user or proposed by an agent and approved by the user. See
-- docs/design/09-agent-events-and-memory.md and 19-agent-events-stage-5-memory.md.
--
-- Only an approved entry ever reaches an agent. A candidate is an agent's proposal waiting for
-- the user; rejecting one keeps it, so the same proposal is recognised when it comes back;
-- revoking an approved entry takes it out of every future prompt without forgetting it.
CREATE TABLE memory_entries (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    -- One paragraph, at most a few hundred characters: a note, not a document.
    text            TEXT NOT NULL,
    state           TEXT NOT NULL CHECK (state IN ('candidate', 'approved', 'rejected', 'revoked')),
    -- Who wrote it: the user, in the app, or an agent through `ys memory propose`.
    author          TEXT NOT NULL CHECK (author IN ('user', 'agent')),
    -- Where an agent's proposal came from, when the launch environment said so. The names are
    -- kept beside the ids, so the citation survives the workspace and the run.
    harness_id      TEXT,
    workspace_id    TEXT REFERENCES workspaces(id) ON DELETE SET NULL,
    workspace_name  TEXT,
    run_id          TEXT REFERENCES agent_runs(id) ON DELETE SET NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE INDEX memory_entries_by_project ON memory_entries (project_id, state);

-- Every change to an entry, oldest first: who did what, and the text an edit replaced.
CREATE TABLE memory_history (
    seq             INTEGER PRIMARY KEY AUTOINCREMENT,
    entry_id        TEXT NOT NULL REFERENCES memory_entries(id) ON DELETE CASCADE,
    at              INTEGER NOT NULL,
    action          TEXT NOT NULL CHECK (action IN
                        ('proposed', 'written', 'approved', 'edited', 'rejected', 'revoked', 'restored')),
    by              TEXT NOT NULL CHECK (by IN ('user', 'agent')),
    previous_text   TEXT
);

-- Whether a project's approved entries go into its agents' first messages. Off until the user
-- turns it on; a project with no row here is off.
CREATE TABLE project_memory (
    project_id      TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    share           INTEGER NOT NULL DEFAULT 0
);
