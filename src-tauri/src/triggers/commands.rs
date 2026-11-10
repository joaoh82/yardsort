//! The window's side of triggers: the list, a trigger's history, the presets, saving, switching,
//! **Reset**, removing, and **Tell the agent**. The rules live in the core
//! (`yardsort_core::trigger`); what is checked here is checked the same way for `ys trigger`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;
use yardsort_core::schedule::{clock, ScheduleNotify, ScheduleTarget};
use yardsort_core::smoke::deliver::Delivery;
use yardsort_core::store::{Store, TriggerFireRow, TriggerRow};
use yardsort_core::trigger::{
    self, fire, presets, Draft, Filters, TriggerEvent, TriggerStanding, DONE_SECTION,
};

use super::Listener;
use crate::error::{IpcError, IpcResult};
use crate::state::blocking;
use crate::workflows::commands::AppLook;

/// How many of a trigger's fires are read to know where it stands, as `ys trigger` reads.
const STANDING_FIRES: usize = 20;

/// Triggers changed: one was added, changed, switched, reset or removed, an event was gathered,
/// a fire was decided, or a run one started ended. The window reloads what it shows of them.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct TriggersChanged {}

/// A trigger, as the view lists it. Not `Trigger`: a workflow file's `trigger:` has that name.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TriggerItem {
    pub id: String,
    pub name: String,
    /// The preset it was made from, if it was.
    pub preset: Option<String>,
    pub project_id: String,
    /// `None` once the workspace is deleted; `workspace_name` keeps what it was called.
    pub workspace_id: Option<String>,
    pub workspace_name: String,
    /// The agent session it belongs to, for one that listens to one agent only.
    pub session_id: Option<String>,
    /// That session's title, while it has a record.
    pub session_title: Option<String>,
    /// Whether that session's agent is running: a session trigger whose agent ended has no one
    /// to listen to.
    pub session_running: bool,
    pub events: Vec<TriggerEvent>,
    pub filters: Filters,
    pub target: ScheduleTarget,
    pub target_id: String,
    pub inputs: BTreeMap<String, String>,
    pub quiet_ms: u32,
    pub cap: u32,
    pub notify: ScheduleNotify,
    pub enabled: bool,
    /// The trigger in words: "When a review comes in on the pull request, run the workflow
    /// `address-review`".
    pub when: String,
    pub standing: TriggerStanding,
    /// Its newest line is a cap still in force, which **Reset** lifts — whether or not a run it
    /// started before is still going.
    pub capped: bool,
    /// The newest line of its history that was decided.
    pub last: Option<TriggerFire>,
    /// What it knows of its workspace's pull request, for one on a `pr.*` event.
    pub watch: Option<PullRequestWatch>,
    /// For one about pull requests elsewhere (`filters.elsewhere`): each the workspace tracks,
    /// as its watch last saw it.
    pub watching_elsewhere: Vec<TrackedWatch>,
    pub created_at: f64,
}

/// A workspace's pull request as the watch last saw it.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestWatch {
    /// `None` until the branch has one.
    pub number: Option<u32>,
    /// It was merged or closed; the branch's next one is looked for.
    pub closed: bool,
    /// Why the last look could not be made.
    pub problem: Option<String>,
    /// When it was last looked at; 0 for never.
    pub looked_at: f64,
}

/// A pull request the workspace tracks elsewhere, as a trigger's watch last saw it.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TrackedWatch {
    /// `owner/name#7`.
    pub name: String,
    pub url: String,
    /// It was merged or closed.
    pub closed: bool,
    /// Why the last look could not be made.
    pub problem: Option<String>,
    /// When it was last looked at; 0 for never.
    pub looked_at: f64,
}

