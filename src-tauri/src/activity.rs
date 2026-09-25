//! The activity timeline's side of IPC: pages of a workspace's events, the diagnostics, the
//! switches, and clearing — and the inbox watcher that takes in what agents' hooks report.
//! The recording itself happens in the core (`yardsort_core::activity`) wherever something is
//! launched, exits, or is drained from the inbox; nothing here writes an event.

use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::error::{IpcError, IpcResult};
use crate::state::{blocking, AppState};
use crate::store::{DiagnosticRow, EventRow, Store};
use crate::workspaces::commands::{settings_info, SettingsInfo};

/// New events were recorded for these workspaces; a timeline showing one should ask again.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct ActivityChanged {
    pub workspace_ids: Vec<String>,
}

/// Take what the inbox holds into the store, and tell the timeline which workspaces moved.
/// Anything the drain had to leave — a Codex turn its session file has not finished — is
/// handed to the watcher, which looks again on its own clock: nothing else would.
pub fn drain_inbox(handle: &AppHandle, store: &Store, data_dir: &Path) {
    let report = yardsort_core::activity::import_inbox(store, data_dir);
    if !report.workspaces.is_empty() {
        let _ = ActivityChanged {
            workspace_ids: report.workspaces.into_iter().collect(),
        }
        .emit(handle);
    }
    if report.deferred > 0 {
        if let Some(state) = handle.try_state::<AppState>() {
            if let Some(watcher) = state.inbox_watcher.lock().unwrap().as_ref() {
                watcher.look_again();
            }
        }
    }
}

/// Wait this long after the last file lands before draining: a tool call is two files in
/// quick succession, and one drain covers both.
const INBOX_QUIET: Duration = Duration::from_millis(200);
/// While a drain has left something for later, look again this often. Codex writes a turn's
/// end to its session file within moments of announcing it; five seconds is patience, not
/// polling, and the core gives up on a turn after five minutes, which ends the looking.
const RETRY_DEFERRED: Duration = Duration::from_secs(5);

/// How long the watcher's thread waits for a signal before draining on its own: forever when
/// nothing was deferred, [`RETRY_DEFERRED`] otherwise.
fn wait_for(deferred: usize) -> Option<Duration> {
    (deferred > 0).then_some(RETRY_DEFERRED)
}

/// Watches until dropped.
pub struct InboxWatcher {
    _watcher: RecommendedWatcher,
    wake: mpsc::Sender<()>,
}

impl InboxWatcher {
    /// Drain now, and keep draining on the retry clock while anything stays deferred.
    pub fn look_again(&self) {
        let _ = self.wake.send(());
    }
}

/// Watch the inbox directory and drain it after each burst of files. The directory is
/// created first — a watch needs something to watch — and the drain runs on its own thread,
/// so a hook landing during a page load never touches the UI thread.
pub fn watch_inbox(handle: AppHandle, data_dir: &Path) -> notify::Result<InboxWatcher> {
    let dir = yardsort_core::activity::inbox_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let (tx, rx) = mpsc::channel::<()>();
    let wake = tx.clone();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok_and(|event| brings_entries(&event.kind)) {
            let _ = tx.send(());
        }
    })?;
    watcher.watch(&dir, RecursiveMode::NonRecursive)?;
    let data_dir = data_dir.to_path_buf();
    std::thread::spawn(move || {
        let mut deferred = 0;
        loop {
            let woken = match wait_for(deferred) {
                Some(wait) => match rx.recv_timeout(wait) {
                    Ok(()) => true,
                    Err(RecvTimeoutError::Timeout) => false,
                    Err(RecvTimeoutError::Disconnected) => break,
                },
                None => match rx.recv() {
                    Ok(()) => true,
                    Err(_) => break,
                },
            };
            if woken {
                // Let the burst finish; whatever else arrives meanwhile is one drain.
                std::thread::sleep(INBOX_QUIET);
                while rx.try_recv().is_ok() {}
            }
            let Some(state) = handle.try_state::<AppState>() else {
                continue;
            };
            let report = yardsort_core::activity::import_inbox(&state.store, &data_dir);
            if !report.workspaces.is_empty() {
                let _ = ActivityChanged {
                    workspace_ids: report.workspaces.into_iter().collect(),
                }
                .emit(&handle);
            }
            deferred = report.deferred;
        }
    });
    Ok(InboxWatcher {
        _watcher: watcher,
        wake,
    })
}

