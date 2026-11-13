//! The window's side of schedules: the list, a schedule's history, saving, switching, removing
//! and **Run now**. The rules live in the core (`yardsort_core::schedule`); what is checked
//! here is checked the same way for `ys schedule`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;
use yardsort_core::schedule::fire::{self, ScheduleOutcome};
use yardsort_core::schedule::{
    self, clock, when, Draft, IfMissed, ScheduleNotify, ScheduleSpec, ScheduleStanding,
    ScheduleTarget,
};
use yardsort_core::store::{ScheduleRow, ScheduleRunRow, Store};

use super::SchedulesChanged;
use crate::error::{IpcError, IpcResult};
use crate::state::blocking;
use crate::workflows::commands::AppLook;

/// How many times to come the dialog shows beside what it is being told.
const UPCOMING: usize = 5;

/// A schedule, as the view lists it.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub id: String,
    pub name: String,
    pub target: ScheduleTarget,
    pub target_id: String,
    pub project_id: String,
    /// `None` once the workspace is deleted; `workspace_name` keeps what it was called.
    pub workspace_id: Option<String>,
    pub workspace_name: String,
    /// It makes a workspace in the project for each run. `workspace_id` is then `None`, and
    /// `workspace_name` is what they are called: empty names them after the workflow.
    pub new_workspace: bool,
    /// The branch each new workspace starts from; `None` is the default branch.
    pub base_branch: Option<String>,
    /// The most of its own workspaces that may be there at once.
    pub keep: u32,
    /// The workspaces it made that are still there, oldest first.
    pub made: Vec<String>,
    pub inputs: BTreeMap<String, String>,
    /// `None` for a spec this Yardsort cannot read.
    pub spec: Option<ScheduleSpec>,
    /// The spec in words: "Weekdays at 09:00".
    pub when: String,
    pub missed: IfMissed,
    pub notify: ScheduleNotify,
    pub enabled: bool,
    pub next_at: Option<f64>,
    pub standing: ScheduleStanding,
    /// The newest line of its history.
    pub last: Option<ScheduleRun>,
    pub created_at: f64,
}

/// One time a schedule came due.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRun {
    pub id: String,
    pub schedule_id: String,
    pub due_at: f64,
    pub fired_at: f64,
    pub outcome: ScheduleOutcome,
    /// Asked for with Run now, not by the clock.
    pub manual: bool,
    /// The workflow run's or the smoke test run's id, when one was queued.
    pub run_id: Option<String>,
    pub note: Option<String>,
    /// The run's status once it ended; `None` while it is going.
    pub result: Option<String>,
    /// The run waits for the user to approve the test file.
    pub awaiting_approval: bool,
}

/// A schedule as the dialog sends it.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleDraft {
    /// The schedule being changed; `None` for a new one.
    pub id: Option<String>,
    pub name: Option<String>,
    pub target: ScheduleTarget,
    pub target_id: String,
    /// Not read when `new_workspace` is given.
    pub workspace_id: String,
    /// Make a workspace for each run, in this project. Workflows only.
    pub new_workspace: Option<NewWorkspaceEachRun>,
    pub inputs: BTreeMap<String, String>,
    pub spec: ScheduleSpec,
    pub missed: IfMissed,
    pub notify: ScheduleNotify,
    /// For a test: the file's hash when the dialog showed it. Saving approves that file, and
    /// only that file.
    pub content_hash: Option<String>,
}

/// A workspace made for each run of a schedule.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NewWorkspaceEachRun {
    pub project_id: String,
    /// `None` or empty: the repository's default branch.
    pub base_branch: Option<String>,
    /// `None` or empty: named after the workflow.
    pub name: Option<String>,
    /// `None`: five.
    pub keep: Option<u32>,
}

/// What a spec comes to, for the dialog as it is filled in.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SchedulePreview {
    /// What is wrong with it, or nothing.
    pub problem: Option<String>,
    pub when: String,
    /// The next few times it would fire.
    pub next: Vec<f64>,
}

/// A run a schedule started, for marking it in a run list.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledRun {
    pub run_id: String,
    pub schedule_id: String,
    pub schedule_name: String,
}