/// One fire of a trigger: what it gathered and what became of it.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TriggerFire {
    pub id: String,
    pub trigger_id: String,
    /// `session:<record>`, `pr:<number>` or `workspace:<id>`: what its runs are counted for.
    pub subject: String,
    /// The newest event it gathered.
    pub event: String,
    /// How many events it gathered.
    pub events: u32,
    pub first_at: f64,
    pub due_at: f64,
    pub decided_at: Option<f64>,
    /// `gathering`, `waiting`, `started`, `skipped`, `capped` or `refused`.
    pub outcome: String,
    pub run_id: Option<String>,
    /// Why it was refused or skipped.
    pub note: Option<String>,
    /// What it gathered, in words: "1 review and 2 comments from bob", the agent's note.
    pub summary: Option<String>,
    /// The run's status once it ended; `None` while it is going.
    pub result: Option<String>,
    /// The run waits for the user to approve the test file.
    pub awaiting_approval: bool,
    pub depth: u32,
}

/// A trigger as the dialog sends it.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TriggerDraft {
    /// The trigger being changed; `None` for a new one.
    pub id: Option<String>,
    pub name: Option<String>,
    pub preset: Option<String>,
    pub workspace_id: String,
    pub session_id: Option<String>,
    pub events: Vec<TriggerEvent>,
    pub filters: Filters,
    pub target: ScheduleTarget,
    pub target_id: String,
    pub inputs: BTreeMap<String, String>,
    pub quiet_ms: u32,
    pub cap: u32,
    pub notify: ScheduleNotify,
}

/// What saving a trigger came to.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SavedTrigger {
    pub trigger: TriggerItem,
    /// Agents already running that were not told about `ys workspace done` when they started,
    /// for a trigger that newly listens for it: what **Tell the agent** offers to tell.
    pub untold: Vec<UntoldAgent>,
}

/// An agent running in a workspace, by its session record.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UntoldAgent {
    pub session_id: String,
    pub title: String,
}

/// A ready-made trigger, for the composer and the dialog.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TriggerPreset {
    pub id: String,
    pub name: String,
    pub does: String,
    pub events: Vec<TriggerEvent>,
    /// The built-in workflow it runs.
    pub workflow: String,
    /// The one input it asks for, if any.
    pub asks: Option<PresetQuestion>,
    pub quiet_ms: u32,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PresetQuestion {
    /// The workflow's input it answers: `reviewer`, `suite`.
    pub input: String,
    /// "Who reviews".
    pub question: String,
}

/// A run a trigger started, for marking it in a run list.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TriggeredRun {
    pub run_id: String,
    pub trigger_id: String,
    pub trigger_name: String,
}

fn fire_item(row: &TriggerFireRow) -> TriggerFire {
    let facts: BTreeMap<String, String> = serde_json::from_str(&row.facts).unwrap_or_default();
    TriggerFire {
        id: row.id.clone(),
        trigger_id: row.trigger_id.clone(),
        subject: row.subject.clone(),
        event: row.event.clone(),
        events: row.events_count.max(0) as u32,
        first_at: row.first_at as f64,
        due_at: row.due_at as f64,
        decided_at: row.decided_at.map(|at| at as f64),
        outcome: row.outcome.clone(),
        run_id: row.run_id.clone(),
        note: row.note.clone(),
        summary: facts
            .get("summary")
            .or_else(|| facts.get("note"))
            .filter(|text| !text.trim().is_empty())
            .cloned(),
        result: row.result.clone(),
        awaiting_approval: row.told_waiting && row.result.is_none(),
        depth: row.depth.max(0) as u32,
    }
}

