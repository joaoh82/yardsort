//! The smoke test runner's home in the app: queued runs are carried out here, on threads of
//! their own, and the window is told what changed. See `yardsort_core::smoke::runner` for what
//! a run does; this module owns the driver, and what only the app can do: emit events.
//!
//! The runner is ticked from the workflow driver's thread, every second and whenever that
//! thread is woken, so it runs only while the app holds the profile's lock: one app on a
//! profile carries its runs out, and `ys test run` refuses when none is open.

use std::sync::Arc;

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;
use yardsort_core::smoke::runner::{self, Notice};

use crate::state::AppState;

pub mod commands;

/// Runs changed: one was queued, approved, moved on, finished or cancelled, or a step changed.
/// The window reloads what it shows of them.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct TestRunsChanged {}

/// A step's output file grew. The tab tailing it reads on.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct TestRunOutput {
    pub run_id: String,
    pub step_id: String,
}

/// Managed by Tauri while the app drives runs.
pub struct Runner {
    driver: runner::Driver,
}

/// Make the driver. Called by the workflow driver once it holds the profile's lock.
pub fn start(app: &AppHandle) {
    let handle = app.clone();
    let notify: runner::Notify = Arc::new(move |notice| match notice {
        Notice::Runs => {
            let _ = TestRunsChanged {}.emit(&handle);
        }
        Notice::Output { run_id, step_id } => {
            let _ = TestRunOutput { run_id, step_id }.emit(&handle);
        }
    });
    app.manage(Runner {
        driver: runner::Driver::new(notify),
    });
}

/// Start what is queued and notice what ended. Called on the workflow driver's thread.
pub fn tick(handle: &AppHandle, state: &AppState) {
    let Some(runner) = handle.try_state::<Runner>() else {
        return;
    };
    match runner
        .driver
        .tick(&state.store, &state.data_dir, &state.env())
    {
        Ok(true) => {
            let _ = TestRunsChanged {}.emit(handle);
        }
        Ok(false) => {}
        Err(error) => eprintln!("smoke tests: cannot read runs: {}", error.message),
    }
}

/// Tell the window runs changed, from a command that changed one.
pub fn changed(app: &AppHandle) {
    let _ = TestRunsChanged {}.emit(app);
    if let Some(driver) = app.try_state::<crate::workflows::Driver>() {
        driver.wake();
    }
}