fn run_item(row: ScheduleRunRow) -> ScheduleRun {
    ScheduleRun {
        awaiting_approval: row.told_waiting && row.result.is_none(),
        id: row.id,
        schedule_id: row.schedule_id,
        due_at: row.due_at as f64,
        fired_at: row.fired_at as f64,
        outcome: ScheduleOutcome::parse(&row.outcome).unwrap_or(ScheduleOutcome::Refused),
        manual: row.manual,
        run_id: row.run_id,
        note: row.note,
        result: row.result,
    }
}

fn item(
    store: &Store,
    row: ScheduleRow,
    last: Option<ScheduleRunRow>,
    going: Option<&ScheduleRunRow>,
) -> IpcResult<Schedule> {
    let spec = ScheduleSpec::parse(&row.spec);
    let made = if row.new_workspace {
        store.schedule_workspaces(&row.id)?
    } else {
        Vec::new()
    };
    Ok(Schedule {
        made,
        new_workspace: row.new_workspace,
        keep: u32::try_from(row.keep).unwrap_or(schedule::KEEP_DEFAULT),
        standing: schedule::standing(&row, last.as_ref(), going),
        base_branch: row.base_branch,
        when: spec
            .as_ref()
            .map(|spec| schedule::describe(spec, &clock::now().timezone()))
            .unwrap_or_else(|| "Unknown".to_owned()),
        spec,
        target: ScheduleTarget::parse(&row.target_kind).unwrap_or(ScheduleTarget::Workflow),
        missed: IfMissed::parse(&row.missed).unwrap_or_default(),
        notify: ScheduleNotify::parse(&row.notify).unwrap_or_default(),
        inputs: serde_json::from_str(&row.inputs).unwrap_or_default(),
        id: row.id,
        name: row.name,
        target_id: row.target_id,
        project_id: row.project_id,
        workspace_id: row.workspace_id,
        workspace_name: row.workspace_name,
        enabled: row.enabled,
        next_at: row.next_at.map(|at| at as f64),
        last: last.map(run_item),
        created_at: row.created_at as f64,
    })
}

/// One schedule with the newest line of its history, and where the run it started stands.
fn one(store: &Store, row: ScheduleRow) -> IpcResult<Schedule> {
    let last = store.schedule_runs(&row.id, 1)?.into_iter().next();
    let going = schedule::going_runs(store)?;
    let going = going.get(&row.id);
    item(store, row, last, going)
}

/// Tell the window schedules changed, and have the thread look now: one may be due at once.
fn changed(app: &AppHandle) {
    let _ = SchedulesChanged {}.emit(app);
    if let Some(scheduler) = app.try_state::<super::Scheduler>() {
        scheduler.wake();
    }
}

/// Every schedule of every project, by name.
#[tauri::command]
#[specta::specta]
pub async fn schedule_list(app: AppHandle) -> IpcResult<Vec<Schedule>> {
    blocking(app, |state| {
        let mut latest: BTreeMap<String, ScheduleRunRow> = state
            .store
            .latest_schedule_runs()?
            .into_iter()
            .map(|run| (run.schedule_id.clone(), run))
            .collect();
        let going = schedule::going_runs(&state.store)?;
        state
            .store
            .schedules()?
            .into_iter()
            .map(|row| {
                let last = latest.remove(&row.id);
                let going = going.get(&row.id);
                item(&state.store, row, last, going)
            })
            .collect()
    })
    .await
}

/// A schedule's history, newest first.
#[tauri::command]
#[specta::specta]
pub async fn schedule_runs(
    app: AppHandle,
    schedule_id: String,
    limit: u32,
) -> IpcResult<Vec<ScheduleRun>> {
    blocking(app, move |state| {
        Ok(state
            .store
            .schedule_runs(&schedule_id, limit as usize)?
            .into_iter()
            .map(run_item)
            .collect())
    })
    .await
}

/// The runs schedules started, for run lists to mark.
#[tauri::command]
#[specta::specta]
pub async fn schedule_started_runs(app: AppHandle) -> IpcResult<Vec<ScheduledRun>> {
    blocking(app, |state| {
        Ok(state
            .store
            .scheduled_runs()?
            .into_iter()
            .map(|(run_id, schedule_id, schedule_name)| ScheduledRun {
                run_id,
                schedule_id,
                schedule_name,
            })
            .collect())
    })
    .await
}