/// A trigger with where it stands, as the list shows it. `fires` are its newest, newest first.
fn item(store: &Store, row: TriggerRow, fires: &[TriggerFireRow]) -> IpcResult<TriggerItem> {
    let events = trigger::events_of(&row);
    let session = match &row.session_id {
        Some(id) => store.session(id)?,
        None => None,
    };
    let filters = Filters::parse(&row.filters);
    let on_pull_requests = events.iter().any(|event| event.of_a_pull_request());
    let mut watching_elsewhere = Vec::new();
    if let (Some(workspace), true, true) = (&row.workspace_id, on_pull_requests, filters.elsewhere)
    {
        for tracked in store.tracked_pull_requests(workspace)? {
            let watch = store.tracked_pr_watch(&tracked.id)?;
            let name = yardsort_core::tracked::repo_from_key(&tracked.repo).map_or_else(
                || format!("{}#{}", tracked.repo, tracked.number),
                |repo| format!("{}/{}#{}", repo.owner, repo.name, tracked.number),
            );
            watching_elsewhere.push(TrackedWatch {
                name,
                url: tracked.url,
                closed: watch.as_ref().is_some_and(|w| w.closed),
                problem: watch.as_ref().and_then(|w| w.problem.clone()),
                looked_at: watch.map_or(0.0, |w| w.looked_at as f64),
            });
        }
    }
    let watch = match &row.workspace_id {
        Some(workspace) if on_pull_requests && !filters.elsewhere => {
            store.pr_watch(workspace)?.map(|watch| PullRequestWatch {
                number: watch.number,
                closed: watch.closed,
                problem: watch.problem,
                looked_at: watch.looked_at as f64,
            })
        }
        _ => None,
    };
    Ok(TriggerItem {
        standing: trigger::standing_now(store, &row, fires)?,
        capped: trigger::cap_in_force(store, &row, fires)?,
        // A closed one's number is not where the next review will come in.
        when: trigger::describe_on(
            &row,
            watch
                .as_ref()
                .filter(|watch| !watch.closed)
                .and_then(|watch| watch.number),
        ),
        last: fires
            .iter()
            .find(|fire| fire.outcome != "gathering")
            .map(fire_item),
        filters,
        target: ScheduleTarget::parse(&row.target_kind).unwrap_or(ScheduleTarget::Workflow),
        notify: ScheduleNotify::parse(&row.notify).unwrap_or_default(),
        inputs: serde_json::from_str(&row.inputs).unwrap_or_default(),
        session_title: session.as_ref().map(|session| session.title.clone()),
        session_running: session.as_ref().is_some_and(|session| session.running),
        quiet_ms: row.quiet_ms.clamp(0, u32::MAX as i64) as u32,
        cap: row.cap.clamp(0, u32::MAX as i64) as u32,
        events,
        watch,
        watching_elsewhere,
        id: row.id,
        name: row.name,
        preset: row.preset,
        project_id: row.project_id,
        workspace_id: row.workspace_id,
        workspace_name: row.workspace_name,
        session_id: row.session_id,
        target_id: row.target_id,
        enabled: row.enabled,
        created_at: row.created_at as f64,
    })
}

fn one(store: &Store, row: TriggerRow) -> IpcResult<TriggerItem> {
    let fires = store.trigger_fires(&row.id, STANDING_FIRES)?;
    item(store, row, &fires)
}

/// Tell the window triggers changed, and have the thread look now: a trigger switched on may
/// have a watch to look at, and a fire let go by Reset may be due.
pub(crate) fn changed(app: &AppHandle) {
    let _ = TriggersChanged {}.emit(app);
    if let Some(listener) = app.try_state::<Listener>() {
        listener.wake();
    }
}

/// Every trigger of every project, by name.
#[tauri::command]
#[specta::specta]
pub async fn trigger_list(app: AppHandle) -> IpcResult<Vec<TriggerItem>> {
    blocking(app, |state| {
        state
            .store
            .triggers()?
            .into_iter()
            .map(|row| one(&state.store, row))
            .collect()
    })
    .await
}

/// A trigger's history, newest first.
#[tauri::command]
#[specta::specta]
pub async fn trigger_fires(
    app: AppHandle,
    trigger_id: String,
    limit: u32,
) -> IpcResult<Vec<TriggerFire>> {
    blocking(app, move |state| {
        Ok(state
            .store
            .trigger_fires(&trigger_id, limit as usize)?
            .iter()
            .map(fire_item)
            .collect())
    })
    .await
}

