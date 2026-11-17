//! The Notifications list's home in the app: what the core is told that only the app hears —
//! a terminal going quiet, an agent's process ending — and the window's commands. The rules,
//! and the list itself, are the core's (`yardsort_core::notifications`). See
//! `docs/design/46-notifications.md`.

use std::collections::HashSet;
use std::sync::{Mutex, PoisonError};

use pty_host::HostEvent;
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;
use yardsort_core::notifications::{self, Lines, Notification, NotificationKind, NotificationPart};
use yardsort_core::store::now_ms;
use yardsort_core::trigger::turns::{Change, Turns};

use crate::error::IpcResult;
use crate::state::{blocking, AppState};

/// The list changed: a row was added, answered or seen. The window reads it again.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct NotificationsChanged {}

/// One row of the list, as the window shows it. Times are epoch milliseconds.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NotificationItem {
    pub id: String,
    pub kind: NotificationKind,
    pub part: NotificationPart,
    pub project_id: String,
    pub project_name: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub branch: Option<String>,
    /// The agent's conversation, by its record's id.
    pub session_id: Option<String>,
    /// What the conversation is called: its label, or the start of its first message.
    pub session_title: String,
    /// Which agent: `claude`, `codex`…
    pub harness_id: Option<String>,
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
    /// The start of what the agent said last.
    pub said: Option<String>,
}

impl From<Notification> for NotificationItem {
    fn from(row: Notification) -> Self {
        Self {
            id: row.id,
            kind: row.kind,
            part: row.part,
            project_id: row.project_id,
            project_name: row.project_name,
            workspace_id: row.workspace_id,
            workspace_name: row.workspace_name,
            branch: row.branch,
            session_id: row.session_id,
            session_title: row.session_title,
            harness_id: row.harness_id,
            terminal_id: row.terminal_id,
            at: row.at as f64,
            settled_at: row.settled_at.map(|at| at as f64),
            worked_ms: row.worked_ms.map(|ms| ms as f64),
            exit_code: row.exit_code.and_then(|code| i32::try_from(code).ok()),
            command: row.command,
            said: row.said,
        }
    }
}

/// Managed by Tauri for as long as the app runs.
#[derive(Default)]
pub struct Notifications {
    /// Each terminal's turns, so a quiet terminal and its harness's own report of the same turn
    /// are one row.
    turns: Turns,
    /// What rows are shown with, read when the list is asked for.
    lines: Lines,
    /// When the core was last told the list is hearing from the terminals.
    heard: Mutex<Option<std::time::Instant>>,
    /// Terminals the window was asked to close. An agent ended that way did not stop with an
    /// error, whatever its process says on the way out.
    closing: Mutex<HashSet<String>>,
}

impl Notifications {
    /// Every host event passes through here, after the core has settled an exit's record.
    /// `ended` is the conversation whose process an exit ended, when it was an agent's.
    pub fn observe(&self, handle: &AppHandle, event: &HostEvent, ended: Option<&str>) {
        let Some(state) = handle.try_state::<AppState>() else {
            return;
        };
        let now = now_ms();
        self.hearing(&state, now);
        let settled = matches!(self.turns.observe(event, now), Some(Change::Settled { .. }));
        let changed = match event {
            HostEvent::Quiet { id, busy_ms } if settled => {
                notifications::settled_terminal(&state.store, &id.0, *busy_ms, now)
            }
            HostEvent::Exited { id, exit } => {
                let closed = self
                    .closing
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&id.0);
                let Some(session) = ended else {
                    return;
                };
                // A signal is a person, or the app, ending it: not a failure to tell of.
                let error = (!exit.success && exit.signal.is_none() && !closed)
                    .then(|| i64::from(exit.code));
                notifications::exited(&state.store, session, error, now)
            }
            _ => return,
        };
        tell(handle, changed);
    }

    /// Say, every few seconds at most, that the list is hearing from the terminals: what
    /// prints after the last such word printed while Yardsort was closed.
    fn hearing(&self, state: &AppState, now: i64) {
        let mut heard = self.heard.lock().unwrap_or_else(PoisonError::into_inner);
        if heard.is_some_and(|at| at.elapsed() < HEARD_EVERY) {
            return;
        }
        *heard = Some(std::time::Instant::now());
        if let Err(error) = notifications::heard(&state.store, now) {
            eprintln!("notifications: {error}");
        }
    }

    /// The window is closing `terminal` on purpose.
    pub fn closing(&self, terminal: &str) {
        self.closing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(terminal.to_owned());
    }
}

