//! Triggers' home in the app: a thread that hears what happened — agents' turns and exits, done
//! signals, runs that ended — adds it to the triggers listening for it, decides the fires that
//! are due, and tells the user how the runs it started ended. See
//! `yardsort_core::trigger::fire` for the rules and `yardsort_core::trigger::sources` for what
//! is heard; this module owns the thread, the host's activity, which only the app hears, and
//! what only the app can do: wake the drivers, emit events, show notifications.
//!
//! It runs only in the app that drives runs — the one holding the profile's lock — so a trigger
//! fires once however many windows are open on a profile.
//!
//! A thread of its own, for the schedules thread's reason: queueing a workflow that works with
//! a pull request asks the forge, and a slow forge must not stall every run in the profile.
//!
//! The watched pull requests — a workspace's own, and those it tracks elsewhere — are looked at
//! by a second thread, once a minute each (`yardsort_core::trigger::watch`): every look is a question to GitHub that may take up to
//! its 20-second limit, and the first thread's one-second tick must not wait for it. What a look
//! finds is handed to the first thread, which is the only one that gathers and decides.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use pty_host::HostEvent;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;
use tauri_specta::Event;
use yardsort_core::schedule::clock;
use yardsort_core::smoke::deliver::{self, Delivery, Tried};
use yardsort_core::trigger::fire::Observation;
use yardsort_core::trigger::turns::{Change, Turns};
use yardsort_core::trigger::{fire, sources, watch};

use crate::state::AppState;
use crate::workflows::commands::AppLook;

pub mod commands;

use commands::TriggersChanged;

/// How often it looks when nothing wakes it sooner: how late a fire whose quiet time is up can
/// be decided.
const TICK: Duration = Duration::from_secs(1);

/// How often the pull request thread checks which watches are due a look. Each is looked at
/// once a minute at most (`watch::LOOK_EVERY_MS`).
const WATCH_TICK: Duration = Duration::from_secs(5);

/// Managed by Tauri while the thread runs.
pub struct Listener {
    wake: Mutex<Sender<()>>,
    turns: Turns,
    /// What the host said since the last look, with when.
    heard: Mutex<Vec<(Change, i64)>>,
    /// What looks at pull requests found since the last look.
    forge: Mutex<Vec<Observation>>,
    /// **Tell the agent**: what agents are to be told about `ys workspace done`, each once it
    /// is quiet. Tried at every look, which the host's quiet events wake.
    tells: Mutex<Vec<Delivery>>,
}

impl Listener {
    /// Every host event passes through here. A turn ending, or an agent going back to work, is
    /// a reason to look now.
    pub fn observe(&self, event: &HostEvent) {
        let now = clock::now().timestamp_millis();
        if let Some(change) = self.turns.observe(event, now) {
            self.heard
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((change, now));
            self.wake();
        }
    }

    /// Tell these agents, each once it is quiet.
    pub fn tell(&self, deliveries: Vec<Delivery>) {
        self.tells
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(deliveries);
        self.wake();
    }

    /// Type what can be typed now; keep what waits for a busy agent. An agent that ended, or
    /// that cannot be typed into, is not told.
    fn deliver(&self, host: &dyn pty_host::TerminalHost) {
        let waiting =
            std::mem::take(&mut *self.tells.lock().unwrap_or_else(PoisonError::into_inner));
        let mut kept = Vec::new();
        for delivery in waiting {
            match deliver::try_deliver(host, &delivery) {
                Tried::Delivered => {}
                Tried::NotYet => kept.push(delivery),
                Tried::Gone(why) => eprintln!("triggers: could not tell an agent: {why}"),
            }
        }
        let mut tells = self.tells.lock().unwrap_or_else(PoisonError::into_inner);
        // Anything asked for while this ran goes behind what is still waiting.
        kept.append(&mut tells);
        *tells = kept;
    }

    /// Look now rather than at the next tick.
    pub fn wake(&self) {
        let _ = self
            .wake
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send(());
    }
}

/// Start listening, if this app is the one that drives runs. Called only in the copy holding
/// the profile (`one_copy`), after the workflow driver has started.
pub fn start(app: &AppHandle) {
    if app.try_state::<crate::workflows::Driver>().is_none() {
        eprintln!("triggers: another Yardsort on this profile fires them; not firing here");
        return;
    }
    let (wake, woken) = mpsc::channel();
    app.manage(Listener {
        wake: Mutex::new(wake),
        turns: Turns::default(),
        heard: Mutex::new(Vec::new()),
        forge: Mutex::new(Vec::new()),
        tells: Mutex::new(Vec::new()),
    });
    let handle = app.clone();
    let spawned = std::thread::Builder::new()
        .name("triggers".into())
        .spawn(move || look(&handle, &woken));
    if let Err(error) = spawned {
        eprintln!("triggers: cannot start the thread: {error}");
    }
    let handle = app.clone();
    let spawned = std::thread::Builder::new()
        .name("trigger-pull-requests".into())
        .spawn(move || watch_pull_requests(&handle));
    if let Err(error) = spawned {
        eprintln!("triggers: cannot start the pull request thread: {error}");
    }
}

