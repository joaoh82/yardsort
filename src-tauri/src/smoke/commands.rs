//! The window's side of smoke tests: a workspace's tests, runs and their steps, the approval
//! that lets a run go, answers to `manual` steps, and the project's test environment. The
//! rules live in the core (`yardsort_core::smoke`); a run started here goes through the same
//! `runs::queue` as `ys test run`, and the runner in this module's parent carries it out.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::AppHandle;
use yardsort_core::smoke::{self, runs, InputKind, Problem};
use yardsort_core::store::{TestRunRow, TestStepRow};

use crate::error::{IpcError, IpcResult};
use crate::state::blocking;

/// One test of a workspace, as the window lists it: enough to show, run and approve it. The
/// parsed steps stay in the core; the file's text is what the approval card shows.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestItem {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// The file, as an absolute path.
    pub path: String,
    /// The file exactly as written.
    pub text: String,
    /// Empty when the test can run.
    pub problems: Vec<Problem>,
    pub runnable: bool,
    pub uses_browser: bool,
    /// What a run asks for first.
    pub inputs: Vec<TestInput>,
}

/// An input of a test, for the Run dialog.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestInput {
    pub id: String,
    /// `text` or `choice`.
    pub kind: String,
    pub label: String,
    pub required: bool,
    pub default: Option<String>,
    pub options: Vec<String>,
}

/// One run, as the window lists it.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestRun {
    pub id: String,
    pub project_id: Option<String>,
    pub workspace_id: Option<String>,
    pub workspace_name: String,
    pub test_id: String,
    pub test_name: String,
    /// The file as it was when the run was asked for: what an approval is of.
    pub definition: String,
    pub content_hash: String,
    pub inputs: BTreeMap<String, String>,
    /// `awaiting_approval`, `queued`, `running`, `passed`, `failed`, `refused`, `cancelled` or
    /// `errored`.
    pub status: String,
    /// `app`, `cli` or `workflow`.
    pub requested_by: String,
    pub session_record_id: Option<String>,
    pub port: Option<u16>,
    pub error: Option<String>,
    pub output_dir: Option<String>,
    /// A cancel was asked for while it ran; the teardown is running.
    pub stopping: bool,
    pub created_at: f64,
    pub started_at: Option<f64>,
    pub ended_at: Option<f64>,
}

/// One step of a run.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestStepRun {
    pub step_id: String,
    /// `setup`, `steps` or `teardown`.
    pub phase: String,
    pub action: String,
    /// `pending`, `running`, `waiting`, `passed`, `failed`, `skipped` or `cancelled`.
    pub status: String,
    pub started_at: Option<f64>,
    pub ended_at: Option<f64>,
    pub exit: Option<i32>,
    /// What the step left. `stdout` and `stderr` are paths; `prompt` is a `manual` step's
    /// question, rendered.
    pub outputs: BTreeMap<String, String>,
    pub note: Option<String>,
    pub screenshots: Vec<String>,
}

/// What starting a run came to.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestStarted {
    pub run_id: String,
    /// The file at this hash has not been approved: the run waits for the user.
    pub awaiting_approval: bool,
}

/// One entry of a project's test environment.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestEnvEntry {
    pub name: String,
    pub value: String,
}

fn run_from(row: TestRunRow) -> TestRun {
    TestRun {
        inputs: serde_json::from_str(&row.inputs).unwrap_or_default(),
        stopping: row.cancel_requested_at.is_some() && row.status == "running",
        id: row.id,
        project_id: row.project_id,
        workspace_id: row.workspace_id,
        workspace_name: row.workspace_name,
        test_id: row.test_id,
        test_name: row.test_name,
        definition: row.definition,
        content_hash: row.content_hash,
        status: row.status,
        requested_by: row.requested_by,
        session_record_id: row.session_record_id,
        port: row.port,
        error: row.error,
        output_dir: row.output_dir,
        created_at: row.created_at as f64,
        started_at: row.started_at.map(|t| t as f64),
        ended_at: row.ended_at.map(|t| t as f64),
    }
}

fn step_from(row: TestStepRow) -> TestStepRun {
    TestStepRun {
        outputs: row.outputs(),
        screenshots: row.screenshots(),
        step_id: row.step_id,
        phase: row.phase,
        action: row.action,
        status: row.status,
        started_at: row.started_at.map(|t| t as f64),
        ended_at: row.ended_at.map(|t| t as f64),
        exit: row.exit.and_then(|e| i32::try_from(e).ok()),
        note: row.note,
    }
}