/// The runs triggers started, for run lists to mark.
#[tauri::command]
#[specta::specta]
pub async fn trigger_started_runs(app: AppHandle) -> IpcResult<Vec<TriggeredRun>> {
    blocking(app, |state| {
        Ok(state
            .store
            .triggered_runs()?
            .into_iter()
            .map(|(run_id, trigger_id, trigger_name)| TriggeredRun {
                run_id,
                trigger_id,
                trigger_name,
            })
            .collect())
    })
    .await
}

/// The ready-made triggers, as `ys trigger presets` lists them.
#[tauri::command]
#[specta::specta]
pub fn trigger_presets() -> Vec<TriggerPreset> {
    presets::PRESETS
        .iter()
        .map(|preset| TriggerPreset {
            id: preset.id.to_owned(),
            name: preset.name.to_owned(),
            does: preset.does.to_owned(),
            events: preset.events.to_vec(),
            workflow: preset.workflow.to_owned(),
            asks: preset.asks.map(|(input, question)| PresetQuestion {
                input: input.to_owned(),
                question: question.to_owned(),
            }),
            quiet_ms: preset.quiet_ms().clamp(0, u32::MAX as i64) as u32,
        })
        .collect()
}

/// Add a trigger, or change one. Everything its fires would be refused for is refused here, so
/// the person saving it is the one who reads why.
#[tauri::command]
#[specta::specta]
pub async fn trigger_save(app: AppHandle, draft: TriggerDraft) -> IpcResult<SavedTrigger> {
    let saved = blocking(app.clone(), move |state| {
        let now = clock::now().timestamp_millis();
        let asked = Draft {
            name: draft.name.as_deref(),
            preset: draft.preset.as_deref(),
            workspace_id: &draft.workspace_id,
            session_id: draft.session_id.as_deref(),
            events: &draft.events,
            filters: &draft.filters,
            target: draft.target,
            target_id: &draft.target_id,
            inputs: &draft.inputs,
            quiet_ms: i64::from(draft.quiet_ms),
            cap: i64::from(draft.cap),
            notify: draft.notify,
        };
        // An agent is told about `ys workspace done` when it starts, if something listens for
        // it then. Read before saving: once saved, this trigger listens.
        let told = trigger::listens_for(
            &state.store,
            &draft.workspace_id,
            TriggerEvent::WorkspaceDone,
        )?;
        let look = AppLook(state);
        let row = match &draft.id {
            Some(id) => trigger::change(&state.store, &state.data_dir, &look, id, &asked, now)?,
            None => trigger::add(&state.store, &state.data_dir, &look, &asked, "app", now)?,
        };
        let untold = if told || !draft.events.contains(&TriggerEvent::WorkspaceDone) {
            Vec::new()
        } else {
            running_agents(
                &state.store,
                &draft.workspace_id,
                draft.session_id.as_deref(),
            )?
        };
        Ok(SavedTrigger {
            trigger: one(&state.store, row)?,
            untold,
        })
    })
    .await?;
    changed(&app);
    Ok(saved)
}

/// The agents running in a workspace, or only the one a session trigger belongs to.
fn running_agents(
    store: &Store,
    workspace_id: &str,
    only: Option<&str>,
) -> IpcResult<Vec<UntoldAgent>> {
    Ok(store
        .sessions(workspace_id)?
        .into_iter()
        .filter(|session| session.running && session.pty_session_id.is_some())
        .filter(|session| only.is_none_or(|only| session.id == only))
        .map(|session| UntoldAgent {
            session_id: session.id,
            title: session.title,
        })
        .collect())
}

