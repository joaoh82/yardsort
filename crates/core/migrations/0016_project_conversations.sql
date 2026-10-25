-- Whether a project's agents may read the other conversations in their workspace: what the user
-- typed and what each agent replied, read from the agents' own transcripts when asked and never
-- stored here. Off until the user turns it on; a project with no row here is off, because the
-- text goes to another agent's provider. See docs/design/29-conversation-context.md.
CREATE TABLE project_conversations (
    project_id      TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    share           INTEGER NOT NULL DEFAULT 0
);
