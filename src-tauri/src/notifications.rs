//! The Notifications list's home in the app: the core's keeper of it, told what only the app
//! hears, and the window's commands. The rules, and the list itself, are the core's
//! (`yardsort_core::notifications`). See `docs/design/46-notifications.md`.
//!
//! The window is shown one list: this computer's rows, and with them the rows of each other
//! computer this one reaches, as last read ([`remote_host::computer::notifications`]). A row
//! from another computer names it, and marking it seen is said there.

use pty_host::HostEvent;
use remote_host::computer::notifications as there;
use remote_proto::notification as wire;
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;
use yardsort_core::notifications::{Keeper, Notification, NotificationKind, NotificationPart};
use yardsort_core::store::now_ms;

use crate::error::{IpcError, IpcResult};
use crate::state::{blocking, AppState};

/// The list changed: a row was added, answered or seen, here or on another computer. The
/// window reads it again.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct NotificationsChanged {}

/// One row of the list, as the window shows it. Times are epoch milliseconds.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NotificationItem {
    /// The row's id on the computer that keeps it. Two computers may use the same one.
    pub id: String,
    /// The other computer the row is on, by its id; `None` for this one.
    pub host_id: Option<String>,
    /// What that computer is called.
    pub host_name: Option<String>,
    pub kind: NotificationKind,
    pub part: NotificationPart,
    pub project_id: String,
    pub project_name: String,
    /// The workspace it happened in. Only a schedule or a trigger can be without one.
    pub workspace_id: Option<String>,
    /// What that workspace is called, or was. Empty for a schedule that makes its own.
    pub workspace_name: String,
    pub branch: Option<String>,
    /// The agent's conversation, by its record's id.
    pub session_id: Option<String>,
    /// What the row is about, within its workspace: the conversation's label or the start of
    /// its first message, or the name of the workflow, the test, the schedule or the trigger.
    pub title: String,
    /// Which agent: `claude`, `codex`…
    pub harness_id: Option<String>,
    /// The workflow's run, the smoke test's run, the schedule or the trigger, by its id. Of
    /// another computer's, only a workflow's run is known.
    pub subject_id: Option<String>,
    /// The workflow of a row about a workflow's run, on this computer.
    pub workflow_id: Option<String>,
    /// The step that waits, for a workflow's question or approval.
    pub step_id: Option<String>,
    /// The conversation's terminal, while its process runs.
    pub terminal_id: Option<String>,
    /// When it happened.
    pub at: f64,
    /// When it was answered or seen.
    pub settled_at: Option<f64>,
    /// How long the agent worked before it finished, when that is known.
    pub worked_ms: Option<f64>,
    /// What its process exited with, for one that stopped.
    pub exit_code: Option<i32>,
    /// The command the agent wants to run, while its question is open.
    pub command: Option<String>,
    /// The start of what the agent said last, or of what a workflow asks.
    pub said: Option<String>,
    /// Why, for something that went wrong: a failed run's error, or the reason a schedule or a
    /// trigger could not start.
    pub note: Option<String>,
}

impl From<Notification> for NotificationItem {
    fn from(row: Notification) -> Self {
        Self {
            id: row.id,
            host_id: None,
            host_name: None,
            kind: row.kind,
            part: row.part,
            project_id: row.project_id,
            project_name: row.project_name,
            workspace_id: row.workspace_id,
            workspace_name: row.workspace_name,
            branch: row.branch,
            session_id: row.session_id,
            title: row.title,
            harness_id: row.harness_id,
            subject_id: row.subject_id,
            workflow_id: row.workflow_id,
            step_id: row.step_id,
            terminal_id: row.terminal_id,
            at: row.at as f64,
            settled_at: row.settled_at.map(|at| at as f64),
            worked_ms: row.worked_ms.map(|ms| ms as f64),
            exit_code: row.exit_code.and_then(|code| i32::try_from(code).ok()),
            command: row.command,
            said: row.said,
            note: row.note,
        }
    }
}

