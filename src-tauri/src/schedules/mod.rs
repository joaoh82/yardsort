//! Schedules' home in the app: a thread that looks, every second, for schedules whose time has
//! come, asks for their runs, and tells the user how the runs it started ended. See
//! `yardsort_core::schedule::fire` for the rules; this module owns the thread, the clock it
//! reads, and what only the app can do: wake the drivers, emit events, show notifications.
//!
//! It runs only in the app that drives runs — the one holding the profile's lock — so a
//! schedule fires once however many windows are open on a profile, and never while a run it
//! queued would have nobody to carry it out.
//!
//! A thread of its own, not the workflow driver's: queueing a workflow that works with a pull
//! request asks the forge, and a slow forge must not stall every run in the profile. Nor may
//! it stall every other schedule for long: that question is given a limit
//! (`yardsort_core::workflow::runs::PULL_REQUEST_LIMIT`), since this is the only thread that
//! fires schedules and tells how their runs ended.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;
use tauri_specta::Event;
use yardsort_core::schedule::{clock, fire};

use crate::state::AppState;
use crate::workflows::commands::AppLook;

pub mod commands;

/// How often it looks when nothing wakes it sooner. A schedule's finest grain is a minute, so
/// this is how late a time can be acted on, not how often anything fires.
const TICK: Duration = Duration::from_secs(1);

/// Schedules changed: one was added, changed, switched, removed or fired, or a run one started
/// ended. The window reloads what it shows of them.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct SchedulesChanged {}

/// Managed by Tauri while the thread runs.
pub struct Scheduler {
    wake: Mutex<Sender<()>>,
}

impl Scheduler {
    /// Look now rather than at the next tick.
    pub fn wake(&self) {
        let _ = self
            .wake
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send(());
    }
}

/// Start looking, if this app is the one that drives runs. Called only in the copy holding the
/// profile (`one_copy`), after the workflow driver has started.
pub fn start(app: &AppHandle) {
    if app.try_state::<crate::workflows::Driver>().is_none() {
        eprintln!("schedules: another Yardsort on this profile fires them; not firing here");
        return;
    }
    let (wake, woken) = mpsc::channel();
    app.manage(Scheduler {
        wake: Mutex::new(wake),
    });
    let handle = app.clone();
    let spawned = std::thread::Builder::new()
        .name("schedules".into())
        .spawn(move || look(&handle, &woken));
    if let Err(error) = spawned {
        eprintln!("schedules: cannot start the thread: {error}");
    }
}

fn look(handle: &AppHandle, woken: &Receiver<()>) {
    // The machine's offset from UTC the last time round. `None` at start, so times of day are
    // worked out once for the zone the app opened in.
    let mut offset = None;
    // What the list looked like the last time round, so a schedule `ys` added, switched or
    // removed reaches the window without waiting for one to fire.
    let mut stamp = None;
    loop {
        match woken.recv_timeout(TICK) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        while woken.try_recv().is_ok() {}
        let Some(state) = handle.try_state::<AppState>() else {
            continue;
        };
        let now = clock::now();
        let mut changed = false;

        let here = clock::utc_offset_seconds(&now);
        if offset != Some(here) {
            offset = Some(here);
            match fire::rezone(&state.store, &now) {
                Ok(moved) => changed |= moved,
                Err(error) => eprintln!("schedules: cannot read schedules: {}", error.message),
            }
        }

        match fire::tick(&state.store, &state.data_dir, &AppLook(&state), &now) {
            Ok(fired) => {
                changed |= !fired.is_empty();
                for notice in fired.iter().filter_map(|fired| fired.notice.as_ref()) {
                    tell(handle, notice);
                }
                if fired.iter().any(|fired| fired.run.run_id.is_some()) {
                    // A run was queued: its driver should look now, and the window's run lists
                    // are out of date.
                    let _ = crate::workflows::commands::WorkflowRunsChanged {}.emit(handle);
                    crate::smoke::changed(handle);
                }
            }
            Err(error) => eprintln!("schedules: cannot read schedules: {}", error.message),
        }

        match fire::settle(&state.store) {
            Ok(settled) => {
                changed |= settled.changed;
                for notice in &settled.notices {
                    tell(handle, notice);
                }
            }
            Err(error) => eprintln!("schedules: cannot read runs: {}", error.message),
        }

        if let Ok(now) = state.store.schedules_stamp() {
            changed |= stamp.is_some_and(|before| before != now);
            stamp = Some(now);
        }

        if changed {
            let _ = SchedulesChanged {}.emit(handle);
        }
    }
}

/// Show a desktop notification about a schedule.
fn tell(handle: &AppHandle, notice: &fire::Notice) {
    let shown = handle
        .notification()
        .builder()
        .title(&notice.title)
        .body(&notice.body)
        .show();
    if let Err(error) = shown {
        eprintln!("schedules: could not show a notification: {error}");
    }
}
