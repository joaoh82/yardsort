//! Asking for a run, and what a run can know about where it runs.
//!
//! [`queue`] is the one door for a run, whoever asks — `ys` today, the app's Run dialog later.
//! Everything that can be checked before anything happens is checked here, and a refusal writes
//! nothing: the workflow must be ready, this version must be able to carry out every step, the
//! workspace must be there, and every input must be given and make sense.

use std::collections::BTreeMap;
use std::path::Path;

use super::engine::Inputs;
use super::{find, template, Action, InputKind, SessionRef, Workflow};
use crate::error::{IpcError, IpcResult};
use crate::store::{NewWorkflowRun, ProjectRow, Queued, Store, WorkspaceRow};

/// A request for a run.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    pub workflow_id: &'a str,
    pub workspace_id: &'a str,
    /// As given, by input id. Checked against what the workflow asks for.
    pub inputs: &'a BTreeMap<String, String>,
    /// `app` or `cli`.
    pub requested_by: &'a str,
}

/// Queue a run and return its id. `harness` checks that an agent can be started: it returns the
/// harness's id as Yardsort spells it, or why not.
pub fn queue(
    store: &Store,
    data_dir: &Path,
    request: &Request<'_>,
    harness: &dyn Fn(&str) -> Result<String, String>,
) -> IpcResult<String> {
    let entry = find(data_dir, request.workflow_id).ok_or_else(|| {
        IpcError::new(
            "workflow_not_found",
            format!("There is no workflow `{}`.", request.workflow_id),
        )
    })?;
    let Some(workflow) = entry.workflow.as_ref().filter(|_| entry.runnable()) else {
        return Err(IpcError::new(
            "workflow_invalid",
            format!(
                "`{}` has problems and cannot run. `ys workflow show {}` lists them.",
                entry.id, entry.id
            ),
        ));
    };
    if let Some(why) = unsupported(workflow) {
        return Err(IpcError::new("workflow_unsupported", why));
    }
    let workspace = store
        .workspace(request.workspace_id)?
        .filter(|w| !w.forgotten)
        .ok_or_else(|| IpcError::new("workspace_not_found", "That workspace is not there."))?;
    if workspace.archived {
        return Err(IpcError::new(
            "workspace_archived",
            format!(
                "`{}` is archived. Restore it before running a workflow in it.",
                workspace.name
            ),
        ));
    }
    let inputs = check_inputs(workflow, request.inputs, harness)
        .map_err(|why| IpcError::new("workflow_inputs", why))?;
    for step in &workflow.steps {
        if let Action::StartSession { harness: named, .. } = &step.action {
            if !named.contains("{{") {
                harness(named).map_err(|why| IpcError::new("harness_not_found", why))?;
            }
        }
    }

    let id = uuid::Uuid::new_v4().to_string();
    let inputs_json = serde_json::to_string(&inputs).unwrap_or_else(|_| "{}".to_owned());
    let steps: Vec<(&str, &str)> = workflow
        .steps
        .iter()
        .map(|step| (step.id.as_str(), step.action.name()))
        .collect();
    let queued = store.queue_workflow_run(&NewWorkflowRun {
        id: &id,
        workflow_id: &workflow.id,
        workflow_name: &workflow.name,
        definition: &entry.text,
        project_id: &workspace.project_id,
        workspace_id: &workspace.id,
        workspace_name: &workspace.name,
        inputs: &inputs_json,
        requested_by: request.requested_by,
        steps: &steps,
    })?;
    match queued {
        Queued::Queued => Ok(id),
        Queued::AlreadyRunning(other) => Err(IpcError::new(
            "workflow_already_running",
            format!(
                "`{}` is already running in `{}` (run {}). Wait for it, or cancel it.",
                workflow.id,
                workspace.name,
                crate::memory::short_id(&other)
            ),
        )),
    }
}

