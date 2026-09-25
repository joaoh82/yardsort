//! Terminal commands: a thin adapter between the webview and the PTY host.
//!
//! The launching itself lives in [`yardsort_core::launch`], so the `ys` CLI can do it too. What
//! is left here is the webview's side of it: the request and response shapes, the raw-byte
//! output channel, and building a [`Launcher`] out of app state.

use pty_host::{AttachmentId, HostError, HostEvent, SessionId, SessionInfo, TermSize};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::{Channel, InvokeResponseBody, IpcResponse};
use tauri::{AppHandle, State};

use crate::error::IpcResult;
use crate::state::{blocking, AppState};
use yardsort_core::launch::{self, Launcher};

pub use yardsort_core::env::EnvInfo;
pub use yardsort_core::launch::{
    resolve_launch, HarnessRequest, Launch, ResolvedLaunch, HARNESS_LABEL, HARNESS_SESSION_LABEL,
    RECORD_LABEL, WORKSPACE_LABEL,
};

/// Emitted for every [`HostEvent`].
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
pub struct PtyHostEvent(pub HostEvent);

/// Terminal output, sent over the channel as raw bytes rather than JSON: the webview receives
/// an `ArrayBuffer` it can hand straight to xterm.js.
///
/// The generated binding types this as `number[]` because specta only sees the `Vec<u8>`;
/// `src/lib/ipc.ts` corrects that in one place.
#[derive(Debug, Clone, Type)]
#[specta(transparent)]
pub struct RawBytes(Vec<u8>);

impl IpcResponse for RawBytes {
    fn body(self) -> tauri::Result<InvokeResponseBody> {
        Ok(InvokeResponseBody::Raw(self.0))
    }
}

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SpawnRequest {
    /// Program to run. `None` starts the user's shell.
    pub program: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    /// Working directory. `None` means the home directory. Ignored when `workspace_id` is set.
    pub cwd: Option<String>,
    /// Run inside this workspace: the core looks up its folder (the webview never supplies
    /// paths for this) and labels the session so it can be matched back to the workspace.
    pub workspace_id: Option<String>,
    /// Run a harness instead of `program`. Requires `workspace_id`.
    pub harness: Option<HarnessRequest>,
    pub size: TermSize,
}

#[tauri::command]
#[specta::specta]
pub async fn pty_spawn(app: AppHandle, request: SpawnRequest) -> IpcResult<SessionInfo> {
    // Resolving the environment and forking are both blocking.
    blocking(app, move |state| {
        let launch = match (request.harness, request.program) {
            (Some(harness), _) => Launch::Harness(harness),
            (None, Some(program)) => Launch::Program {
                program,
                args: request.args,
            },
            (None, None) => Launch::Shell,
        };
        match request.workspace_id {
            Some(workspace_id) => spawn_in_workspace(state, &workspace_id, launch, request.size),
            None => {
                let resolved = resolve_launch(launch, &state.settings.get().harnesses)?;
                start(state, resolved, request.cwd, request.size)
            }
        }
    })
    .await
}

/// Borrow app state as a [`Launcher`] for the length of one launch.
///
/// The environment is resolved on first use and the settings are a snapshot, so both are
/// temporaries — hence a closure rather than a returned value. Blocking: resolving the
/// environment runs the user's login shell.
pub(crate) fn with_launcher<T>(state: &AppState, f: impl FnOnce(&Launcher<'_>) -> T) -> T {
    let settings = state.settings.get();
    let env = state.env();
    f(&Launcher {
        store: &state.store,
        host: state.host.as_ref(),
        env: &env,
        harnesses: &settings.harnesses,
        activity: &settings.activity,
        launched_by: yardsort_core::activity::LaunchedBy::App,
    })
}

/// Start something in a workspace's folder, labelled so it can be matched back to it.
pub fn spawn_in_workspace(
    state: &AppState,
    workspace_id: &str,
    launch: Launch,
    size: TermSize,
) -> IpcResult<SessionInfo> {
    with_launcher(state, |launcher| {
        launcher.in_workspace(workspace_id, launch, size)
    })
}

/// Spawn a resolved launch.
pub fn start(
    state: &AppState,
    resolved: ResolvedLaunch,
    cwd: Option<String>,
    size: TermSize,
) -> IpcResult<SessionInfo> {
    with_launcher(state, |launcher| launcher.start(resolved, cwd, size))
}

/// A program can exit before its record exists, in which case the exit event found nothing to
/// update. Call this once the record is written to catch up.
pub fn settle_record(state: &AppState, session: &SessionInfo) {
    launch::settle_record(&state.store, state.host.as_ref(), session);
}

/// Stream a session into `output`: first a snapshot that repaints the terminal, then live bytes.
#[tauri::command]
#[specta::specta]
pub async fn pty_attach(
    state: State<'_, AppState>,
    id: SessionId,
    output: Channel<RawBytes>,
) -> IpcResult<AttachmentId> {
    let sink = Box::new(move |bytes: &[u8]| {
        // A failed send means the webview side is gone; returning false detaches us.
        output.send(RawBytes(bytes.to_vec())).is_ok()
    });
    Ok(state.host.attach(&id, sink)?)
}

#[tauri::command]
#[specta::specta]
pub async fn pty_detach(
    state: State<'_, AppState>,
    id: SessionId,
    attachment: AttachmentId,
) -> IpcResult<()> {
    Ok(state.host.detach(&id, attachment)?)
}

#[tauri::command]
#[specta::specta]
pub async fn pty_write(state: State<'_, AppState>, id: SessionId, data: String) -> IpcResult<()> {
    Ok(state.host.write(&id, data.as_bytes())?)
}

#[tauri::command]
#[specta::specta]
pub async fn pty_resize(
    state: State<'_, AppState>,
    id: SessionId,
    size: TermSize,
) -> IpcResult<()> {
    Ok(state.host.resize(&id, size)?)
}

#[tauri::command]
#[specta::specta]
pub async fn pty_kill(state: State<'_, AppState>, id: SessionId) -> IpcResult<()> {
    match state.host.kill(&id) {
        // Killing something that already ended is not a failure worth reporting.
        Ok(()) | Err(HostError::SessionExited(_)) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Kill (if needed) and forget a session.
#[tauri::command]
#[specta::specta]
pub async fn pty_close(state: State<'_, AppState>, id: SessionId) -> IpcResult<()> {
    Ok(state.host.remove(&id)?)
}

#[tauri::command]
#[specta::specta]
pub async fn pty_list(state: State<'_, AppState>) -> IpcResult<Vec<SessionInfo>> {
    Ok(state.host.list())
}

#[tauri::command]
#[specta::specta]
pub async fn env_info(app: AppHandle, reload: bool) -> IpcResult<EnvInfo> {
    blocking(app, move |state| {
        let env = if reload {
            state.reload_env()
        } else {
            state.env()
        };
        Ok(EnvInfo::from(&*env))
    })
    .await
}
