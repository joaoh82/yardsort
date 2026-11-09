//! Asking for a run, and what a run can know about where it runs.
//!
//! [`queue`] is the one door for a run, whoever asks — `ys` today, the app's Run dialog later.
//! Everything that can be checked before anything happens is checked here, and a refusal writes
//! nothing: the workflow must be ready, the workspace must be there, every input must be given
//! and make sense, and a workflow that works with the pull request needs the workspace to have
//! one open.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::engine::Inputs;
use super::{find, template, Action, InputKind, SessionRef, Workflow};
use crate::error::{IpcError, IpcResult};
use crate::forge::PullRequest;
use crate::git::Git;
use crate::store::{NewWorkflowRun, ProjectRow, Queued, Store, WorkspaceRow};

/// What queuing has to find out from outside: whoever asks for a run answers these.
pub trait Look {
    /// Whether an agent can be started: its id as Yardsort spells it, or why not.
    fn harness(&self, wanted: &str) -> Result<String, String>;
    /// The workspace's open pull request, `None` when it has none, or why the forge could not
    /// be asked.
    fn pull_request(&self, workspace: &WorkspaceRow) -> Result<Option<PullRequest>, String>;
    /// The workspace's pull request as a trigger watches it: pull request `number`, or the open
    /// one for its branch when `None`; `None` when it has none. A door that does not ask the
    /// forge about this says so.
    fn watch_pull_request(
        &self,
        _workspace: &WorkspaceRow,
        _number: Option<u32>,
    ) -> Result<Option<crate::forge::PrLook>, String> {
        Err("Pull requests are not watched from here.".to_owned())
    }
}

/// What a run found out about where it runs, kept with it (`workflow_runs.context`). A run's
/// `{{ pr.… }}` and `session: origin` mean these for as long as it runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Facts {
    /// The workspace's open pull request when the run was queued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<PrFacts>,
    /// The terminal id of the workspace's own agent when the run started: the session
    /// `origin_record` names when it is given, or else the newest live harness session, before
    /// the run started any of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// The conversation record `session: origin` must be, when the run was asked for on behalf
    /// of one agent session: a session trigger's. Its live terminal becomes `origin` when the
    /// run starts, whichever session is newest then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_record: Option<String>,
    /// A schedule started the run, so nobody is there: a `run_test` step whose test would wait
    /// for a person fails at once, where a run somebody started waits with them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unattended: bool,
    /// How many triggered runs led to this one: 0 for a run a person or a schedule asked for.
    /// What stops a chain of triggers (see `trigger::MOST_DEPTH`).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub depth: i64,
    /// What the event that started the run left, for `{{ event.* }}`. Empty for a run nobody's
    /// event started.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub event: BTreeMap<String, String>,
}

fn is_zero(depth: &i64) -> bool {
    *depth == 0
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PrFacts {
    pub number: u32,
    pub url: String,
    pub title: String,
}

impl Facts {
    pub fn parse(json: &str) -> Self {
        serde_json::from_str(json).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_owned())
    }

    /// A `pr.…` or `event.…` variable.
    pub fn var(&self, path: &[String]) -> Option<String> {
        if let [namespace, field] = path {
            if namespace == "event" {
                return self.event.get(field).cloned();
            }
        }
        let pr = self.pr.as_ref()?;
        match path {
            [namespace, field] if namespace == "pr" => match field.as_str() {
                "number" => Some(pr.number.to_string()),
                "url" => Some(pr.url.clone()),
                "title" => Some(pr.title.clone()),
                _ => None,
            },
            _ => None,
        }
    }
}

/// A request for a run.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    pub workflow_id: &'a str,
    pub workspace_id: &'a str,
    /// As given, by input id. Checked against what the workflow asks for.
    pub inputs: &'a BTreeMap<String, String>,
    /// `app` or `cli`.
    pub requested_by: &'a str,
    /// Nobody is there to watch it: a schedule is asking. See [`Facts::unattended`].
    pub unattended: bool,
    /// The conversation record `session: origin` must be. See [`Facts::origin_record`].
    pub origin_record: Option<&'a str>,
    /// See [`Facts::depth`].
    pub depth: i64,
    /// What the event that asked for it left. See [`Facts::event`].
    pub event: Option<&'a BTreeMap<String, String>>,
}

