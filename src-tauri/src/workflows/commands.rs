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
    /// Variables the run will render as nothing, and why. A run still goes ahead.
    pub empty: Vec<String>,
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
        let rows = match workflow_id.as_deref() {
            Some(id) => state.store.workflow_runs_of(id, limit as usize)?,
            None => state.store.workflow_runs(None, limit as usize)?,
        };
        Ok(rows.into_iter().map(run_from).collect())
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
        let git = crate::git::Git::new(&state.env())?;
        let empty = match runs::Place::load(&state.store, &workspace.id, &git)? {
            Some(place) => {
                let shared = state.store.memory_shared(&workspace.project_id)?;
                runs::empty_variables(&workflow, &place, shared)
            }
            None => Vec::new(),
        };
        Ok(RunPreview {
            needs_pull_request,
            pull_request,
            pull_request_problem,
            needs_origin: runs::needs_origin(&workflow),
            empty,
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

/// A workflow a model wrote from a description, checked, for the editor. Nothing is saved.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Described {
    pub text: String,
    /// What is still wrong with it, marked in the editor; empty when it checks out.
    pub problems: Vec<Problem>,
    /// How many answers it took: one, or two when the first had problems.
    pub tries: u32,
    /// Who wrote it, in words.
    pub writer: String,
}

/// Have a model write a workflow from `description`, through Drafting: the agent the user has
/// in its non-interactive mode, or their Anthropic key. The answer is checked as a typed file
/// is, and sent back once with its problems when it has any.
///
/// The agent runs in an empty git repository of the profile's own, not in any project of the
/// user's: a description is not a change to describe, and nothing here should read or touch
/// their work. A repository, not a bare folder, because Codex refuses to run outside one.
#[tauri::command]
#[specta::specta]
pub async fn workflow_describe(app: AppHandle, description: String) -> IpcResult<Described> {
    use yardsort_core::workflow::describe;
    if description.trim().is_empty() {
        return Err(IpcError::new(
            "workflow_no_description",
            "Say what the workflow should do first.",
        ));
    }
    let handle = app.clone();
    let (agent, key, root, writer) = blocking(app, |state| {
        let (agent, key) = crate::draft::commands::writers(state, None)?;
        let git = crate::git::Git::new(&state.env())?;
        let root = describe::scratch_dir(&state.data_dir, &git)?;
        let writer = crate::draft::commands::writer_label(state, agent.as_ref());
        Ok((agent, key, root, writer))
    })
    .await?;

    let mut so_far: Option<describe::Described> = None;
    while let Some(prompt) = describe::next_prompt(&description, so_far.as_ref()) {
        // An agent in its non-interactive mode gets one prompt, so the standing instruction —
        // the file format, without which it cannot answer — goes in front of the question. The
        // key's path is given it as the system prompt, as Drafting does.
        let prompt = match agent {
            Some(_) => format!("{}\n\n{prompt}", describe::SYSTEM),
            None => prompt,
        };
        let prepared = crate::draft::commands::Prepared {
            agent: agent.clone(),
            key: key.clone(),
            root: root.clone(),
            system: describe::SYSTEM,
            prompt,
        };
        let answer = crate::draft::commands::run(handle.clone(), prepared).await?;
        let tries = so_far.as_ref().map_or(0, |d| d.tries) + 1;
        so_far = Some(describe::finish(&answer, tries));
    }
    let done = so_far.expect("the first prompt is always asked");
    Ok(Described {
        text: done.text,
        problems: done.problems,
        tries: done.tries,
        writer,
    })
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

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use yardsort_core::env::{EnvSource, ShellEnv};
    use yardsort_core::git::testing::git;
    use yardsort_core::harness::HarnessDef;
    use yardsort_core::program::Program;
    use yardsort_core::workflow::describe;

    /// Stands in for a writer that refuses to run outside a repository, as Codex does: it
    /// checks, then prints a workflow.
    fn writer_needing_a_repository(dir: &std::path::Path) -> (Program, HarnessDef) {
        let script = dir.join("picky-writer");
        std::fs::write(
            &script,
            "#!/bin/sh\ngit rev-parse --is-inside-work-tree >/dev/null 2>&1 || { echo 'Not inside a trusted directory' >&2; exit 1; }\nprintf 'id: picky\\nname: Picky\\nversion: 1\\ntrigger:\\n  kind: manual\\nsteps:\\n  - id: tell\\n    action: notify\\n    title: Hi\\n'\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = ShellEnv {
            vars: std::env::vars().collect(),
            source: EnvSource::Process,
            warning: None,
        };
        let mut def = HarnessDef::custom("picky");
        def.write_args = vec!["{prompt}".into()];
        (Program::at(script, &env), def)
    }

    #[test]
    fn a_writer_that_needs_a_repository_writes_in_the_scratch_one_and_not_in_a_bare_folder() {
        let dir = tempfile::tempdir().unwrap();
        let (program, def) = writer_needing_a_repository(dir.path());
        let bare = dir.path().join("bare");
        std::fs::create_dir_all(&bare).unwrap();
        let refused = crate::draft::ask_agent(&program, &def, &bare, "write").unwrap_err();
        assert!(
            refused.message.contains("Not inside a trusted directory"),
            "{}",
            refused.message
        );

        let scratch = describe::scratch_dir(dir.path(), &git()).unwrap();
        let written = crate::draft::ask_agent(&program, &def, &scratch, "write").unwrap();
        let done = describe::finish(&written, 1);
        assert!(done.problems.is_empty(), "{:?}", done.problems);
        assert_eq!(
            std::fs::read_dir(&scratch).unwrap().count(),
            1,
            "the writer left nothing beside .git"
        );
    }
}
