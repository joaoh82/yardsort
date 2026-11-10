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
use yardsort_core::launch::Launcher;

pub use yardsort_core::env::EnvInfo;
pub use yardsort_core::launch::{
    resolve_launch, HarnessRequest, Launch, ResolvedLaunch, HARNESS_LABEL, RECORD_LABEL,
};

/// Emitted for every [`HostEvent`].
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
pub struct PtyHostEvent(pub HostEvent);

/// The sessions this app has heard from.
///
/// The daemon tells every client about every session, including the ones `ys` started from a
/// terminal. A session's first word is the first the app knows of it, which is when the window
/// is told to give it a tab — and, when `ys workspace new` made it, to show its workspace.
#[derive(Default)]
pub struct Heard(std::sync::Mutex<std::collections::HashSet<String>>);

impl Heard {
    /// Sessions already there, which the window picks up on its own when it loads.
    pub fn already(&self, ids: impl IntoIterator<Item = String>) {
        self.0.lock().unwrap().extend(ids);
    }

    /// The session's id, when this event is the first heard from it. An exit is the last, so
    /// its session is forgotten then, and the set does not grow for as long as the app runs.
    pub fn first<'a>(&self, event: &'a HostEvent) -> Option<&'a str> {
        let (HostEvent::Busy { id } | HostEvent::Quiet { id, .. } | HostEvent::Exited { id, .. }) =
            event;
        let mut heard = self.0.lock().unwrap();
        let first = if matches!(event, HostEvent::Exited { .. }) {
            !heard.remove(&id.0)
        } else {
            heard.insert(id.0.clone())
        };
        first.then_some(id.0.as_str())
    }
}

/// How often, and how many times, to look for a session the daemon does not list yet. A
/// session's first output can come before the spawn has added it to the list — the pump starts
/// first — and the list is asked for over a connection of its own, so it can lose that race.
const LOOK_EVERY: std::time::Duration = std::time::Duration::from_millis(50);
const LOOKS: u32 = 40;

/// If `ys` started session `id`, tell the window, as for one a workflow started. Blocks on the
/// daemon, and waits for it to list the session, so it is not for the thread that delivers its
/// events.
pub fn announce_if_cli(handle: &AppHandle, id: &str) {
    use tauri::Manager;
    use tauri_specta::Event;
    let Some(state) = handle.try_state::<AppState>() else {
        return;
    };
    let Some(workspace_id) = cli_workspace(|| state.host.list(), id, LOOK_EVERY, LOOKS) else {
        return;
    };
    crate::activity::launched(&state);
    let _ = crate::workflows::SessionStarted {
        workspace_id,
        session_id: id.to_owned(),
    }
    .emit(handle);
}

/// The workspace of session `id` when `ys` started it. A session not listed yet is looked for
/// again, `looks` times in all, `every` apart; one that is listed is decided on at once.
fn cli_workspace(
    list: impl Fn() -> Vec<SessionInfo>,
    id: &str,
    every: std::time::Duration,
    looks: u32,
) -> Option<String> {
    for look in 1..=looks {
        if let Some(session) = list().into_iter().find(|s| s.id.0 == id) {
            return yardsort_core::launch::started_by_cli(&session).map(str::to_owned);
        }
        if look < looks {
            std::thread::sleep(every);
        }
    }
    None
}

/// Terminal output, sent over the channel as raw bytes rather than JSON: the webview receives
/// an `ArrayBuffer` it can hand straight to xterm.js.
///
/// The generated binding types this as `number[]` because specta only sees the `Vec<u8>`;
/// `src/lib/ipc.ts` corrects that in one place.
#[derive(Debug, Clone, Type)]
#[specta(transparent)]
pub struct RawBytes(Vec<u8>);

impl RawBytes {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

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
        data_dir: &state.data_dir,
    })
}

