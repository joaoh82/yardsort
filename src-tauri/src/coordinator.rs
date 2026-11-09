//! The Floor Manager's commands: what the panel in the window shows, starting it, and stopping
//! it. What it is and how it is launched is [`yardsort_core::coordinator`].

use pty_host::{HostError, SessionInfo, SessionState, TermSize};
use serde::Serialize;
use specta::Type;
use tauri::AppHandle;

use crate::error::IpcResult;
use crate::state::blocking;
use crate::terminal::with_launcher;
use yardsort_core::coordinator;
use yardsort_core::launch::HarnessRequest;

/// What the panel needs to draw itself.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CoordinatorStatus {
    /// The one the daemon holds: running, or the last to end. `None` before the first start.
    pub session: Option<SessionInfo>,
    /// The harness it runs, or ran.
    pub harness: Option<String>,
    /// Its folder, where its instructions are.
    pub folder: String,
    /// Whether this copy of the app has a `ys` of its own to give it. Without one it finds
    /// whatever `ys` the user installed, or none.
    pub has_ys: bool,
}

fn status(state: &crate::state::AppState) -> CoordinatorStatus {
    let session = coordinator::current(state.host.as_ref());
    CoordinatorStatus {
        harness: session
            .as_ref()
            .and_then(|s| s.labels.get(coordinator::LABEL).cloned()),
        session,
        folder: coordinator::home(&state.data_dir)
            .to_string_lossy()
            .into_owned(),
        has_ys: crate::ys::Layout::detect(&state.env()).bundled.is_some(),
    }
}

/// What the Floor Manager panel shows: the one the daemon holds, and where it lives.
#[tauri::command]
#[specta::specta]
pub async fn coordinator_status(app: AppHandle) -> IpcResult<CoordinatorStatus> {
    blocking(app, |state| Ok(status(state))).await
}

/// Start the Floor Manager with `harness`, or hand back the one already running.
#[tauri::command]
#[specta::specta]
pub async fn coordinator_start(
    app: AppHandle,
    harness: String,
    size: TermSize,
) -> IpcResult<CoordinatorStatus> {
    blocking(app, move |state| {
        let ys = crate::ys::Layout::detect(&state.env()).bundled;
        let ys_dir = ys.as_deref().and_then(std::path::Path::parent);
        let request = HarnessRequest {
            id: harness,
            model: None,
            effort: None,
            prompt: None,
            handoff: false,
            skip_memory: true,
            quiet_memory: true,
            skip_context: true,
            fork_packet: None,
        };
        with_launcher(state, |launcher| {
            launcher.coordinator(request, ys_dir, size)
        })?;
        Ok(status(state))
    })
    .await
}

/// End the running Floor Manager. Its last screen stays until it is started again.
#[tauri::command]
#[specta::specta]
pub async fn coordinator_stop(app: AppHandle) -> IpcResult<CoordinatorStatus> {
    blocking(app, |state| {
        if let Some(session) = coordinator::current(state.host.as_ref()) {
            if matches!(session.state, SessionState::Running) {
                match state.host.kill(&session.id) {
                    Ok(()) | Err(HostError::SessionExited(_)) => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        Ok(status(state))
    })
    .await
}
