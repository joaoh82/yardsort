//! The window's side of smoke tests: a workspace's tests, runs and their steps, the approval
//! that lets a run go, answers to `manual` steps, and the project's test environment. The
//! rules live in the core (`yardsort_core::smoke`); a run started here goes through the same
//! `runs::queue` as `ys test run`, and the runner in this module's parent carries it out.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use yardsort_core::smoke::deliver::{try_deliver, Delivery, Tried};
use yardsort_core::smoke::words::{test_words, StepWords};
use yardsort_core::smoke::{self, exec, report, runs, InputKind, Problem};
use yardsort_core::store::{Store, TestRunRow, TestStepRow};

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
    /// The test has a `browser` step and no browser was found on the machine: it cannot run
    /// until one is installed or named in the project's settings.
    pub browser_missing: bool,
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

/// One test in the Tests view's catalog: a `TestItem` with where it lives and how it last did,
/// so the view can show every test across every workspace with its verdict and what it needs.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestListing {
    pub item: TestItem,
    pub project_id: String,
    pub project_name: String,
    pub workspace_id: String,
    pub workspace_name: String,
    /// Env names the test uses that the project has not set: it would fail for want of them.
    pub env_missing: Vec<String>,
    /// The newest run of this test in this workspace, or `None` when it has never run here.
    pub last_run: Option<LastRun>,
}

/// How a test last did, for the catalog's verdict column.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LastRun {
    pub run_id: String,
    /// The run's `status`: `passed`, `failed`, `awaiting_approval`, and the rest.
    pub status: String,
    pub created_at: f64,
    pub ended_at: Option<f64>,
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

/// What a project asks of its runs.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestSettings {
    /// Show every run's steps and ask, whether or not the file was approved before.
    pub ask_every_time: bool,
    /// Ended runs older than this many days are removed, with their output.
    pub keep_days: u32,
    /// The browser `browser` steps use; empty means find one on the machine.
    pub browser: String,
}

/// The longest a project may keep its runs: ten years, which is to say "keep them".
const MAX_KEEP_DAYS: u32 = 3650;

/// A file as the person approving it reads it: every step in words, the environment names it
/// uses, and what it was before, when it was approved before and has changed since.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestReview {
    pub steps: Vec<StepWords>,
    pub env: Vec<EnvUse>,
    /// This exact file is approved for the project already.
    pub approved: bool,
    /// The project asks to see every run.
    pub ask_every_time: bool,
    /// The file as it was when it was last approved, when that is not this file.
    pub previous: Option<String>,
    /// The hash of the file these words are of: what [`test_run_start`] must still see to run
    /// without the approval card, so a later edit is never approved unseen.
    pub content_hash: String,
    /// A step runs a program (`run`, `shell` or `start`), which is given the whole test
    /// environment as `YS_ENV_*` — not only the names templated in the file.
    pub runs_a_program: bool,
    /// Every name in the project's test environment: what a program step can read, in full.
    pub environment: Vec<String>,
}

/// One `{{ env.NAME }}` a test uses, and whether the project's test environment sets it.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EnvUse {
    pub name: String,
    pub set: bool,
}

/// A piece of a step's output, from where the last piece ended.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OutputChunk {
    pub text: String,
    /// Where `text` starts; past what was asked for when the window had fallen far behind.
    pub from: f64,
    /// Where to read from next.
    pub next: f64,
    pub size: f64,
}

/// How much of a step's output one read returns, at most. The window tails from where it was,
/// so it only ever reads this much at once when it had fallen that far behind.
const OUTPUT_READ_BYTES: usize = 256 * 1024;

/// What sending a result to an agent came to.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestSent {
    /// Typed in now. Otherwise the agent was busy, and it goes once the agent is quiet.
    pub delivered: bool,
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