/// A row of another computer's list. `None` for a kind or a part this Yardsort does not know:
/// a newer one there has something this one cannot show.
fn item_there(there: there::There) -> Option<NotificationItem> {
    let row = there.row;
    let kind = match row.kind {
        wire::NotificationKind::AgentAsks => NotificationKind::AgentAsks,
        wire::NotificationKind::AgentFinished => NotificationKind::AgentFinished,
        wire::NotificationKind::AgentFailed => NotificationKind::AgentFailed,
        wire::NotificationKind::AgentStopped => NotificationKind::AgentStopped,
        wire::NotificationKind::WorkflowApproval => NotificationKind::WorkflowApproval,
        wire::NotificationKind::WorkflowQuestion => NotificationKind::WorkflowQuestion,
        wire::NotificationKind::WorkflowSucceeded => NotificationKind::WorkflowSucceeded,
        wire::NotificationKind::WorkflowFailed => NotificationKind::WorkflowFailed,
        wire::NotificationKind::TestWaits => NotificationKind::TestWaits,
        wire::NotificationKind::ScheduleRefused => NotificationKind::ScheduleRefused,
        wire::NotificationKind::ScheduleNoWorkspace => NotificationKind::ScheduleNoWorkspace,
        wire::NotificationKind::TriggerRefused => NotificationKind::TriggerRefused,
        wire::NotificationKind::TriggerCapped => NotificationKind::TriggerCapped,
        wire::NotificationKind::TriggerNoWorkspace => NotificationKind::TriggerNoWorkspace,
        wire::NotificationKind::Other => return None,
    };
    let part = match row.part {
        wire::NotificationPart::Needs => NotificationPart::Needs,
        wire::NotificationPart::Finished => NotificationPart::Finished,
        wire::NotificationPart::Earlier => NotificationPart::Earlier,
        wire::NotificationPart::Other => return None,
    };
    Some(NotificationItem {
        id: row.id,
        host_id: Some(there.host_id),
        host_name: Some(there.host_name),
        kind,
        part,
        project_id: row.project.to_string(),
        project_name: row.project_name,
        workspace_id: row.workspace.map(|id| id.to_string()),
        workspace_name: row.workspace_name,
        branch: row.branch,
        session_id: row.session.map(|id| id.to_string()),
        title: row.title,
        harness_id: row.harness,
        subject_id: row.run,
        workflow_id: None,
        step_id: row.step,
        terminal_id: row.terminal.map(|id| id.to_string()),
        at: row.at as f64,
        settled_at: row.settled_at.map(|at| at as f64),
        worked_ms: row.worked_ms.map(|ms| ms as f64),
        exit_code: row.exit_code,
        command: row.command,
        said: row.said,
        note: row.note,
    })
}

/// Tell the window the list changed.
pub fn changed(handle: &AppHandle) {
    let _ = NotificationsChanged {}.emit(handle);
}

/// The app has just started: make the list's keeper, and have it catch up with the terminals
/// found alive and with what was reported and what came to wait while the app was closed.
pub fn start(handle: &AppHandle) {
    let Some(state) = handle.try_state::<AppState>() else {
        return;
    };
    let window = handle.clone();
    let keeper = Keeper::new(move || changed(&window));
    keeper.start(&state.store, &state.host.list());
    handle.manage(keeper);
}

/// Every host event passes through here, after the core has settled an exit's record. `ended`
/// is the conversation whose process an exit ended, when it was an agent's.
pub fn observe(handle: &AppHandle, event: &HostEvent, ended: Option<&str>) {
    if let (Some(state), Some(keeper)) =
        (handle.try_state::<AppState>(), handle.try_state::<Keeper>())
    {
        keeper.observe(&state.store, event, ended);
    }
}

/// The window is closing `terminal` on purpose.
pub fn closing(handle: &AppHandle, terminal: &str) {
    if let Some(keeper) = handle.try_state::<Keeper>() {
        keeper.closing(terminal);
    }
}