/// A workspace's smoke tests: every file in its worktree's `.yardsort/tests`.
#[tauri::command]
#[specta::specta]
pub async fn test_list(app: AppHandle, workspace_id: String) -> IpcResult<Vec<TestItem>> {
    blocking(app, move |state| {
        let workspace = state
            .store
            .workspace(&workspace_id)?
            .ok_or_else(|| IpcError::new("workspace_not_found", "That workspace is not there."))?;
        Ok(smoke::catalog(Path::new(&workspace.path))
            .into_iter()
            .map(|entry| TestItem {
                runnable: entry.runnable(),
                uses_browser: entry.uses_browser(),
                inputs: entry
                    .test
                    .as_ref()
                    .map(|test| {
                        test.inputs
                            .iter()
                            .map(|input| TestInput {
                                id: input.id.clone(),
                                kind: match input.kind {
                                    InputKind::Text => "text".to_owned(),
                                    InputKind::Choice => "choice".to_owned(),
                                },
                                label: input.label.clone(),
                                required: input.required,
                                default: input.default.clone(),
                                options: input.options.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                id: entry.id,
                name: entry.name,
                description: entry.description,
                path: entry.path,
                text: entry.text,
                problems: entry.problems,
            })
            .collect())
    })
    .await
}

/// Runs, newest first: of one workspace, or of all.
#[tauri::command]
#[specta::specta]
pub async fn test_runs(
    app: AppHandle,
    workspace_id: Option<String>,
    limit: u32,
) -> IpcResult<Vec<TestRun>> {
    blocking(app, move |state| {
        Ok(state
            .store
            .test_runs(workspace_id.as_deref(), limit as usize)?
            .into_iter()
            .map(run_from)
            .collect())
    })
    .await
}

/// One run, with its steps in the order they run.
#[tauri::command]
#[specta::specta]
pub async fn test_run_steps(app: AppHandle, run_id: String) -> IpcResult<Vec<TestStepRun>> {
    blocking(app, move |state| {
        Ok(state
            .store
            .test_steps(&run_id)?
            .into_iter()
            .map(step_from)
            .collect())
    })
    .await
}

/// Start a run. The same checks as `ys test run`; a file not yet approved waits.
#[tauri::command]
#[specta::specta]
pub async fn test_run_start(
    app: AppHandle,
    test_id: String,
    workspace_id: String,
    inputs: BTreeMap<String, String>,
) -> IpcResult<TestStarted> {
    let queued = blocking(app.clone(), move |state| {
        runs::queue(
            &state.store,
            &runs::Request {
                test_id: &test_id,
                workspace_id: &workspace_id,
                inputs: &inputs,
                requested_by: "app",
                session_record_id: None,
            },
        )
    })
    .await?;
    super::changed(&app);
    Ok(TestStarted {
        run_id: queued.id,
        awaiting_approval: queued.awaiting_approval,
    })
}

/// The user, having seen the run's file, lets it go: the file at this hash is approved for the
/// project, and the run is queued. False when the run was not waiting.
#[tauri::command]
#[specta::specta]
pub async fn test_run_approve(app: AppHandle, run_id: String) -> IpcResult<bool> {
    let approved = blocking(app.clone(), move |state| {
        Ok(state.store.approve_test_run(&run_id)?)
    })
    .await?;
    super::changed(&app);
    Ok(approved)
}

/// The user chose not to run the file. Nothing is recorded against it.
#[tauri::command]
#[specta::specta]
pub async fn test_run_refuse(app: AppHandle, run_id: String) -> IpcResult<bool> {
    let refused = blocking(app.clone(), move |state| {
        Ok(state.store.refuse_test_run(&run_id)?)
    })
    .await?;
    super::changed(&app);
    Ok(refused)
}

/// Stop a run: at once when it has not started, after its teardown when it is running.
#[tauri::command]
#[specta::specta]
pub async fn test_run_cancel(app: AppHandle, run_id: String) -> IpcResult<bool> {
    let cancelled = blocking(app.clone(), move |state| {
        Ok(state
            .store
            .cancel_test_run(&run_id, "Cancelled in Yardsort.")?)
    })
    .await?;
    super::changed(&app);
    Ok(cancelled)
}

/// Pass or fail a `manual` step the run is waiting on, with a note.
#[tauri::command]
#[specta::specta]
pub async fn test_step_answer(
    app: AppHandle,
    run_id: String,
    step_id: String,
    passed: bool,
    note: Option<String>,
) -> IpcResult<bool> {
    let answered = blocking(app.clone(), move |state| {
        Ok(state
            .store
            .answer_test_step(&run_id, &step_id, passed, note.as_deref())?)
    })
    .await?;
    super::changed(&app);
    Ok(answered)
}

/// A project's test environment: what `{{ env.NAME }}` means in its tests.
#[tauri::command]
#[specta::specta]
pub async fn test_env_get(app: AppHandle, project_id: String) -> IpcResult<Vec<TestEnvEntry>> {
    blocking(app, move |state| {
        Ok(state
            .store
            .project_test_env(&project_id)?
            .into_iter()
            .map(|(name, value)| TestEnvEntry { name, value })
            .collect())
    })
    .await
}

/// Replace a project's test environment. Names are environment variable names.
#[tauri::command]
#[specta::specta]
pub async fn test_env_save(
    app: AppHandle,
    project_id: String,
    entries: Vec<TestEnvEntry>,
) -> IpcResult<()> {
    blocking(app, move |state| {
        if state.store.project(&project_id)?.is_none() {
            return Err(IpcError::new(
                "project_not_found",
                "That project is not there.",
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for entry in &entries {
            if !smoke::is_env_name(&entry.name) {
                return Err(IpcError::new(
                    "test_env_name",
                    format!(
                        "`{}` is not an environment variable name. Use letters, digits and `_`, \
                         not starting with a digit.",
                        entry.name
                    ),
                ));
            }
            if !seen.insert(entry.name.as_str()) {
                return Err(IpcError::new(
                    "test_env_name",
                    format!("`{}` is given twice.", entry.name),
                ));
            }
        }
        let entries: Vec<(String, String)> =
            entries.into_iter().map(|e| (e.name, e.value)).collect();
        Ok(state.store.set_project_test_env(&project_id, &entries)?)
    })
    .await
}
