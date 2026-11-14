//! Session records' commands: listing, resuming, forking and forgetting a workspace's agent
//! conversations. What a record allows, and continuing one, is the core's
//! ([`yardsort_core::sessions`]), shared with `ys session resume` and `ys session fork`.

use pty_host::{SessionInfo, TermSize};
use tauri::AppHandle;

use crate::error::{IpcError, IpcResult};
use crate::harness;
use crate::state::{blocking, AppState};
use crate::terminal::with_launcher;
use yardsort_core::activity::context::{self, ContextUsage};
pub use yardsort_core::sessions::{Continue, SessionRecord};

pub(crate) fn list(state: &AppState, workspace_id: &str) -> IpcResult<Vec<SessionRecord>> {
    yardsort_core::sessions::list(&state.store, &state.settings.get().harnesses, workspace_id)
}

/// Start a harness on an existing conversation, typing `prompt` into it once it is ready.
pub(crate) fn continue_session(
    state: &AppState,
    record_id: &str,
    how: Continue,
    size: TermSize,
    prompt: Option<String>,
) -> IpcResult<SessionInfo> {
    let session = with_launcher(state, |launcher| {
        launcher.continue_session(record_id, how, size, prompt)
    })?;
    crate::activity::launched(state);
    Ok(session)
}

#[tauri::command]
#[specta::specta]
pub async fn sessions_list(app: AppHandle, workspace_id: String) -> IpcResult<Vec<SessionRecord>> {
    blocking(app, move |state| list(state, &workspace_id)).await
}

/// How full a conversation's context is, as the agent last reported it, and what to type to
/// compact it. `None` when nothing was reported since it started or last compacted.
#[tauri::command]
#[specta::specta]
pub async fn session_context(app: AppHandle, id: String) -> IpcResult<Option<ContextUsage>> {
    blocking(app, move |state| {
        let Some(row) = state.store.session(&id)? else {
            return Ok(None);
        };
        let compact = harness::find(&row.harness_id, &state.settings.get().harnesses)
            .and_then(|def| def.compact_command);
        Ok(context::session_context(&state.store, &row, compact)?)
    })
    .await
}

/// Continue a conversation whose process has ended, in a new terminal.
#[tauri::command]
#[specta::specta]
pub async fn session_resume(app: AppHandle, id: String, size: TermSize) -> IpcResult<SessionInfo> {
    blocking(app, move |state| {
        continue_session(state, &id, Continue::Resume, size, None)
    })
    .await
}

/// Start a copy of a conversation — running or not — that goes its own way from here.
#[tauri::command]
#[specta::specta]
pub async fn session_fork(app: AppHandle, id: String, size: TermSize) -> IpcResult<SessionInfo> {
    blocking(app, move |state| {
        continue_session(state, &id, Continue::Fork, size, None)
    })
    .await
}

/// Drop a record from the history. The harness's own copy of the conversation is not touched.
#[tauri::command]
#[specta::specta]
pub async fn session_forget(app: AppHandle, id: String) -> IpcResult<()> {
    blocking(app, move |state| {
        if state.store.session(&id)?.is_some_and(|row| row.running) {
            return Err(IpcError::new(
                "session_running",
                "Close the session's terminal before removing it from the history.",
            ));
        }
        state.store.remove_session(&id)?;
        Ok(())
    })
    .await
}

/// Give a conversation a label, so its tab says what it is doing. Empty clears it. The label
/// is the record's, so it outlives the process: a resume keeps it.
#[tauri::command]
#[specta::specta]
pub async fn session_set_label(app: AppHandle, id: String, label: String) -> IpcResult<String> {
    blocking(app, move |state| {
        let label = yardsort_core::sessions::tidy_label(&label);
        if !state.store.set_session_label(&id, &label)? {
            return Err(IpcError::new(
                "unknown_session",
                "That conversation is no longer in the history.",
            ));
        }
        Ok(label)
    })
    .await
}
