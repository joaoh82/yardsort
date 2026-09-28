//! The Workflows view's side of IPC: the catalog, checking and saving files, and runs. The
//! rules live in the core (`yardsort_core::workflow`); a run started here goes through the same
//! `runs::queue` as `ys workflow run`, and the driver in this module's parent carries it out.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;
use specta::Type;
use tauri::AppHandle;
use tauri_specta::Event;
use yardsort_core::forge::{Gh, PullRequest};
use yardsort_core::store::{WorkflowRunRow, WorkflowStepRow, WorkspaceRow};
use yardsort_core::workflow::runs::{self, Facts, Look, PrFacts};
use yardsort_core::workflow::{self, Entry, Problem, Workflow};

use crate::error::{IpcError, IpcResult};
use crate::state::{blocking, AppState};

/// Runs changed: one was queued, moved on, finished or cancelled. The window reloads what it
/// shows of them.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunsChanged {}

/// A workflow as the sidebar and the view list it.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowItem {
    #[serde(flatten)]
    pub entry: Entry,
    /// Its runs queued or running now, in any workspace.
    pub active_runs: u32,
}

/// What checking a file's text found.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowCheck {
    /// The workflow, when the text checks out.
    pub workflow: Option<Workflow>,
    pub id: Option<String>,
    pub problems: Vec<Problem>,
}

/// One run, as the view lists it.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRun {
    pub id: String,
    pub workflow_id: String,
    pub workflow_name: String,
    pub workspace_id: Option<String>,
    pub workspace_name: String,
    /// `queued`, `running`, `succeeded`, `failed` or `cancelled`.
    pub status: String,
    /// `app` or `cli`.
    pub requested_by: String,
    pub error: Option<String>,
    pub inputs: BTreeMap<String, String>,
    pub pull_request: Option<PrFacts>,
    pub created_at: f64,
    pub started_at: Option<f64>,
    pub ended_at: Option<f64>,
}

/// One step of a run.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStepRun {
    pub step_id: String,
    pub action: String,
    /// `pending`, `running`, `waiting`, `succeeded`, `failed`, `skipped` or `cancelled`.
    pub status: String,
    pub started_at: Option<f64>,
    pub ended_at: Option<f64>,
    pub outputs: BTreeMap<String, String>,
    pub note: Option<String>,
}

/// What the Run dialog shows before starting: what the run would find.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunPreview {
    /// The workflow works with the workspace's pull request.
    pub needs_pull_request: bool,
    /// Found: the workspace's open pull request.
    pub pull_request: Option<PrFacts>,
    /// Why there is none to use, when one is needed.
    pub pull_request_problem: Option<String>,
    /// The workflow talks to the workspace's own agent.
    pub needs_origin: bool,
}

fn run_from(row: WorkflowRunRow) -> WorkflowRun {
    let facts = Facts::parse(&row.context);
    WorkflowRun {
        inputs: serde_json::from_str(&row.inputs).unwrap_or_default(),
        pull_request: facts.pr,
        id: row.id,
        workflow_id: row.workflow_id,
        workflow_name: row.workflow_name,
        workspace_id: row.workspace_id,
        workspace_name: row.workspace_name,
        status: row.status,
        requested_by: row.requested_by,
        error: row.error,
        created_at: row.created_at as f64,
        started_at: row.started_at.map(|t| t as f64),
        ended_at: row.ended_at.map(|t| t as f64),
    }
}

fn step_from(row: WorkflowStepRow) -> WorkflowStepRun {
    WorkflowStepRun {
        outputs: serde_json::from_str(&row.outputs).unwrap_or_default(),
        step_id: row.step_id,
        action: row.action,
        status: row.status,
        started_at: row.started_at.map(|t| t as f64),
        ended_at: row.ended_at.map(|t| t as f64),
        note: row.note,
    }
}

fn changed(app: &AppHandle) {
    let _ = WorkflowRunsChanged {}.emit(app);
}

/// Every workflow, built in and the user's, with how many runs of each are going now.
#[tauri::command]
#[specta::specta]
pub async fn workflow_list(app: AppHandle) -> IpcResult<Vec<WorkflowItem>> {
    blocking(app, |state| {
        let active = state.store.active_workflow_runs()?;
        Ok(workflow::catalog(&state.data_dir)
            .into_iter()
            .map(|entry| WorkflowItem {
                active_runs: active
                    .iter()
                    .filter(|run| run.workflow_id == entry.id)
                    .count() as u32,
                entry,
            })
            .collect())
    })
    .await
}

/// Check a file's text as it is being written: the editor's marks and the chart come from this.
#[tauri::command]
#[specta::specta]
pub fn workflow_check(text: String) -> WorkflowCheck {
    match workflow::parse(&text) {
        Ok(workflow) => WorkflowCheck {
            id: Some(workflow.id.clone()),
            workflow: Some(workflow),
            problems: Vec::new(),
        },
        Err(invalid) => WorkflowCheck {
            workflow: None,
            id: invalid.id,
            problems: invalid.problems,
        },
    }
}

/// Save a file: `path` is the user's file being edited, or `None` for a new one. Returns the id
/// it was saved under.
#[tauri::command]
#[specta::specta]
pub async fn workflow_save(
    app: AppHandle,
    path: Option<String>,
    text: String,
) -> IpcResult<String> {
    blocking(app, move |state| {
        let editing = path.map(PathBuf::from);
        workflow::save(&state.data_dir, editing.as_deref(), &text)?;
        Ok(match workflow::parse(&text) {
            Ok(workflow) => workflow.id,
            Err(invalid) => invalid.id.unwrap_or_default(),
        })
    })
    .await
}