/// **Tell the agent**: type what an agent is told about `ys workspace done` at its start into
/// these running agents, each once it is quiet and never into a busy one, as `send_to_session`
/// does. An agent that ends first is not told.
#[tauri::command]
#[specta::specta]
pub async fn trigger_tell_agents(app: AppHandle, session_ids: Vec<String>) -> IpcResult<u32> {
    let deliveries = blocking(app.clone(), move |state| {
        let mut deliveries = Vec::new();
        for id in &session_ids {
            let Some(session) = state.store.session(id)? else {
                continue;
            };
            let (true, Some(pty)) = (session.running, session.pty_session_id) else {
                continue;
            };
            deliveries.push(Delivery {
                run_id: String::new(),
                session_id: pty,
                text: DONE_SECTION.to_owned(),
            });
        }
        Ok(deliveries)
    })
    .await?;
    let count = deliveries.len() as u32;
    let listener = app.try_state::<Listener>().ok_or_else(|| {
        IpcError::new(
            "triggers_elsewhere",
            "Another Yardsort on this profile listens for triggers; tell the agent from there.",
        )
    })?;
    listener.tell(deliveries);
    Ok(count)
}

/// Switch a trigger on or off. Switched on, only what happens from now counts.
#[tauri::command]
#[specta::specta]
pub async fn trigger_set_enabled(
    app: AppHandle,
    trigger_id: String,
    enabled: bool,
) -> IpcResult<TriggerItem> {
    let switched = blocking(app.clone(), move |state| {
        let now = clock::now().timestamp_millis();
        let row = trigger::set_enabled(&state.store, &trigger_id, enabled, now)?;
        one(&state.store, row)
    })
    .await?;
    changed(&app);
    Ok(switched)
}

/// Lift a trigger's cap: the runs it started stop counting against it.
#[tauri::command]
#[specta::specta]
pub async fn trigger_reset(app: AppHandle, trigger_id: String) -> IpcResult<TriggerItem> {
    let reset = blocking(app.clone(), move |state| {
        fire::reset(&state.store, &trigger_id)?;
        let row = state
            .store
            .trigger(&trigger_id)?
            .ok_or_else(|| IpcError::new("trigger_not_found", "That trigger is not there."))?;
        one(&state.store, row)
    })
    .await?;
    changed(&app);
    Ok(reset)
}

/// Remove a trigger and its history. The runs it started stay where they are.
#[tauri::command]
#[specta::specta]
pub async fn trigger_remove(app: AppHandle, trigger_id: String) -> IpcResult<()> {
    blocking(app.clone(), move |state| {
        if state.store.remove_trigger(&trigger_id)? {
            Ok(())
        } else {
            Err(IpcError::new(
                "trigger_not_found",
                "That trigger is not there.",
            ))
        }
    })
    .await?;
    changed(&app);
    Ok(())
}

/// A preset ticked in the composer, with the answer to its one question.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PresetPick {
    pub id: String,
    pub answer: Option<String>,
}

/// Make each ticked preset a trigger on a new workspace, as `ys trigger add --preset` does.
/// Why each one that could not be added was refused, in words.
pub(crate) fn add_presets(
    state: &crate::state::AppState,
    workspace_id: &str,
    picks: &[PresetPick],
) -> Vec<String> {
    let now = clock::now().timestamp_millis();
    let mut refused = Vec::new();
    for pick in picks {
        let Some(preset) = presets::find(&pick.id) else {
            refused.push(format!("There is no preset `{}`.", pick.id));
            continue;
        };
        let mut inputs = BTreeMap::new();
        if let (Some((input, _)), Some(answer)) = (preset.asks, &pick.answer) {
            inputs.insert(input.to_owned(), answer.clone());
        }
        let added = trigger::add(
            &state.store,
            &state.data_dir,
            &AppLook(state),
            &Draft {
                name: Some(preset.name),
                preset: Some(preset.id),
                workspace_id,
                session_id: None,
                events: preset.events,
                filters: &Filters::default(),
                target: ScheduleTarget::Workflow,
                target_id: preset.workflow,
                inputs: &inputs,
                quiet_ms: preset.quiet_ms(),
                cap: trigger::DEFAULT_CAP,
                notify: ScheduleNotify::default(),
            },
            "app",
            now,
        );
        if let Err(error) = added {
            refused.push(format!(
                "“{}” was not added: {}",
                preset.name, error.message
            ));
        }
    }
    refused
}