/// Look at each watched pull request once a minute, and hand what changed to the trigger
/// thread.
fn watch_pull_requests(handle: &AppHandle) {
    loop {
        std::thread::sleep(WATCH_TICK);
        let (Some(state), Some(listener)) = (
            handle.try_state::<AppState>(),
            handle.try_state::<Listener>(),
        ) else {
            continue;
        };
        let git = match crate::git::Git::new(&state.env()) {
            Ok(git) => git,
            Err(error) => {
                eprintln!("triggers: {error}");
                continue;
            }
        };
        let store = &state.store;
        let due = match watch::due(store, clock::now().timestamp_millis()) {
            Ok(due) => due,
            Err(error) => {
                eprintln!("triggers: cannot read the watches: {}", error.message);
                continue;
            }
        };
        for watched in due {
            // The time of this look, not of the tick: a look before it may have taken a while.
            let now = clock::now().timestamp_millis();
            match watch::look_at_watched(store, &AppLook(&state), &git, &watched, now) {
                Ok(seen) if seen.is_empty() => {}
                Ok(seen) => {
                    listener
                        .forge
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .extend(seen);
                    listener.wake();
                }
                Err(error) => eprintln!(
                    "triggers: cannot look at {}'s pull request {}: {}",
                    watched.workspace.name,
                    watched.tracked.as_ref().map_or("", |t| t.url.as_str()),
                    error.message
                ),
            }
        }
    }
}

fn look(handle: &AppHandle, woken: &Receiver<()>) {
    // What the list looked like the last time round, so a trigger `ys` added, switched or
    // removed reaches the window without waiting for one to fire.
    let mut stamp = None;
    loop {
        match woken.recv_timeout(TICK) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        while woken.try_recv().is_ok() {}
        let (Some(state), Some(listener)) = (
            handle.try_state::<AppState>(),
            handle.try_state::<Listener>(),
        ) else {
            continue;
        };
        // Git says what a worktree's HEAD is and whether it is clean. Without git nothing a
        // trigger starts could run anyway.
        let git = match crate::git::Git::new(&state.env()) {
            Ok(git) => git,
            Err(error) => {
                eprintln!("triggers: {error}");
                continue;
            }
        };
        let store = &state.store;
        let now = clock::now().timestamp_millis();
        let mut changed = false;
        listener.deliver(state.host.as_ref());

        let mut seen = Vec::new();
        let heard = std::mem::take(
            &mut *listener
                .heard
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        for (change, at) in heard {
            let result = match change {
                Change::Settled { terminal } => {
                    sources::settled_terminal(store, &git, &terminal, at).map(|found| {
                        seen.extend(found);
                    })
                }
                Change::Busy { terminal } => sources::busy_terminal(store, &terminal).map(|_| ()),
            };
            if let Err(error) = result {
                eprintln!("triggers: cannot read a session: {}", error.message);
            }
        }
        seen.extend(std::mem::take(
            &mut *listener
                .forge
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        ));
        for read in [
            sources::read_events(store, &listener.turns, &git),
            sources::read_done(store),
            sources::read_custom(store),
            sources::read_endings(store, now),
        ] {
            match read {
                Ok(found) => seen.extend(found),
                Err(error) => eprintln!("triggers: cannot read what happened: {}", error.message),
            }
        }
        for observation in &seen {
            match fire::observe(store, observation) {
                Ok(gathered) => changed |= gathered > 0,
                Err(error) => eprintln!("triggers: cannot gather an event: {}", error.message),
            }
        }

        match fire::tick(store, &state.data_dir, &AppLook(&state), &git, now) {
            Ok(fired) => {
                changed |= !fired.is_empty();
                for notice in fired.iter().filter_map(|fired| fired.notice.as_ref()) {
                    tell(handle, notice);
                }
                if fired.iter().any(|fired| fired.fire.run_id.is_some()) {
                    // A run was queued: its driver should look now, and the window's run lists
                    // are out of date.
                    let _ = crate::workflows::commands::WorkflowRunsChanged {}.emit(handle);
                    crate::smoke::changed(handle);
                }
            }
            Err(error) => eprintln!("triggers: cannot decide fires: {}", error.message),
        }

        match fire::settle(store) {
            Ok(settled) => {
                changed |= settled.changed;
                for notice in &settled.notices {
                    tell(handle, notice);
                }
            }
            Err(error) => eprintln!("triggers: cannot read runs: {}", error.message),
        }

        if let Ok(now) = store.triggers_stamp() {
            changed |= stamp.is_some_and(|before| before != now);
            stamp = Some(now);
        }

        if changed {
            let _ = TriggersChanged {}.emit(handle);
        }
    }
}

/// Show a desktop notification about a trigger.
fn tell(handle: &AppHandle, notice: &fire::Notice) {
    let shown = handle
        .notification()
        .builder()
        .title(&notice.title)
        .body(&notice.body)
        .show();
    if let Err(error) = shown {
        eprintln!("triggers: could not show a notification: {error}");
    }
}