/// What a spec comes to: what is wrong with it, what it reads as, and when it would next fire.
/// Nothing is saved.
#[tauri::command]
#[specta::specta]
pub async fn schedule_preview(spec: ScheduleSpec) -> IpcResult<SchedulePreview> {
    let now = clock::now();
    let problem = schedule::check(&spec, &now).err();
    let next = match &problem {
        Some(_) => Vec::new(),
        None => when::upcoming(&spec, &now, UPCOMING)
            .unwrap_or_default()
            .iter()
            .map(|at| at.timestamp_millis() as f64)
            .collect(),
    };
    Ok(SchedulePreview {
        problem,
        when: schedule::describe(&spec, &now.timezone()),
        next,
    })
}

/// Add a schedule, or change one. Everything its run would be refused for is refused here, so
/// the person saving it is the one who reads why.
#[tauri::command]
#[specta::specta]
pub async fn schedule_save(app: AppHandle, draft: ScheduleDraft) -> IpcResult<Schedule> {
    let saved = blocking(app.clone(), move |state| {
        let now = clock::now();
        let asked = Draft {
            name: draft.name.as_deref(),
            target: draft.target,
            target_id: &draft.target_id,
            workspace_id: &draft.workspace_id,
            new_workspace: draft.new_workspace.as_ref().map(|each| schedule::EachRun {
                project_id: &each.project_id,
                base: each.base_branch.as_deref(),
                name: each.name.as_deref(),
                keep: each.keep,
            }),
            inputs: &draft.inputs,
            spec: &draft.spec,
            missed: draft.missed,
            notify: draft.notify,
        };
        let look = AppLook(state);
        let row = match &draft.id {
            Some(id) => schedule::change(&state.store, &state.data_dir, &look, id, &asked, &now)?,
            None => schedule::add(&state.store, &state.data_dir, &look, &asked, "app", &now)?,
        };
        // The dialog showed the test's steps: saving is the approval of the file it showed.
        if let (ScheduleTarget::Test, Some(shown)) = (draft.target, &draft.content_hash) {
            yardsort_core::smoke::runs::approve_seen(
                &state.store,
                &draft.workspace_id,
                &draft.target_id,
                shown,
            )?;
        }
        one(&state.store, row)
    })
    .await?;
    changed(&app);
    Ok(saved)
}

/// Switch a schedule on or off. Switched on, it starts from now: times that passed while it
/// was off are not made up for.
#[tauri::command]
#[specta::specta]
pub async fn schedule_set_enabled(
    app: AppHandle,
    schedule_id: String,
    enabled: bool,
) -> IpcResult<Schedule> {
    let switched = blocking(app.clone(), move |state| {
        let row = schedule::set_enabled(&state.store, &schedule_id, enabled, &clock::now())?;
        one(&state.store, row)
    })
    .await?;
    changed(&app);
    Ok(switched)
}

/// Remove a schedule and its history. The runs it started stay where they are.
#[tauri::command]
#[specta::specta]
pub async fn schedule_remove(app: AppHandle, schedule_id: String) -> IpcResult<()> {
    blocking(app.clone(), move |state| {
        if state.store.remove_schedule(&schedule_id)? {
            Ok(())
        } else {
            Err(IpcError::new(
                "schedule_not_found",
                "That schedule is not there.",
            ))
        }
    })
    .await?;
    changed(&app);
    Ok(())
}

/// Start a schedule's run now. Its next time stays where it is.
#[tauri::command]
#[specta::specta]
pub async fn schedule_run_now(app: AppHandle, schedule_id: String) -> IpcResult<ScheduleRun> {
    let started = blocking(app.clone(), move |state| {
        let now = clock::now().timestamp_millis();
        let env = state.env();
        let git = crate::git::Git::new(&env)?;
        let root = state.worktree_root()?;
        let settings = state.settings.get();
        let fired = fire::run_now(
            &state.store,
            &state.data_dir,
            &AppLook(state),
            &yardsort_core::workspaces::Workspaces {
                env: &env,
                store: &state.store,
                git: &git,
                worktree_root: &root,
                settings: &settings.workspaces,
            },
            &schedule_id,
            now,
        )?;
        Ok(run_item(fired.run))
    })
    .await?;
    // A run was queued: its driver looks now, and the run lists are out of date.
    let _ = crate::workflows::commands::WorkflowRunsChanged {}.emit(&app);
    crate::smoke::changed(&app);
    changed(&app);
    Ok(started)
}