/// How often, at most, the core is told the list is still hearing from the terminals.
const HEARD_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

/// The app has just started and found these terminals alive. An agent among them that stopped
/// printing while the app was closed finished something nobody saw. Then read what agents'
/// hooks reported meanwhile.
pub fn start(handle: &AppHandle) {
    let Some(state) = handle.try_state::<AppState>() else {
        return;
    };
    handle.manage(Notifications::default());
    // What their harnesses reported first: a turn told that way is not told again below.
    read(handle);
    let waiting: Vec<notifications::Waiting> = state
        .host
        .list()
        .into_iter()
        .filter(|session| {
            session.has_output
                && !session.busy
                && matches!(session.state, pty_host::SessionState::Running)
        })
        .map(|session| notifications::Waiting {
            terminal: session.id.0,
            idle_ms: session.idle_ms,
        })
        .collect();
    tell(
        handle,
        notifications::catch_up(&state.store, &waiting, now_ms()),
    );
}

/// Take in what agents' hooks reported since the last look. Called after every drain of the
/// inbox, on whichever thread drained it.
pub fn read(handle: &AppHandle) {
    let (Some(state), Some(list)) = (
        handle.try_state::<AppState>(),
        handle.try_state::<Notifications>(),
    ) else {
        return;
    };
    tell(
        handle,
        notifications::read_events(&state.store, &list.turns),
    );
}

fn tell(handle: &AppHandle, changed: yardsort_core::store::StoreResult<bool>) {
    match changed {
        Ok(true) => {
            let _ = NotificationsChanged {}.emit(handle);
        }
        Ok(false) => {}
        Err(error) => eprintln!("notifications: {error}"),
    }
}

/// The list: what needs an answer, what finished and was not seen, and the newest of what left.
#[tauri::command]
#[specta::specta]
pub async fn notification_list(app: AppHandle) -> IpcResult<Vec<NotificationItem>> {
    blocking(app.clone(), move |state| {
        let now = now_ms();
        let mut rows = notifications::list(&state.store, now)?;
        if let Some(list) = app.try_state::<Notifications>() {
            let sources = crate::conversations::sources(state);
            list.lines
                .fill(&state.store, &sources, &state.data_dir, &mut rows, now);
        }
        Ok(rows.into_iter().map(NotificationItem::from).collect())
    })
    .await
}

/// These finished rows were looked at.
#[tauri::command]
#[specta::specta]
pub async fn notification_seen(app: AppHandle, ids: Vec<String>) -> IpcResult<()> {
    let left = blocking(app.clone(), move |state| {
        Ok(notifications::seen(&state.store, &ids, now_ms())?)
    })
    .await?;
    tell(&app, Ok(left > 0));
    Ok(())
}

/// This terminal is in front of its person: what its agent finished is seen.
#[tauri::command]
#[specta::specta]
pub async fn notification_seen_terminal(app: AppHandle, terminal_id: String) -> IpcResult<()> {
    let left = blocking(app.clone(), move |state| {
        Ok(notifications::seen_terminal(
            &state.store,
            &terminal_id,
            now_ms(),
        )?)
    })
    .await?;
    tell(&app, Ok(left > 0));
    Ok(())
}

/// Everything finished is seen.
#[tauri::command]
#[specta::specta]
pub async fn notification_seen_all(app: AppHandle) -> IpcResult<()> {
    let left = blocking(app.clone(), move |state| {
        Ok(notifications::seen_all(&state.store, now_ms())?)
    })
    .await?;
    tell(&app, Ok(left > 0));
    Ok(())
}