/// Start something in a workspace's folder, labelled so it can be matched back to it.
pub fn spawn_in_workspace(
    state: &AppState,
    workspace_id: &str,
    launch: Launch,
    size: TermSize,
) -> IpcResult<SessionInfo> {
    let session = with_launcher(state, |launcher| {
        launcher.in_workspace(workspace_id, launch, size)
    })?;
    crate::activity::launched(state);
    Ok(session)
}

/// Spawn a resolved launch.
pub fn start(
    state: &AppState,
    resolved: ResolvedLaunch,
    cwd: Option<String>,
    size: TermSize,
) -> IpcResult<SessionInfo> {
    let session = with_launcher(state, |launcher| launcher.start(resolved, cwd, size))?;
    crate::activity::launched(state);
    Ok(session)
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

#[cfg(test)]
mod tests {
    use super::*;
    use pty_host::ExitInfo;

    fn session(id: &str, by: &str) -> SessionInfo {
        SessionInfo {
            id: SessionId(id.into()),
            program: "agent".into(),
            args: vec![],
            cwd: None,
            pid: None,
            size: TermSize { cols: 80, rows: 24 },
            labels: [
                (yardsort_core::launch::LAUNCHED_BY_LABEL.into(), by.into()),
                (yardsort_core::launch::WORKSPACE_LABEL.into(), "ws-1".into()),
            ]
            .into(),
            state: pty_host::SessionState::Running,
            has_output: true,
            busy: true,
            busy_ms: 0,
            idle_ms: 0,
        }
    }

    /// The daemon can tell of a session's first output before it lists the session: the pump
    /// starts before the spawn records it. That session is still announced once it is listed,
    /// and one that is never listed is given up on after the last look.
    #[test]
    fn a_session_heard_from_before_the_daemon_lists_it_is_still_found() {
        let asked = std::cell::Cell::new(0);
        let list = |by: &'static str, listed_from: u32| {
            let asked = &asked;
            move || {
                asked.set(asked.get() + 1);
                if asked.get() >= listed_from {
                    vec![session("other", "cli"), session("new", by)]
                } else {
                    vec![session("other", "cli")]
                }
            }
        };
        let every = std::time::Duration::from_millis(1);

        let found = cli_workspace(list("cli", 3), "new", every, 40);
        assert_eq!(found.as_deref(), Some("ws-1"));
        assert_eq!(asked.get(), 3, "asked until it was listed, and no more");

        asked.set(0);
        assert_eq!(cli_workspace(list("app", 1), "new", every, 40), None);
        assert_eq!(
            asked.get(),
            1,
            "a session the app started is decided on at once"
        );

        asked.set(0);
        assert_eq!(cli_workspace(list("cli", u32::MAX), "new", every, 5), None);
        assert_eq!(asked.get(), 5, "never listed: given up after the last look");
    }

    fn busy(id: &str) -> HostEvent {
        HostEvent::Busy {
            id: SessionId(id.into()),
        }
    }

    /// Only a session's first event is news; one already running when the app started, or
    /// already heard from, is not. An exit is the last word, heard once.
    #[test]
    fn a_session_is_news_only_the_first_time_it_is_heard_from() {
        let heard = Heard::default();
        heard.already(["running-at-start".to_owned()]);
        assert_eq!(heard.first(&busy("running-at-start")), None);

        assert_eq!(heard.first(&busy("new")), Some("new"));
        assert_eq!(heard.first(&busy("new")), None);
        let quiet = HostEvent::Quiet {
            id: SessionId("new".into()),
            busy_ms: 10,
        };
        assert_eq!(heard.first(&quiet), None);
        let exited = |id: &str| HostEvent::Exited {
            id: SessionId(id.into()),
            exit: ExitInfo {
                code: 0,
                success: true,
                signal: None,
            },
        };
        assert_eq!(heard.first(&exited("new")), None, "heard from before");
        assert_eq!(
            heard.first(&exited("silent")),
            Some("silent"),
            "a session that exits without a word is still news"
        );
    }
}