/// Why this version cannot carry out `workflow`, when it cannot. Pull requests and the
/// workspace's own agent come in the next version.
pub fn unsupported(workflow: &Workflow) -> Option<String> {
    let later = |what: &str| {
        Some(format!(
            "`{}` {what}, which this version of Yardsort cannot do yet.",
            workflow.id
        ))
    };
    for step in &workflow.steps {
        match &step.action {
            Action::WaitPrActivity { .. } => {
                return later("waits for activity on a pull request");
            }
            Action::WaitSession {
                session: SessionRef::Origin,
                ..
            }
            | Action::SendToSession {
                session: SessionRef::Origin,
                ..
            } => return later("talks to the workspace's own agent (`session: origin`)"),
            _ => {}
        }
        if texts(&step.action)
            .iter()
            .flat_map(|text| template::placeholders(text))
            .any(|var| var.path.first().is_some_and(|n| n == "pr"))
        {
            return later("uses the workspace's pull request (`{{ pr.… }}`)");
        }
    }
    None
}

/// Every piece of template text a step has.
fn texts(action: &Action) -> Vec<&str> {
    match action {
        Action::StartSession {
            harness,
            prompt,
            model,
            effort,
            ..
        } => [
            Some(harness.as_str()),
            prompt.as_deref(),
            model.as_deref(),
            effort.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect(),
        Action::SendToSession { prompt, .. } => vec![prompt],
        Action::Notify { title, body } => [Some(title.as_str()), body.as_deref()]
            .into_iter()
            .flatten()
            .collect(),
        Action::WaitSession { .. } | Action::WaitPrActivity { .. } => Vec::new(),
    }
}

/// The inputs a run gets: what was given, defaults for the rest. Refuses unknown inputs, missing
/// required ones, a choice that is not an option, and an agent that cannot be started.
pub fn check_inputs(
    workflow: &Workflow,
    given: &BTreeMap<String, String>,
    harness: &dyn Fn(&str) -> Result<String, String>,
) -> Result<Inputs, String> {
    if let Some(unknown) = given
        .keys()
        .find(|key| !workflow.inputs.iter().any(|i| &i.id == *key))
    {
        let asks = if workflow.inputs.is_empty() {
            "It asks for none.".to_owned()
        } else {
            format!(
                "It asks for {}.",
                workflow
                    .inputs
                    .iter()
                    .map(|i| format!("`{}`", i.id))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        return Err(format!(
            "`{}` has no input `{unknown}`. {asks}",
            workflow.id
        ));
    }
    let mut inputs = Inputs::new();
    let mut missing = Vec::new();
    for input in &workflow.inputs {
        let value = given
            .get(&input.id)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
            .or_else(|| input.default.clone());
        let Some(value) = value else {
            if input.required {
                missing.push(format!("`{}` ({})", input.id, input.label));
            }
            continue;
        };
        let value = match input.kind {
            InputKind::Choice if !input.options.contains(&value) => {
                return Err(format!(
                    "`{value}` is not one of the answers to `{}`: {}.",
                    input.id,
                    input.options.join(", ")
                ))
            }
            InputKind::Harness => harness(&value)?,
            _ => value,
        };
        inputs.insert(input.id.clone(), value);
    }
    if !missing.is_empty() {
        return Err(format!("`{}` needs {}.", workflow.id, missing.join(", ")));
    }
    Ok(inputs)
}

/// What a run knows about where it runs, for its `{{ project.… }}` and `{{ workspace.… }}`.
#[derive(Debug, Clone)]
pub struct Place {
    pub project: ProjectRow,
    pub workspace: WorkspaceRow,
    /// The workspace's first message, if it was started with one.
    pub task: Option<String>,
}

impl Place {
    pub fn load(store: &Store, workspace_id: &str) -> IpcResult<Option<Self>> {
        let Some(workspace) = store.workspace(workspace_id)? else {
            return Ok(None);
        };
        let Some(project) = store.project(&workspace.project_id)? else {
            return Ok(None);
        };
        let task = store.session_prompts(workspace_id)?.into_iter().next();
        Ok(Some(Self {
            project,
            workspace,
            task,
        }))
    }

    /// A `project.…` or `workspace.…` variable. Others are the caller's.
    pub fn var(&self, path: &[String]) -> Option<String> {
        let path: Vec<&str> = path.iter().map(String::as_str).collect();
        let w = &self.workspace;
        match path.as_slice() {
            ["project", "name"] => Some(self.project.name.clone()),
            ["project", "root"] => Some(self.project.root_path.clone()),
            ["workspace", "name"] => Some(w.name.clone()),
            ["workspace", "path"] => Some(w.path.clone()),
            ["workspace", "branch"] => w.branch.clone(),
            ["workspace", "base_branch"] => w.base_branch.clone(),
            ["workspace", "task"] => self.task.clone(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
