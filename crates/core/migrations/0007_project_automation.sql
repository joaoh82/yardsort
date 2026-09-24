CREATE TABLE project_automation (
    project_id TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    config TEXT NOT NULL
);