/// Queue a run and return its id.
pub fn queue(
    store: &Store,
    data_dir: &Path,
    request: &Request<'_>,
    look: &dyn Look,
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
    let harness = |wanted: &str| look.harness(wanted);
    let inputs = check_inputs(workflow, request.inputs, &harness)
        .map_err(|why| IpcError::new("workflow_inputs", why))?;
    for step in &workflow.steps {
        if let Action::StartSession { harness: named, .. } = &step.action {
            if !named.contains("{{") {
                look.harness(named)
                    .map_err(|why| IpcError::new("harness_not_found", why))?;
            }
        }
    }
    // Last, because it asks the forge over the network.
    let mut facts = Facts {
        unattended: request.unattended,
        origin_record: request.origin_record.map(str::to_owned),
        depth: request.depth,
        event: request.event.cloned().unwrap_or_default(),
        ..Facts::default()
    };
    if needs_pull_request(workflow) {
        let found = look
            .pull_request(&workspace)
            .map_err(|why| IpcError::new("pull_request_unknown", why))?;
        let pr = found
            .filter(|pr| pr.state == crate::forge::PullRequestState::Open)
            .ok_or_else(|| {
                IpcError::new(
                    "pull_request_missing",
                    format!(
                        "`{}` works with the workspace's pull request, and `{}` has no open one. \
                         Open one first.",
                        workflow.id, workspace.name
                    ),
                )
            })?;
        facts.pr = Some(PrFacts {
            number: pr.number,
            url: pr.url,
            title: pr.title,
        });
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
        context: &facts.to_json(),
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

/// The open pull request for the branch `workspace` has checked out: the branch as git says,
/// the pull request as `gh` says when asked about that branch, the newest when there are more.
/// For [`Look::pull_request`] implementations.
///
/// `gh` is given [`PULL_REQUEST_LIMIT`] to answer, and not answering is an error like any
/// other. Queuing waits on this, and the schedules thread queues: one workflow on a network
/// that does not answer must not hold every other schedule until their times count as missed.
pub fn pull_request_of(
    gh: Option<&crate::forge::Gh>,
    git: &crate::git::Git,
    workspace: &WorkspaceRow,
) -> Result<Option<PullRequest>, String> {
    pull_request_within(gh, git, workspace, PULL_REQUEST_LIMIT)
}

/// How long `gh` gets to say whether a workspace has a pull request.
pub const PULL_REQUEST_LIMIT: std::time::Duration = std::time::Duration::from_secs(20);

fn pull_request_within(
    gh: Option<&crate::forge::Gh>,
    git: &crate::git::Git,
    workspace: &WorkspaceRow,
    limit: std::time::Duration,
) -> Result<Option<PullRequest>, String> {
    let gh = gh.ok_or_else(|| {
        "Finding the workspace's pull request needs the GitHub CLI (`gh`) on your PATH.".to_owned()
    })?;
    let root = Path::new(&workspace.path);
    let branch = match git.head(root).map_err(|e| e.to_string())? {
        crate::git::Head::Branch(name) => name,
        crate::git::Head::Unborn(_) | crate::git::Head::Detached(_) => {
            return Err(format!(
                "`{}` is not on a branch, so it has no pull request.",
                workspace.name
            ))
        }
    };
    let listed = gh
        .open_pull_requests_for(root, &branch, limit)
        .map_err(|error| {
            let asked = format!(
                "Could not ask GitHub for `{}`'s pull request",
                workspace.name
            );
            match &error {
                _ if error.is_logged_out() => {
                    format!("{asked}: gh is not logged in. Run `gh auth login`.")
                }
                crate::forge::ForgeError::Failed { stderr, .. } => {
                    format!("{asked}: gh says {stderr}")
                }
                other => format!("{asked}: {other}"),
            }
        })?;
    Ok(crate::forge::pull_request_for(&listed, &branch).cloned())
}

/// What a run of `workflow` in this place would render as nothing, and why: a variable the
/// file uses that the workspace does not have. Not a refusal — an empty line in a prompt is
/// allowed — but something to know before starting.
pub fn empty_variables(workflow: &Workflow, place: &Place, memory_shared: bool) -> Vec<String> {
    let used = variables(workflow);
    let name = &place.workspace.name;
    let mut empty = Vec::new();
    let events: Vec<String> = used
        .iter()
        .filter(|var| var.starts_with("event."))
        .map(|var| format!("`{{{{ {var} }}}}`"))
        .collect();
    if !events.is_empty() {
        empty.push(format!(
            "{} will be empty: only a run a trigger starts has an event.",
            events.join(", ")
        ));
    }
    if used.contains("workspace.task") && place.task.is_none() {
        empty.push(format!(
            "`{{{{ workspace.task }}}}` will be empty: `{name}` was started without a first message."
        ));
    }
    if used.contains("workspace.branch") && place.branch.is_none() {
        empty.push(format!(
            "`{{{{ workspace.branch }}}}` will be empty: `{name}` is not on a branch."
        ));
    }
    if used.contains("workspace.base_branch") && place.base_branch.is_none() {
        empty.push(format!(
            "`{{{{ workspace.base_branch }}}}` will be empty: `{name}` is on the default branch, \
             which is based on nothing."
        ));
    }
    if used.contains("memory") && !memory_shared {
        empty.push(format!(
            "`{{{{ memory }}}}` will be empty: {} does not give its agents its memory.",
            place.project.name
        ));
    }
    empty
}

/// Every `{{ variable }}` a workflow's steps use, as dotted paths: `workspace.task`,
/// `event.note`.
pub fn variables(workflow: &Workflow) -> std::collections::BTreeSet<String> {
    workflow
        .steps
        .iter()
        .flat_map(|step| texts(&step.action))
        .flat_map(template::placeholders)
        .map(|var| var.path.join("."))
        .collect()
}

/// Whether a run of `workflow` needs the workspace's pull request: it waits for activity on it,
/// or names it in a `{{ pr.… }}`.
pub fn needs_pull_request(workflow: &Workflow) -> bool {
    workflow.steps.iter().any(|step| {
        matches!(step.action, Action::WaitPrActivity { .. })
            || texts(&step.action)
                .iter()
                .flat_map(|text| template::placeholders(text))
                .any(|var| var.path.first().is_some_and(|n| n == "pr"))
    })
}

/// Whether a run of `workflow` talks to the workspace's own agent (`session: origin`).
pub fn needs_origin(workflow: &Workflow) -> bool {
    workflow.steps.iter().any(|step| {
        matches!(
            step.action,
            Action::WaitSession {
                session: SessionRef::Origin,
                ..
            } | Action::SendToSession {
                session: SessionRef::Origin,
                ..
            }
        )
    })
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
        Action::RunTest { test, inputs, .. } => {
            let mut texts = vec![test.as_str()];
            texts.extend(inputs.values().map(String::as_str));
            texts
        }
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
    /// The branch checked out: the workspace's own, or, for the project's own checkout, which
    /// has no branch of Yardsort's making, whatever git has checked out there.
    pub branch: Option<String>,
    /// What the branch was started from, or, when Yardsort did not start it, the repository's
    /// default branch — unless that is the branch itself, which is based on nothing.
    pub base_branch: Option<String>,
}

impl Place {
    pub fn load(store: &Store, workspace_id: &str, git: &Git) -> IpcResult<Option<Self>> {
        let Some(workspace) = store.workspace(workspace_id)? else {
            return Ok(None);
        };
        let Some(project) = store.project(&workspace.project_id)? else {
            return Ok(None);
        };
        let task = store.session_prompts(workspace_id)?.into_iter().next();
        // Git is asked only where the row does not know, and a folder git cannot answer for —
        // gone, or not a repository — is simply one with no branch.
        let root = Path::new(&workspace.path);
        let branch = workspace.branch.clone().or_else(|| match git.head(root) {
            Ok(crate::git::Head::Branch(name)) => Some(name),
            _ => None,
        });
        let base_branch = workspace
            .base_branch
            .clone()
            .or_else(|| git.default_branch(root).ok().flatten())
            .filter(|base| Some(base) != branch.as_ref());
        Ok(Some(Self {
            project,
            workspace,
            task,
            branch,
            base_branch,
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
            ["workspace", "branch"] => self.branch.clone(),
            ["workspace", "base_branch"] => self.base_branch.clone(),
            ["workspace", "task"] => self.task.clone(),
            _ => None,
        }
    }
}

/// For tests elsewhere in the crate: any agent is there, and the workspace's pull request is
/// `pr`.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Accepting {
    pub pr: Option<PullRequest>,
}

#[cfg(test)]
impl Look for Accepting {
    fn harness(&self, wanted: &str) -> Result<String, String> {
        Ok(wanted.to_owned())
    }

    fn pull_request(&self, _workspace: &WorkspaceRow) -> Result<Option<PullRequest>, String> {
        Ok(self.pr.clone())
    }
}

#[cfg(test)]
mod tests;
