//! The activity timeline's side of IPC: pages of a workspace's events, the diagnostics, the
//! switches, and clearing. The recording itself happens in the core (`yardsort_core::activity`)
//! wherever something is launched or exits; nothing here writes an event.

use serde::Serialize;
use specta::Type;
use tauri::AppHandle;

use crate::error::{IpcError, IpcResult};
use crate::state::blocking;
use crate::store::{DiagnosticRow, EventRow};
use crate::workspaces::commands::{settings_info, SettingsInfo};

/// One recorded fact, as the timeline shows it. Timestamps are epoch milliseconds; `payload`
/// is the event's JSON, whose shape depends on `kind` (see the guide).
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvent {
    /// Ingestion order: the cursor for paging, and the tie-breaker for display.
    pub seq: f64,
    pub id: String,
    pub schema_version: u32,
    pub workspace_id: String,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub occurred_at: f64,
    pub received_at: f64,
    pub kind: String,
    /// Who said so, how, and how sure to be: `yardsort` / `lifecycle` / `observed` for
    /// everything recorded today.
    pub producer: String,
    pub method: String,
    pub fidelity: String,
    pub privacy_class: String,
    pub payload: String,
}

impl From<EventRow> for ActivityEvent {
    fn from(row: EventRow) -> Self {
        Self {
            // Epoch milliseconds and sequence numbers fit a double exactly for a very long time.
            seq: row.seq as f64,
            id: row.id,
            schema_version: u32::try_from(row.schema_version).unwrap_or(u32::MAX),
            workspace_id: row.workspace_id,
            session_id: row.session_id,
            run_id: row.run_id,
            occurred_at: row.occurred_at as f64,
            received_at: row.received_at as f64,
            kind: row.kind,
            producer: row.producer,
            method: row.method,
            fidelity: row.fidelity,
            privacy_class: row.privacy_class,
            payload: row.payload,
        }
    }
}

/// One page of a workspace's timeline, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPage {
    pub events: Vec<ActivityEvent>,
    /// Whether asking again with the last event's `seq` as `before` would find more.
    pub has_more: bool,
}

/// How many events a page holds at most, whatever is asked for.
const MAX_PAGE: u32 = 200;

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ActivityCounter {
    pub name: String,
    pub count: f64,
    pub last_at: f64,
    pub last_detail: Option<String>,
}

impl From<DiagnosticRow> for ActivityCounter {
    fn from(row: DiagnosticRow) -> Self {
        Self {
            name: row.name,
            count: row.count as f64,
            last_at: row.last_at as f64,
            last_detail: row.last_detail,
        }
    }
}

/// What there is, and what went wrong recording it. For Settings → General and bug reports.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ActivityDiagnostics {
    pub events: f64,
    pub runs: f64,
    /// Where the daemon keeps exits for a closed window, and how many are waiting there.
    pub spool_dir: String,
    pub spool_pending: u32,
    pub counters: Vec<ActivityCounter>,
}

/// A page of a workspace's events, newest first: those before `before_seq` (or the newest,
/// when `None`), at most `limit` of them.
#[tauri::command]
#[specta::specta]
pub async fn activity_timeline(
    app: AppHandle,
    workspace_id: String,
    before_seq: Option<f64>,
    limit: u32,
) -> IpcResult<ActivityPage> {
    blocking(app, move |state| {
        let limit = limit.clamp(1, MAX_PAGE) as usize;
        // One more than asked for says whether there is another page, without a count query.
        let mut rows =
            state
                .store
                .events(&workspace_id, before_seq.map(|seq| seq as i64), limit + 1)?;
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        Ok(ActivityPage {
            events: rows.into_iter().map(ActivityEvent::from).collect(),
            has_more,
        })
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn activity_diagnostics(app: AppHandle) -> IpcResult<ActivityDiagnostics> {
    blocking(app, move |state| {
        let (events, runs) = state.store.activity_counts()?;
        let spool = pty_ipc::spool::Spool::new(yardsort_core::activity::spool_dir(&state.data_dir));
        Ok(ActivityDiagnostics {
            events: events as f64,
            runs: runs as f64,
            spool_dir: spool.dir().display().to_string(),
            spool_pending: spool
                .entries()
                .map(|entries| u32::try_from(entries.len()).unwrap_or(u32::MAX))
                .unwrap_or(0),
            counters: state
                .store
                .diagnostics()?
                .into_iter()
                .map(ActivityCounter::from)
                .collect(),
        })
    })
    .await
}

/// Forget recorded activity — one workspace's, or all of it. Runs still going keep their row,
/// so their exit can still be matched; nothing about the processes themselves is touched.
#[tauri::command]
#[specta::specta]
pub async fn activity_clear(app: AppHandle, workspace_id: Option<String>) -> IpcResult<()> {
    blocking(app, move |state| {
        state.store.clear_activity(workspace_id.as_deref())?;
        Ok(())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn settings_save_activity(
    app: AppHandle,
    record_lifecycle: bool,
    show_timeline: bool,
) -> IpcResult<SettingsInfo> {
    blocking(app, move |state| {
        state
            .settings
            .update(|settings| {
                settings.activity.record_lifecycle = record_lifecycle;
                settings.activity.show_timeline = show_timeline;
            })
            .map_err(|error| {
                IpcError::new(
                    "settings_write",
                    format!("Could not save settings: {error}"),
                )
            })?;
        settings_info(state)
    })
    .await
}