/// One catalog [`Entry`](smoke::Entry) as a [`TestItem`]. `has_browser` is whether a browser was
/// found on the machine, so a `browser` test says whether it can run.
fn item_from(entry: smoke::Entry, has_browser: bool) -> TestItem {
    TestItem {
        runnable: entry.runnable(),
        browser_missing: entry.uses_browser() && !has_browser,
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
        // One search of the machine for a browser, so a `browser` test says whether it can run.
        let browser = smoke::browser::find_browser(
            &state
                .store
                .project_test_settings(&workspace.project_id)?
                .browser,
        );
        Ok(smoke::catalog(Path::new(&workspace.path))
            .into_iter()
            .map(|entry| item_from(entry, browser.is_some()))
            .collect())
    })
    .await
}

/// Every test in every workspace on show, for the Tests view: each file with where it lives, the
/// env names the project has not set that it needs, and how its last run here went. The window
/// reads this on opening the view and on Refresh, the way Tasks reads the forge. Archived
/// workspaces, whose worktree is gone from disk, are left out; they have no files to read.
#[tauri::command]
#[specta::specta]
pub async fn test_catalog(app: AppHandle) -> IpcResult<Vec<TestListing>> {
    blocking(app, move |state| {
        let project_names: BTreeMap<String, String> = state
            .store
            .projects()?
            .into_iter()
            .map(|project| (project.id, project.name))
            .collect();
        // The newest run of each (workspace, test), for the verdict column.
        let mut latest: BTreeMap<(String, String), TestRunRow> = BTreeMap::new();
        for row in state.store.latest_test_runs()? {
            if let Some(workspace_id) = row.workspace_id.clone() {
                latest.insert((workspace_id, row.test_id.clone()), row);
            }
        }
        // The browser presence and the set env names are per project; find each once.
        let mut has_browser: BTreeMap<String, bool> = BTreeMap::new();
        let mut env_set: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut out = Vec::new();
        for workspace in state.store.workspaces()? {
            if workspace.archived {
                continue;
            }
            let project_id = workspace.project_id.clone();
            let Some(project_name) = project_names.get(&project_id).cloned() else {
                continue;
            };
            if !has_browser.contains_key(&project_id) {
                let settings = state.store.project_test_settings(&project_id)?;
                has_browser.insert(
                    project_id.clone(),
                    smoke::browser::find_browser(&settings.browser).is_some(),
                );
                env_set.insert(
                    project_id.clone(),
                    state
                        .store
                        .project_test_env(&project_id)?
                        .into_iter()
                        .map(|(name, _)| name)
                        .collect(),
                );
            }
            let browser = has_browser[&project_id];
            let set = &env_set[&project_id];
            for entry in smoke::catalog(Path::new(&workspace.path)) {
                let env_missing: Vec<String> = entry
                    .test
                    .as_ref()
                    .map(|test| {
                        runs::env_names(test)
                            .into_iter()
                            .filter(|name| !set.contains(name))
                            .collect()
                    })
                    .unwrap_or_default();
                let last_run = latest
                    .get(&(workspace.id.clone(), entry.id.clone()))
                    .map(|row| LastRun {
                        run_id: row.id.clone(),
                        status: row.status.clone(),
                        created_at: row.created_at as f64,
                        ended_at: row.ended_at.map(|at| at as f64),
                    });
                out.push(TestListing {
                    item: item_from(entry, browser),
                    project_id: project_id.clone(),
                    project_name: project_name.clone(),
                    workspace_id: workspace.id.clone(),
                    workspace_name: workspace.name.clone(),
                    env_missing,
                    last_run,
                });
            }
        }
        Ok(out)
    })
    .await
}

/// A smoke test a model wrote from a description, checked, for the editor. Nothing is saved
/// until the person presses Save. Mirrors workflow drafting's `Described`.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestDescribed {
    pub text: String,
    /// What is still wrong with it, marked in the editor; empty when it checks out.
    pub problems: Vec<Problem>,
    /// How many answers it took: one, or two when the first had problems.
    pub tries: u32,
    /// Who wrote it, in words.
    pub writer: String,
}