/// Take in what agents' hooks reported since the last look. Called after every drain of the
/// inbox, on whichever thread drained it.
pub fn read(handle: &AppHandle) {
    if let (Some(state), Some(keeper)) =
        (handle.try_state::<AppState>(), handle.try_state::<Keeper>())
    {
        keeper.read_events(&state.store);
    }
}

/// A workflow's run, a smoke test's run, a schedule or a trigger changed, here or from `ys`:
/// have the list agree with where they stand now. Called beside each word to the window that
/// one of them changed, from whichever thread says it; the reading is done off that thread.
pub fn look(handle: &AppHandle) {
    let handle = handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let (Some(state), Some(keeper)) =
            (handle.try_state::<AppState>(), handle.try_state::<Keeper>())
        {
            keeper.look(&state.store);
        }
    });
}

/// A workspace or a project went away, and its rows with it; a schedule or a trigger that ran
/// there now has nowhere to. The window reads the list again, and reading it looks at where
/// things stand.
pub fn reread(handle: &AppHandle) {
    if let Some(keeper) = handle.try_state::<Keeper>() {
        keeper.removed();
    }
}

fn keeper(app: &AppHandle) -> IpcResult<tauri::State<'_, Keeper>> {
    app.try_state::<Keeper>().ok_or_else(|| {
        IpcError::new(
            "notifications_not_ready",
            "The Notifications list is not ready yet.",
        )
    })
}

/// The list: what needs an answer, what finished and was not seen, and the newest of what
/// left, on this computer and on each other one it reaches now.
#[tauri::command]
#[specta::specta]
pub async fn notification_list(app: AppHandle) -> IpcResult<Vec<NotificationItem>> {
    let handle = app.clone();
    let mut rows: Vec<NotificationItem> = blocking(app.clone(), move |state| {
        let sources = crate::conversations::sources(state);
        let rows = keeper(&handle)?.list(
            &state.store,
            Some((&sources, state.data_dir.as_path())),
            now_ms(),
        )?;
        Ok(rows.into_iter().map(NotificationItem::from).collect())
    })
    .await?;
    rows.extend(
        there::listed(&crate::computers::home(&app))
            .into_iter()
            .filter_map(item_there),
    );
    Ok(rows)
}

/// These finished rows of this computer's list were looked at.
#[tauri::command]
#[specta::specta]
pub async fn notification_seen(app: AppHandle, ids: Vec<String>) -> IpcResult<()> {
    let handle = app.clone();
    blocking(app, move |state| {
        Ok(keeper(&handle)?.seen(&state.store, &ids)?)
    })
    .await
}

/// These finished rows of another computer's list were looked at here. It is said there.
#[tauri::command]
#[specta::specta]
pub async fn notification_seen_there(
    app: AppHandle,
    host_id: String,
    ids: Vec<String>,
) -> IpcResult<()> {
    there::seen(&crate::computers::home(&app), &host_id, ids)
        .await
        .map_err(crate::computers::ipc)
}

/// This terminal is in front of its person: what its agent finished is seen.
#[tauri::command]
#[specta::specta]
pub async fn notification_seen_terminal(app: AppHandle, terminal_id: String) -> IpcResult<()> {
    let handle = app.clone();
    blocking(app, move |state| {
        Ok(keeper(&handle)?.seen_terminal(&state.store, &terminal_id)?)
    })
    .await
}

/// Everything finished is seen: here, and on each other computer that can be told so now.
#[tauri::command]
#[specta::specta]
pub async fn notification_seen_all(app: AppHandle) -> IpcResult<()> {
    let handle = app.clone();
    blocking(app.clone(), move |state| {
        Ok(keeper(&handle)?.seen_all(&state.store)?)
    })
    .await?;
    there::seen_all(&crate::computers::home(&app))
        .await
        .map_err(crate::computers::ipc)
}
