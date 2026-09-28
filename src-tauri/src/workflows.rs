//! The workflow driver's home in the app: a background thread, never the UI's, that moves
//! queued and running workflow runs on. See `yardsort_core::workflow::driver` for what a tick
//! does; this module owns the thread, the wake-ups and what only the app can do.
//!
//! It wakes every second, and at once when a session goes quiet or ends. Runs are picked up
//! from the database whoever queued them: `ys workflow run` writes the row and nothing else.
//!
//! The driver runs only while it holds the profile's app lock (`yardsort_core::presence`). That
//! is also how `ys` knows the app is running, and it keeps two apps on one profile from driving
//! the same run twice.

use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use pty_host::{HostEvent, SessionInfo, TermSize};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;
use tauri_specta::Event;
use yardsort_core::launch::{HarnessRequest, Launch};
use yardsort_core::presence::AppLock;
use yardsort_core::workflow::driver::{self, Hands, Settled};

use crate::state::AppState;

/// How often the driver looks when nothing wakes it sooner: queued runs start within this, and
/// timeouts are this precise.
const TICK: Duration = Duration::from_secs(1);

/// The size an agent is started at. Its tab resizes it when the window shows it.
const SIZE: TermSize = TermSize {
    cols: 120,
    rows: 30,
};

/// A workflow run started a session in a workspace: the window should show it as a tab.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct SessionStarted {
    pub workspace_id: String,
    pub session_id: String,
}

/// Managed by Tauri while the driver runs.
pub struct Driver {
    wake: Mutex<Sender<()>>,
    settled: Settled,
    _lock: AppLock,
}

impl Driver {
    /// Every host event passes through here. A quiet or an exit is a reason to look now.
    pub fn observe(&self, event: &HostEvent) {
        if self.settled.observe(event) {
            let _ = self
                .wake
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .send(());
        }
    }
}

/// Take the profile's lock and start driving. Without the lock another app on this profile is
/// already the driver, and this one leaves runs to it.
pub fn start(app: &AppHandle, data_dir: &Path) {
    let lock = match AppLock::take(data_dir) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            eprintln!("workflows: another Yardsort on this profile runs them; not driving here");
            return;
        }
        Err(error) => {
            eprintln!("workflows: cannot take the app lock, not driving: {error}");
            return;
        }
    };
    let (wake, woken) = mpsc::channel();
    app.manage(Driver {
        wake: Mutex::new(wake),
        settled: Settled::default(),
        _lock: lock,
    });
    let handle = app.clone();
    let spawned = std::thread::Builder::new()
        .name("workflows".into())
        .spawn(move || drive(&handle, &woken));
    if let Err(error) = spawned {
        eprintln!("workflows: cannot start the driver: {error}");
    }
}

fn drive(handle: &AppHandle, woken: &Receiver<()>) {
    let mut recovered = HashSet::new();
    loop {
        match woken.recv_timeout(TICK) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        // One pass answers every wake that arrived meanwhile.
        while woken.try_recv().is_ok() {}
        let (Some(state), Some(driver)) =
            (handle.try_state::<AppState>(), handle.try_state::<Driver>())
        else {
            continue;
        };
        let hands = AppHands {
            handle,
            state: &state,
        };
        if let Err(error) = driver::tick(
            &state.store,
            state.host.as_ref(),
            &hands,
            &driver.settled,
            &mut recovered,
        ) {
            eprintln!("workflows: cannot read runs: {}", error.message);
        }
    }
}

struct AppHands<'a> {
    handle: &'a AppHandle,
    state: &'a AppState,
}

impl Hands for AppHands<'_> {
    fn start(&self, workspace_id: &str, request: HarnessRequest) -> Result<SessionInfo, String> {
        let session = crate::terminal::spawn_in_workspace(
            self.state,
            workspace_id,
            Launch::Harness(request),
            SIZE,
        )
        .map_err(|error| error.message)?;
        let _ = SessionStarted {
            workspace_id: workspace_id.to_owned(),
            session_id: session.id.0.clone(),
        }
        .emit(self.handle);
        Ok(session)
    }

    fn notify(&self, title: &str, body: Option<&str>) -> Result<(), String> {
        let mut notification = self.handle.notification().builder().title(title);
        if let Some(body) = body {
            notification = notification.body(body);
        }
        notification
            .show()
            .map_err(|error| format!("Could not show the notification: {error}"))
    }

    fn handoff(&self, workspace_id: &str) -> Option<String> {
        use yardsort_core::activity::handoff;
        let state = self.state;
        let (row, root) = crate::changes::commands::workspace(state, workspace_id).ok()?;
        let git = crate::git::Git::new(&state.env()).ok()?;
        let facts = handoff::facts(
            &state.store,
            &git,
            &root,
            &row.id,
            row.base_branch.as_deref(),
        )
        .ok()?;
        Some(handoff::render(&facts))
    }
}