/// Copy a workflow into the user's folder. With no `asId`, a built-in's copy replaces it.
/// Returns the copy's id.
#[tauri::command]
#[specta::specta]
pub async fn workflow_copy(app: AppHandle, id: String, as_id: Option<String>) -> IpcResult<String> {
    blocking(app, move |state| {
        workflow::copy(&state.data_dir, &id, as_id.as_deref())?;
        Ok(as_id.unwrap_or(id))
    })
    .await
}

/// Delete one of the user's workflow files. The window asks first.
#[tauri::command]
#[specta::specta]
pub async fn workflow_remove(app: AppHandle, path: String) -> IpcResult<()> {
    blocking(app, move |state| {
        workflow::remove(&state.data_dir, &PathBuf::from(path))
    })
    .await
}

/// Runs, newest first: of one workflow, or of all.
#[tauri::command]
#[specta::specta]
pub async fn workflow_runs(
    app: AppHandle,
    workflow_id: Option<String>,
    limit: u32,
) -> IpcResult<Vec<WorkflowRun>> {
    blocking(app, move |state| {
        // The store lists by workspace; a workflow's runs are picked from a wider page.
        let rows = state
            .store
            .workflow_runs(None, (limit as usize).saturating_mul(4).max(200))?;
        Ok(rows
            .into_iter()
            .filter(|run| {
                workflow_id
                    .as_deref()
                    .is_none_or(|id| run.workflow_id == id)
            })
            .take(limit as usize)
            .map(run_from)
            .collect())
    })
    .await
}

/// A run's steps, in file order.
#[tauri::command]
#[specta::specta]
pub async fn workflow_run_steps(app: AppHandle, run_id: String) -> IpcResult<Vec<WorkflowStepRun>> {
    blocking(app, move |state| {
        Ok(state
            .store
            .workflow_steps(&run_id)?
            .into_iter()
            .map(step_from)
            .collect())
    })
    .await
}

/// What a run of `workflowId` in `workspaceId` would find, for the Run dialog to show.
#[tauri::command]
#[specta::specta]
pub async fn workflow_preview(
    app: AppHandle,
    workflow_id: String,
    workspace_id: String,
) -> IpcResult<RunPreview> {
    blocking(app, move |state| {
        let entry = workflow::find(&state.data_dir, &workflow_id)
            .ok_or_else(|| IpcError::new("workflow_not_found", "That workflow is not there."))?;
        let Some(workflow) = entry.workflow else {
            return Err(IpcError::new(
                "workflow_invalid",
                "This workflow has problems and cannot run.",
            ));
        };
        let workspace = state
            .store
            .workspace(&workspace_id)?
            .ok_or_else(|| IpcError::new("workspace_not_found", "That workspace is not there."))?;
        let needs_pull_request = runs::needs_pull_request(&workflow);
        let (pull_request, pull_request_problem) = if needs_pull_request {
            match AppLook(state).pull_request(&workspace) {
                Ok(Some(pr)) if pr.state == yardsort_core::forge::PullRequestState::Open => (
                    Some(PrFacts {
                        number: pr.number,
                        url: pr.url,
                        title: pr.title,
                    }),
                    None,
                ),
                Ok(_) => (
                    None,
                    Some(format!("`{}` has no open pull request.", workspace.name)),
                ),
                Err(why) => (None, Some(why)),
            }
        } else {
            (None, None)
        };
        Ok(RunPreview {
            needs_pull_request,
            pull_request,
            pull_request_problem,
            needs_origin: runs::needs_origin(&workflow),
        })
    })
    .await
}

/// Start a run. The same checks as `ys workflow run`; the driver picks it up at once.
#[tauri::command]
#[specta::specta]
pub async fn workflow_start(
    app: AppHandle,
    workflow_id: String,
    workspace_id: String,
    inputs: BTreeMap<String, String>,
) -> IpcResult<String> {
    let run = blocking(app.clone(), move |state| {
        runs::queue(
            &state.store,
            &state.data_dir,
            &runs::Request {
                workflow_id: &workflow_id,
                workspace_id: &workspace_id,
                inputs: &inputs,
                requested_by: "app",
            },
            &AppLook(state),
        )
    })
    .await?;
    if let Some(driver) = tauri::Manager::try_state::<super::Driver>(&app) {
        driver.wake();
    }
    changed(&app);
    Ok(run)
}

/// Cancel a run. Agents it started keep running.
#[tauri::command]
#[specta::specta]
pub async fn workflow_cancel(app: AppHandle, run_id: String) -> IpcResult<bool> {
    let cancelled = blocking(app.clone(), move |state| {
        Ok(state
            .store
            .cancel_workflow_run(&run_id, "Cancelled in Yardsort.")?)
    })
    .await?;
    changed(&app);
    Ok(cancelled)
}

/// What the app answers when a run is queued: the agents set up here and found on `PATH`, and
/// the pull request from `gh`.
struct AppLook<'a>(&'a AppState);

impl Look for AppLook<'_> {
    fn harness(&self, wanted: &str) -> Result<String, String> {
        let all = crate::workspaces::commands::list_harnesses(self.0);
        let found = all
            .iter()
            .find(|h| h.def.id.eq_ignore_ascii_case(wanted))
            .ok_or_else(|| format!("No agent called {wanted:?} is set up in Yardsort."))?;
        if found.resolved_path.is_none() {
            return Err(format!(
                "{} is set up, but `{}` is not on your PATH.",
                found.def.id, found.def.command
            ));
        }
        Ok(found.def.id.clone())
    }

    fn pull_request(&self, workspace: &WorkspaceRow) -> Result<Option<PullRequest>, String> {
        let env = self.0.env();
        let git = crate::git::Git::new(&env).map_err(|e| e.to_string())?;
        runs::pull_request_of(Gh::find(&env).as_ref(), &git, workspace)
    }
}
