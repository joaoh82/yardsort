-- What a workflow run found out about where it runs, kept with it as a JSON object: the
-- workspace's pull request when the run was queued, and the workspace's own agent when it
-- started. A run's `{{ pr.… }}` and `session: origin` mean these, however long it runs and
-- whatever happens in the workspace meanwhile. See docs/design/21-workflows.md.
ALTER TABLE workflow_runs ADD COLUMN context TEXT NOT NULL DEFAULT '{}';
