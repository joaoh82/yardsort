//! Closing the window when agents are still working.
//!
//! Before the daemon this needed no thought: quitting killed everything, because everything was
//! a child of the app. Now closing the window is a disconnect and the agents carry on — which is
//! the point, but it is also exactly the kind of thing a person should not discover by accident.
//! So a quit with work in flight asks first, and names what is at stake.

use std::sync::atomic::{AtomicBool, Ordering};

use pty_host::SessionState;
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::error::IpcResult;
use crate::state::{blocking, AppState};
use crate::terminal::HARNESS_LABEL;

/// Asks the webview to put the question to the user. It answers with [`app_quit`].
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct QuitRequested {
    /// The harness sessions still running, as ids the frontend can match to its tabs.
    pub agents: Vec<String>,
}

/// Whether the question has already been asked. A second attempt to close — the user clicking
/// the X again because nothing appeared — goes straight through, so a webview that has stopped
/// answering can never trap somebody in their own app.
static ASKED: AtomicBool = AtomicBool::new(false);

/// Called for every close request. Returns true when the window may close straight away.
///
/// Otherwise the close is held and the decision made on a thread of its own: asking the host
/// what is running means a round trip to the daemon, and the window event handler runs on the
/// thread that draws the UI. Either the dialog appears, or the app exits a moment later.
pub fn may_close_now(app: &AppHandle) -> bool {
    if ASKED.swap(true, Ordering::SeqCst) {
        return true;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("quit-check".into())
        .spawn(move || {
            let agents = running_agents(&app);
            // Nothing at stake, or nobody listening to be asked: go. Leaving the agents running
            // is the safe half of the choice anyway.
            if agents.is_empty() || (QuitRequested { agents }).emit(&app).is_err() {
                app.exit(0);
            }
        });
    if let Err(error) = spawned {
        eprintln!("could not check what is still running: {error}");
        return true;
    }
    false
}

/// The still-running harness sessions. Shells are not counted: a shell sitting at a prompt is
/// not work in progress, and asking about it would make the question meaningless.
fn running_agents(app: &AppHandle) -> Vec<String> {
    let Some(state) = app.try_state::<AppState>() else {
        return Vec::new();
    };
    state
        .host
        .list()
        .into_iter()
        .filter(|session| matches!(session.state, SessionState::Running))
        .filter(|session| session.labels.contains_key(HARNESS_LABEL))
        .map(|session| session.id.0)
        .collect()
}

/// The user's answer. `stop_agents` kills everything the daemon runs and stops it; otherwise the
/// agents are left to carry on and the app simply lets go of them.
#[tauri::command]
#[specta::specta]
pub async fn app_quit(app: AppHandle, stop_agents: bool) -> IpcResult<()> {
    let quitting = app.clone();
    blocking(app, move |state| {
        if stop_agents {
            // Kill by hand as well as asking the daemon to: under `YARDSORT_NO_DAEMON` there is
            // no daemon to ask, and the sessions would otherwise outlive this call by a whisker.
            for session in state.host.list() {
                let _ = state.host.kill(&session.id);
            }
            if let Some(client) = &state.daemon_client {
                let _ = client.shutdown(true);
            }
        }
        Ok(())
    })
    .await?;
    // Past the question now: let the close through rather than ask it again.
    ASKED.store(true, Ordering::SeqCst);
    quitting.exit(0);
    Ok(())
}

/// Let the next close request ask again. Called when the user cancels, so that closing the
/// window later still puts the question.
#[tauri::command]
#[specta::specta]
pub async fn quit_cancelled() -> IpcResult<()> {
    ASKED.store(false, Ordering::SeqCst);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A webview that has stopped answering must not be able to trap somebody in the app: the
    /// question is asked once, and a second attempt closes.
    #[test]
    fn asking_twice_is_not_asking_at_all() {
        ASKED.store(false, Ordering::SeqCst);
        assert!(!ASKED.swap(true, Ordering::SeqCst), "the first ask");
        assert!(
            ASKED.swap(true, Ordering::SeqCst),
            "the second goes through"
        );
        ASKED.store(false, Ordering::SeqCst);
    }
}