/// Have a model write a smoke test from `description`, through Drafting — the same writers the
/// workflow editor uses (the agent the user has in its non-interactive mode, or their Anthropic
/// key). The answer is checked as a typed file is, and sent back once with its problems when it
/// has any. The agent runs in an empty git repository of the profile's own, not in any project
/// of the user's: a description is not a change to describe, and nothing here reads their work.
#[tauri::command]
#[specta::specta]
pub async fn test_describe(app: AppHandle, description: String) -> IpcResult<TestDescribed> {
    use yardsort_core::smoke::describe;
    if description.trim().is_empty() {
        return Err(IpcError::new(
            "test_no_description",
            "Say what the smoke test should check first.",
        ));
    }
    let handle = app.clone();
    let (agent, key, root, writer) = blocking(app, |state| {
        let (agent, key) = crate::draft::commands::workflow_writers(state)?;
        let git = crate::git::Git::new(&state.env())?;
        // The same profile-owned scratch repository workflow drafting uses.
        let root = yardsort_core::workflow::describe::scratch_dir(&state.data_dir, &git)?;
        let writer = crate::draft::commands::writer_label(state, agent.as_ref());
        Ok((agent, key, root, writer))
    })
    .await?;

    let mut so_far: Option<describe::Described> = None;
    while let Some(prompt) = describe::next_prompt(&description, so_far.as_ref()) {
        // An agent in its non-interactive mode gets one prompt, so the standing instruction —
        // the format, without which it cannot answer — goes in front of the question. The key's
        // path is given it as the system prompt, as Drafting does.
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
    Ok(TestDescribed {
        text: done.text,
        problems: done.problems,
        tries: done.tries,
        writer,
    })
}

/// Save a test file into a workspace's worktree, at `.yardsort/tests/<id>.yaml`, and return where
/// it went. This is how **Describe it** persists what a model wrote; the file is then an ordinary
/// uncommitted change in the worktree. Nothing is approved or run by saving.
#[tauri::command]
#[specta::specta]
pub async fn test_save(app: AppHandle, workspace_id: String, text: String) -> IpcResult<String> {
    blocking(app, move |state| {
        let workspace = state
            .store
            .workspace(&workspace_id)?
            .ok_or_else(|| IpcError::new("workspace_not_found", "That workspace is not there."))?;
        let path = smoke::save(Path::new(&workspace.path), &text)?;
        Ok(path.to_string_lossy().into_owned())
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

/// Every run not ended yet, across all workspaces: what the window keeps in view no matter how
/// much history there is, so a run waiting for approval or still going is never crowded out of
/// the list by newer, finished runs.
#[tauri::command]
#[specta::specta]
pub async fn test_active_runs(app: AppHandle) -> IpcResult<Vec<TestRun>> {
    blocking(app, move |state| {
        Ok(state
            .store
            .active_test_runs()?
            .into_iter()
            .map(run_from)
            .collect())
    })
    .await
}

/// Particular runs by id, skipping any that are gone: the window loads the runs its open tabs
/// point at this way, so a tab stays usable even once its run has aged out of the history list.
#[tauri::command]
#[specta::specta]
pub async fn test_runs_by_id(app: AppHandle, ids: Vec<String>) -> IpcResult<Vec<TestRun>> {
    blocking(app, move |state| {
        let mut out = Vec::new();
        for id in &ids {
            if let Some(row) = state.store.test_run(id)? {
                out.push(run_from(row));
            }
        }
        Ok(out)
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

/// Start a run from the Run dialog. The same checks as `ys test run`; and since the dialog
/// showed the file's steps, pressing Run is the approval — but only of the file the dialog
/// showed. `content_hash` is the file's hash when the dialog read it; the run is approved on the
/// spot only when the file queued now still has that hash. If an agent edited the file in
/// between, the run is left `awaiting_approval` so the card shows the new version — a file never
/// runs on an approval of a version nobody saw.
#[tauri::command]
#[specta::specta]
pub async fn test_run_start(
    app: AppHandle,
    test_id: String,
    workspace_id: String,
    inputs: BTreeMap<String, String>,
    content_hash: String,
) -> IpcResult<TestStarted> {
    let started = blocking(app.clone(), move |state| {
        let queued = runs::queue_seen(
            &state.store,
            &runs::Request {
                test_id: &test_id,
                workspace_id: &workspace_id,
                inputs: &inputs,
                requested_by: "app",
                session_record_id: None,
            },
            &content_hash,
        )?;
        Ok(TestStarted {
            run_id: queued.id,
            awaiting_approval: queued.awaiting_approval,
        })
    })
    .await?;
    super::changed(&app);
    Ok(started)
}

/// A test as the Run dialog shows it before it runs: its steps in words, its environment
/// names, and what changed since it was last approved.
#[tauri::command]
#[specta::specta]
pub async fn test_review(
    app: AppHandle,
    workspace_id: String,
    test_id: String,
) -> IpcResult<TestReview> {
    blocking(app, move |state| {
        let workspace = state
            .store
            .workspace(&workspace_id)?
            .ok_or_else(|| IpcError::new("workspace_not_found", "That workspace is not there."))?;
        let entry = smoke::find(Path::new(&workspace.path), &test_id).ok_or_else(|| {
            IpcError::new(
                "test_not_found",
                format!("There is no smoke test `{test_id}` in this workspace."),
            )
        })?;
        let test = entry.test.as_ref().ok_or_else(|| {
            IpcError::new(
                "test_invalid",
                format!("`{}` has problems and cannot run.", entry.id),
            )
        })?;
        review(
            &state.store,
            &workspace.project_id,
            &entry.id,
            test,
            &entry.text,
            &runs::content_hash(&entry.text),
        )
    })
    .await
}

/// A waiting run's file as the approval card shows it: the same as [`test_review`], of the
/// file as it was when the run was asked for.
#[tauri::command]
#[specta::specta]
pub async fn test_run_review(app: AppHandle, run_id: String) -> IpcResult<TestReview> {
    blocking(app, move |state| {
        let row = run_row(&state.store, &run_id)?;
        let test = smoke::parse(&row.definition).map_err(|invalid| {
            IpcError::new(
                "test_invalid",
                format!(
                    "The run's file does not check out: {}",
                    invalid
                        .problems
                        .first()
                        .map(|p| p.message.clone())
                        .unwrap_or_default()
                ),
            )
        })?;
        let project_id = row.project_id.clone().unwrap_or_default();
        review(
            &state.store,
            &project_id,
            &row.test_id,
            &test,
            &row.definition,
            &row.content_hash,
        )
    })
    .await
}

fn run_row(store: &Store, run_id: &str) -> IpcResult<TestRunRow> {
    store
        .test_run(run_id)?
        .ok_or_else(|| IpcError::new("test_run_not_found", "That run is not there."))
}

fn review(
    store: &Store,
    project_id: &str,
    test_id: &str,
    test: &smoke::Test,
    text: &str,
    hash: &str,
) -> IpcResult<TestReview> {
    let set: BTreeSet<String> = store
        .project_test_env(project_id)?
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let settings = store.project_test_settings(project_id)?;
    let previous = store
        .last_approved_definition(project_id, test_id)?
        .filter(|before| before != text);
    // Any `run`/`shell`/`start` step gets the whole test environment, so if the test runs a
    // program the card has to say the exposure is the whole of it, not only the templated names.
    let runs_a_program = test.all_steps().any(|(_, step)| {
        matches!(
            step.action,
            smoke::Action::Run { .. } | smoke::Action::Shell { .. } | smoke::Action::Start { .. }
        )
    });
    Ok(TestReview {
        steps: test_words(test),
        env: runs::env_names(test)
            .into_iter()
            .map(|name| EnvUse {
                set: set.contains(&name),
                name,
            })
            .collect(),
        approved: store.test_approved(project_id, test_id, hash)?,
        ask_every_time: settings.ask_every_time,
        previous,
        content_hash: hash.to_owned(),
        runs_a_program,
        environment: set.into_iter().collect(),
    })
}

/// A piece of a step's stdout or stderr, from `from`: the run tab tails the file with this,
/// reading on each time the runner says the file grew.
#[tauri::command]
#[specta::specta]
pub async fn test_step_output(
    app: AppHandle,
    run_id: String,
    step_id: String,
    stream: String,
    from: f64,
) -> IpcResult<OutputChunk> {
    blocking(app, move |state| {
        let step = state
            .store
            .test_step(&run_id, &step_id)?
            .ok_or_else(|| IpcError::new("test_step_not_found", "That step is not there."))?;
        let outputs = step.outputs();
        let Some(path) = outputs
            .get(stream.as_str())
            .filter(|_| matches!(stream.as_str(), "stdout" | "stderr"))
        else {
            return Ok(OutputChunk {
                text: String::new(),
                from: 0.0,
                next: 0.0,
                size: 0.0,
            });
        };
        let chunk = exec::read_from(Path::new(path), from.max(0.0) as u64, OUTPUT_READ_BYTES)
            .map_err(|error| {
                IpcError::new(
                    "test_output_unreadable",
                    format!("Cannot read the step's output: {error}"),
                )
            })?;
        Ok(OutputChunk {
            text: chunk.text,
            from: chunk.from as f64,
            next: chunk.next as f64,
            size: chunk.size as f64,
        })
    })
    .await
}

/// Type a run's result into an agent's terminal, the way `ys test wait` would have printed it
/// there: now if the agent is quiet, otherwise once it is. The session is one of the
/// workspace's, chosen in the window.
#[tauri::command]
#[specta::specta]
pub async fn test_run_send(
    app: AppHandle,
    run_id: String,
    session_id: String,
) -> IpcResult<TestSent> {
    let handle = app.clone();
    blocking(app, move |state| {
        let row = run_row(&state.store, &run_id)?;
        if !row.ended() {
            return Err(IpcError::new(
                "test_run_going",
                "The run has not ended yet. Send its result once it has.",
            ));
        }
        let steps = state.store.test_steps(&row.id)?;
        let delivery = Delivery {
            run_id: row.id.clone(),
            session_id,
            text: report::for_agent(&row, &steps),
        };
        match try_deliver(state.host.as_ref(), &delivery) {
            Tried::Delivered => Ok(TestSent { delivered: true }),
            Tried::NotYet => {
                let runner = handle.try_state::<super::Runner>().ok_or_else(|| {
                    IpcError::new(
                        "test_runner_absent",
                        "Another Yardsort on this profile carries runs out; send it from there.",
                    )
                })?;
                runner.later(delivery);
                Ok(TestSent { delivered: false })
            }
            Tried::Gone(why) => Err(IpcError::new("test_send_session", why)),
        }
    })
    .await
}

/// What a project asks of its runs.
#[tauri::command]
#[specta::specta]
pub async fn test_settings_get(app: AppHandle, project_id: String) -> IpcResult<TestSettings> {
    blocking(app, move |state| {
        let settings = state.store.project_test_settings(&project_id)?;
        Ok(TestSettings {
            ask_every_time: settings.ask_every_time,
            keep_days: settings.keep_days,
            browser: settings.browser,
        })
    })
    .await
}

/// Change what a project asks of its runs. A shorter retention takes effect within the day.
#[tauri::command]
#[specta::specta]
pub async fn test_settings_save(
    app: AppHandle,
    project_id: String,
    settings: TestSettings,
) -> IpcResult<()> {
    blocking(app, move |state| {
        if state.store.project(&project_id)?.is_none() {
            return Err(IpcError::new(
                "project_not_found",
                "That project is not there.",
            ));
        }
        if settings.keep_days == 0 || settings.keep_days > MAX_KEEP_DAYS {
            return Err(IpcError::new(
                "test_keep_days",
                format!("Keep runs for between 1 and {MAX_KEEP_DAYS} days."),
            ));
        }
        Ok(state.store.set_project_test_settings(
            &project_id,
            &yardsort_core::store::TestSettings {
                ask_every_time: settings.ask_every_time,
                keep_days: settings.keep_days,
                browser: settings.browser.trim().to_owned(),
            },
        )?)
    })
    .await
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