/// Whether an event on the inbox directory can mean a new entry: a file created or renamed
/// into place. A drain *reads* the directory, which on Linux is an `Access` event of its own,
/// and a drain that woke the next drain would never let the worker sleep. Removals are the
/// drain's too.
fn brings_entries(kind: &EventKind) -> bool {
    matches!(kind, EventKind::Create(_) | EventKind::Modify(_))
}

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
    /// Where agents' hooks leave what they report, and how many entries are waiting there.
    pub inbox_dir: String,
    pub inbox_pending: u32,
    /// The settings file Claude Code launches are given when capture is on.
    pub claude_hooks_file: String,
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
        let inbox = yardsort_core::activity::inbox::Inbox::new(yardsort_core::activity::inbox_dir(
            &state.data_dir,
        ));
        let pending = |entries: std::io::Result<Vec<std::path::PathBuf>>| {
            entries
                .map(|entries| u32::try_from(entries.len()).unwrap_or(u32::MAX))
                .unwrap_or(0)
        };
        Ok(ActivityDiagnostics {
            events: events as f64,
            runs: runs as f64,
            spool_dir: spool.dir().display().to_string(),
            spool_pending: pending(spool.entries()),
            inbox_dir: inbox.dir().display().to_string(),
            inbox_pending: pending(inbox.entries()),
            claude_hooks_file: yardsort_core::activity::claude::settings_path(&state.data_dir)
                .display()
                .to_string(),
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
    capture_claude: bool,
    capture_codex: bool,
    capture_opencode: bool,
) -> IpcResult<SettingsInfo> {
    blocking(app, move |state| {
        state
            .settings
            .update(|settings| {
                settings.activity.record_lifecycle = record_lifecycle;
                settings.activity.show_timeline = show_timeline;
                settings.activity.capture_claude = capture_claude;
                settings.activity.capture_codex = capture_codex;
                settings.activity.capture_opencode = capture_opencode;
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

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, AccessMode, CreateKind, ModifyKind, RemoveKind, RenameMode};

    #[test]
    fn only_a_file_arriving_wakes_the_drain_never_the_drain_itself() {
        assert!(brings_entries(&EventKind::Create(CreateKind::File)));
        assert!(brings_entries(&EventKind::Modify(ModifyKind::Name(
            RenameMode::To
        ))));
        assert!(brings_entries(&EventKind::Modify(ModifyKind::Any)));
        // What a drain does: lists the directory, then removes what it took.
        assert!(!brings_entries(&EventKind::Access(AccessKind::Open(
            AccessMode::Any
        ))));
        assert!(!brings_entries(&EventKind::Access(AccessKind::Close(
            AccessMode::Read
        ))));
        assert!(!brings_entries(&EventKind::Remove(RemoveKind::File)));
        assert!(!brings_entries(&EventKind::Any));
        assert!(!brings_entries(&EventKind::Other));
    }

    /// A deferred entry — a Codex turn whose session file is still being written — gets a
    /// retry on a timer, because nothing else will wake the drain for it: the session file is
    /// not watched, and the agent may then sit waiting for the user with no exit in sight.
    #[test]
    fn the_watcher_looks_again_on_its_own_only_while_something_is_deferred() {
        assert_eq!(
            wait_for(0),
            None,
            "nothing waiting: sleep until a file lands"
        );
        assert_eq!(wait_for(1), Some(RETRY_DEFERRED));
        assert_eq!(wait_for(7), Some(RETRY_DEFERRED));
        assert!(
            RETRY_DEFERRED.as_millis() < yardsort_core::activity::codex::NOT_READY_GRACE_MS as u128
        );
    }
}
